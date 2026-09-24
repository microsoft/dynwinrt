// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::codegen::python::{self, generate_runtime_support_module};
use dynwinrt_codegen::meta::{self, ClassMeta, InterfaceMeta};
use dynwinrt_codegen::types::TypeMeta;
use windows_metadata::{
    FieldAttributes, MethodAttributes, MethodCallAttributes, MethodImplAttributes, ParamAttributes,
    Signature, Type, TypeAttributes, Value, writer,
};

const AS_INTERFACE: &str = "    def as_interface(self, interface_class):
        return _dynwinrt_as_interface(self._obj, interface_class)
";

fn interface(name: &str, iid: &str) -> InterfaceMeta {
    InterfaceMeta {
        name: name.into(),
        namespace: "Contoso".into(),
        iid: iid.into(),
        ..Default::default()
    }
}

fn assert_imports_helper(module: &str) {
    let (_, imports) = module
        .split_once("from ._runtime import (")
        .expect("runtime support import");
    let (imports, _) = imports.split_once(')').expect("closed import list");
    assert!(
        imports
            .split(',')
            .any(|name| name.trim() == "_dynwinrt_as_interface"),
        "{module}"
    );
}

#[test]
fn every_as_interface_delegates_to_the_runtime_support_helper() {
    let class = ClassMeta {
        name: "Widget".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Widget".into(),
        default_interface: Some(interface("IWidget", "11111111-1111-1111-1111-111111111111")),
        required_interfaces: vec![interface("IExtra", "22222222-2222-2222-2222-222222222222")],
        ..Default::default()
    };
    let known = HashSet::from(["Widget".to_string()]);
    let py = common::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    assert_imports_helper(&py);
    let (runtime_class, embedded_interface) = py
        .split_once("\nclass IExtra:")
        .expect("embedded runtime interface");
    assert!(
        runtime_class.contains("_dynwinrt_runtime_class_type = True")
            && runtime_class.contains(AS_INTERFACE),
        "{runtime_class}"
    );
    assert!(
        embedded_interface.contains(AS_INTERFACE),
        "{embedded_interface}"
    );

    let py = common::generate_interface(
        &interface("IWidget", "11111111-1111-1111-1111-111111111111"),
        &HashSet::from(["IWidget".to_string()]),
        &HashSet::new(),
    );
    assert_imports_helper(&py);
    assert!(py.contains(AS_INTERFACE), "{py}");
}

#[test]
fn as_interface_helper_rejects_runtime_classes_with_project_as_guidance() {
    let runtime = generate_runtime_support_module();
    let helper = runtime
        .split_once("def _dynwinrt_as_interface(native, interface_class):\n")
        .map(|(_, helper)| helper.split("\n\n\n").next().unwrap_or(helper))
        .expect("as_interface helper");
    for expected in [
        "from_value = getattr(interface_class, 'from_value', None)",
        "return from_value(native)",
        "_dynwinrt_runtime_class_type",
        "_dynwinrt_projectable_class_type",
        "raise TypeError(",
        "as_interface() requires a generated interface class, but {name} is a",
        "Use dynwinrt.project_as(obj, {name}) to cast to a runtime class.",
        "as_interface() requires a generated interface class, not {name}.",
    ] {
        assert!(helper.contains(expected), "missing {expected:?}:\n{helper}");
    }
}

/// A metadata struct whose declaration takes the helper's module name.
const COLLIDING: &str = "_dynwinrt_as_interface";
const ALIASED_IMPORT: &str = "_dynwinrt_as_interface as _dynwinrt_as_interface_2";
const ALIASED_AS_INTERFACE: &str = "    def as_interface(self, interface_class):
        return _dynwinrt_as_interface_2(self._obj, interface_class)
";
// Windows.Foundation.Uri implements both, so the generated module can cast a
// real object: IUriRuntimeClass and IUriRuntimeClassWithAbsoluteCanonicalUri.
const URI_RUNTIME_CLASS: (u32, u16, u16, [u8; 8]) = (
    0x9e365e57,
    0x48b2,
    0x4160,
    [0x95, 0x6f, 0xc7, 0x38, 0x51, 0x20, 0xbb, 0xfc],
);
const URI_ABSOLUTE_CANONICAL: (u32, u16, u16, [u8; 8]) = (
    0x758d9661,
    0x221c,
    0x480f,
    [0xa3, 0x39, 0x50, 0x65, 0x66, 0x73, 0xf4, 0x6f],
);

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "python-as-interface-{}-{}",
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

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let venv = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("bindings")
                .join("py")
                .join(".venv")
                .join("Scripts")
                .join("python.exe");
            if venv.is_file() {
                venv
            } else {
                PathBuf::from("python")
            }
        })
}

