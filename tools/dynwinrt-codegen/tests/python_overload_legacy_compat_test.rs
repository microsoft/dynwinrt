// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use windows_metadata::{
    FieldAttributes, MethodAttributes, MethodCallAttributes, MethodImplAttributes, ParamAttributes,
    Signature, Type, TypeAttributes, Value, writer,
};

const NAMESPACE: &str = "Tests.OverloadCompatibility";
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

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
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
            "not (_legacy_bound is not None and _dynwinrt_legacy_int_guard(_legacy_bound[0]))"
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
from pyviews.tests__overload_compatibility__enum_probe import EnumProbe
from pyviews.tests__overload_compatibility__pair_probe import PairProbe
from pyviews.tests__overload_compatibility__i_alias_canonical import IAliasCanonical
from pyviews.tests__overload_compatibility__i_alias_legacy_int import IAliasLegacyInt
from pyviews.tests__overload_compatibility__i_alias_legacy_string import IAliasLegacyString
from pyviews.tests__overload_compatibility__i_enum_legacy import IEnumLegacy
from pyviews.tests__overload_compatibility__i_enum_pair_legacy import IEnumPairLegacy
from pyviews.tests__overload_compatibility__i_string_canonical import IStringCanonical
from pyviews.tests__overload_compatibility__i_string_pair_canonical import IStringPairCanonical

calls = []
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

class EnumPairLegacy:
    def bar(self, mode, label):
        calls.append(("enum-pair-legacy", int(mode), label))
        return 501

class StringPairCanonical:
    def bar_text(self, mode, enabled):
        calls.append(("string-pair-canonical", mode, enabled))
        return 601

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
            r#""results": {"unexpected_int_error": "unexpected-int-error", "alias_positional": 201, "alias_keyword": 201, "enum_positional": 301, "enum_keyword": 301, "text_positional": 401, "nonoverlap_positional": 601, "nonoverlap_keyword": 601}"#
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            r#""calls": [["alias-legacy-string", "7"], ["alias-legacy-string", "8"], ["enum-legacy", 1], ["enum-legacy", 1], ["string-canonical", "not numeric"], ["string-pair-canonical", "1", true], ["string-pair-canonical", "1", false]]"#
        ),
        "{stdout}"
    );
}
