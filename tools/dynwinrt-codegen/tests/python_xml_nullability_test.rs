// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::codegen::{python, python_stub};
use dynwinrt_codegen::meta::{self, InterfaceMeta, MethodMeta};
use dynwinrt_codegen::types::{TypeIdentity, TypeIdentityKind, TypeMeta};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
const VALID: &str = include_str!("fixtures/python_xml_nullability_valid.py");
const INVALID: &str = include_str!("fixtures/python_xml_nullability_invalid.py");
const NATIVE: &str = include_str!("fixtures/python_xml_nullability_native.py");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn class_and_standalone_interface_stubs_use_exact_xml_null_facts() {
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let stub = |name: &str| {
        fs::read_to_string(
            fixture
                .0
                .join("xml_generated")
                .join(format!("windows__data__xml__dom__{name}.pyi")),
        )
        .unwrap()
    };
    let contains = |text: &str, signature: &str| {
        assert!(text.contains(signature), "missing {signature}\n{text}");
    };
    for module in ["xml_document", "i_xml_document"] {
        let text = stub(module);
        for signature in [
            "def document_element(self) -> XmlElement | None: ...",
            "def doctype(self) -> XmlDocumentType | None: ...",
            "def get_element_by_id(self, element_id: str) -> XmlElement | None: ...",
            "def implementation(self) -> XmlDomImplementation: ...",
            "def create_element(self, tag_name: str) -> XmlElement: ...",
        ] {
            contains(&text, signature);
        }
    }
    for module in ["xml_document", "xml_element", "xml_text", "i_xml_node"] {
        let text = stub(module);
        for property in [
            "parent_node",
            "previous_sibling",
            "next_sibling",
            "first_child",
            "last_child",
        ] {
            contains(
                &text,
                &format!("def {property}(self) -> IXmlNode | None: ..."),
            );
        }
        contains(&text, "def owner_document(self) -> XmlDocument | None: ...");
        contains(&text, "def child_nodes(self) -> XmlNodeList: ...");
        contains(&text, "def clone_node(self, deep: bool) -> IXmlNode: ...");
    }
    for module in ["xml_element", "i_xml_element"] {
        let text = stub(module);
        for signature in [
            "def get_attribute_node(self, attribute_name: str) -> XmlAttribute | None: ...",
            "def get_attribute_node_ns(self, namespace_uri: 'DynWinRTValue | _DynWinRTObject', local_name: str) -> XmlAttribute | None: ...",
            "def set_attribute_node(self, new_attribute: 'XmlAttributeLike') -> XmlAttribute | None: ...",
            "def set_attribute_node_ns(self, new_attribute: 'XmlAttributeLike') -> XmlAttribute | None: ...",
            "def remove_attribute_node(self, attribute_node: 'XmlAttributeLike') -> XmlAttribute: ...",
        ] {
            contains(&text, signature);
        }
    }
    let runtime = fs::read_to_string(
        fixture
            .0
            .join("xml_generated")
            .join("windows__data__xml__dom__xml_document.py"),
    )
    .unwrap();
    contains(
        &runtime,
        "def implementation(self) -> XmlDomImplementation | None:",
    );
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

fn success(output: Output) {
    assert!(output.status.success(), "{}", diagnostics(&output));
}

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python"))
}

fn python_available(arguments: &[&str], required: &str) -> bool {
    let output = Command::new(python()).args(arguments).output().unwrap();
    assert!(
        output.status.success() || std::env::var(required).as_deref() != Ok("1"),
        "{required}=1: {}",
        diagnostics(&output),
    );
    if !output.status.success() {
        eprintln!("Skipping Python XML probe: {}", diagnostics(&output));
    }
    output.status.success()
}

impl Fixture {
    fn new() -> Option<Self> {
        let winmd = std::env::var_os("DYNWINRT_WINDOWS_WINMD")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(WINDOWS_WINMD));
        if !winmd.is_file() {
            assert_ne!(
                std::env::var("DYNWINRT_REQUIRE_XML_NULLABILITY").as_deref(),
                Ok("1"),
                "XML nullability tests require Windows.winmd",
            );
            eprintln!("Skipping XML nullability tests: Windows.winmd not found");
            return None;
        }
        let fixture = Self(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("target")
                .join(format!(
                    "px{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed),
                )),
        );
        fs::create_dir_all(&fixture.0).unwrap();
        let package = fixture.0.join("xml_generated");
        success(
            Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
                .args(["generate", "--winmd"])
                .arg(&winmd)
                .args([
                    "--class-name",
                    "Windows.Data.Xml.Dom.XmlDocument,Windows.Data.Xml.Dom.XmlElement,\
                     Windows.Data.Xml.Dom.XmlText,Windows.Data.Xml.Dom.XmlLoadSettings,\
                     Windows.Data.Xml.Dom.IXmlNode",
                    "--lang",
                    "py",
                    "--output",
                ])
                .arg(&package)
                .output()
                .unwrap(),
        );

