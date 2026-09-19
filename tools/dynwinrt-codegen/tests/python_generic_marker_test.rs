// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use dynwinrt_codegen::codegen::{python::PythonProjectionContext, python_stub};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta};
use dynwinrt_codegen::types::{TypeIdentity, TypeIdentityKind, TypeKind, TypeMeta, TypeRef};
use windows_metadata::{
    FieldAttributes, GenericParamAttributes, MethodAttributes, MethodCallAttributes,
    MethodImplAttributes, ParamAttributes, Signature, Type, TypeAttributes, TypeName, Value,
    writer,
};

const PIID: &str = "62a02000-6281-4900-b782-040302010910";

fn payload(namespace: &str, name: &str) -> TypeMeta {
    TypeMeta::Struct {
        namespace: namespace.into(),
        name: name.into(),
        fields: vec![],
    }
}

fn token(arguments: Vec<TypeMeta>) -> InterfaceMeta {
    InterfaceMeta {
        namespace: "Boxes".into(),
        name: "IToken_Payload".into(),
        generic_name: Some("IToken`1".into()),
        generic_piid: Some(PIID.into()),
        generic_args: arguments,
        methods: vec![MethodMeta {
            name: "Ping".into(),
            vtable_index: 6,
            return_type: Some(TypeMeta::I32),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn markers(stub: &str) -> Vec<String> {
    stub.lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("def _dynwinrt_iid_")
                .and_then(|line| line.strip_suffix("(self) -> None: ..."))
                .map(str::to_string)
        })
        .collect()
}

fn marker(context: &PythonProjectionContext, interface: &InterfaceMeta) -> String {
    let stub = python_stub::generate_interface_stub(context, interface);
    assert!(stub.contains("def ping(self) -> int: ..."), "{stub}");
    let markers = markers(&stub);
    assert_eq!(markers.len(), 1, "{stub}");
    markers[0].clone()
}

#[test]
fn closed_generic_markers_preserve_recursive_case_namespace_kind_and_argument_order() {
    let named_arguments = [
        payload("Alpha", "Payload"),
        payload("Beta", "Payload"),
        payload("alpha", "Payload"),
        payload("Alpha", "payload"),
        payload("Alpha.Beta", "Payload"),
        payload("Alpha_Beta", "Payload"),
        TypeMeta::Interface {
            namespace: "Alpha".into(),
            name: "Payload".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
        },
        TypeMeta::RuntimeClass {
            namespace: "Alpha".into(),
            name: "Payload".into(),
            default_interface: None,
        },
        TypeMeta::Delegate {
            namespace: "Alpha".into(),
            name: "Payload".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
        },
        TypeMeta::Enum {
            namespace: "Alpha".into(),
            name: "Payload".into(),
            underlying: Box::new(TypeMeta::I32),
            members: vec![],
            is_flags: false,
            doc: None,
            deprecated: None,
        },
        TypeMeta::I32,
        TypeMeta::U32,
    ];
    let mut interfaces = named_arguments
        .iter()
        .cloned()
        .map(|argument| token(vec![argument]))
        .collect::<Vec<_>>();
    for argument in &named_arguments {
        interfaces.push(token(vec![TypeMeta::Parameterized {
            namespace: "Boxes".into(),
            name: "IToken`1".into(),
            piid: PIID.into(),
            args: vec![argument.clone()],
        }]));
    }
    for args in [
        vec![named_arguments[0].clone(), named_arguments[1].clone()],
        vec![named_arguments[1].clone(), named_arguments[0].clone()],
    ] {
        let mut pair = token(args);
        pair.generic_name = Some("IPair`2".into());
        interfaces.push(pair);
    }
    for (namespace, definition) in [
        ("OtherBoxes", "IToken`1"),
        ("boxes", "IToken`1"),
        ("Boxes", "itoken`1"),
        ("Boxes", "IOther`1"),
    ] {
        let mut interface = token(vec![named_arguments[0].clone()]);
        interface.namespace = namespace.into();
        interface.generic_name = Some(definition.into());
        interfaces.push(interface);
    }
    let mut seen = HashSet::new();
    for interface in interfaces {
        let context = PythonProjectionContext::new([interface.type_identity()], true).unwrap();
        assert!(
            seen.insert(marker(&context, &interface)),
            "collapsed identity: {:?}",
            interface.type_identity()
        );
    }
}

#[test]
fn closed_generic_markers_ignore_projection_context_order_aliases_and_unresolved_iids() {
    let alpha = token(vec![payload("Alpha", "Payload")]);
    let beta = token(vec![payload("Beta", "Payload")]);
    let standalone = PythonProjectionContext::new([alpha.type_identity()], true).unwrap();
    let expected = marker(&standalone, &alpha);
    for identities in [
        vec![alpha.type_identity(), beta.type_identity()],
        vec![beta.type_identity(), alpha.type_identity()],
    ] {
        for packaged in [false, true] {
            let context = PythonProjectionContext::new(identities.clone(), packaged).unwrap();
            assert_ne!(
                context.projected_name_for_interface(&alpha),
                standalone.projected_name_for_interface(&alpha)
            );
            assert_eq!(marker(&context, &alpha), expected);
            let mut renamed = alpha.clone();
            renamed.name = "AnIndependentLocalAlias".into();
            renamed.generic_name = Some("IToken".into());
            renamed.iid = "11111111-1111-1111-1111-111111111111".into();
            assert_eq!(marker(&context, &renamed), expected);
        }
    }
    // Older hand-built metadata may omit generic_name. Resolve its semantic
    // identity before the projection supplies a different local class name.
    let mut legacy = alpha.clone();
    legacy.generic_name = None;
    let peer = TypeIdentity::named(TypeIdentityKind::Interface, "Boxes", &legacy.name);
    let context = PythonProjectionContext::new([legacy.type_identity(), peer], true).unwrap();
    assert_eq!(marker(&context, &legacy), expected);
}

#[test]
fn class_default_required_and_inherited_identity_views_share_generic_markers() {
    let alpha = token(vec![payload("Alpha", "Payload")]);
    let mut beta = token(vec![payload("Beta", "Payload")]);
    beta.name = "LocalBetaAlias".into();
    let base = ClassMeta {
        namespace: "Boxes".into(),
        name: "Holder".into(),
        full_name: "Boxes.Holder".into(),
        default_interface: Some(alpha.clone()),
        required_interfaces: vec![alpha.clone(), beta.clone()],
        ..Default::default()
    };
    let derived = ClassMeta {
        namespace: "Boxes".into(),
        name: "Derived".into(),
        full_name: "Boxes.Derived".into(),
        base_class: Some(TypeRef {
            namespace: "Boxes".into(),
            name: "Holder".into(),
            kind: TypeKind::Class,
        }),
        required_interfaces: vec![beta.clone()],
        ..Default::default()
    };
    let context = PythonProjectionContext::new(
        [
            alpha.type_identity(),
            beta.type_identity(),
            TypeIdentity::named(TypeIdentityKind::Class, "Boxes", "Holder"),
            TypeIdentity::named(TypeIdentityKind::Class, "Boxes", "Derived"),
        ],
        true,
    )
    .unwrap();
    let alpha_marker = marker(&context, &alpha);
    let beta_marker = marker(&context, &beta);
    assert_ne!(alpha_marker, beta_marker);
    let stub = python_stub::generate_class_stub(&context, &base, &HashSet::new());
    let mut expected = vec![alpha_marker, beta_marker.clone()];
    expected.sort();
    assert_eq!(markers(&stub), expected, "{stub}");
    assert!(stub.contains("def _dynwinrt_class_boxes_holder(self) -> None: ..."));
    let stub = python_stub::generate_class_stub(&context, &derived, &HashSet::new());
    assert!(
        stub.contains("from .boxes__holder import _HolderIdentity"),
        "{stub}"
    );
    assert!(
        stub.contains("class _DerivedIdentity(_HolderIdentity, Protocol):"),
        "{stub}"
    );
    assert_eq!(markers(&stub), [beta_marker]);
    assert!(stub.contains(
        "def as_interface(self, interface_class: _DynWinRTProjector[_InterfaceT]) -> _InterfaceT:"
    ));
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("python-generic-markers-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn define_interface(
    file: &mut writer::File,
    namespace: &str,
    name: &str,
    id: u32,
) -> writer::TypeDef {
    let interface = file.TypeDef(
        namespace,
        name,
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public
            | TypeAttributes::Interface
            | TypeAttributes::Abstract
            | TypeAttributes::WindowsRuntime,
    );
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
    interface
}

fn define_method(file: &mut writer::File, name: &str, result: Type, params: Vec<Type>) {
    file.MethodDef(
        name,
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: result,
            types: params.clone(),
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    for index in 0..params.len() {
        file.Param("value", index as u16 + 1, ParamAttributes::In);
    }
}

fn write_metadata(path: &Path, namespace: &str) {
    let mut file = writer::File::new("GenericMarkers");
    let base = file.TypeRef("System", "ValueType");
    file.TypeDef(
        namespace,
        "Payload",
        writer::TypeDefOrRef::TypeRef(base),
        TypeAttributes::Public
            | TypeAttributes::Sealed
            | TypeAttributes::SequentialLayout
            | TypeAttributes::WindowsRuntime,
    );
    file.Field("Value", &Type::I32, FieldAttributes::Public);
    let interface = define_interface(&mut file, "Boxes", "IToken`1", 0x62a02000);
    file.GenericParam(
        "T",
        writer::TypeOrMethodDef::TypeDef(interface),
        0,
        GenericParamAttributes::default(),
    );
    define_method(&mut file, "Ping", Type::I32, vec![]);
    define_interface(
        &mut file,
        namespace,
        "IUse",
        if namespace == "Alpha" {
            0x62a02001
        } else {
            0x62a02002
        },
    );
    let payload = Type::named(namespace, "Payload");
    let closed = Type::Name(TypeName {
        namespace: "Boxes".into(),
        name: "IToken`1".into(),
        generics: vec![payload.clone()],
    });
    define_method(&mut file, "UseToken", Type::I32, vec![closed]);
    define_method(&mut file, "Echo", payload.clone(), vec![payload]);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn independently_generated_packages_reject_wrong_closures_and_accept_identical_closures() {
    let fixture = Fixture::new();
    for (package, namespace) in [
        ("alpha", "Alpha"),
        ("beta", "Beta"),
        ("alpha_copy", "Alpha"),
    ] {
        let metadata = fixture.0.join("inputs").join(package).join("Input.winmd");
        write_metadata(&metadata, namespace);
        let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args(["generate", "--winmd"])
            .arg(&metadata)
            .args([
                "--class-name",
                &format!("{namespace}.IUse"),
                "--lang",
                "py",
                "--output",
            ])
            .arg(fixture.0.join(package))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", diagnostics(&output));
    }
    let python = std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python"));
    if !Command::new(&python)
        .args(["-m", "mypy", "--version"])
        .output()
        .is_ok_and(|output| output.status.success())
    {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref(),
            Ok("1"),
            "DYNWINRT_REQUIRE_MYPY=1 but mypy is unavailable"
        );
        eprintln!("Skipping static consumer checks: mypy unavailable; set DYNWINRT_TEST_PYTHON.");
        return;
    }
    let imports = "\
from typing import assert_type
from alpha.boxes.i_token_payload import IToken_Payload as AlphaToken
from beta.boxes.i_token_payload import IToken_Payload as BetaToken
from alpha_copy.boxes.i_token_payload import IToken_Payload as SameToken
from alpha.alpha.i_use import IUse as AlphaUse
from alpha.alpha.payload import Payload as AlphaPayload
from beta.beta.payload import Payload as BetaPayload
";
    let valid = format!(
        "{imports}\n\
def same_identity(use: AlphaUse, token: SameToken, payload: AlphaPayload) -> AlphaToken:
    assert_type(token, SameToken)
    assert_type(token.ping(), int)
    assert_type(use.use_token(token), int)
    assert_type(use.echo(payload), AlphaPayload)
    return token

def reverse_identity(token: AlphaToken) -> SameToken:
    return token
"
    );
    let invalid = format!(
        "{imports}\n\
def wrong_identity(use: AlphaUse, token: BetaToken) -> AlphaToken:
    assert_type(token, BetaToken)
    assert_type(use.use_token(token), int)
    assigned: AlphaToken = token
    return token

def ordinary_control(use: AlphaUse, payload: BetaPayload) -> AlphaPayload:
    return use.echo(payload)
"
    );
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("bindings")
        .join("py");
    for (name, source) in [("valid", valid), ("invalid", invalid)] {
        let consumer = format!("{name}.py");
        fs::write(fixture.0.join(&consumer), source).unwrap();
        let output = Command::new(&python)
            .args([
                "-B",
                "-m",
                "mypy",
                "--strict",
                "--no-incremental",
                "--follow-imports=normal",
                "--no-pretty",
                "--show-error-codes",
                "--cache-dir",
                ".mypy_cache",
                "alpha",
                "beta",
                "alpha_copy",
                &consumer,
            ])
            .current_dir(&fixture.0)
            .env("MYPYPATH", &runtime)
            .output()
            .unwrap();
        let text = diagnostics(&output);
        if name == "valid" {
            assert!(output.status.success(), "{text}");
        } else {
            assert_eq!(output.status.code(), Some(1), "{text}");
            let errors = text
                .lines()
                .filter(|line| line.contains(": error:"))
                .collect::<Vec<_>>();
            assert_eq!(errors.len(), 4, "{text}");
            assert!(
                errors.iter().all(|line| line.starts_with("invalid.py:")),
                "{text}"
            );
            assert_eq!(
                errors
                    .iter()
                    .filter(|line| line.contains("[arg-type]"))
                    .count(),
                2,
                "{text}"
            );
            assert_eq!(
                errors
                    .iter()
                    .filter(|line| line.contains("[assignment]"))
                    .count(),
                1,
                "{text}"
            );
            assert_eq!(
                errors
                    .iter()
                    .filter(|line| line.contains("[return-value]"))
                    .count(),
                1,
                "{text}"
            );
        }
    }
}
