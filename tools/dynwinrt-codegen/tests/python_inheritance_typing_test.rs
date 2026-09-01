// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::HashSet;
use std::path::Path;

use dynwinrt_codegen::meta::{
    self, ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta,
};
use dynwinrt_codegen::types::{TypeKind, TypeMeta, TypeRef};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

#[test]
fn stubs_model_runtime_class_and_interface_bases_without_runtime_inheritance() {
    let base_interface = TypeMeta::Interface {
        namespace: "Contoso".into(),
        name: "IBase".into(),
        iid: "22222222-2222-2222-2222-222222222222".into(),
    };
    let interface = InterfaceMeta {
        name: "IDerived".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        base_interfaces: vec![base_interface],
        methods: vec![
            MethodMeta {
                name: "Ping".into(),
                vtable_index: 6,
                ..Default::default()
            },
            MethodMeta {
                name: "UseBase".into(),
                vtable_index: 7,
                params: vec![ParamMeta {
                    name: "value".into(),
                    typ: TypeMeta::RuntimeClass {
                        namespace: "Contoso".into(),
                        name: "Base".into(),
                        default_interface: None,
                    },
                    direction: ParamDirection::In,
                }],
                return_type: Some(TypeMeta::RuntimeClass {
                    namespace: "Contoso".into(),
                    name: "Base".into(),
                    default_interface: None,
                }),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let interface_stub =
        common::generate_interface_stub(&interface, &HashSet::new(), &HashSet::new());
    assert!(
        !interface_stub.contains("from .contoso__i_base import"),
        "{interface_stub}"
    );
    assert!(
        interface_stub.contains("from typing import Protocol, Self"),
        "{interface_stub}"
    );
    assert!(
        interface_stub.contains("class _IDerivedIdentity(Protocol):"),
        "{interface_stub}"
    );
    assert!(
        interface_stub.contains("def _dynwinrt_iid_contoso_iderived(self) -> None:"),
        "{interface_stub}"
    );
    assert!(
        interface_stub.contains("class IDerived(_IDerivedIdentity, Protocol):"),
        "{interface_stub}"
    );

    let class = ClassMeta {
        name: "Derived".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Derived".into(),
        base_class: Some(TypeRef {
            namespace: "Contoso".into(),
            name: "Base".into(),
            kind: TypeKind::Class,
        }),
        default_interface: Some(interface),
        required_interfaces: vec![InterfaceMeta {
            name: "IExtra".into(),
            namespace: "Contoso".into(),
            iid: "33333333-3333-3333-3333-333333333333".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let class_stub = common::generate_class_stub(
        &class,
        &HashSet::from(["Base".into(), "Derived".into(), "IDerived".into()]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert!(
        class_stub.contains("from .contoso__base import _BaseIdentity"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains("class _DerivedIdentity(_BaseIdentity, Protocol):"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains("def _dynwinrt_class_contoso_derived(self) -> None:"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains("def _dynwinrt_iid_contoso_iderived(self) -> None:"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains("def _dynwinrt_iid_contoso_iextra(self) -> None:"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains("class DerivedLike(_DerivedIdentity, Protocol):"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains("def ping(self) -> None:"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains("from .contoso__base import Base, BaseLike"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains("def use_base(self, value: 'BaseLike') -> Base | None:"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains("class Derived(DerivedLike, _DynWinRTRuntimeClass):"),
        "{class_stub}"
    );
    assert!(
        class_stub.contains(
            "def as_interface(self, interface_class: _DynWinRTProjector[_InterfaceT]) -> _InterfaceT:"
        ),
        "{class_stub}"
    );
}

#[test]
fn real_windows_metadata_exposes_xaml_and_stream_typing_relationships() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let stack_panel = meta::parse_class(WINDOWS_WINMD, "Windows.UI.Xaml.Controls", "StackPanel")
        .expect("StackPanel metadata");
    let panel = stack_panel.base_class.as_ref().expect("StackPanel base");
    assert_eq!(panel.namespace, "Windows.UI.Xaml.Controls");
    assert_eq!(panel.name, "Panel");

    let stream = meta::parse_class(
        WINDOWS_WINMD,
        "Windows.Media.SpeechSynthesis",
        "SpeechSynthesisStream",
    )
    .expect("SpeechSynthesisStream metadata");
    assert!(
        stream
            .required_interfaces
            .iter()
            .any(|interface| interface.name == "IRandomAccessStream"),
        "{:#?}",
        stream.required_interfaces
    );
    let known = stream
        .all_interfaces()
        .map(|interface| interface.name.clone())
        .chain(["SpeechSynthesisStream".into()])
        .collect::<HashSet<_>>();
    let stream_stub =
        common::generate_class_stub(&stream, &known, &HashSet::new(), &HashSet::new());
    assert!(
        stream_stub
            .contains("class SpeechSynthesisStreamLike(_SpeechSynthesisStreamIdentity, Protocol):"),
        "{stream_stub}"
    );
    assert!(
        stream_stub.contains("def seek(self, position: int)"),
        "{stream_stub}"
    );
    assert!(
        stream_stub.contains(
            "class SpeechSynthesisStream(SpeechSynthesisStreamLike, _DynWinRTRuntimeClass):"
        ),
        "{stream_stub}"
    );
    let stream_interfaces = meta::parse_interfaces(WINDOWS_WINMD, "Windows.Storage.Streams");
    let with_content_type = stream_interfaces
        .iter()
        .find(|interface| interface.name == "IRandomAccessStreamWithContentType")
        .expect("IRandomAccessStreamWithContentType metadata");
    let base_names = with_content_type
        .base_interfaces
        .iter()
        .filter_map(|base| match base {
            TypeMeta::Interface { name, .. } => Some(name.as_str()),
            TypeMeta::Parameterized { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    assert!(base_names.contains("IRandomAccessStream"));
    assert!(base_names.contains("IContentTypeProvider"));
    assert!(!base_names.contains("IClosable"));
    assert!(!base_names.contains("IInputStream"));
    assert!(!base_names.contains("IOutputStream"));

    let known_interfaces = stream_interfaces
        .iter()
        .map(|interface| interface.name.clone())
        .collect::<HashSet<_>>();
    let with_content_stub =
        common::generate_interface_stub(with_content_type, &known_interfaces, &HashSet::new());
    let with_content_runtime =
        common::generate_interface(with_content_type, &known_interfaces, &HashSet::new());
    assert!(
        with_content_stub
            .contains("class IRandomAccessStreamWithContentType(_IRandomAccessStreamWithContentTypeIdentity, Protocol):"),
        "{with_content_stub}"
    );
    assert!(
        !with_content_stub.contains(
            "class IRandomAccessStreamWithContentType(IRandomAccessStream, IContentTypeProvider"
        ),
        "{with_content_stub}"
    );
    assert!(
        with_content_runtime.contains("def as_interface(self, interface_class):"),
        "{with_content_runtime}"
    );
    assert!(
        !with_content_runtime.contains("def seek(")
            && !with_content_runtime.contains("def content_type("),
        "{with_content_runtime}"
    );

    let random_access = stream_interfaces
        .iter()
        .find(|interface| interface.name == "IRandomAccessStream")
        .expect("IRandomAccessStream metadata");
    let random_access_bases = random_access
        .base_interfaces
        .iter()
        .filter_map(|base| match base {
            TypeMeta::Interface { name, .. } => Some(name.as_str()),
            TypeMeta::Parameterized { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    assert!(random_access_bases.contains("IInputStream"));
    assert!(random_access_bases.contains("IOutputStream"));
    assert!(!random_access_bases.contains("IClosable"));
}
