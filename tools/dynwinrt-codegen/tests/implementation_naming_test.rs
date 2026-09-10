// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use windows_metadata::{
    MethodAttributes, MethodCallAttributes, MethodImplAttributes, ParamAttributes, Signature, Type,
    TypeAttributes, Value, writer,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "implementation-names-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
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

fn names(path: &Path, definitions: &[(&str, &str, &str, u32)]) {
    names_with_callback(path, definitions, false);
}

fn names_with_callback(path: &Path, definitions: &[(&str, &str, &str, u32)], callback_owner: bool) {
    let mut file = writer::File::new("ImplementationNames");
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
    for &(namespace, name, method, id) in definitions {
        let interface = file.TypeDef(
            namespace,
            name,
            writer::TypeDefOrRef::default(),
            TypeAttributes::Public
                | TypeAttributes::Interface
                | TypeAttributes::Abstract
                | TypeAttributes::WindowsRuntime,
        );
        let values = vec![
            Value::U32(id),
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
            writer::HasAttribute::TypeDef(interface),
            writer::AttributeType::MemberRef(constructor),
            &values
                .into_iter()
                .map(|value| (String::new(), value))
                .collect::<Vec<_>>(),
        );
        file.MethodDef(
            method,
            &Signature {
                flags: MethodCallAttributes::HASTHIS,
                return_type: Type::I32,
                types: vec![],
            },
            MethodAttributes::Public
                | MethodAttributes::Abstract
                | MethodAttributes::Virtual
                | MethodAttributes::NewSlot,
            MethodImplAttributes::default(),
        );
        if callback_owner && name == "IComplex" {
            file.MethodDef(
                "UseCallback",
                &Signature {
                    flags: MethodCallAttributes::HASTHIS,
                    return_type: Type::Void,
                    types: vec![Type::named("Contoso", "Callback")],
                },
                MethodAttributes::Public
                    | MethodAttributes::Abstract
                    | MethodAttributes::Virtual
                    | MethodAttributes::NewSlot,
                MethodImplAttributes::default(),
            );
            file.Param("callback", 1, ParamAttributes::In);
            for method in ["Split", "Delegate0"] {
                file.MethodDef(
                    method,
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
                file.Param("first", 1, ParamAttributes::Out);
            }
        }
    }
    if callback_owner {
        let base = file.TypeRef("System", "MulticastDelegate");
        let delegate = file.TypeDef(
            "Contoso",
            "Callback",
            writer::TypeDefOrRef::TypeRef(base),
            TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
        );
        let values = vec![
            Value::U32(0x31d44799),
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
            writer::HasAttribute::TypeDef(delegate),
            writer::AttributeType::MemberRef(constructor),
            &values
                .into_iter()
                .map(|value| (String::new(), value))
                .collect::<Vec<_>>(),
        );
        file.MethodDef(
            ".ctor",
            &Signature {
                flags: MethodCallAttributes::HASTHIS,
                return_type: Type::Void,
                types: vec![],
            },
            MethodAttributes::Public | MethodAttributes::SpecialName,
            MethodImplAttributes::default(),
        );
        file.MethodDef(
            "Invoke",
            &Signature {
                flags: MethodCallAttributes::HASTHIS,
                return_type: Type::I32,
                types: vec![Type::I32],
            },
            MethodAttributes::Public | MethodAttributes::Virtual | MethodAttributes::NewSlot,
            MethodImplAttributes::default(),
        );
        file.Param("first", 1, ParamAttributes::Out);
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
}

#[test]
fn delegate_and_named_result_helpers_share_the_symbol_allocator() {
    let fixture = Fixture::new();
    let metadata = fixture.0.join("metadata").join("Complex.winmd");
    names_with_callback(
        &metadata,
        &[
            ("Contoso", "IComplex", "ReadValue", 0x31d44751),
            (
                "Contoso",
                "IComplexImplementationDelegate0",
                "RealDelegateName",
                0x31d44752,
            ),
            (
                "Contoso",
                "IComplexImplementationDelegate0Result",
                "RealResultName",
                0x31d44753,
            ),
            (
                "Contoso",
                "IComplexImplementationSplitResult",
                "RealSplitName",
                0x31d44754,
            ),
        ],
        true,
    );
    let names = "Contoso.IComplex,Contoso.IComplexImplementationDelegate0,Contoso.IComplexImplementationDelegate0Result,Contoso.IComplexImplementationSplitResult";
    for lang in ["js", "py"] {
        let output = fixture.0.join(if lang == "py" { "pyviews" } else { "js" });
        success(generate(&metadata, &output, lang, names, &[]));
        if lang == "js" {
            typecheck_js(&output);
            continue;
        }
        let inventory = python_inventory(&output);
        let record = inventory
            .iter()
            .find(|record| record["identity"]["name"] == "IComplex")
            .unwrap();
        let helpers = record["implementation"]["helpers"]
            .as_array()
            .expect("complete delegate contract must remain implementable");
        let helper = |key: &str| {
            helpers.iter().find(|item| item["key"] == key).unwrap()["name"]
                .as_str()
                .unwrap()
        };
        let delegate = helper("delegate:0");
        let delegate_result = helper("delegate-result:0");
        let split = helper("method-result:8");
        let method_result = helper("method-result:9");
        assert_ne!(delegate_result, method_result);
        assert_ne!(delegate, "IComplexImplementationDelegate0");
        assert_ne!(split, "IComplexImplementationSplitResult");
        typecheck_py(
            &fixture.0,
            &format!(
                r#"
from pyviews import (
    IComplex, IComplexImplementationDelegate0, IComplexImplementationDelegate0Result,
    IComplexImplementationSplitResult, {delegate}, {delegate_result}, {split}, {method_result},
)
class Handler:
    def read_value(self) -> int: return 1
    def use_callback(self, callback: {delegate} | None) -> None:
        if callback is not None:
            result: {delegate_result} = callback()
    def split(self) -> {split}: return {{"first": 1, "result": 2}}
    def delegate0(self) -> {method_result}: return {{"first": 3, "result": 4}}
IComplex.implementation(Handler())
IComplexImplementationDelegate0.from_value
IComplexImplementationDelegate0Result.from_value
IComplexImplementationSplitResult.from_value
"#
            ),
        );
        if runtime_available() {
            success(Command::new(python()).args(["-c", r#"
import pyviews as g
for name in ('IComplexImplementationDelegate0', 'IComplexImplementationDelegate0Result', 'IComplexImplementationSplitResult'):
    assert hasattr(getattr(g, name), 'from_value'), name
"#]).current_dir(&fixture.0).output().unwrap());
        }
    }
}

fn generate(metadata: &Path, output: &Path, lang: &str, selected: &str, extra: &[&str]) -> Output {
    let runtime = root()
        .join("bindings")
        .join("js")
        .join("dist")
        .join("winrt.js");
    let runtime = runtime.to_string_lossy().replace('\\', "/");
    Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd"])
        .arg(metadata)
        .args(["--output"])
        .arg(output)
        .args([
            "--lang",
            lang,
            "--class-name",
            selected,
            "--import-name",
            &runtime,
        ])
        .args(extra)
        .output()
        .unwrap()
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn typecheck_js(output: &Path) {
    let tsc = root()
        .join("bindings")
        .join("js")
        .join("node_modules")
        .join("typescript")
        .join("bin")
        .join("tsc");
    if !tsc.is_file()
        || !root()
            .join("bindings")
            .join("js")
            .join("dist")
            .join("winrt.d.ts")
            .is_file()
    {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref(),
            Ok("1"),
            "the native naming E2E requires built runtime declarations and the existing TypeScript dependency"
        );
        eprintln!("Native naming typechecks run after building bindings in the E2E job.");
        return;
    }
    success(
        Command::new("node")
            .arg(tsc)
            .args([
                "--noEmit",
                "--strict",
                "--target",
                "ES2022",
                "--module",
                "Node16",
                "--moduleResolution",
                "Node16",
                "--types",
                "node",
                "--typeRoots",
            ])
            .arg(
                root()
                    .join("bindings")
                    .join("js")
                    .join("node_modules")
                    .join("@types"),
            )
            .arg(output.join("index.d.ts"))
            .output()
            .unwrap(),
    );
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

fn typecheck_py(directory: &Path, source: &str) {
    if !Command::new(python())
        .args(["-m", "mypy", "--version"])
        .output()
        .is_ok_and(|output| output.status.success())
    {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref(),
            Ok("1"),
            "mypy is required by the native naming E2E"
        );
        eprintln!(
            "Python naming typechecks require the E2E environment's existing mypy dependency."
        );
        return;
    }
    fs::write(directory.join("consumer.py"), source).unwrap();
    success(
        Command::new(python())
            .args([
                "-m",
                "mypy",
                "--strict",
                "--no-incremental",
                "--follow-imports=silent",
                "consumer.py",
            ])
            .current_dir(directory)
            .env(
                "MYPYPATH",
                std::env::join_paths([root().join("bindings").join("py"), directory.to_path_buf()])
                    .unwrap(),
            )
            .output()
            .unwrap(),
    );
}

fn runtime_available() -> bool {
    let available = Command::new(python())
        .args([
            "-c",
            "import dynwinrt; assert hasattr(dynwinrt, 'DynWinRTImplementationHandle')",
        ])
        .output()
        .is_ok_and(|result| result.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "runtime naming regressions require the current Python binding"
    );
    available
}

fn python_inventory(directory: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(directory.join(".dynwinrt-generated-types"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn snapshot(directory: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(directory, directory, &mut files);
    files
}

#[test]
fn helper_collision_keeps_real_metadata_types_and_owned_handler_exports() {
    let fixture = Fixture::new();
    let metadata = fixture.0.join("metadata").join("Names.winmd");
    names(
        &metadata,
        &[
            ("Contoso", "IValue", "ReadValue", 0x31d44731),
            ("Contoso", "IValueHandlers", "ReadRealInterface", 0x31d44732),
        ],
    );
    let js = fixture.0.join("js");
    let py = fixture.0.join("pyviews");
    success(generate(
        &metadata,
        &js,
        "js",
        "Contoso.IValue,Contoso.IValueHandlers",
        &[],
    ));
    success(generate(
        &metadata,
        &py,
        "py",
        "Contoso.IValue,Contoso.IValueHandlers",
        &[],
    ));
    typecheck_js(&js);
    let records = python_inventory(&py);
    let item = records
        .iter()
        .find(|record| record["identity"]["name"] == "IValue")
        .unwrap();
    let helper = item["implementation"]["handler_export"].as_str().unwrap();
    assert_ne!(helper, "IValueHandlers");
    typecheck_py(
        &fixture.0,
        &format!(
            r#"
from pyviews import IValue, IValueHandlers, {helper}
from pyviews.contoso.i_value_handlers import IValueHandlers as Deep
from typing import assert_type
class Handler:
    def read_value(self) -> int: return 1
class Real:
    def read_real_interface(self) -> int: return 2
handler: {helper} = Handler()
assert_type(IValue.implement(handler).value, IValue)
assert_type(Deep.implement(Real()).value, IValueHandlers)
with IValue.implement(handler, interfaces=[(IValueHandlers, Real())]) as impl:
    assert_type(impl.value, IValue)
"#
        ),
    );
    if runtime_available() {
        success(Command::new(python()).args(["-c", r#"
import pyviews as g
import dynwinrt as dw
from pyviews.contoso.i_value_handlers import IValueHandlers
assert g.IValueHandlers is IValueHandlers and hasattr(IValueHandlers, 'from_value')
class Handler:
    def read_value(self): return 1
class Real:
    def read_real_interface(self): return 2
with dw.RoApartment(1), g.IValue.implement(Handler(), interfaces=[(IValueHandlers, Real())]) as impl:
    assert impl.value.read_value() == 1
    other = IValueHandlers.from_implementation(impl)
    try: assert other.read_real_interface() == 2
    finally: dw.release_projected(other)
"#]).current_dir(&fixture.0).output().unwrap());
        success(Command::new("node").args(["-e", r#"
const assert = require('node:assert/strict');
const g = require('./js');
const deep = require('./js/contoso/IValueHandlers.js');
assert.equal(typeof g.IValueHandlers.implement, 'function');
const impl = g.IValue.implement({ readValue: () => 1 }, { interfaces: [[g.IValueHandlers, { readRealInterface: () => 2 }]] });
try {
    assert.equal(impl.value.readValue(), 1);
    const other = deep.IValueHandlers.fromImplementation(impl);
    try { assert.equal(other.readRealInterface(), 2); } finally { g.releaseProjected(other); }
} finally { impl.dispose(); }
"#]).current_dir(&fixture.0).output().unwrap());
    }
}

#[test]
fn incremental_helper_aliases_match_clean_generation_and_retain_unrelated_modules() {
    let fixture = Fixture::new();
    let initial = fixture.0.join("initial").join("Initial.winmd");
    let extra = fixture.0.join("extra").join("Extra.winmd");
    let combined = fixture.0.join("combined").join("Combined.winmd");
    let first = [
        ("AlphaBeta", "IWidget", "ReadAlphaBeta", 0x31d44733),
        ("Control", "IWidget", "ReadControl", 0x31d44734),
    ];
    let second = [("Alpha.Beta", "IWidget", "ReadAlphaDotBeta", 0x31d44735)];
    names(&initial, &first);
    names(&extra, &second);
    names(&combined, &[first.as_slice(), second.as_slice()].concat());
    for lang in ["js", "py"] {
        let incremental = fixture.0.join(format!("{lang}_incremental"));
        let clean = fixture.0.join(format!("{lang}_clean"));
        success(generate(
            &initial,
            &incremental,
            lang,
            "AlphaBeta.IWidget,Control.IWidget",
            &[],
        ));
        let retained = if lang == "js" {
            "control\\IWidget.js"
        } else {
            "control__i_widget.py"
        };
        let original = fs::read(incremental.join(retained)).unwrap();
        // New manifests retain helper identity and eligibility without old WinMD.
        success(generate(
            &extra,
            &incremental,
            lang,
            "Alpha.Beta.IWidget",
            &[],
        ));
        assert_eq!(original, fs::read(incremental.join(retained)).unwrap());
        success(generate(
            &combined,
            &clean,
            lang,
            "Alpha.Beta.IWidget,Control.IWidget,AlphaBeta.IWidget",
            &[],
        ));
        if lang == "js" {
            assert_eq!(
                fs::read(incremental.join("index.d.ts")).unwrap(),
                fs::read(clean.join("index.d.ts")).unwrap()
            );
            typecheck_js(&incremental);
            typecheck_js(&clean);
        } else {
            assert_eq!(
                fs::read(incremental.join("_implementation_types.pyi")).unwrap(),
                fs::read(clean.join("_implementation_types.pyi")).unwrap()
            );
            let exports = |path: &Path| {
                python_inventory(path)
                    .into_iter()
                    .map(|record| {
                        (
                            record["identity"].to_string(),
                            (
                                record["implementation"]["handler_export"].clone(),
                                record["implementation"]["interface_export"].clone(),
                            ),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
            };
            assert_eq!(exports(&incremental), exports(&clean));
            typecheck_py(
                &fixture.0,
                r#"
from py_incremental.alpha.beta.i_widget import IWidget as A
from py_incremental.alpha_beta.i_widget import IWidget as B
from py_incremental.control.i_widget import IWidget as C
class One:
    def read_alpha_dot_beta(self) -> int: return 1
class Two:
    def read_alpha_beta(self) -> int: return 2
class Three:
    def read_control(self) -> int: return 3
with A.implement(One(), interfaces=[(B, Two()), (C, Three())]) as impl:
    value: A = impl.value
"#,
            );
        }
        let before = snapshot(&incremental);
        success(generate(
            &extra,
            &incremental,
            lang,
            "Alpha.Beta.IWidget",
            &[],
        ));
        assert_eq!(
            before,
            snapshot(&incremental),
            "repeated {lang} append must be stable"
        );
    }
}

#[test]
fn appending_a_real_type_renames_only_the_retained_helper_not_its_interface() {
    let fixture = Fixture::new();
    let first = fixture.0.join("first").join("First.winmd");
    let second = fixture.0.join("second").join("Second.winmd");
    let both = fixture.0.join("both").join("Both.winmd");
    let definitions = [
        ("Contoso", "IValue", "ReadValue", 0x31d44761),
        ("Contoso", "IValueHandlers", "ReadRealInterface", 0x31d44762),
    ];
    names(&first, &definitions[..1]);
    names(&second, &definitions[1..]);
    names(&both, &definitions);
    for lang in ["js", "py"] {
        let output = fixture.0.join(format!("{lang}views"));
        let clean = fixture.0.join(format!("{lang}clean"));
        success(generate(&first, &output, lang, "Contoso.IValue", &[]));
        success(generate(
            &second,
            &output,
            lang,
            "Contoso.IValueHandlers",
            &[],
        ));
        success(generate(
            &both,
            &clean,
            lang,
            "Contoso.IValueHandlers,Contoso.IValue",
            &[],
        ));
        if lang == "js" {
            assert_eq!(
                fs::read(output.join("index.d.ts")).unwrap(),
                fs::read(clean.join("index.d.ts")).unwrap()
            );
            typecheck_js(&output);
            let source = fs::read_to_string(output.join("contoso").join("IValue.d.ts")).unwrap();
            assert!(source.contains("export type { IValueHandlers as "));
            assert!(
                source.contains("export class IValue")
                    || source.contains("export declare class IValue")
            );
        } else {
            let inventory = python_inventory(&output);
            let real = inventory
                .iter()
                .find(|record| record["identity"]["name"] == "IValueHandlers")
                .unwrap();
            assert_eq!(real["implementation"]["interface_export"], "IValueHandlers");
            typecheck_py(
                &fixture.0,
                r#"
from pyviews import IValue, IValueHandlers
class First:
    def read_value(self) -> int: return 1
class Second:
    def read_real_interface(self) -> int: return 2
with IValue.implement(First(), interfaces=[(IValueHandlers, Second())]) as impl:
    value: IValue = impl.value
"#,
            );
            if runtime_available() {
                success(Command::new(python()).args(["-c", "import pyviews as g; from pyviews.contoso.i_value_handlers import IValueHandlers; assert g.IValueHandlers is IValueHandlers; assert hasattr(g.IValueHandlers, 'from_value')"])
                    .current_dir(&fixture.0).output().unwrap());
            }
        }
        let previous = snapshot(&output);
        success(generate(
            &second,
            &output,
            lang,
            "Contoso.IValueHandlers",
            &[],
        ));
        assert_eq!(previous, snapshot(&output));
    }
}

#[test]
fn python_inventory_migration_uses_metadata_not_stub_text_and_fails_atomically() {
    let fixture = Fixture::new();
    let initial = fixture.0.join("initial").join("Initial.winmd");
    let extra = fixture.0.join("extra").join("Extra.winmd");
    names(&initial, &[("Original", "IFirst", "Read", 0x31d44741)]);
    names(&extra, &[("Extra", "ISecond", "Other", 0x31d44742)]);
    for (suffix, no_pyi) in [("typed", false), ("runtime", true)] {
        let output = fixture.0.join(suffix);
        success(generate(&initial, &output, "py", "Original.IFirst", &[]));
        let records = python_inventory(&output)
            .into_iter()
            .map(|mut record| {
                record.as_object_mut().unwrap().remove("schema_version");
                record.as_object_mut().unwrap().remove("implementation");
                record.to_string()
            })
            .collect::<Vec<_>>();
        fs::write(
            output.join(".dynwinrt-generated-types"),
            records.join("\n") + "\n",
        )
        .unwrap();
        if no_pyi {
            success(generate(
                &extra,
                &output,
                "py",
                "Extra.ISecond",
                &["--no-pyi"],
            ));
            assert!(!output.join("_implementation_types.pyi").exists());
        } else {
            let before = snapshot(&output);
            let rejected = generate(&extra, &output, "py", "Extra.ISecond", &[]);
            assert!(!rejected.status.success());
            let message = String::from_utf8_lossy(&rejected.stderr);
            assert!(
                message.contains("Original.IFirst") && message.contains("--ref"),
                "{message}"
            );
            assert_eq!(before, snapshot(&output));
            success(generate(
                &extra,
                &output,
                "py",
                "Extra.ISecond",
                &["--ref", initial.to_str().unwrap()],
            ));
            assert!(
                python_inventory(&output)
                    .iter()
                    .all(|record| record["schema_version"] == 2)
            );
            assert_eq!(
                fs::read_to_string(output.join("_implementation_types.pyi"))
                    .unwrap()
                    .matches(" import _ImplementationPair")
                    .count(),
                2
            );
        }
    }
}

#[test]
fn typed_append_after_no_pyi_requires_regenerating_missing_declarations() {
    let fixture = Fixture::new();
    let first = fixture.0.join("first").join("First.winmd");
    let second = fixture.0.join("second").join("Second.winmd");
    names(&first, &[("Original", "IFirst", "Read", 0x31d44771)]);
    names(&second, &[("Extra", "ISecond", "Other", 0x31d44772)]);
    let output = fixture.0.join("pyviews");
    success(generate(
        &first,
        &output,
        "py",
        "Original.IFirst",
        &["--no-pyi"],
    ));
    assert!(!output.join("original__i_first.pyi").exists());
    let before = snapshot(&output);
    let rejected = generate(&second, &output, "py", "Extra.ISecond", &[]);
    assert!(!rejected.status.success());
    let error = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        error.contains("retained .pyi") && error.contains("Original.IFirst"),
        "{error}"
    );
    assert_eq!(before, snapshot(&output));
    success(generate(
        &second,
        &output,
        "py",
        "Original.IFirst,Extra.ISecond",
        &["--ref", first.to_str().unwrap()],
    ));
    assert!(output.join("original__i_first.pyi").is_file());
    assert_eq!(
        fs::read_to_string(output.join("_implementation_types.pyi"))
            .unwrap()
            .matches(" import _ImplementationPair")
            .count(),
        2
    );
}

#[test]
fn legacy_interface_facade_cannot_override_a_current_runtime_class() {
    let fixture = Fixture::new();
    let first = fixture.0.join("first").join("First.winmd");
    names(&first, &[("Contoso", "IValue", "Read", 0x31d44781)]);
    let output = fixture.0.join("pyviews");
    success(generate(&first, &output, "py", "Contoso.IValue", &[]));
    let legacy = python_inventory(&output)
        .into_iter()
        .map(|mut record| {
            record.as_object_mut().unwrap().remove("schema_version");
            record.as_object_mut().unwrap().remove("implementation");
            record.to_string()
        })
        .collect::<Vec<_>>();
    fs::write(
        output.join(".dynwinrt-generated-types"),
        legacy.join("\n") + "\n",
    )
    .unwrap();
    let metadata = fixture.0.join("class.winmd");
    let mut file = writer::File::new("ClassPrecedence");
    let object = file.TypeRef("System", "Object");
    file.TypeDef(
        "Contoso",
        "IValue",
        writer::TypeDefOrRef::TypeRef(object),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    fs::write(&metadata, file.into_stream()).unwrap();
    success(generate(&metadata, &output, "py", "Contoso.IValue", &[]));
    let inventory = python_inventory(&output);
    assert!(
        inventory
            .iter()
            .any(|record| record["kind"] == "class" && record["identity"]["name"] == "IValue")
    );
    assert!(
        !inventory
            .iter()
            .any(|record| record["kind"] == "interface" && record["identity"]["name"] == "IValue")
    );
    for extension in ["py", "pyi"] {
        assert!(
            !fs::read_to_string(output.join(format!("__init__.{extension}")))
                .unwrap()
                .contains("IValueHandlers")
        );
        assert!(
            !fs::read_to_string(output.join(format!("contoso__i_value.{extension}")))
                .unwrap()
                .contains("def implement(")
        );
    }
    assert!(
        !fs::read_to_string(output.join("_implementation_types.pyi"))
            .unwrap()
            .contains("contoso__i_value")
    );
}

#[test]
fn legacy_migration_reuses_records_already_present_without_their_metadata() {
    let fixture = Fixture::new();
    let initial = fixture.0.join("initial").join("Initial.winmd");
    let extra = fixture.0.join("extra").join("Extra.winmd");
    names(&initial, &[("Original", "IFirst", "Read", 0x31d44741)]);
    names(&extra, &[("Extra", "ISecond", "Other", 0x31d44742)]);
    let third = fixture.0.join("third").join("Third.winmd");
    names(&third, &[("Last", "IThird", "Last", 0x31d44743)]);
    let output = fixture.0.join("partial");
    success(generate(&initial, &output, "py", "Original.IFirst", &[]));
    success(generate(&extra, &output, "py", "Extra.ISecond", &[]));
    let lines = python_inventory(&output)
        .into_iter()
        .map(|mut record| {
            if record["identity"]["name"] == "IFirst" {
                record.as_object_mut().unwrap().remove("schema_version");
                record.as_object_mut().unwrap().remove("implementation");
            }
            record.to_string()
        })
        .collect::<Vec<_>>();
    fs::write(
        output.join(".dynwinrt-generated-types"),
        lines.join("\n") + "\n",
    )
    .unwrap();
    let retained = fs::read(output.join("extra__i_second.py")).unwrap();
    success(generate(
        &third,
        &output,
        "py",
        "Last.IThird",
        &["--ref", initial.to_str().unwrap()],
    ));
    assert_eq!(
        retained,
        fs::read(output.join("extra__i_second.py")).unwrap()
    );
    assert!(
        python_inventory(&output)
            .iter()
            .all(|record| record["schema_version"] == 2)
    );
}