        // Exclusive interfaces are not public CLI roots. Resolve their actual
        // declarations independently, then exercise the standalone renderer.
        let winmd = winmd.to_str().unwrap();
        let declarations = ["XmlDocument", "XmlElement"].map(|name| {
            meta::parse_class(winmd, "Windows.Data.Xml.Dom", name)
                .unwrap()
                .default_interface
                .unwrap()
        });
        let seed = InterfaceMeta {
            namespace: "Test".into(),
            name: "IXmlDeclarations".into(),
            methods: declarations
                .iter()
                .map(|interface| MethodMeta {
                    name: format!("Get{}", interface.name),
                    return_type: Some(TypeMeta::Interface {
                        namespace: interface.namespace.clone(),
                        name: interface.name.clone(),
                        iid: interface.iid.clone(),
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let resolved = meta::resolve_python_dependencies(winmd, &[], &[seed], &[]);
        let context = python::PythonProjectionContext::new(
            resolved
                .classes
                .iter()
                .map(|class| {
                    TypeIdentity::named(TypeIdentityKind::Class, &class.namespace, &class.name)
                })
                .chain(resolved.interfaces.iter().map(InterfaceMeta::type_identity))
                .chain(resolved.enums.iter().map(TypeMeta::type_identity)),
            true,
        )
        .unwrap();
        for declaration in &declarations {
            let interface = resolved
                .interfaces
                .iter()
                .find(|interface| {
                    interface.namespace == declaration.namespace
                        && interface.name == declaration.name
                })
                .unwrap();
            assert_eq!(interface.iid, declaration.iid);
            let module = context.implementation_module(&interface.type_identity());
            fs::write(
                package.join(format!("{module}.py")),
                python::generate_interface(&context, interface),
            )
            .unwrap();
            fs::write(
                package.join(format!("{module}.pyi")),
                python_stub::generate_interface_stub(&context, interface),
            )
            .unwrap();
        }
        Some(fixture)
    }
}

#[test]
fn guarded_xml_reads_pass_and_unguarded_reads_fail_strict_typechecking() {
    if !python_available(&["-m", "mypy", "--version"], "DYNWINRT_REQUIRE_MYPY") {
        return;
    }
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let expected_lines = INVALID
        .lines()
        .enumerate()
        .filter_map(|(index, line)| line.ends_with("# unsafe").then_some(index + 1))
        .collect::<BTreeSet<_>>();
    assert_eq!(expected_lines.len(), 20);
    for (name, consumer) in [("valid.py", VALID), ("invalid.py", INVALID)] {
        fs::write(fixture.0.join(name), consumer).unwrap();
        let output = Command::new(python())
            .args([
                "-B",
                "-m",
                "mypy",
                "--strict",
                "--no-incremental",
                "--no-pretty",
                "--show-error-codes",
            ])
            .arg(name)
            .current_dir(&fixture.0)
            .env_remove("MYPYPATH")
            .output()
            .unwrap();
        let text = diagnostics(&output);
        if name == "valid.py" {
            success(output);
        } else {
            assert_eq!(output.status.code(), Some(1), "{text}");
            let errors = text
                .lines()
                .filter(|line| line.contains(": error:"))
                .collect::<Vec<_>>();
            assert_eq!(errors.len(), expected_lines.len(), "{text}");
            let lines = errors
                .iter()
                .map(|line| {
                    assert!(line.starts_with("invalid.py:"), "{text}");
                    assert!(line.ends_with("[union-attr]"), "{text}");
                    line.split(':').nth(1).unwrap().parse().unwrap()
                })
                .collect::<BTreeSet<usize>>();
            assert_eq!(lines, expected_lines, "{text}");
        }

        if let Some(pyright) = std::env::var_os("DYNWINRT_PYRIGHT") {
            let output = Command::new(pyright)
                .args(["--pythonpath"])
                .arg(python())
                .args(["--outputjson", name])
                .current_dir(&fixture.0)
                .output()
                .unwrap();
            let text = diagnostics(&output);
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let errors = report["generalDiagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|diagnostic| diagnostic["severity"] == "error")
                .collect::<Vec<_>>();
            if name == "valid.py" {
                assert!(errors.is_empty(), "{text}");
                success(output);
            } else {
                assert_eq!(output.status.code(), Some(1), "{text}");
                assert_eq!(errors.len(), expected_lines.len(), "{text}");
                let lines = errors
                    .iter()
                    .map(|error| {
                        assert_eq!(error["rule"], "reportOptionalMemberAccess", "{text}");
                        assert!(
                            Path::new(error["file"].as_str().unwrap()).ends_with(name),
                            "{text}",
                        );
                        error["range"]["start"]["line"].as_u64().unwrap() as usize + 1
                    })
                    .collect::<BTreeSet<_>>();
                assert_eq!(lines, expected_lines, "{text}");
            }
        }
    }
}

#[test]
fn generated_xml_results_match_native_null_and_non_null_states() {
    if !python_available(
        &[
            "-c",
            "from dynwinrt import RoApartment, projected_lifetime_scope",
        ],
        "DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME",
    ) {
        return;
    }
    let Some(fixture) = Fixture::new() else {
        return;
    };
    let output = Command::new(python())
        .args(["-B", "-c"])
        .arg(format!("import sys; sys.path.insert(0, '.')\n{NATIVE}"))
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    println!("{}", diagnostics(&output));
    success(output);
}
