// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use windows_metadata::{
    MethodAttributes, MethodCallAttributes, MethodImplAttributes, ParamAttributes, Signature, Type,
    TypeAttributes, TypeName, Value, writer,
};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
const NAMESPACE: &str = "Audit.HelperMembers";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join(format!(
                "pmc{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| "python".into())
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_source_diagnostics(output: Output, no_pyi: bool, invalid_consumer: bool) {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let fails = !no_pyi || invalid_consumer;
    assert_eq!(output.status.success(), !fails, "{text}");
    let errors = text
        .lines()
        .filter(|line| line.contains(": error:"))
        .collect::<Vec<_>>();
    let consumer = errors
        .iter()
        .filter(|line| line.starts_with("invalid.py:"))
        .collect::<Vec<_>>();
    assert_eq!(
        consumer.len(),
        if invalid_consumer { 5 } else { 0 },
        "{text}"
    );
    for (line, code) in consumer.iter().zip([
        "[arg-type]",
        "[arg-type]",
        "[arg-type]",
        "[arg-type]",
        "[operator]",
    ]) {
        assert!(line.ends_with(code), "{text}");
    }
    let errors = errors
        .into_iter()
        .filter(|line| !line.starts_with("invalid.py:"))
        .collect::<Vec<_>>();
    if no_pyi {
        assert!(errors.is_empty(), "{text}");
        return;
    }
    let expected = [
        (
            "audit__helper_members__native_vector.pyi",
            "Return type \"int\" of \"extend\"",
        ),
        (
            "audit__helper_members__native_vector.pyi",
            "Argument 1 of \"extend\"",
        ),
        (
            "audit__helper_members__native_property_map.pyi",
            "Signature of \"update\"",
        ),
        (
            "audit__helper_members__native_map.pyi",
            "Signature of \"update\"",
        ),
        (
            "audit__helper_members__native_map.pyi",
            "Signature of \"setdefault\"",
        ),
    ];
    assert_eq!(errors.len(), expected.len(), "{text}");
    for (file, message) in expected {
        assert_eq!(
            errors
                .iter()
                .filter(|line| {
                    line.contains(file) && line.contains(message) && line.ends_with("[override]")
                })
                .count(),
            1,
            "{text}"
        );
    }
}

fn attribute(
    file: &mut writer::File,
    owner: writer::HasAttribute,
    name: &str,
    types: Vec<Type>,
    values: Vec<Value>,
) {
    let attribute = file.TypeRef("Windows.Foundation.Metadata", name);
    let constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types,
        },
        writer::MemberRefParent::TypeRef(attribute),
    );
    file.Attribute(
        owner,
        writer::AttributeType::MemberRef(constructor),
        &values
            .into_iter()
            .map(|value| (String::new(), value))
            .collect::<Vec<_>>(),
    );
}

fn native_interface(
    file: &mut writer::File,
    name: &str,
    id: u32,
    methods: &[(&str, Option<&str>, bool)],
) {
    let definition = file.TypeDef(
        NAMESPACE,
        name,
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public
            | TypeAttributes::Interface
            | TypeAttributes::Abstract
            | TypeAttributes::WindowsRuntime,
    );
    attribute(
        file,
        writer::HasAttribute::TypeDef(definition),
        "GuidAttribute",
        vec![
            Type::U32,
            Type::U16,
            Type::U16,
            Type::U8,
            Type::U8,
            Type::U8,
            Type::U8,
            Type::U8,
            Type::U8,
            Type::U8,
            Type::U8,
        ],
        vec![
            Value::U32(id),
            Value::U16(0x7342),
            Value::U16(0x4830),
            Value::U8(0x87),
            Value::U8(0xa1),
            Value::U8(0x11),
            Value::U8(0x12),
            Value::U8(0x13),
            Value::U8(0x14),
            Value::U8(0x15),
            Value::U8(0x16),
        ],
    );
    for &(name, alias, property) in methods {
        let mut attributes = MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot;
        if property {
            attributes |= MethodAttributes::SpecialName;
        }
        let method = file.MethodDef(
            name,
            &Signature {
                flags: MethodCallAttributes::HASTHIS,
                return_type: Type::I32,
                types: if property { vec![] } else { vec![Type::I32] },
            },
            attributes,
            MethodImplAttributes::default(),
        );
        if !property {
            file.Param("value", 1, ParamAttributes::In);
        }
        if let Some(alias) = alias {
            attribute(
                file,
                writer::HasAttribute::MethodDef(method),
                "OverloadAttribute",
                vec![Type::String],
                vec![Value::Utf8(alias.into())],
            );
        }
    }
}

