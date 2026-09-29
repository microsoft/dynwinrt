// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::codegen::python;
use dynwinrt_codegen::meta::{self, InterfaceMeta};
use dynwinrt_codegen::types::{TypeIdentity, TypeIdentityKind, TypeMeta};
use windows_metadata::{
    FieldAttributes, MethodAttributes, MethodCallAttributes, MethodImplAttributes, ParamAttributes,
    Signature, Type, TypeAttributes, Value, writer,
};

const NAMESPACE: &str = "Tests.OverloadCompatibility";
const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "python-overload-legacy-{}-{}",
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

fn root() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap();
    let text = path.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let venv = root()
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

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn guid(file: &mut writer::File, owner: writer::TypeDef, value: u32) {
    let attribute = file.TypeRef("Windows.Foundation.Metadata", "GuidAttribute");
    let constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![
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
        },
        writer::MemberRefParent::TypeRef(attribute),
    );
    let values = [
        Value::U32(value),
        Value::U16(0x6281),
        Value::U16(0x4900),
        Value::U8(0xb7),
        Value::U8(0x82),
        Value::U8(4),
        Value::U8(3),
        Value::U8(2),
        Value::U8(1),
        Value::U8(9),
        Value::U8(0x10),
    ];
    file.Attribute(
        writer::HasAttribute::TypeDef(owner),
        writer::AttributeType::MemberRef(constructor),
        &values
            .into_iter()
            .map(|value| (String::new(), value))
            .collect::<Vec<_>>(),
    );
}

fn overload(file: &mut writer::File, method: writer::MethodDef, name: &str) {
    let attribute = file.TypeRef("Windows.Foundation.Metadata", "OverloadAttribute");
    let constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![Type::String],
        },
        writer::MemberRefParent::TypeRef(attribute),
    );
    file.Attribute(
        writer::HasAttribute::MethodDef(method),
        writer::AttributeType::MemberRef(constructor),
        &[(String::new(), Value::Utf8(name.into()))],
    );
}

fn interface(
    file: &mut writer::File,
    name: &str,
    id: u32,
    raw_name: &str,
    abi_name: &str,
    parameters: &[(&str, Type)],
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
    guid(file, definition, id);
    let method = file.MethodDef(
        raw_name,
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::I32,
            types: parameters.iter().map(|(_, typ)| typ.clone()).collect(),
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    for (index, (name, _)) in parameters.iter().enumerate() {
        file.Param(name, index as u16 + 1, ParamAttributes::In);
    }
    if abi_name != raw_name {
        overload(file, method, abi_name);
    }
}

fn runtime_class(file: &mut writer::File, name: &str, interfaces: &[&str]) {
    let object = file.TypeRef("System", "Object");
    let definition = file.TypeDef(
        NAMESPACE,
        name,
        writer::TypeDefOrRef::TypeRef(object),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    let default_attribute = file.TypeRef("Windows.Foundation.Metadata", "DefaultAttribute");
    let default_constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![],
        },
        writer::MemberRefParent::TypeRef(default_attribute),
    );
    for (index, interface) in interfaces.iter().enumerate() {
        let implementation = file.InterfaceImpl(definition, &Type::named(NAMESPACE, *interface));
        if index == 0 {
            file.Attribute(
                writer::HasAttribute::InterfaceImpl(implementation),
                writer::AttributeType::MemberRef(default_constructor),
                &[],
            );
        }
    }
}

