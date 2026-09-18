// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::codegen::{project, python, python_stub, render_dts, render_js};
use dynwinrt_codegen::meta::{self, InterfaceMeta, ParamDirection};
use dynwinrt_codegen::types::TypeMeta;
use windows_metadata::{
    AsRow, FieldAttributes, GenericParamAttributes, MethodAttributes, MethodCallAttributes,
    MethodImplAttributes, ParamAttributes, Signature, Type, TypeAttributes, TypeName, Value,
    reader, writer,
};

const NAMESPACE: &str = "Tests.RawSignature";
const BYREF: u8 = 0x10;
const PTR: u8 = 0x0f;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn parse(bytes: Vec<u8>) -> InterfaceMeta {
        let directory = Self(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!(
                    "winrt-raw-signature-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                )),
        );
        fs::create_dir_all(&directory.0).unwrap();
        let path = directory.0.join("Input.winmd");
        fs::write(&path, bytes).unwrap();
        meta::parse_interfaces(path.to_str().unwrap(), NAMESPACE)
            .into_iter()
            .find(|interface| interface.name == "IProbe")
            .expect("writer-authored WinMD must contain IProbe")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn add_iid(file: &mut writer::File, definition: writer::HasAttribute, id: u32) {
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
    file.Attribute(
        definition,
        writer::AttributeType::MemberRef(constructor),
        &[
            Value::U32(id),
            Value::U16(0x7ba5),
            Value::U16(0x4036),
            Value::U8(0xa0),
            Value::U8(0x24),
            Value::U8(1),
            Value::U8(2),
            Value::U8(3),
            Value::U8(4),
            Value::U8(5),
            Value::U8(6),
        ]
        .into_iter()
        .map(|value| (String::new(), value))
        .collect::<Vec<_>>(),
    );
}

fn add_method(
    file: &mut writer::File,
    name: &str,
    parameters: &[(String, Type, ParamAttributes)],
    return_type: Type,
) {
    file.MethodDef(
        name,
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type,
            types: parameters.iter().map(|(_, typ, _)| typ.clone()).collect(),
        },
        MethodAttributes::Public
            | MethodAttributes::Virtual
            | MethodAttributes::Abstract
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    for (position, (name, _, flags)) in parameters.iter().enumerate() {
        file.Param(name, (position + 1).try_into().unwrap(), *flags);
    }
}

fn fixture(
    parameters: Vec<(String, Type, ParamAttributes)>,
    return_type: Type,
    delegate: bool,
) -> Vec<u8> {
    fixture_file(parameters, return_type, delegate).into_stream()
}

fn fixture_file(
    parameters: Vec<(String, Type, ParamAttributes)>,
    return_type: Type,
    delegate: bool,
) -> writer::File {
    let mut file = writer::File::new("RawSignatureControls");
    let interface_flags = TypeAttributes::Public
        | TypeAttributes::Abstract
        | TypeAttributes::Interface
        | TypeAttributes::WindowsRuntime;
    let interface = file.TypeDef(
        NAMESPACE,
        "IProbe",
        writer::TypeDefOrRef::default(),
        interface_flags,
    );
    add_iid(
        &mut file,
        writer::HasAttribute::TypeDef(interface),
        0x509f6031,
    );
    if delegate {
        add_method(
            &mut file,
            "Take",
            &[parameter(
                "callback",
                Type::named(NAMESPACE, "Callback"),
                ParamAttributes::In,
            )],
            Type::Void,
        );
    } else {
        add_method(&mut file, "Take", &parameters, return_type.clone());
    }

    let generic = file.TypeDef(
        NAMESPACE,
        "IBox`1",
        writer::TypeDefOrRef::default(),
        interface_flags,
    );
    add_iid(
        &mut file,
        writer::HasAttribute::TypeDef(generic),
        0x509f6032,
    );
    file.GenericParam(
        "T",
        writer::TypeOrMethodDef::TypeDef(generic),
        0,
        GenericParamAttributes::default(),
    );
    let modifier = file.TypeDef(
        NAMESPACE,
        "IgnoredModifier",
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public,
    );
    assert_eq!(writer::TypeDefOrRef::TypeDef(modifier).encode(), 0x10);

    if delegate {
        let base = file.TypeRef("System", "MulticastDelegate");
        let callback = file.TypeDef(
            NAMESPACE,
            "Callback",
            writer::TypeDefOrRef::TypeRef(base),
            TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
        );
        add_iid(
            &mut file,
            writer::HasAttribute::TypeDef(callback),
            0x509f6033,
        );
        add_method(&mut file, ".ctor", &[], Type::Void);
        add_method(&mut file, "Invoke", &parameters, return_type);
    }
    file
}

fn parameter(name: &str, typ: Type, flags: ParamAttributes) -> (String, Type, ParamAttributes) {
    (name.into(), typ, flags)
}

fn with_method<T>(
    bytes: &[u8],
    delegate: bool,
    read: impl FnOnce(reader::MethodDef<'_>, usize) -> T,
) -> T {
    let image = bytes.to_vec();
    let image_address = image.as_ptr() as usize;
    let index = reader::Index::new(vec![reader::File::new(image).unwrap()]);
    let definition = index
        .get(NAMESPACE, if delegate { "Callback" } else { "IProbe" })
        .next()
        .unwrap();
    let method = definition
        .methods()
        .find(|method| method.name() == if delegate { "Invoke" } else { "Take" })
        .unwrap();
    read(method, image_address)
}

fn raw_signature(bytes: &[u8], delegate: bool) -> Vec<u8> {
    with_method(bytes, delegate, |method, _| {
        let mut blob = method.blob(4);
        (0..blob.len()).map(|_| blob.read_u8()).collect()
    })
}

fn patch_signature(mut bytes: Vec<u8>, delegate: bool, edit: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let (offset, original) = with_method(&bytes, delegate, |method, image_address| {
        let mut blob = method.blob(4);
        // File::new retains its input Vec; locate this MethodDef's actual blob,
        // not an incidental matching byte sequence elsewhere in the PE image.
        let offset = (blob.as_ptr() as usize).checked_sub(image_address).unwrap();
        let signature = (0..blob.len()).map(|_| blob.read_u8()).collect::<Vec<_>>();
        (offset, signature)
    });
    let payload = bytes.get_mut(offset..offset + original.len()).unwrap();
    assert_eq!(payload, original);
    edit(payload);
    let expected = payload.to_vec();
    assert_eq!(raw_signature(&bytes, delegate), expected);
    bytes
}

fn pointer_to_byref(bytes: Vec<u8>, delegate: bool) -> Vec<u8> {
    patch_signature(bytes, delegate, |signature| {
        let pointer = signature.len() - 2;
        assert_eq!(&signature[pointer..], &[PTR, 0x08]);
        signature[pointer] = BYREF;
    })
}

fn generated(interface: &InterfaceMeta) -> [String; 4] {
    let known = HashSet::from([interface.name.clone()]);
    let projected = project::project_interface(
        &Default::default(),
        interface,
        &known,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let context = common::projection_context(
        &[],
        std::slice::from_ref(interface),
        &known,
        &HashSet::new(),
    );
    [
        render_js::render(&projected),
        render_dts::render(&projected),
        python::generate_interface(&context, interface),
        python_stub::generate_interface_stub(&context, interface),
    ]
}

fn assert_implementable(interface: &InterfaceMeta) {
    assert!(
        interface.implementation_metadata.diagnostics.is_empty(),
        "{:?}",
        interface.implementation_metadata.diagnostics
    );
    let [js, dts, py, pyi] = generated(interface);
    assert!(js.contains("DynWinRtImplementationHandle.create"), "{js}");
    assert!(
        dts.contains("IProbeHandlers") && !dts.contains("handlers: never"),
        "{dts}"
    );
    assert!(py.contains("DynWinRTImplementationHandle._create"), "{py}");
    assert!(pyi.contains("def implement("), "{pyi}");
}

fn assert_rejected(interface: &InterfaceMeta, diagnostic: &str) {
    assert!(
        interface
            .implementation_metadata
            .diagnostics
            .iter()
            .any(|reason| reason.contains(diagnostic)),
        "missing {diagnostic:?} in {:?}",
        interface.implementation_metadata.diagnostics
    );
    let [js, dts, py, pyi] = generated(interface);
    for (extension, output) in [("js", &js), ("d.ts", &dts), ("py", &py), ("pyi", &pyi)] {
        assert!(output.contains(diagnostic), "{extension}: {output}");
        assert!(output.contains("class IProbe"), "{extension}: {output}");
    }
    assert!(!js.contains("DynWinRtImplementationHandle.create"));
    assert!(!py.contains("DynWinRTImplementationHandle._create"));
    assert!(dts.contains("handlers: never"));
    assert!(!pyi.contains("def implement("));
}

#[test]
fn scalar_byref_input_fails_closed_without_changing_outbound_metadata() {
    for flags in [ParamAttributes::In, ParamAttributes::default()] {
        let value = fixture(
            vec![parameter("value", Type::I32, flags)],
            Type::Void,
            false,
        );
        let pointer = fixture(
            vec![parameter(
                "value",
                Type::PtrMut(Box::new(Type::I32), 1),
                flags,
            )],
            Type::Void,
            false,
        );
        let byref = pointer_to_byref(pointer.clone(), false);
        assert_eq!(raw_signature(&value, false), [0x20, 1, 1, 0x08]);
        assert_eq!(raw_signature(&pointer, false), [0x20, 1, 1, PTR, 0x08]);
        assert_eq!(raw_signature(&byref, false), [0x20, 1, 1, BYREF, 0x08]);
        for bytes in [&value, &byref] {
            assert_eq!(
                with_method(bytes, false, |method, _| method.signature(&[]).types),
                [Type::I32],
                "upstream 0.59 erases scalar BYREF"
            );
        }
        assert_eq!(
            with_method(&pointer, false, |method, _| method.signature(&[]).types),
            [Type::PtrMut(Box::new(Type::I32), 1)]
        );
        let value = Fixture::parse(value);
        let byref = Fixture::parse(byref);
        assert_implementable(&value);
        assert_rejected(&Fixture::parse(pointer), "unsupported native type PtrMut");
        assert_rejected(&byref, "input BYREF");
        assert!(
            byref
                .implementation_metadata
                .diagnostics
                .iter()
                .any(|reason| {
                    reason.contains("Take") && reason.contains("value") && reason.contains("BYREF")
                })
        );
        assert_eq!(
            format!("{:?}", value.methods),
            format!("{:?}", byref.methods),
            "reverse evidence must not rewrite permissive outbound metadata"
        );
        let [js, dts, py, pyi] = generated(&byref);
        assert!(
            js.contains("take(value)") && js.contains(".addIn(DynWinRtType.i32())"),
            "{js}"
        );
        assert!(dts.contains("take(value: number): void"), "{dts}");
        assert!(
            py.contains("def take(") && py.contains(".add_in(DynWinRTType.i32_type())"),
            "{py}"
        );
        assert!(
            pyi.contains("value: int") && pyi.contains("def take("),
            "{pyi}"
        );
    }
}

#[test]
fn modifiers_are_tokens_not_reference_markers_and_cannot_hide_byref() {
    for modifier in [0x1f, 0x20] {
        for byref in [false, true] {
            let inner = if byref {
                Type::PtrMut(Box::new(Type::I32), 1)
            } else {
                Type::I32
            };
            let bytes = fixture(
                vec![parameter(
                    "value",
                    Type::ConstRef(Box::new(inner)),
                    ParamAttributes::In,
                )],
                Type::Void,
                false,
            );
            let bytes = patch_signature(bytes, false, |signature| {
                assert_eq!(&signature[..4], &[0x20, 1, 1, 0x1f]);
                assert!(signature[4] < 0x80);
                signature[3] = modifier;
                signature[4] = 0x10; // TypeDefOrRefEncoded for IgnoredModifier, not BYREF.
                if byref {
                    assert_eq!(&signature[5..], &[PTR, 0x08]);
                    signature[5] = BYREF;
                } else {
                    assert_eq!(&signature[5..], &[0x08]);
                }
            });
            let interface = Fixture::parse(bytes);
            if byref {
                assert_rejected(&interface, "input BYREF");
            } else {
                assert_implementable(&interface);
            }
        }
    }
}

fn boxed(typ: Type) -> Type {
    Type::Name(TypeName {
        namespace: NAMESPACE.into(),
        name: "IBox`1".into(),
        generics: vec![typ],
    })
}

#[test]
fn scalar_out_and_pass_fill_receive_arrays_preserve_their_contracts() {
    for delegate in [false, true] {
        let bytes = fixture(
            vec![
                parameter(
                    "pass",
                    Type::Array(Box::new(Type::I32)),
                    ParamAttributes::In,
                ),
                parameter(
                    "fill",
                    Type::Array(Box::new(Type::I32)),
                    ParamAttributes::Out,
                ),
                parameter(
                    "receive",
                    Type::ArrayRef(Box::new(Type::I32)),
                    ParamAttributes::Out,
                ),
                parameter("plainScalar", Type::I32, ParamAttributes::Out),
                parameter(
                    "scalar",
                    Type::PtrMut(Box::new(Type::I32), 1),
                    ParamAttributes::Out,
                ),
            ],
            Type::Array(Box::new(Type::I32)),
            delegate,
        );
        let interface = Fixture::parse(pointer_to_byref(bytes, delegate));
        assert_implementable(&interface);
        let method = if delegate {
            &interface.implementation_metadata.delegates[0].invoke
        } else {
            &interface.methods[0]
        };
        assert_eq!(
            method
                .params
                .iter()
                .map(|parameter| &parameter.direction)
                .collect::<Vec<_>>(),
            [
                &ParamDirection::In,
                &ParamDirection::OutFill,
                &ParamDirection::Out,
                &ParamDirection::Out,
                &ParamDirection::Out
            ]
        );
        assert_eq!(method.params[3].typ, TypeMeta::I32);
        assert_eq!(method.params[4].typ, TypeMeta::I32);
        assert_eq!(
            method.return_type,
            Some(TypeMeta::Array(Box::new(TypeMeta::I32)))
        );
    }
}

#[test]
fn nested_generic_values_and_counts_do_not_shift_parameter_evidence() {
    let nested = boxed(boxed(Type::I32));
    let mut parameters = vec![
        parameter("nested", nested.clone(), ParamAttributes::In),
        parameter(
            "pass",
            Type::Array(Box::new(nested.clone())),
            ParamAttributes::In,
        ),
        parameter(
            "fill",
            Type::Array(Box::new(nested.clone())),
            ParamAttributes::Out,
        ),
        parameter(
            "receive",
            Type::ArrayRef(Box::new(nested.clone())),
            ParamAttributes::Out,
        ),
    ];
    parameters
        .extend((4..16).map(|position| {
            parameter(&format!("value{position}"), Type::I32, ParamAttributes::In)
        }));
    let valid = fixture(parameters.clone(), nested.clone(), false);
    assert_eq!(
        raw_signature(&valid, false)[1],
        BYREF,
        "parameter count is not a type qualifier"
    );
    assert_implementable(&Fixture::parse(valid));
    parameters.last_mut().unwrap().1 = Type::PtrMut(Box::new(Type::I32), 1);
    let byref = pointer_to_byref(fixture(parameters, nested, false), false);
    let interface = Fixture::parse(byref);
    assert_rejected(&interface, "input BYREF");
    assert!(
        interface
            .implementation_metadata
            .diagnostics
            .iter()
            .any(|reason| reason.contains("value15"))
    );
}

#[test]
fn delegate_invoke_cannot_bypass_raw_input_reference_validation() {
    let value = fixture(
        vec![parameter("value", Type::I32, ParamAttributes::In)],
        Type::I32,
        true,
    );
    assert_implementable(&Fixture::parse(value));
    let pointer = fixture(
        vec![parameter(
            "value",
            Type::PtrMut(Box::new(Type::I32), 1),
            ParamAttributes::In,
        )],
        Type::I32,
        true,
    );
    assert_rejected(
        &Fixture::parse(pointer.clone()),
        "unsupported native type PtrMut",
    );
    let byref = Fixture::parse(pointer_to_byref(pointer, true));
    assert_rejected(&byref, "input BYREF");
    assert!(
        byref
            .implementation_metadata
            .diagnostics
            .iter()
            .any(|reason| {
                reason.contains("Callback") && reason.contains("Invoke") && reason.contains("value")
            })
    );
}

#[test]
fn byref_return_is_not_erased_into_a_logical_value_output() {
    for delegate in [false, true] {
        let value = fixture(vec![], Type::I32, delegate);
        assert_implementable(&Fixture::parse(value));
        let pointer = fixture(vec![], Type::PtrMut(Box::new(Type::I32), 1), delegate);
        let byref = pointer_to_byref(pointer, delegate);
        assert_eq!(raw_signature(&byref, delegate), [0x20, 0, BYREF, 0x08]);
        assert_eq!(
            with_method(&byref, delegate, |method, _| method
                .signature(&[])
                .return_type),
            Type::I32
        );
        assert_rejected(&Fixture::parse(byref), "BYREF return");
    }
}

fn struct_fixture(field_type: Type, delegate: bool) -> Vec<u8> {
    let mut file = fixture_file(
        vec![parameter(
            "container",
            Type::named(NAMESPACE, "Container"),
            ParamAttributes::In,
        )],
        Type::Void,
        delegate,
    );
    let base = file.TypeRef("System", "ValueType");
    let flags = TypeAttributes::Public
        | TypeAttributes::Sealed
        | TypeAttributes::SequentialLayout
        | TypeAttributes::WindowsRuntime;
    file.TypeDef(
        NAMESPACE,
        "Payload",
        writer::TypeDefOrRef::TypeRef(base),
        flags,
    );
    file.Field("Value", &field_type, FieldAttributes::Public);
    file.TypeDef(
        NAMESPACE,
        "Container",
        writer::TypeDefOrRef::TypeRef(base),
        flags,
    );
    file.Field(
        "Payload",
        &Type::named(NAMESPACE, "Payload"),
        FieldAttributes::Public,
    );
    file.into_stream()
}

fn with_field<T>(bytes: &[u8], read: impl FnOnce(reader::Field<'_>, usize) -> T) -> T {
    let image = bytes.to_vec();
    let image_address = image.as_ptr() as usize;
    let index = reader::Index::new(vec![reader::File::new(image).unwrap()]);
    let definition = index.get(NAMESPACE, "Payload").next().unwrap();
    let field = definition.fields().next().unwrap();
    read(field, image_address)
}

#[test]
fn recursive_struct_fields_cannot_erase_byref_into_a_value_layout() {
    for delegate in [false, true] {
        let value = struct_fixture(Type::I32, delegate);
        let pointer = struct_fixture(Type::PtrMut(Box::new(Type::I32), 1), delegate);
        let (offset, raw) = with_field(&pointer, |field, image_address| {
            assert_eq!(field.ty(), Type::PtrMut(Box::new(Type::I32), 1));
            let mut blob = field.blob(2);
            let offset = (blob.as_ptr() as usize).checked_sub(image_address).unwrap();
            let signature = (0..blob.len()).map(|_| blob.read_u8()).collect::<Vec<_>>();
            (offset, signature)
        });
        assert_eq!(raw, [0x06, PTR, 0x08]);
        let mut byref = pointer.clone();
        assert_eq!(&byref[offset..offset + raw.len()], raw);
        byref[offset + 1] = BYREF;
        with_field(&byref, |field, _| {
            let mut blob = field.blob(2);
            let signature = (0..blob.len()).map(|_| blob.read_u8()).collect::<Vec<_>>();
            assert_eq!(signature, [0x06, BYREF, 0x08]);
            assert_eq!(
                field.ty(),
                Type::I32,
                "upstream also erases scalar field BYREF"
            );
        });
        let value = Fixture::parse(value);
        assert_implementable(&value);
        assert_rejected(&Fixture::parse(pointer), "unsupported native type PtrMut");
        let byref = Fixture::parse(byref);
        assert_rejected(&byref, "BYREF field");
        let method = if delegate { "Invoke" } else { "Take" };
        assert!(
            byref
                .implementation_metadata
                .diagnostics
                .iter()
                .any(|reason| {
                    reason.contains("Payload")
                        && reason.contains("Value")
                        && reason.contains(method)
                })
        );
        assert_eq!(
            format!("{:?}", value.methods),
            format!("{:?}", byref.methods)
        );
    }
}

#[test]
fn receive_array_input_stays_unsupported() {
    for delegate in [false, true] {
        let bytes = fixture(
            vec![parameter(
                "receive",
                Type::ArrayRef(Box::new(Type::I32)),
                ParamAttributes::In,
            )],
            Type::Void,
            delegate,
        );
        assert_rejected(&Fixture::parse(bytes), "input BYREF");
    }
}

#[test]
fn stock_winrt_scalar_out_and_array_signatures_remain_implementable() {
    let winmd = r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
    if !Path::new(winmd).is_file() {
        eprintln!("Skipping stock signature controls: Windows.winmd is unavailable");
        return;
    }
    let foundation = meta::parse_interfaces(winmd, "Windows.Foundation");
    let mut interfaces = ["IStringable", "IClosable", "IPropertyValue"]
        .into_iter()
        .map(|name| {
            foundation
                .iter()
                .find(|interface| interface.name == name)
                .unwrap()
                .clone()
        })
        .collect::<Vec<_>>();
    interfaces.push(
        meta::parse_class(winmd, "Windows.Storage.Streams", "DataReader")
            .unwrap()
            .default_interface
            .unwrap(),
    );
    for interface in interfaces {
        assert!(
            interface.implementation_metadata.diagnostics.is_empty(),
            "{}: {:?}",
            interface.name,
            interface.implementation_metadata.diagnostics
        );
        let [js, dts, py, pyi] = generated(&interface);
        assert!(js.contains("DynWinRtImplementationHandle.create"), "{js}");
        assert!(!dts.contains("handlers: never"), "{dts}");
        assert!(py.contains("DynWinRTImplementationHandle._create"), "{py}");
        assert!(pyi.contains("def implement("), "{pyi}");
        eprintln!("Validated stock signature evidence for {}", interface.name);
    }
}