fn collection_class(file: &mut writer::File, name: &str, native: &str, vector: bool) {
    let object = file.TypeRef("System", "Object");
    let class = file.TypeDef(
        NAMESPACE,
        name,
        writer::TypeDefOrRef::TypeRef(object),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    let default = file.InterfaceImpl(class, &Type::named(NAMESPACE, native));
    attribute(
        file,
        writer::HasAttribute::InterfaceImpl(default),
        "DefaultAttribute",
        vec![],
        vec![],
    );
    file.InterfaceImpl(
        class,
        &Type::Name(TypeName {
            namespace: "Windows.Foundation.Collections".into(),
            name: if vector { "IVector`1" } else { "IMap`2" }.into(),
            generics: if vector {
                vec![Type::Object]
            } else {
                vec![Type::String, Type::Object]
            },
        }),
    );
}

fn metadata(path: &Path) {
    let mut file = writer::File::new("CollectionMemberCollisions");
    native_interface(
        &mut file,
        "IMapNative",
        0x4a219101,
        &[
            ("Update", None, false),
            ("Transform", Some("Setdefault"), false),
        ],
    );
    native_interface(
        &mut file,
        "IVectorNative",
        0x4a219102,
        &[
            ("Extend", None, false),
            ("Transform", Some("__iadd__"), false),
        ],
    );
    native_interface(
        &mut file,
        "IPropertyNative",
        0x4a219103,
        &[("get_Update", None, true)],
    );
    native_interface(&mut file, "IViews", 0x4a219104, &[]);
    for (name, collection, arguments) in [
        ("GetMapping", "IMap`2", vec![Type::String, Type::Object]),
        ("GetVector", "IVector`1", vec![Type::Object]),
    ] {
        file.MethodDef(
            name,
            &Signature {
                flags: MethodCallAttributes::HASTHIS,
                return_type: Type::Name(TypeName {
                    namespace: "Windows.Foundation.Collections".into(),
                    name: collection.into(),
                    generics: arguments,
                }),
                types: vec![],
            },
            MethodAttributes::Public
                | MethodAttributes::Abstract
                | MethodAttributes::Virtual
                | MethodAttributes::NewSlot,
            MethodImplAttributes::default(),
        );
    }
    collection_class(&mut file, "NativeMap", "IMapNative", false);
    collection_class(&mut file, "NativeVector", "IVectorNative", true);
    collection_class(&mut file, "NativePropertyMap", "IPropertyNative", false);
    fs::write(path, file.into_stream()).unwrap();
}

fn generate(fixture: &Fixture, metadata: &Path, no_pyi: bool) -> PathBuf {
    let package = fixture.0.join("views");
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
    command
        .args(["generate", "--winmd"])
        .arg(metadata)
        .args([
            "--ref",
            WINDOWS_WINMD,
            "--namespace",
            NAMESPACE,
            "--lang",
            "py",
            "--output",
        ])
        .arg(&package);
    if no_pyi {
        command.arg("--no-pyi");
    }
    success(command.output().unwrap());
    package
}

const CONSUMER: &str = r#"from typing import assert_type
from dynwinrt import DynWinRTValue
from views.audit__helper_members__native_map import NativeMap, NativeMapLike, IMap_String_Object as EmbeddedMap
from views.audit__helper_members__native_vector import NativeVector, NativeVectorLike
from views.audit__helper_members__native_property_map import NativePropertyMap, NativePropertyMapLike
from views.windows__foundation__collections__i_map_string_object import IMap_String_Object

def native(mapping: NativeMap, vector: NativeVector, property_map: NativePropertyMap,
           mapping_like: NativeMapLike, vector_like: NativeVectorLike,
           property_like: NativePropertyMapLike) -> None:
    assert_type(mapping.update(23), int)
    assert_type(mapping.setdefault(23), int)
    assert_type(mapping.transform(23), int)
    assert_type(vector.extend(23), int)
    assert_type(vector.iadd__(23), int)
    assert_type(vector.transform(23), int)
    assert_type(property_map.update, int)
    assert_type(mapping_like.update(23), int)
    assert_type(mapping_like.setdefault(23), int)
    assert_type(vector_like.extend(23), int)
    assert_type(vector_like.iadd__(23), int)
    assert_type(property_like.update, int)

def helpers(mapping: IMap_String_Object, embedded: EmbeddedMap, raw: DynWinRTValue) -> None:
    assert_type(mapping.update({"entry": raw}), None)
    assert_type(embedded.update(entry=raw), None)
    assert_type(mapping.setdefault("entry", raw), DynWinRTValue | None)
    assert_type(embedded.setdefault("entry", raw), DynWinRTValue | None)
"#;

const INLINE_CONSUMER: &str = r#"from typing import assert_type
from dynwinrt import DynWinRTValue
from views.audit__helper_members__native_map import NativeMap, IMap_String_Object as EmbeddedMap
from views.audit__helper_members__native_vector import NativeVector
from views.audit__helper_members__native_property_map import NativePropertyMap
from views.windows__foundation__collections__i_map_string_object import IMap_String_Object

def native(mapping: NativeMap, vector: NativeVector, property_map: NativePropertyMap) -> None:
    assert_type(mapping.update(23), int)
    assert_type(mapping.setdefault(23), int)
    assert_type(mapping.transform(23), int)
    assert_type(vector.extend(23), int)
    assert_type(vector.iadd__(23), int)
    assert_type(vector.transform(23), int)
    assert_type(property_map.update, int)

def helpers(mapping: IMap_String_Object, embedded: EmbeddedMap, raw: DynWinRTValue) -> None:
    assert_type(mapping.update({"entry": raw}), None)
    assert_type(embedded.update(entry=raw), None)
    assert_type(mapping.setdefault("entry", raw), DynWinRTValue | None)
    assert_type(embedded.setdefault("entry", raw), DynWinRTValue | None)
"#;

const INVALID_CONSUMER: &str = r#"from dynwinrt import DynWinRTValue
from views.audit__helper_members__native_map import NativeMap
from views.audit__helper_members__native_vector import NativeVector
from views.audit__helper_members__native_property_map import NativePropertyMap

def invalid(mapping: NativeMap, vector: NativeVector,
            property_map: NativePropertyMap, raw: DynWinRTValue) -> None:
    mapping.update({"entry": raw})
    mapping.setdefault(raw)
    vector.extend([raw])
    vector.iadd__([raw])
    property_map.update(raw)
"#;

const RUNTIME: &str = r#"import ast
from pathlib import Path
from dynwinrt import (
    DynWinRTImplementation, DynWinRTImplementationMethod, DynWinRTInterfacePlan,
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, RoApartment, WinGUID,
    projected_lifetime_scope,
)
from views.audit.helper_members import NativeMap, NativeVector, NativePropertyMap
from views.windows.foundation.collections import IMap_String_Object, IVector_Object
from views.audit__helper_members__native_map import IMap_String_Object as EmbeddedMap

T = DynWinRTType
def plan(name, iid, methods):
    interface = T.register_interface(name, iid)
    definitions = []
    for slot, method in enumerate(methods, 6):
        member = method[0]
        if len(method) == 2:
            signature = method[1]
        else:
            signature = DynWinRTMethodSig()
            for typ in method[1]:
                signature = signature.add_in(typ)
            if method[2] is not None:
                signature = signature.add_out(method[2])
        interface = interface.add_method(member, signature)
        definitions.append(DynWinRTImplementationMethod(member, slot, signature))
    return DynWinRTInterfacePlan.create(name, interface, definitions)

map_view = T.parameterized(WinGUID.parse("e480ce40-a338-4ada-adcf-272272e48cb9"), [T.hstring(), T.object()])
map_iid = T.parameterized(WinGUID.parse("3c2925fe-8519-45c1-aa79-197b6718c1c1"), [T.hstring(), T.object()]).iid()
map_plan = plan("Audit.HelperMembers.IMap", map_iid, [
    ("Lookup", [T.hstring()], T.object()), ("get_Size", [], T.u32_type()),
    ("HasKey", [T.hstring()], T.bool_type()), ("GetView", [], map_view),
    ("Insert", [T.hstring(), T.object()], T.bool_type()), ("Remove", [T.hstring()], None),
    ("Clear", [], None),
])
vector_view = T.parameterized(WinGUID.parse("bbe1fa4c-b0e3-4583-baef-1f1b2e483e56"), [T.object()])
vector_iid = T.parameterized(WinGUID.parse("913337e9-11a1-4345-a3a2-4e7f956e222d"), [T.object()]).iid()
vector_plan = plan("Audit.HelperMembers.IVector", vector_iid, [
    ("GetAt", [T.u32_type()], T.object()), ("get_Size", [], T.u32_type()),
    ("GetView", [], vector_view),
    ("IndexOf", DynWinRTMethodSig().add_in(T.object()).add_out(T.u32_type()).add_out(T.bool_type())),
    ("SetAt", [T.u32_type(), T.object()], None),
    ("InsertAt", [T.u32_type(), T.object()], None), ("RemoveAt", [T.u32_type()], None),
    ("Append", [T.object()], None), ("RemoveAtEnd", [], None), ("Clear", [], None),
    ("GetMany", DynWinRTMethodSig().add_in(T.u32_type()).add_out_fill(T.array_type(T.object())).add_out(T.u32_type())),
    ("ReplaceAll", [T.array_type(T.object())], None),
])
calls = []
store = {}
with RoApartment(1), projected_lifetime_scope():
    def map_dispatch(interface, slot, args):
        if interface == 0:
            calls.append((slot, args[0].to_int() if args else None))
            return [DynWinRTValue.from_i32((slot - 5) * 100 + (args[0].to_int() if args else 1))]
        key = args[0].to_string() if args else None
        if slot == 6:
            return [store[key]]
        if slot == 7:
            return [DynWinRTValue.from_u32(len(store))]
        if slot == 8:
            return [DynWinRTValue.from_bool(key in store)]
        if slot == 10:
            replaced = key in store
            store[key] = args[1].cast(WinGUID.parse("00000000-0000-0000-c000-000000000046"))
            return [DynWinRTValue.from_bool(replaced)]
        if slot == 11:
            del store[key]
            return []
        if slot == 12:
            store.clear()
            return []
        raise AssertionError(f"unexpected map slot {slot}")

    native_map = plan("Audit.HelperMembers.IMapNative",
        WinGUID.parse("4a219101-7342-4830-87a1-111213141516"),
        [("Update", [T.i32_type()], T.i32_type()), ("Transform", [T.i32_type()], T.i32_type())])
    owner = DynWinRTImplementation.create([native_map, map_plan], map_dispatch)
    source = owner.to_value()
    mapping = NativeMap._from_native(source)
    assert mapping.update(23) == 123 and calls == [(6, 23)]
    assert mapping.setdefault(23) == 223
    assert mapping.transform(24) == 224
    assert calls == [(6, 23), (7, 23), (7, 24)]
    assert not source.is_released()
    ordinary = IVector_Object.create([])
    assert ordinary.extend([source]) is None
    assert ordinary.__iadd__([source]) is ordinary
    result = ordinary[0]
    assert result.identity_raw() == source.identity_raw()
    result.release()
    for view_type in (IMap_String_Object, EmbeddedMap):
        view = view_type.from_value(source)
        assert view.update({"entry": source}) is None
        result = view.setdefault("default", source)
        assert isinstance(result, DynWinRTValue)
        assert result.identity_raw() == source.identity_raw()
        result.release()

    native_vector = plan("Audit.HelperMembers.IVectorNative",
        WinGUID.parse("4a219102-7342-4830-87a1-111213141516"),
        [("Extend", [T.i32_type()], T.i32_type()), ("Transform", [T.i32_type()], T.i32_type())])
    def vector_dispatch(interface, slot, args):
        assert interface == 0
        calls.append((slot, args[0].to_int()))
        return [DynWinRTValue.from_i32((slot - 5) * 100 + args[0].to_int())]
    vector_owner = DynWinRTImplementation.create([native_vector, vector_plan], vector_dispatch)
    vector_source = vector_owner.to_value()
    vector = NativeVector._from_native(vector_source)
    assert vector.extend(23) == 123
    assert vector.iadd__(23) == 223
    assert vector.transform(24) == 224
    assert calls[-3:] == [(6, 23), (7, 23), (7, 24)]
    assert vector.__iadd__(25) is vector
    assert calls[-1] == (6, 25)

    property_plan = plan("Audit.HelperMembers.IPropertyNative",
        WinGUID.parse("4a219103-7342-4830-87a1-111213141516"),
        [("get_Update", [], T.i32_type())])
    property_owner = DynWinRTImplementation.create([property_plan, map_plan], map_dispatch)
    property_source = property_owner.to_value()
    property_map = NativePropertyMap._from_native(property_source)
    assert property_map.update == 101 and calls[-1] == (6, None)

    for cls, names in (
        (NativeMap, ("update", "setdefault")),
        (NativeVector, ("extend", "iadd__", "__iadd__")),
        (NativePropertyMap, ("update",)),
    ):
        path = Path(cls.__init__.__code__.co_filename)
        node = next(node for node in ast.parse(path.read_text(encoding="utf-8")).body
                    if isinstance(node, ast.ClassDef) and node.name == cls.__name__)
        for name in names:
            definitions = [member for member in node.body if isinstance(member, ast.FunctionDef) and member.name == name]
            assert len(definitions) <= 1, (cls, name, len(definitions))
    store.clear()
assert source.is_released() and vector_source.is_released() and property_source.is_released()
print("native-member-collisions-ok", flush=True)
"#;

#[test]
fn native_collection_members_keep_their_names_dispatch_and_aliases_in_both_output_modes() {
    if !Path::new(WINDOWS_WINMD).is_file() {
        eprintln!("Skipping collection member SDK test: Windows.winmd unavailable.");
        return;
    }
    for no_pyi in [false, true] {
        let fixture = Fixture::new();
        let metadata_path = fixture.0.join("Members.winmd");
        metadata(&metadata_path);
        generate(&fixture, &metadata_path, no_pyi);
        let runtime = Command::new(python())
            .args(["-c", "from dynwinrt import DynWinRTImplementation"])
            .output()
            .is_ok_and(|output| output.status.success());
        assert!(
            runtime
                || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1")
        );
        if runtime {
            fs::write(fixture.0.join("runtime.py"), RUNTIME).unwrap();
            success(
                Command::new(python())
                    .args(["-B", "runtime.py"])
                    .current_dir(&fixture.0)
                    .output()
                    .unwrap(),
            );
        }
        let mypy = Command::new(python())
            .args(["-m", "mypy", "--version"])
            .output()
            .is_ok_and(|output| output.status.success());
        assert!(mypy || std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref() != Ok("1"));
        if mypy {
            fs::write(
                fixture.0.join("consumer.py"),
                if no_pyi { INLINE_CONSUMER } else { CONSUMER },
            )
            .unwrap();
            let mut command = Command::new(python());
            command
                .args([
                    "-B",
                    "-m",
                    "mypy",
                    "--strict",
                    "--follow-imports=silent",
                    "--no-incremental",
                    "consumer.py",
                ])
                .current_dir(&fixture.0);
            if std::env::var("DYNWINRT_TEST_INSTALLED_RUNTIME").as_deref() == Ok("1") {
                command.env_remove("MYPYPATH");
            } else {
                command.env(
                    "MYPYPATH",
                    Path::new(env!("CARGO_MANIFEST_DIR")).join(r"..\..\bindings\py"),
                );
            }
            // Main already reports these exact nominal ABC overrides.
            assert_source_diagnostics(command.output().unwrap(), no_pyi, false);
            fs::write(fixture.0.join("invalid.py"), INVALID_CONSUMER).unwrap();
            command.arg("invalid.py");
            assert_source_diagnostics(command.output().unwrap(), no_pyi, true);
        }
    }
}
