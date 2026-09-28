// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::codegen::python::{self, PythonProjectionContext};
use dynwinrt_codegen::meta::{self, ClassMeta, InterfaceMeta};
use dynwinrt_codegen::types::{TypeIdentity, TypeIdentityKind};
use windows_metadata::{MethodCallAttributes, Signature, Type, TypeAttributes, TypeName, writer};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
const COLLIDING: &str = "_dynwinrt_collection_item";
const HELPER_ALIAS: &str = "_dynwinrt_collection_item_2";
const COLLECTIONS: &str = "Windows.Foundation.Collections";

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "python-collection-helper-collision-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
        fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python"))
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn runtime_available() -> bool {
    let available = Command::new(python())
        .args([
            "-c",
            "from dynwinrt import DynWinRTValue; assert hasattr(DynWinRTValue, 'create_vector')",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "the collection-helper collision probe requires the current Python binding"
    );
    if !available {
        eprintln!("Skipping the collection-helper runtime probe; set DYNWINRT_TEST_PYTHON.");
    }
    available
}

fn closed(namespace: &str, name: &str, generics: Vec<Type>) -> Type {
    Type::Name(TypeName {
        namespace: namespace.into(),
        name: name.into(),
        generics,
    })
}

fn default_attribute(file: &mut writer::File, implementation: writer::InterfaceImpl) {
    let attribute = file.TypeRef("Windows.Foundation.Metadata", "DefaultAttribute");
    let constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![],
        },
        writer::MemberRefParent::TypeRef(attribute),
    );
    file.Attribute(
        writer::HasAttribute::InterfaceImpl(implementation),
        writer::AttributeType::MemberRef(constructor),
        &[],
    );
}