fn metadata(path: &Path) {
    let mut file = writer::File::new("PythonOverloadLegacyCompatibility");
    let enum_base = file.TypeRef("System", "Enum");
    let _mode = file.TypeDef(
        NAMESPACE,
        "Mode",
        writer::TypeDefOrRef::TypeRef(enum_base),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    file.Field(
        "value__",
        &Type::I32,
        FieldAttributes::Public | FieldAttributes::SpecialName | FieldAttributes::RTSpecialName,
    );
    let one = file.Field(
        "One",
        &Type::named(NAMESPACE, "Mode"),
        FieldAttributes::Public
            | FieldAttributes::Static
            | FieldAttributes::Literal
            | FieldAttributes::HasDefault,
    );
    file.Constant(writer::HasConstant::Field(one), &Value::I32(1));

    interface(
        &mut file,
        "IAliasCanonical",
        0x51931901,
        "Foo",
        "Foo",
        &[("text", Type::String)],
    );
    interface(
        &mut file,
        "IAliasLegacyString",
        0x51931902,
        "Foo",
        "FooVersion",
        &[("value", Type::String)],
    );
    interface(
        &mut file,
        "IAliasLegacyInt",
        0x51931903,
        "Foo",
        "FooVersion",
        &[("value", Type::I32)],
    );
    runtime_class(
        &mut file,
        "AliasProbe",
        &["IAliasCanonical", "IAliasLegacyString", "IAliasLegacyInt"],
    );

    interface(
        &mut file,
        "IEnumLegacy",
        0x51931904,
        "Foo",
        "Foo",
        &[("value", Type::named(NAMESPACE, "Mode"))],
    );
    interface(
        &mut file,
        "IStringCanonical",
        0x51931905,
        "Foo",
        "FooText",
        &[("value", Type::String)],
    );
    runtime_class(&mut file, "EnumProbe", &["IEnumLegacy", "IStringCanonical"]);
    interface(
        &mut file,
        "IEnumPairLegacy",
        0x51931906,
        "Bar",
        "Bar",
        &[
            ("mode", Type::named(NAMESPACE, "Mode")),
            ("label", Type::String),
        ],
    );
    interface(
        &mut file,
        "IStringPairCanonical",
        0x51931907,
        "Bar",
        "BarText",
        &[("mode", Type::String), ("enabled", Type::Bool)],
    );
    runtime_class(
        &mut file,
        "PairProbe",
        &["IEnumPairLegacy", "IStringPairCanonical"],
    );
    interface(
        &mut file,
        "IIntBoolLegacy",
        0x51931908,
        "Qux",
        "Qux",
        &[("value", Type::I32)],
    );
    interface(
        &mut file,
        "IBoolCanonical",
        0x51931909,
        "Qux",
        "QuxBool",
        &[("value", Type::Bool)],
    );
    runtime_class(
        &mut file,
        "BoolProbe",
        &["IIntBoolLegacy", "IBoolCanonical"],
    );
    interface(
        &mut file,
        "IIntBoolPairLegacy",
        0x5193190a,
        "Quux",
        "Quux",
        &[("value", Type::I32), ("label", Type::String)],
    );
    interface(
        &mut file,
        "IBoolPairCanonical",
        0x5193190b,
        "Quux",
        "QuuxBool",
        &[("value", Type::Bool), ("enabled", Type::Bool)],
    );
    runtime_class(
        &mut file,
        "BoolPairProbe",
        &["IIntBoolPairLegacy", "IBoolPairCanonical"],
    );
    interface(
        &mut file,
        "IIntBoolArityLegacy",
        0x5193190c,
        "Zap",
        "Zap",
        &[("value", Type::I32)],
    );
    interface(
        &mut file,
        "IBoolArityCanonical",
        0x5193190d,
        "Zap",
        "ZapBool",
        &[("value", Type::Bool), ("enabled", Type::Bool)],
    );
    runtime_class(
        &mut file,
        "BoolArityProbe",
        &["IIntBoolArityLegacy", "IBoolArityCanonical"],
    );
    interface(
        &mut file,
        "IIntPairLegacy",
        0x5193190e,
        "Pick",
        "Pick",
        &[("first", Type::I32), ("second", Type::I32)],
    );
    interface(
        &mut file,
        "IBoolWideCanonical",
        0x5193190f,
        "Pick",
        "PickBool",
        &[("first", Type::Bool), ("second", Type::I64)],
    );
    runtime_class(
        &mut file,
        "NumericDomainProbe",
        &["IIntPairLegacy", "IBoolWideCanonical"],
    );
    interface(
        &mut file,
        "IQiLegacy",
        0x51931910,
        "Use",
        "Use",
        &[("target", Type::named(NAMESPACE, "IAliasCanonical"))],
    );
    interface(
        &mut file,
        "IQiCanonical",
        0x51931911,
        "Use",
        "Apply",
        &[("target", Type::named(NAMESPACE, "IAliasLegacyString"))],
    );
    runtime_class(&mut file, "QiDispatchProbe", &["IQiLegacy", "IQiCanonical"]);
    interface(
        &mut file,
        "IIndexLegacy",
        0x51931912,
        "Sift",
        "Sift",
        &[("value", Type::I32)],
    );
    interface(
        &mut file,
        "IIndexStringCanonical",
        0x51931913,
        "Sift",
        "SiftText",
        &[("value", Type::String)],
    );
    runtime_class(
        &mut file,
        "IndexStringProbe",
        &["IIndexLegacy", "IIndexStringCanonical"],
    );
    interface(
        &mut file,
        "IByteLegacy",
        0x51931914,
        "Rank",
        "Rank",
        &[("value", Type::I8)],
    );
    interface(
        &mut file,
        "IModeCanonical",
        0x51931915,
        "Rank",
        "RankMode",
        &[("value", Type::named(NAMESPACE, "Mode"))],
    );
    runtime_class(
        &mut file,
        "EnumComparisonProbe",
        &["IByteLegacy", "IModeCanonical"],
    );
    runtime_class(
        &mut file,
        "_dynwinrt_legacy_call",
        &["IEnumLegacy", "IStringCanonical"],
    );
    runtime_class(
        &mut file,
        "_dynwinrt_legacy_int_guard",
        &["IEnumLegacy", "IStringCanonical"],
    );
    runtime_class(
        &mut file,
        "_dynwinrt_can_cast",
        &["IQiLegacy", "IQiCanonical"],
    );
    runtime_class(
        &mut file,
        "IID_ARG_Tests_OverloadCompatibility_IAliasCanonical",
        &["IQiLegacy", "IQiCanonical"],
    );
    let int_guard_interface = file.TypeDef(
        NAMESPACE,
        "IIntGuardDispatch",
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public
            | TypeAttributes::Interface
            | TypeAttributes::Abstract
            | TypeAttributes::WindowsRuntime,
    );
    guid(&mut file, int_guard_interface, 0x51931916);
    file.MethodDef(
        "Foo",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::I32,
            types: vec![Type::named(NAMESPACE, "Mode")],
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    file.Param("value", 1, ParamAttributes::In);
    let text_method = file.MethodDef(
        "Foo",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::I32,
            types: vec![Type::String],
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    file.Param("value", 1, ParamAttributes::In);
    overload(&mut file, text_method, "FooText");
    file.MethodDef(
        "Use",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::I32,
            types: vec![Type::named(NAMESPACE, "_dynwinrt_legacy_int_guard")],
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    file.Param("target", 1, ParamAttributes::In);

    let registered_iid = "ARG_Tests_OverloadCompatibility_IAliasCanonical";
    interface(
        &mut file,
        registered_iid,
        0x51931917,
        "Identity",
        "Identity",
        &[],
    );
    runtime_class(
        &mut file,
        "IIDRegistrationProbe",
        &[registered_iid, "IQiLegacy", "IQiCanonical"],
    );

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
}

fn async_input_metadata(path: &Path) {
    let mut file = writer::File::new("PythonAsyncOverloadInput");
    let definition = file.TypeDef(
        NAMESPACE,
        "IAsyncInputProbe",
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public
            | TypeAttributes::Interface
            | TypeAttributes::Abstract
            | TypeAttributes::WindowsRuntime,
    );
    guid(&mut file, definition, 0x51931918);
    file.TypeRef("Windows.Foundation", "IAsyncInfo");
    file.MethodDef(
        "Use",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::I32,
            types: vec![Type::named("Windows.Foundation", "IAsyncInfo")],
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    file.Param("target", 1, ParamAttributes::In);
    let numeric = file.MethodDef(
        "Use",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::I32,
            types: vec![Type::I32],
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    file.Param("value", 1, ParamAttributes::In);
    overload(&mut file, numeric, "UseOverload");
    runtime_class(&mut file, "AsyncInputProbe", &["IAsyncInputProbe"]);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
}

#[test]
fn raw_async_interface_arguments_dispatch_to_their_native_slot() {
    let available = Path::new(WINDOWS_WINMD).is_file()
        && Command::new(python())
            .args([
                "-c",
                "import dynwinrt; assert hasattr(dynwinrt, 'DynWinRTImplementationHandle')",
            ])
            .output()
            .is_ok_and(|output| output.status.success());
    if !available {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref(),
            Ok("1"),
            "Windows SDK metadata and native Python implementation runtime are required"
        );
        eprintln!("Skipping live Async argument probe; SDK metadata or runtime unavailable");
        return;
    }

    let fixture = Fixture::new();
    let winmd = fixture.0.join("metadata").join("AsyncInput.winmd");
    async_input_metadata(&winmd);
    let package = fixture.0.join("pyviews");
    success(
        Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args(["generate", "--winmd"])
            .arg(&winmd)
            .args([
                "--ref",
                WINDOWS_WINMD,
                "--namespace",
                NAMESPACE,
                "--lang",
                "py",
                "--output",
            ])
            .arg(&package)
            .output()
            .unwrap(),
    );
    let source =
        fs::read_to_string(package.join("tests__overload_compatibility__async_input_probe.py"))
            .unwrap();
    assert!(
        package
            .join("windows__foundation__i_async_info.py")
            .is_file(),
        "known-interface guards require their projected interface dependency"
    );
    let manifest = fs::read_to_string(package.join("pyproject.toml")).unwrap();
    assert!(
        manifest.contains(&format!(
            "dependencies = [\"dynwinrt=={}\"]",
            env!("CARGO_PKG_VERSION")
        )),
        "{manifest}"
    );
    assert!(
        source.contains("def use(self, *args, **kwargs):")
            && source.contains("IID_ARG_Windows_Foundation_IAsyncInfo = WinGUID.parse(")
            && source.contains("_IAsyncInputProbe.method(6).invoke(")
            && source.contains("_IAsyncInputProbe.method(7).invoke(")
            && !source.contains("_dynwinrt_legacy_call("),
        "{source}"
    );

    let probe = r#"
import importlib
from pathlib import Path
import dynwinrt as dw
from pyviews.tests__overload_compatibility__async_input_probe import AsyncInputProbe
from pyviews.tests__overload_compatibility__i_async_input_probe import IAsyncInputProbe

runtime = importlib.import_module("pyviews._runtime")
info_iid = dw.WinGUID.parse("00000036-0000-0000-c000-000000000046")
wrong_iid = dw.WinGUID.parse("11111111-1111-1111-1111-111111111111")
statics_iid = dw.WinGUID.parse("5984c710-daf2-43c8-8bb4-a4d3eacfd03f")
file_type = dw.DynWinRTType.runtime_class(
    "Windows.Storage.StorageFile",
    dw.DynWinRTType.interface(dw.WinGUID.parse("fa3f6186-4214-428c-a64c-14c9ac7315ea")),
)
statics = dw.DynWinRTType.register_interface("IStorageFileStatics", statics_iid).add_method(
    "GetFileFromPathAsync",
    dw.DynWinRTMethodSig().add_in(dw.DynWinRTType.hstring()).add_out(
        dw.DynWinRTType.i_async_operation(file_type)
    ),
)
calls = []
class Handlers:
    def use(self, target):
        calls.append(("IAsyncInfo", type(target).__name__))
        return 301
    def use_overload(self, value):
        calls.append(("Int32", value))
        return 302

def rejects(call, error_type):
    try:
        call()
    except error_type as error:
        return error
    raise AssertionError(f"Expected {error_type.__name__}")

with dw.RoApartment(1):
    factory = dw.DynWinRTValue.activation_factory("Windows.Storage.StorageFile")
    factory_view = factory.cast(statics_iid)
    try:
        operation = statics.method(6).invoke(
            factory_view, [dw.DynWinRTValue.from_hstring(str(Path(__file__).resolve()))]
        )
    finally:
        factory_view.release()
        factory.release()
    assert "Object value" in str(rejects(operation.as_raw, RuntimeError))
    assert runtime._dynwinrt_can_cast(operation, info_iid), "raw Async QI rejected"
    casted = operation.cast(info_iid)
    scalar = dw.DynWinRTValue.from_i32(5)
    bad_scalar = dw.DynWinRTValue.from_hstring("bad")
    null = dw.DynWinRTValue.null_value()
    assert casted._try_query_interface(info_iid)
    assert operation._try_query_interface(info_iid)
    for value in (scalar, bad_scalar, null):
        assert not value._try_query_interface(info_iid)
        assert not runtime._dynwinrt_can_cast(value, info_iid)
    assert not operation._try_query_interface(wrong_iid)
    assert not runtime._dynwinrt_can_cast(operation, wrong_iid)

    with IAsyncInputProbe.implement(Handlers()) as owner:
        value = AsyncInputProbe._from_native(owner.value._obj)
        try:
            assert value.use(casted) == 301
            assert value.use(operation) == 301
            assert value.use(target=operation) == 301
            assert value.use(5) == 302
            assert value.use(value=9) == 302
            for invalid in (scalar, bad_scalar, null):
                assert "No matching overload for use" in str(
                    rejects(lambda: value.use(invalid), TypeError)
                )
            controlled = OSError(None, "controlled QI failure", None, -2147467259)
            class FailingValue:
                def _try_query_interface(self, _iid): raise controlled
            class OldValue:
                pass
            original = runtime.DynWinRTValue
            try:
                runtime.DynWinRTValue = FailingValue
                assert rejects(
                    lambda: runtime._dynwinrt_can_cast(FailingValue(), info_iid), OSError
                ) is controlled
                runtime.DynWinRTValue = OldValue
                mismatch = rejects(
                    lambda: runtime._dynwinrt_can_cast(OldValue(), info_iid), RuntimeError
                )
                assert "matching dynwinrt runtime" in str(mismatch)
                assert "regenerate all Python bindings" in str(mismatch)
            finally:
                runtime.DynWinRTValue = original

            operation.release()
            assert operation.is_released()
            assert "released" in str(
                rejects(lambda: operation._try_query_interface(info_iid), RuntimeError)
            )
            assert "released" in str(
                rejects(lambda: runtime._dynwinrt_can_cast(operation, info_iid), RuntimeError)
            )
            assert "released" in str(rejects(lambda: value.use(operation), RuntimeError))
        finally:
            dw.release_projected(value)
    casted.release()
    assert calls == [
        ("IAsyncInfo", "IAsyncInfo"),
        ("IAsyncInfo", "IAsyncInfo"),
        ("IAsyncInfo", "IAsyncInfo"),
        ("Int32", 5),
        ("Int32", 9),
    ], calls
print("async-interface-input-ok")
"#;
    fs::write(fixture.0.join("async_probe.py"), probe).unwrap();
    let output = Command::new(python())
        .args(["-B", "async_probe.py"])
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    success(output.clone());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("async-interface-input-ok"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn colliding_legacy_support_names_keep_metadata_types_and_native_targets() {
    let fixture = Fixture::new();
    let winmd = fixture.0.join("metadata").join("Input.winmd");
    metadata(&winmd);
    let winmd_path = winmd.to_str().unwrap();
    let class_names = [
        "_dynwinrt_legacy_call",
        "_dynwinrt_legacy_int_guard",
        "_dynwinrt_can_cast",
        "IID_ARG_Tests_OverloadCompatibility_IAliasCanonical",
        "IIDRegistrationProbe",
    ];
    let classes = class_names
        .iter()
        .map(|name| meta::parse_class(winmd_path, NAMESPACE, name).unwrap())
        .collect::<Vec<_>>();
    let interface =
        meta::parse_public_interface(winmd_path, NAMESPACE, "IIntGuardDispatch").unwrap();
    let identities = meta::parse_namespace(winmd_path, NAMESPACE)
        .iter()
        .map(|class| TypeIdentity::named(TypeIdentityKind::Class, NAMESPACE, &class.name))
        .chain(
            meta::parse_interfaces(winmd_path, NAMESPACE)
                .iter()
                .map(InterfaceMeta::type_identity),
        )
        .chain(
            meta::parse_enums(winmd_path, NAMESPACE)
                .iter()
                .map(TypeMeta::type_identity),
        )
        .collect::<Vec<_>>();
    let runtime_available = Command::new(python())
        .args([
            "-c",
            "import dynwinrt; assert hasattr(dynwinrt, 'DynWinRTImplementationHandle')",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        runtime_available
            || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "native Python implementation runtime is required for the collision probe"
    );

    for packaged in [true, false] {
        let flavor = if packaged { "packaged" } else { "standalone" };
        let parent = fixture.0.join(flavor);
        let package = parent.join("pyviews");
        success(
            Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
                .args(["generate", "--winmd"])
                .arg(&winmd)
                .args(["--namespace", NAMESPACE, "--lang", "py", "--output"])
                .arg(&package)
                .output()
                .unwrap(),
        );
        let context = python::PythonProjectionContext::new(identities.clone(), packaged).unwrap();
        if !packaged {
            for class in &classes {
                let identity = TypeIdentity::named(TypeIdentityKind::Class, NAMESPACE, &class.name);
                fs::write(
                    package.join(format!("{}.py", context.implementation_module(&identity))),
                    python::generate_class(&context, class, &Default::default()),
                )
                .unwrap();
            }
            fs::write(
                package.join(format!(
                    "{}.py",
                    context.implementation_module(&interface.type_identity())
                )),
                python::generate_interface(&context, &interface),
            )
            .unwrap();
        }

        let module = |name: &str, kind| {
            let identity = TypeIdentity::named(kind, NAMESPACE, name);
            context.implementation_module(&identity)
        };
        for (name, kind, helper) in [
            (
                "_dynwinrt_legacy_call",
                TypeIdentityKind::Class,
                "_dynwinrt_legacy_call",
            ),
            (
                "_dynwinrt_legacy_int_guard",
                TypeIdentityKind::Class,
                "_dynwinrt_legacy_int_guard",
            ),
            (
                "_dynwinrt_can_cast",
                TypeIdentityKind::Class,
                "_dynwinrt_can_cast",
            ),
        ] {
            let source =
                fs::read_to_string(package.join(format!("{}.py", module(name, kind)))).unwrap();
            assert!(
                source.contains(&format!("\nclass {name}:")),
                "{flavor}: {source}"
            );
            assert!(
                source.contains(&format!("{helper} as {helper}_2"))
                    && source.contains(&format!("{helper}_2(")),
                "{flavor}: {source}"
            );
        }
        let interface_source = fs::read_to_string(package.join(format!(
            "{}.py",
            module(&interface.name, TypeIdentityKind::Interface)
        )))
        .unwrap();
        assert!(
            interface_source.contains("_dynwinrt_legacy_int_guard as _dynwinrt_legacy_int_guard_2")
                && interface_source.contains("_dynwinrt_legacy_int_guard_2(")
                && interface_source.contains("\nclass IIntGuardDispatch:"),
            "{flavor}: {interface_source}"
        );
        let iid_name = "IID_ARG_Tests_OverloadCompatibility_IAliasCanonical";
        let iid_source = fs::read_to_string(
            package.join(format!("{}.py", module(iid_name, TypeIdentityKind::Class))),
        )
        .unwrap();
        assert!(
            iid_source.contains(&format!("\nclass {iid_name}:")),
            "{iid_source}"
        );
        assert!(
            iid_source.contains(&format!("{iid_name}_2 = WinGUID.parse("))
                && iid_source.contains(&format!("_dynwinrt_can_cast(_bound[0], {iid_name}_2)")),
            "{flavor}: {iid_source}"
        );
        let registration_source = fs::read_to_string(package.join(format!(
            "{}.py",
            module(class_names[4], TypeIdentityKind::Class)
        )))
        .unwrap();
        assert!(
            registration_source.contains(&format!(
                "{iid_name} = WinGUID.parse('51931917-6281-4900-b782-040302010910')"
            )) && registration_source.contains(&format!(
                "{iid_name}_2 = WinGUID.parse('51931901-6281-4900-b782-040302010910')"
            )) && registration_source.contains(&format!(
                "_dynwinrt_can_cast(_legacy_bound[0], {iid_name}_2)"
            )),
            "{flavor}: {registration_source}"
        );

        if !runtime_available {
            eprintln!("Skipping native collision probe; prepared Python binding is unavailable");
            continue;
        }
        let probe = r#"
import importlib
import dynwinrt as dw
from pyviews.tests__overload_compatibility__i_enum_legacy import IEnumLegacy
from pyviews.tests__overload_compatibility__i_string_canonical import IStringCanonical
from pyviews.tests__overload_compatibility__i_qi_legacy import IQiLegacy
from pyviews.tests__overload_compatibility__i_qi_canonical import IQiCanonical
from pyviews.tests__overload_compatibility__i_alias_canonical import IAliasCanonical
from pyviews.{REGISTERED_IID_MODULE} import ARG_Tests_OverloadCompatibility_IAliasCanonical as IRegistration

legacy_module = importlib.import_module("pyviews.{LEGACY_MODULE}")
int_guard_module = importlib.import_module("pyviews.{INT_GUARD_MODULE}")
interface_module = importlib.import_module("pyviews.{INTERFACE_MODULE}")
can_cast_module = importlib.import_module("pyviews.{CAN_CAST_MODULE}")
iid_module = importlib.import_module("pyviews.{IID_MODULE}")
registration_module = importlib.import_module("pyviews.{REGISTRATION_MODULE}")
runtime = importlib.import_module("pyviews._runtime")

Legacy = legacy_module._dynwinrt_legacy_call
IntGuard = int_guard_module._dynwinrt_legacy_int_guard
IIntGuardDispatch = interface_module.IIntGuardDispatch
CanCast = can_cast_module._dynwinrt_can_cast
IidCollision = iid_module.IID_ARG_Tests_OverloadCompatibility_IAliasCanonical
IIDRegistrationProbe = registration_module.IIDRegistrationProbe
assert legacy_module._dynwinrt_legacy_call_2 is runtime._dynwinrt_legacy_call
assert int_guard_module._dynwinrt_legacy_int_guard_2 is runtime._dynwinrt_legacy_int_guard
assert interface_module._dynwinrt_legacy_int_guard_2 is runtime._dynwinrt_legacy_int_guard
assert can_cast_module._dynwinrt_can_cast_2 is runtime._dynwinrt_can_cast
assert iid_module.IID_ARG_Tests_OverloadCompatibility_IAliasCanonical_2 is not IidCollision

calls = []
class EnumHandler:
    def foo(self, value):
        calls.append(("enum", int(value)))
        return 301
class TextHandler:
    def foo_text(self, value):
        calls.append(("text", value))
        return 401
class BothHandlers(EnumHandler, TextHandler):
    def use(self, target):
        return 666
class AliasHandler:
    def foo(self, value):
        return 101
class LegacyQi:
    def use(self, value):
        calls.append(("qi-legacy", type(value).__name__))
        return 1101
class CanonicalQi:
    def apply(self, value):
        calls.append(("qi-canonical", type(value).__name__))
        return 1102
class Registration:
    def identity(self):
        return 1700

with dw.RoApartment(1):
    with IEnumLegacy.implement(
        EnumHandler(), interfaces=[(IStringCanonical, TextHandler())]
    ) as owner:
        value = Legacy._from_native(owner.value._obj)
        try:
            assert value.foo("1") == 301
            assert value.foo(value="2") == 301
            assert value.foo("child") == 401
        finally:
            dw.release_projected(value)

    with IEnumLegacy.implement(
        EnumHandler(), interfaces=[(IStringCanonical, TextHandler())]
    ) as owner:
        value = IntGuard._from_native(owner.value._obj)
        try:
            assert value.foo("1") == 301
            assert value.foo(value="2") == 301
            assert value.foo("child") == 401
        finally:
            dw.release_projected(value)

    with IIntGuardDispatch.implement(BothHandlers()) as owner:
        value = owner.value
        assert value.foo("1") == 301
        assert value.foo(value="2") == 301
        assert value.foo("child") == 401

    with IAliasCanonical.implement(AliasHandler()) as target_owner:
        with IQiLegacy.implement(
            LegacyQi(), interfaces=[(IQiCanonical, CanonicalQi())]
        ) as owner:
            for class_type in (CanCast, IidCollision):
                value = class_type._from_native(owner.value._obj)
                try:
                    assert value.use(target_owner.value._obj) == 1101
                    assert value.use(target=target_owner.value._obj) == 1101
                finally:
                    dw.release_projected(value)
        with IRegistration.implement(
            Registration(),
            interfaces=[(IQiLegacy, LegacyQi()), (IQiCanonical, CanonicalQi())],
        ) as owner:
            value = IIDRegistrationProbe._from_native(owner.value._obj)
            try:
                assert value.identity() == 1700
                assert value.use(target_owner.value._obj) == 1101
                assert value.use(target=target_owner.value._obj) == 1101
            finally:
                dw.release_projected(value)

assert calls == [
    ("enum", 1), ("enum", 2), ("text", "child"),
    ("enum", 1), ("enum", 2), ("text", "child"),
    ("enum", 1), ("enum", 2), ("text", "child"),
    ("qi-legacy", "IAliasCanonical"), ("qi-legacy", "IAliasCanonical"),
    ("qi-legacy", "IAliasCanonical"), ("qi-legacy", "IAliasCanonical"),
    ("qi-legacy", "IAliasCanonical"), ("qi-legacy", "IAliasCanonical"),
], calls
print("overload-support-collisions-ok")
"#;
        let probe = [
            (
                "{LEGACY_MODULE}",
                module(class_names[0], TypeIdentityKind::Class),
            ),
            (
                "{INT_GUARD_MODULE}",
                module(class_names[1], TypeIdentityKind::Class),
            ),
            (
                "{INTERFACE_MODULE}",
                module(&interface.name, TypeIdentityKind::Interface),
            ),
            (
                "{CAN_CAST_MODULE}",
                module(class_names[2], TypeIdentityKind::Class),
            ),
            (
                "{IID_MODULE}",
                module(class_names[3], TypeIdentityKind::Class),
            ),
            (
                "{REGISTERED_IID_MODULE}",
                module(
                    "ARG_Tests_OverloadCompatibility_IAliasCanonical",
                    TypeIdentityKind::Interface,
                ),
            ),
            (
                "{REGISTRATION_MODULE}",
                module(class_names[4], TypeIdentityKind::Class),
            ),
        ]
        .into_iter()
        .fold(probe.to_string(), |source, (placeholder, value)| {
            source.replace(placeholder, &value)
        });
        fs::write(parent.join("collision_probe.py"), probe).unwrap();
        let output = Command::new(python())
            .args(["-B", "collision_probe.py"])
            .current_dir(&parent)
            .output()
            .unwrap();
        success(output.clone());
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("overload-support-collisions-ok"),
            "{flavor}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn old_dispatchers_and_guard_free_conversions_keep_their_exact_targets() {
    let fixture = Fixture::new();
    let winmd = fixture.0.join("metadata").join("Input.winmd");
    metadata(&winmd);
    let package = fixture.0.join("pyviews");
    success(
        Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args(["generate", "--winmd"])
            .arg(&winmd)
            .args(["--namespace", NAMESPACE, "--lang", "py", "--output"])
            .arg(&package)
            .output()
            .unwrap(),
    );
    let alias_source =
        fs::read_to_string(package.join("tests__overload_compatibility__alias_probe.py")).unwrap();
    let alias_class = alias_source
        .split("\nclass IAliasLegacyString:")
        .next()
        .unwrap();
    assert!(
        alias_class.contains("    def foo_version(self, *args, **kwargs):")
            && alias_class.contains("_IAliasLegacyString.method(6).invoke(")
            && !alias_class.contains("    foo_version = foo\n"),
        "{alias_source}"
    );
    let enum_source =
        fs::read_to_string(package.join("tests__overload_compatibility__enum_probe.py")).unwrap();
    assert!(
        enum_source.contains(
            "not (_legacy_bound is not None and (type(_legacy_bound[0]) not in (str,) or _dynwinrt_legacy_int_guard(_legacy_bound[0], -2147483648, 2147483647)))"
        ) && enum_source.contains(
            "return _dynwinrt_legacy_call(self._foo_6_1, ('value',), args, kwargs, 'foo')"
        ),
        "{enum_source}"
    );
    let pair_source =
        fs::read_to_string(package.join("tests__overload_compatibility__pair_probe.py")).unwrap();
    assert!(
        !pair_source.contains("_dynwinrt_legacy_int_guard")
            && pair_source.contains("return self._bar_6_0(*_bound)"),
        "{pair_source}"
    );
    let bool_source =
        fs::read_to_string(package.join("tests__overload_compatibility__bool_probe.py")).unwrap();
    assert!(
        bool_source.contains("not (_legacy_bound is not None) and isinstance(_bound[0], bool)")
            && bool_source.contains(
                "return _dynwinrt_legacy_call(self._qux_6_1, ('value',), args, kwargs, 'qux')"
            )
            && !bool_source.contains("_dynwinrt_legacy_int_guard"),
        "{bool_source}"
    );
    for name in ["bool_pair_probe", "bool_arity_probe"] {
        let source =
            fs::read_to_string(package.join(format!("tests__overload_compatibility__{name}.py")))
                .unwrap();
        assert!(
            !source.contains("_legacy_bound")
                && !source.contains("and not (_legacy_bound is not None"),
            "{source}"
        );
    }
    let numeric_domain_source =
        fs::read_to_string(package.join("tests__overload_compatibility__numeric_domain_probe.py"))
            .unwrap();
    assert!(
        numeric_domain_source.contains(
            "-2147483648 <= int.__index__(_legacy_bound[1]) <= 2147483647"
        ) && numeric_domain_source.contains(
            "return _dynwinrt_legacy_call(self._pick_6_1, ('first', 'second',), args, kwargs, 'pick')"
        ),
        "{numeric_domain_source}"
    );
    let qi_source =
        fs::read_to_string(package.join("tests__overload_compatibility__qi_dispatch_probe.py"))
            .unwrap();
    assert_eq!(
        qi_source
            .matches("_dynwinrt_can_cast(_legacy_bound[0], IID_ARG_Tests_OverloadCompatibility_IAliasCanonical)")
            .count(),
        2,
        "{qi_source}"
    );
    let index_string_source =
        fs::read_to_string(package.join("tests__overload_compatibility__index_string_probe.py"))
            .unwrap();
    assert!(
        index_string_source
            .contains("type(_legacy_bound[0]) not in (str,)) and isinstance(_bound[0], str)")
            && index_string_source.contains(
                "return _dynwinrt_legacy_call(self._sift_6_1, ('value',), args, kwargs, 'sift')"
            ),
        "{index_string_source}"
    );
    let enum_comparison_source =
        fs::read_to_string(package.join("tests__overload_compatibility__enum_comparison_probe.py"))
            .unwrap();
    assert!(
        enum_comparison_source.contains("-128 <= int.__index__(_legacy_bound[0]) <= 127")
            && !enum_comparison_source.contains("-128 <= _legacy_bound[0] <= 127"),
        "{enum_comparison_source}"
    );

    let available = Command::new(python())
        .args([
            "-c",
            "import dynwinrt; assert hasattr(dynwinrt, 'DynWinRTImplementationHandle')",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref(),
            Ok("1"),
            "native Python implementation runtime is required"
        );
        eprintln!("Skipping live compatibility probe; prepared Python binding is unavailable");
        return;
    }

    let probe = r#"
import json
import importlib
import dynwinrt as dw
from pyviews.tests__overload_compatibility__alias_probe import AliasProbe
from pyviews.tests__overload_compatibility__bool_arity_probe import BoolArityProbe
from pyviews.tests__overload_compatibility__bool_pair_probe import BoolPairProbe
from pyviews.tests__overload_compatibility__bool_probe import BoolProbe
from pyviews.tests__overload_compatibility__enum_probe import EnumProbe
from pyviews.tests__overload_compatibility__enum_comparison_probe import EnumComparisonProbe
from pyviews.tests__overload_compatibility__index_string_probe import IndexStringProbe
from pyviews.tests__overload_compatibility__numeric_domain_probe import NumericDomainProbe
from pyviews.tests__overload_compatibility__pair_probe import PairProbe
from pyviews.tests__overload_compatibility__qi_dispatch_probe import QiDispatchProbe
from pyviews.tests__overload_compatibility__i_alias_canonical import IAliasCanonical
from pyviews.tests__overload_compatibility__i_alias_legacy_int import IAliasLegacyInt
from pyviews.tests__overload_compatibility__i_alias_legacy_string import IAliasLegacyString
from pyviews.tests__overload_compatibility__i_bool_arity_canonical import IBoolArityCanonical
from pyviews.tests__overload_compatibility__i_bool_canonical import IBoolCanonical
from pyviews.tests__overload_compatibility__i_bool_pair_canonical import IBoolPairCanonical
from pyviews.tests__overload_compatibility__i_enum_legacy import IEnumLegacy
from pyviews.tests__overload_compatibility__i_enum_pair_legacy import IEnumPairLegacy
from pyviews.tests__overload_compatibility__i_byte_legacy import IByteLegacy
from pyviews.tests__overload_compatibility__i_int_bool_arity_legacy import IIntBoolArityLegacy
from pyviews.tests__overload_compatibility__i_int_bool_legacy import IIntBoolLegacy
from pyviews.tests__overload_compatibility__i_int_bool_pair_legacy import IIntBoolPairLegacy
from pyviews.tests__overload_compatibility__i_int_pair_legacy import IIntPairLegacy
from pyviews.tests__overload_compatibility__i_index_legacy import IIndexLegacy
from pyviews.tests__overload_compatibility__i_index_string_canonical import IIndexStringCanonical
from pyviews.tests__overload_compatibility__i_qi_canonical import IQiCanonical
from pyviews.tests__overload_compatibility__i_qi_legacy import IQiLegacy
from pyviews.tests__overload_compatibility__i_mode_canonical import IModeCanonical
from pyviews.tests__overload_compatibility__i_string_canonical import IStringCanonical
from pyviews.tests__overload_compatibility__i_string_pair_canonical import IStringPairCanonical
from pyviews.tests__overload_compatibility__i_bool_wide_canonical import IBoolWideCanonical
from pyviews.tests__overload_compatibility__mode import Mode

calls = []
conversion_events = []
index_events = []
runtime = importlib.import_module("pyviews._runtime")

class AliasCanonical:
    def foo(self, value):
        calls.append(("alias-canonical", value))
        return 101

class AliasLegacyString:
    def foo_version(self, value):
        calls.append(("alias-legacy-string", value))
        return 201

class AliasLegacyInt:
    def foo_version(self, value):
        calls.append(("alias-legacy-int", value))
        return 202

class EnumLegacy:
    def foo(self, value):
        calls.append(("enum-legacy", int(value)))
        return 301

class StringCanonical:
    def foo_text(self, value):
        calls.append(("string-canonical", value))
        return 401

class NumericString(str):
    def __int__(self):
        conversion_events.append(("numeric-string-int", str(self)))
        return int(str(self))

class IndexString(str):
    def __index__(self):
        index_events.append(("index-string-index", str(self)))
        return 7

class EnumPairLegacy:
    def bar(self, mode, label):
        calls.append(("enum-pair-legacy", int(mode), label))
        return 501

class StringPairCanonical:
    def bar_text(self, mode, enabled):
        calls.append(("string-pair-canonical", mode, enabled))
        return 601

class IntBoolLegacy:
    def qux(self, value):
        calls.append(("int-bool-legacy", value))
        return 701

class BoolCanonical:
    def qux_bool(self, value):
        calls.append(("bool-canonical", value))
        return 702

class IntBoolPairLegacy:
    def quux(self, value, label):
        calls.append(("int-bool-pair-legacy", value, label))
        return 801

class BoolPairCanonical:
    def quux_bool(self, value, enabled):
        calls.append(("bool-pair-canonical", value, enabled))
        return 802

class IntBoolArityLegacy:
    def zap(self, value):
        calls.append(("int-bool-arity-legacy", value))
        return 901

class BoolArityCanonical:
    def zap_bool(self, value, enabled):
        calls.append(("bool-arity-canonical", value, enabled))
        return 902

class IntPairLegacy:
    def pick(self, first, second):
        calls.append(("int-pair-legacy", first, second))
        return 1001

class BoolWideCanonical:
    def pick_bool(self, first, second):
        calls.append(("bool-wide-canonical", first, second))
        return 1002

class QiLegacy:
    def use(self, target):
        calls.append(("qi-legacy", target.__class__.__name__))
        return 1101

class QiCanonical:
    def apply(self, target):
        calls.append(("qi-canonical", target.__class__.__name__))
        return 1102

class IndexLegacy:
    def sift(self, value):
        calls.append(("index-legacy", value))
        return 1201

class IndexStringCanonical:
    def sift_text(self, value):
        calls.append(("index-string-canonical", value))
        return 1202

class ByteLegacy:
    def rank(self, value):
        calls.append(("byte-legacy", value))
        return 1301

class ModeCanonical:
    def rank_mode(self, value):
        calls.append(("mode-canonical", int(value)))
        return 1302

results = {}
class UnexpectedIntError:
    def __int__(self):
        raise RuntimeError("unexpected-int-error")

try:
    runtime._dynwinrt_legacy_int_guard(UnexpectedIntError())
except RuntimeError as error:
    results["unexpected_int_error"] = str(error)

with dw.RoApartment(1):
    with IAliasCanonical.implement(
        AliasCanonical(),
        interfaces=[
            (IAliasLegacyString, AliasLegacyString()),
            (IAliasLegacyInt, AliasLegacyInt()),
        ],
    ) as implementation:
        value = AliasProbe._from_native(implementation.value._obj)
        try:
            results["alias_positional"] = value.foo_version("7")
            results["alias_keyword"] = value.foo_version(value="8")
        finally:
            dw.release_projected(value)

    with IEnumLegacy.implement(
        EnumLegacy(),
        interfaces=[(IStringCanonical, StringCanonical())],
    ) as implementation:
        value = EnumProbe._from_native(implementation.value._obj)
        try:
            results["enum_positional"] = value.foo("1")
            results["enum_keyword"] = value.foo(value="1")
            results["text_positional"] = value.foo("not numeric")
            results["enum_string_subclass"] = value.foo(NumericString("9"))
            results["enum_string_conversion_count"] = len(conversion_events)
        finally:
            dw.release_projected(value)

    with IEnumPairLegacy.implement(
        EnumPairLegacy(),
        interfaces=[(IStringPairCanonical, StringPairCanonical())],
    ) as implementation:
        value = PairProbe._from_native(implementation.value._obj)
        try:
            results["nonoverlap_positional"] = value.bar("1", True)
            results["nonoverlap_keyword"] = value.bar(mode="1", enabled=False)
        finally:
            dw.release_projected(value)

    with IIntBoolLegacy.implement(
        IntBoolLegacy(),
        interfaces=[(IBoolCanonical, BoolCanonical())],
    ) as implementation:
        value = BoolProbe._from_native(implementation.value._obj)
        try:
            results["bool_positional"] = value.qux(True)
            results["bool_keyword"] = value.qux(value=False)
        finally:
            dw.release_projected(value)

    with IIntBoolPairLegacy.implement(
        IntBoolPairLegacy(),
        interfaces=[(IBoolPairCanonical, BoolPairCanonical())],
    ) as implementation:
        value = BoolPairProbe._from_native(implementation.value._obj)
        try:
            results["bool_pair_positional"] = value.quux(True, False)
            results["bool_pair_keyword"] = value.quux(value=False, enabled=True)
        finally:
            dw.release_projected(value)

    with IIntBoolArityLegacy.implement(
        IntBoolArityLegacy(),
        interfaces=[(IBoolArityCanonical, BoolArityCanonical())],
    ) as implementation:
        value = BoolArityProbe._from_native(implementation.value._obj)
        try:
            results["bool_arity_positional"] = value.zap(True, False)
            results["bool_arity_keyword"] = value.zap(value=False, enabled=True)
        finally:
            dw.release_projected(value)

    with IIntPairLegacy.implement(
        IntPairLegacy(),
        interfaces=[(IBoolWideCanonical, BoolWideCanonical())],
    ) as implementation:
        value = NumericDomainProbe._from_native(implementation.value._obj)
        try:
            results["numeric_pair_positional"] = value.pick(True, 5)
            results["numeric_pair_keyword"] = value.pick(first=False, second=5)
            results["numeric_pair_i32_max"] = value.pick(True, 2**31 - 1)
            results["numeric_pair_i32_min"] = value.pick(False, -(2**31))
            results["numeric_pair_wide_high"] = value.pick(True, 2**31)
            results["numeric_pair_wide_low"] = value.pick(False, -(2**31) - 1)
        finally:
            dw.release_projected(value)

    with IAliasCanonical.implement(
        AliasCanonical(),
        interfaces=[(IAliasLegacyString, AliasLegacyString())],
    ) as target_implementation:
        target = AliasProbe._from_native(target_implementation.value._obj)
        try:
            with IQiLegacy.implement(
                QiLegacy(),
                interfaces=[(IQiCanonical, QiCanonical())],
            ) as implementation:
                value = QiDispatchProbe._from_native(implementation.value._obj)
                try:
                    results["qi_positional"] = value.use(target)
                    results["qi_keyword"] = value.use(target=target)
                finally:
                    dw.release_projected(value)
        finally:
            dw.release_projected(target)

    with IIndexLegacy.implement(
        IndexLegacy(),
        interfaces=[(IIndexStringCanonical, IndexStringCanonical())],
    ) as implementation:
        value = IndexStringProbe._from_native(implementation.value._obj)
        try:
            results["index_string_positional"] = value.sift(IndexString("child"))
            results["index_string_keyword"] = value.sift(value=IndexString("child"))
            results["index_string_builtin"] = value.sift("child")
            results["index_string_conversion_count"] = len(index_events)
        finally:
            dw.release_projected(value)

    comparison_events = []
    original_le = Mode.__le__
    Mode.__le__ = lambda self, other: (
        comparison_events.append(("unexpected-le", int(self))),
        (_ for _ in ()).throw(RuntimeError("unexpected comparison")),
    )[1]
    try:
        with IByteLegacy.implement(
            ByteLegacy(),
            interfaces=[(IModeCanonical, ModeCanonical())],
        ) as implementation:
            value = EnumComparisonProbe._from_native(implementation.value._obj)
            try:
                results["enum_comparison_positional"] = value.rank(Mode.One)
                results["enum_comparison_keyword"] = value.rank(value=Mode.One)
                results["enum_comparison_side_effects"] = len(comparison_events)
            finally:
                dw.release_projected(value)
    finally:
        Mode.__le__ = original_le

print(json.dumps({"results": results, "calls": calls}))
"#;
    fs::write(fixture.0.join("probe.py"), probe).unwrap();
    let output = Command::new(python())
        .args(["-B", "probe.py"])
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    success(output.clone());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains(
            r#""results": {"unexpected_int_error": "unexpected-int-error", "alias_positional": 201, "alias_keyword": 201, "enum_positional": 301, "enum_keyword": 301, "text_positional": 401, "enum_string_subclass": 301, "enum_string_conversion_count": 1, "nonoverlap_positional": 601, "nonoverlap_keyword": 601, "bool_positional": 701, "bool_keyword": 701, "bool_pair_positional": 802, "bool_pair_keyword": 802, "bool_arity_positional": 902, "bool_arity_keyword": 902, "numeric_pair_positional": 1001, "numeric_pair_keyword": 1001, "numeric_pair_i32_max": 1001, "numeric_pair_i32_min": 1001, "numeric_pair_wide_high": 1002, "numeric_pair_wide_low": 1002, "qi_positional": 1101, "qi_keyword": 1101, "index_string_positional": 1201, "index_string_keyword": 1201, "index_string_builtin": 1202, "index_string_conversion_count": 2, "enum_comparison_positional": 1301, "enum_comparison_keyword": 1301, "enum_comparison_side_effects": 0}"#
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            r#""calls": [["alias-legacy-string", "7"], ["alias-legacy-string", "8"], ["enum-legacy", 1], ["enum-legacy", 1], ["string-canonical", "not numeric"], ["enum-legacy", 9], ["string-pair-canonical", "1", true], ["string-pair-canonical", "1", false], ["int-bool-legacy", 1], ["int-bool-legacy", 0], ["bool-pair-canonical", true, false], ["bool-pair-canonical", false, true], ["bool-arity-canonical", true, false], ["bool-arity-canonical", false, true], ["int-pair-legacy", 1, 5], ["int-pair-legacy", 0, 5], ["int-pair-legacy", 1, 2147483647], ["int-pair-legacy", 0, -2147483648], ["bool-wide-canonical", true, 2147483648], ["bool-wide-canonical", false, -2147483649], ["qi-legacy", "IAliasCanonical"], ["qi-legacy", "IAliasCanonical"], ["index-legacy", 7], ["index-legacy", 7], ["index-string-canonical", "child"], ["byte-legacy", 1], ["byte-legacy", 1]]"#
        ),
        "{stdout}"
    );
}