fn runtime_available() -> bool {
    let available = Command::new(python())
        .args([
            "-c",
            "import dynwinrt; assert hasattr(dynwinrt, 'RO_INIT_MULTITHREADED')",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "the as_interface collision probe requires the current Python binding"
    );
    if !available {
        eprintln!("Skipping the as_interface runtime probe; set DYNWINRT_TEST_PYTHON.");
    }
    available
}

fn guid(file: &mut writer::File, definition: writer::TypeDef, value: (u32, u16, u16, [u8; 8])) {
    let attribute = file.TypeRef("Windows.Foundation.Metadata", "GuidAttribute");
    let mut types = vec![Type::U32, Type::U16, Type::U16];
    types.extend(std::iter::repeat_n(Type::U8, 8));
    let constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types,
        },
        writer::MemberRefParent::TypeRef(attribute),
    );
    let (data1, data2, data3, data4) = value;
    let mut arguments = vec![Value::U32(data1), Value::U16(data2), Value::U16(data3)];
    arguments.extend(data4.map(Value::U8));
    file.Attribute(
        writer::HasAttribute::TypeDef(definition),
        writer::AttributeType::MemberRef(constructor),
        &arguments
            .into_iter()
            .map(|argument| (String::new(), argument))
            .collect::<Vec<_>>(),
    );
}

fn declare_interface(file: &mut writer::File, name: &str, value: (u32, u16, u16, [u8; 8])) {
    let definition = file.TypeDef(
        "Audit",
        name,
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public
            | TypeAttributes::Interface
            | TypeAttributes::Abstract
            | TypeAttributes::WindowsRuntime,
    );
    guid(file, definition, value);
}