fn collection_class(file: &mut writer::File, namespace: &str, interface: Type) {
    let object = file.TypeRef("System", "Object");
    let class = file.TypeDef(
        namespace,
        COLLIDING,
        writer::TypeDefOrRef::TypeRef(object),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    let implementation = file.InterfaceImpl(class, &interface);
    default_attribute(file, implementation);
}

fn collision_metadata(path: &Path) {
    let mut file = writer::File::new("PythonCollectionHelperCollision");
    collection_class(
        &mut file,
        "Audit.Vector",
        closed(COLLECTIONS, "IVector`1", vec![Type::Object]),
    );
    collection_class(
        &mut file,
        "Audit.Map",
        closed(COLLECTIONS, "IMap`2", vec![Type::Object, Type::Object]),
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
}

fn metadata_set(custom: &Path) -> String {
    format!("{};{WINDOWS_WINMD}", custom.display())
}

fn parse_classes(custom: &Path) -> Vec<ClassMeta> {
    ["Audit.Vector", "Audit.Map"]
        .into_iter()
        .map(|namespace| {
            meta::parse_class(&metadata_set(custom), namespace, COLLIDING)
                .unwrap_or_else(|| panic!("parse {namespace}.{COLLIDING}"))
        })
        .collect()
}

fn class_identity(class: &ClassMeta) -> TypeIdentity {
    TypeIdentity::named(TypeIdentityKind::Class, &class.namespace, &class.name)
}

fn projection_context(classes: &[ClassMeta], packaged: bool) -> PythonProjectionContext {
    let identities = classes.iter().map(class_identity).chain(
        classes
            .iter()
            .flat_map(ClassMeta::all_interfaces)
            .map(InterfaceMeta::type_identity),
    );
    PythonProjectionContext::new(identities, packaged).unwrap()
}

fn module_name(context: &PythonProjectionContext, namespace: &str) -> String {
    context.implementation_module_for_named(TypeIdentityKind::Class, namespace, COLLIDING)
}

fn assert_collision_safe(source: &str, operation: &str) {
    assert!(
        source.contains(
            "from ._runtime import _dynwinrt_collection_item as _dynwinrt_collection_item_2"
        ),
        "{source}"
    );
    assert!(source.contains(&format!("class {COLLIDING}(")), "{source}");
    assert!(
        source.contains(&format!("{HELPER_ALIAS}(")),
        "missing aliased helper in {operation}:\n{source}"
    );
    assert!(
        !source.contains("from ._runtime import _dynwinrt_collection_item\n"),
        "{source}"
    );
}

struct Modules {
    vector_class: String,
    map_class: String,
}

fn modules(context: &PythonProjectionContext) -> Modules {
    Modules {
        vector_class: module_name(context, "Audit.Vector"),
        map_class: module_name(context, "Audit.Map"),
    }
}

fn write_standalone(root: &Path, classes: &[ClassMeta]) -> Modules {
    let package = root.join("pyviews");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("__init__.py"), "").unwrap();
    fs::write(
        package.join("_runtime.py"),
        python::generate_runtime_support_module(),
    )
    .unwrap();

    let context = projection_context(classes, false);
    for class in classes {
        let module = context.implementation_module(&class_identity(class));
        let source = python::generate_class(&context, class, &Default::default());
        assert_collision_safe(&source, &module);
        fs::write(package.join(format!("{module}.py")), source).unwrap();
    }
    modules(&context)
}

fn generate_packaged(root: &Path, custom: &Path, classes: &[ClassMeta]) -> Modules {
    let package = root.join("pyviews");
    success(
        Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args(["generate", "--winmd"])
            .arg(custom)
            .args(["--ref", WINDOWS_WINMD, "--output"])
            .arg(&package)
            .args([
                "--lang",
                "py",
                "--class-name",
                "Audit.Vector._dynwinrt_collection_item,Audit.Map._dynwinrt_collection_item",
            ])
            .output()
            .unwrap(),
    );
    let context = projection_context(classes, true);
    let modules = modules(&context);
    for (module, operation) in [
        (&modules.vector_class, "vector operations"),
        (&modules.map_class, "map operations"),
    ] {
        let source = fs::read_to_string(package.join(format!("{module}.py"))).unwrap();
        assert_collision_safe(&source, operation);
    }
    modules
}

fn runtime_probe(root: &Path, modules: &Modules) {
    fs::write(
        root.join("probe.py"),
        format!(
            r#"
import importlib
import os
import dynwinrt as dw

support = importlib.import_module("pyviews._runtime")
vector_module = importlib.import_module("pyviews.{vector_class}")
map_module = importlib.import_module("pyviews.{map_class}")
Vector = vector_module.{COLLIDING}
Map = map_module.{COLLIDING}

if "DYNWINRT_EXPECT_OLD_COLLECTION_HELPER" not in os.environ:
    for module in (vector_module, map_module):
        assert module.{HELPER_ALIAS} is support._dynwinrt_collection_item
        assert module.{COLLIDING} is not support._dynwinrt_collection_item

with dw.RoApartment(1), dw.projected_lifetime_scope():
    live = dw.DynWinRTValue.activation_factory("Windows.Foundation.Uri")

    vector = Vector._from_native(
        dw.DynWinRTValue.create_vector([], dw.DynWinRTType.object())
    )
    vector.append(None)
    vector.append(live)
    assert vector[0] is None
    item = vector[1]
    assert item is not None and item.identity_raw() == live.identity_raw()
    item.release()
    vector.replace_all([live, None])
    assert vector[1] is None

    mapping = Map._from_native(
        dw.DynWinRTValue.create_map(
            [], [], dw.DynWinRTType.object(), dw.DynWinRTType.object()
        )
    )
    assert mapping.insert(None, None) is False
    assert mapping.lookup(None) is None
    assert mapping.insert(live, live) is False
    value = mapping.lookup(live)
    assert value is not None and value.identity_raw() == live.identity_raw()
    value.release()

    live.release()

print("collection-helper-collision-ok")
"#,
            vector_class = modules.vector_class,
            map_class = modules.map_class,
        ),
    )
    .unwrap();
    let output = Command::new(python())
        .args(["-B", "probe.py"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("collection-helper-collision-ok"),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    success(output);
}

fn old_hardcoded_binding_fails(root: &Path, modules: &Modules) {
    for module in [&modules.vector_class, &modules.map_class] {
        let path = root.join("pyviews").join(format!("{module}.py"));
        let source = fs::read_to_string(&path).unwrap();
        let old = source
            .replace(
                "from ._runtime import _dynwinrt_collection_item as _dynwinrt_collection_item_2",
                "from ._runtime import _dynwinrt_collection_item",
            )
            .replace("_dynwinrt_collection_item_2(", "_dynwinrt_collection_item(");
        fs::write(path, old).unwrap();
    }
    let output = Command::new(python())
        .args(["-B", "probe.py"])
        .current_dir(root)
        .env("DYNWINRT_EXPECT_OLD_COLLECTION_HELPER", "1")
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "old hardcoded helper unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("TypeError"),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn strict_typecheck(root: &Path, modules: &Modules) {
    let available = Command::new(python())
        .args(["-m", "mypy", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref() != Ok("1"),
        "DYNWINRT_REQUIRE_MYPY=1 but mypy is unavailable"
    );
    if !available {
        eprintln!("Skipping collection-helper strict typing: mypy unavailable.");
        return;
    }
    let consumer = format!(
        r#"
from dynwinrt import DynWinRTValue
from pyviews.{vector_class} import {COLLIDING} as Vector
from pyviews.{map_class} import {COLLIDING} as Map

def vectors(vector: Vector, value: DynWinRTValue) -> None:
    vector.append(None)
    vector.append(value)
    vector.replace_all([None, value])

def maps(mapping: Map, value: DynWinRTValue) -> None:
    mapping.insert(None, None)
    mapping.insert(value, value)
"#,
        vector_class = modules.vector_class,
        map_class = modules.map_class,
    );
    fs::write(root.join("typing_probe.py"), &consumer).unwrap();
    let binding_stubs = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("bindings")
        .join("py")
        .canonicalize()
        .unwrap();
    success(
        Command::new(python())
            .args([
                "-B",
                "-m",
                "mypy",
                "--strict",
                "--follow-imports=silent",
                "--no-incremental",
                "--cache-dir",
                "mypy-cache",
                "typing_probe.py",
            ])
            .current_dir(root)
            .env(
                "MYPYPATH",
                std::env::join_paths([binding_stubs, root.to_path_buf()]).unwrap(),
            )
            .output()
            .unwrap(),
    );
    if let Some(pyright) = std::env::var_os("DYNWINRT_PYRIGHT") {
        fs::write(
            root.join("pyright_probe.py"),
            format!("# pyright: strict, reportPrivateUsage=false\n{consumer}"),
        )
        .unwrap();
        success(
            Command::new(pyright)
                .args(["--pythonpath"])
                .arg(python())
                .arg("pyright_probe.py")
                .current_dir(root)
                .output()
                .unwrap(),
        );
    }
}

#[test]
fn collection_helper_aliases_execute_for_packaged_and_standalone_outputs() {
    if !Path::new(WINDOWS_WINMD).is_file() {
        eprintln!("Skipping collection-helper collision test: Windows.winmd unavailable.");
        return;
    }
    let fixture = Fixture::new();
    let custom = fixture.0.join("metadata").join("Collision.winmd");
    collision_metadata(&custom);
    let classes = parse_classes(&custom);
    assert_eq!(classes.len(), 2);

    let standalone = fixture.0.join("standalone");
    let modules = write_standalone(&standalone, &classes);
    if runtime_available() {
        runtime_probe(&standalone, &modules);
        old_hardcoded_binding_fails(&standalone, &modules);
    }

    let packaged = fixture.0.join("packaged");
    let modules = generate_packaged(&packaged, &custom, &classes);
    strict_typecheck(&packaged, &modules);
    if runtime_available() {
        runtime_probe(&packaged, &modules);
    }
}