fn declare_method(file: &mut writer::File, name: &str, result: Type, params: &[(&str, Type)]) {
    file.MethodDef(
        name,
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: result,
            types: params.iter().map(|(_, typ)| typ.clone()).collect(),
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    for (index, (name, _)) in params.iter().enumerate() {
        file.Param(name, index as u16 + 1, ParamAttributes::In);
    }
}

/// `Audit.Widget` casts between two real Uri interfaces, while its default
/// interface also uses a struct declared with the helper's name.
fn colliding_metadata(path: &Path) {
    let mut file = writer::File::new("PythonAsInterfaceCollision");
    let value_type = file.TypeRef("System", "ValueType");
    file.TypeDef(
        "Audit",
        COLLIDING,
        writer::TypeDefOrRef::TypeRef(value_type),
        TypeAttributes::Public
            | TypeAttributes::Sealed
            | TypeAttributes::SequentialLayout
            | TypeAttributes::WindowsRuntime,
    );
    file.Field("Value", &Type::I32, FieldAttributes::Public);
    declare_interface(&mut file, "IWidget", URI_RUNTIME_CLASS);
    declare_method(&mut file, "ReadAbsoluteUri", Type::String, &[]);
    let colliding = Type::named("Audit", COLLIDING);
    declare_method(
        &mut file,
        "Echo",
        colliding.clone(),
        &[("value", colliding)],
    );
    declare_interface(&mut file, "IExtra", URI_ABSOLUTE_CANONICAL);
    declare_method(&mut file, "ReadCanonicalUri", Type::String, &[]);

    let object = file.TypeRef("System", "Object");
    let class = file.TypeDef(
        "Audit",
        "Widget",
        writer::TypeDefOrRef::TypeRef(object),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    let default = file.InterfaceImpl(class, &Type::named("Audit", "IWidget"));
    file.InterfaceImpl(class, &Type::named("Audit", "IExtra"));
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
        writer::HasAttribute::InterfaceImpl(default),
        writer::AttributeType::MemberRef(constructor),
        &[],
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
}

/// Assert that a module which also declares or imports `COLLIDING` binds the
/// helper under an alias, and that every `as_interface()` calls that alias.
fn assert_aliased_as_interface(module: &str, expected_methods: usize) {
    let (_, imports) = module
        .split_once("from ._runtime import (")
        .expect("runtime support import");
    let (imports, _) = imports.split_once(')').expect("closed import list");
    let imports = imports.split(',').map(str::trim).collect::<Vec<_>>();
    assert!(imports.contains(&ALIASED_IMPORT), "{module}");
    assert!(!imports.contains(&COLLIDING), "{module}");
    assert_eq!(
        module.matches("def as_interface(").count(),
        expected_methods,
        "{module}"
    );
    assert_eq!(
        module.matches(ALIASED_AS_INTERFACE).count(),
        expected_methods,
        "{module}"
    );
    assert!(
        !module.contains("return _dynwinrt_as_interface(self._obj"),
        "{module}"
    );
}

fn probe(directory: &Path, module: &str) {
    fs::write(
        directory.join("as_interface_probe.py"),
        format!(
            r#"
import importlib
import dynwinrt as dw

support = importlib.import_module("pyviews._runtime")
module = importlib.import_module("pyviews.{module}")
Widget, IExtra = module.Widget, module.IExtra
assert module._dynwinrt_as_interface_2 is support._dynwinrt_as_interface
assert getattr(module, "{COLLIDING}", None) is not support._dynwinrt_as_interface

factory_iid = dw.WinGUID.parse("44a9796f-723e-4fdf-a218-033e75b0c084")
factory_type = dw.DynWinRTType.register_interface(
    "AsInterfaceCollisionFactory", factory_iid
).add_method(
    "CreateUri",
    dw.DynWinRTMethodSig().add_in(dw.DynWinRTType.hstring()).add_out(dw.DynWinRTType.object()),
)
uri = "https://example.com/as-interface"
with dw.RoApartment(), dw.projected_lifetime_scope():
    factory = dw.DynWinRTValue.activation_factory("Windows.Foundation.Uri").cast(factory_iid)
    widget = Widget._from_native(
        factory_type.method(6).invoke(factory, [dw.DynWinRTValue.from_hstring(uri)])
    )
    factory.release()
    assert widget.read_absolute_uri() == uri
    extra = widget.as_interface(IExtra)
    assert type(extra) is IExtra
    assert extra.read_canonical_uri() == uri
    assert extra.as_interface(IExtra) is extra
    try:
        widget.as_interface(Widget)
    except TypeError as error:
        assert "dynwinrt.project_as(obj, Widget)" in str(error), error
    else:
        raise AssertionError("as_interface() accepted a runtime class")
print("as-interface-collision-ok")
"#
        ),
    )
    .unwrap();
    let output = Command::new(python())
        .args(["-B", "as_interface_probe.py"])
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("as-interface-collision-ok"),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    success(output);
}

#[test]
fn colliding_metadata_declarations_alias_the_as_interface_helper() {
    let fixture = Fixture::new();
    let winmd = fixture.0.join("metadata").join("Collision.winmd");
    colliding_metadata(&winmd);
    let run_runtime = runtime_available();

    // Standalone modules declare the struct inline, next to the class and its
    // embedded IExtra view.
    let class = meta::parse_class(winmd.to_str().unwrap(), "Audit", "Widget").unwrap();
    assert_eq!(class.required_interfaces.len(), 1);
    let structs = python::package_structs(std::slice::from_ref(&class), &[]);
    let identities = [TypeMeta::RuntimeClass {
        namespace: "Audit".into(),
        name: "Widget".into(),
        default_interface: None,
    }
    .type_identity()]
    .into_iter()
    .chain(class.all_interfaces().map(InterfaceMeta::type_identity))
    .chain(structs.iter().map(TypeMeta::type_identity));
    let context = python::PythonProjectionContext::new(identities, false).unwrap();
    let source = python::generate_class(&context, &class, &Default::default());
    assert!(
        source.contains(&format!("\nclass {COLLIDING}:")),
        "{source}"
    );
    assert_aliased_as_interface(&source, 2);
    let standalone = fixture.0.join("standalone");
    let package = standalone.join("pyviews");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("__init__.py"), "").unwrap();
    fs::write(
        package.join("_runtime.py"),
        generate_runtime_support_module(),
    )
    .unwrap();
    fs::write(package.join("audit__widget.py"), &source).unwrap();
    if run_runtime {
        probe(&standalone, "audit__widget");
    }

    // Packaged output resolves the struct from its own module.
    let packaged = fixture.0.join("packaged");
    success(
        Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args(["generate", "--winmd"])
            .arg(&winmd)
            .arg("--output")
            .arg(packaged.join("pyviews"))
            .args(["--lang", "py", "--class-name", "Audit.Widget"])
            .output()
            .unwrap(),
    );
    let source = fs::read_to_string(packaged.join("pyviews").join("audit__widget.py")).unwrap();
    assert_aliased_as_interface(&source, 2);
    if run_runtime {
        probe(&packaged, "audit__widget");
    }
}
