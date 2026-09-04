// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::{project, render_dts, render_js};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta};

fn ibuffer() -> InterfaceMeta {
    InterfaceMeta {
        name: "IBuffer".into(),
        namespace: "Windows.Storage.Streams".into(),
        iid: "905A0FE0-BC53-11DF-8C49-001E4FC686DA".into(),
        ..Default::default()
    }
}

fn buffer_class() -> ClassMeta {
    ClassMeta {
        name: "Buffer".into(),
        namespace: "Windows.Storage.Streams".into(),
        full_name: "Windows.Storage.Streams.Buffer".into(),
        default_interface: Some(ibuffer()),
        ..Default::default()
    }
}

#[test]
fn javascript_projects_copied_ibuffer_conversions() {
    let class = buffer_class();
    let projected = project::project_class(
        &Default::default(),
        &class,
        &HashSet::from(["Buffer".into(), "IBuffer".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);

    assert!(js.contains("static fromBuffer(data)"));
    assert!(js.contains("Buffer._fromNative(DynWinRtValue.fromBuffer(data))"));
    assert!(js.contains("toBuffer()"));
    assert!(js.contains("return this._obj.toBuffer();"));
    assert!(dts.contains("data: Parameters<typeof DynWinRtValue.fromBuffer>[0]"));
    assert!(dts.contains("ReturnType<DynWinRtValue['toBuffer']>"));

    let interface = ibuffer();
    let projected = project::project_interface(
        &Default::default(),
        &interface,
        &HashSet::from(["IBuffer".into()]),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let js = render_js::render(&projected);
    let dts = render_dts::render(&projected);
    assert!(js.contains("IBuffer.from(DynWinRtValue.fromBuffer(data))"));
    assert!(dts.contains("static fromBuffer("));
    assert!(dts.contains("toBuffer(): ReturnType<DynWinRtValue['toBuffer']>;"));
}

#[test]
fn python_projects_copied_ibuffer_conversions() {
    let class = buffer_class();
    let known = HashSet::from(["Buffer".into(), "IBuffer".into()]);
    let implementation = common::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let stub = common::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());
    assert!(implementation.contains("def from_bytes(data: bytes | bytearray) -> 'Buffer':"));
    assert!(implementation.contains("return Buffer._from_native(DynWinRTValue.from_bytes(data))"));
    assert!(implementation.contains("return self._obj.to_bytes()"));
    assert!(stub.contains("def from_bytes(data: bytes | bytearray) -> 'Buffer': ..."));
    assert!(stub.contains("def to_bytes(self) -> bytes: ..."));

    let interface = ibuffer();
    let implementation = common::generate_interface(
        &interface,
        &HashSet::from(["IBuffer".into()]),
        &HashSet::new(),
    );
    let stub = common::generate_interface_stub(
        &interface,
        &HashSet::from(["IBuffer".into()]),
        &HashSet::new(),
    );
    assert!(implementation.contains("def from_bytes(data: bytes | bytearray) -> 'IBuffer':"));
    assert!(implementation.contains("return IBuffer._from_native(DynWinRTValue.from_bytes(data))"));
    assert!(stub.contains("def from_bytes(data: bytes | bytearray) -> 'IBuffer': ..."));
    assert!(stub.contains("def to_bytes(self) -> bytes: ..."));
}

#[test]
fn lookalike_interfaces_do_not_receive_ibuffer_conversions() {
    let mut interface = ibuffer();
    interface.namespace = "Contoso".into();
    let implementation = common::generate_interface(
        &interface,
        &HashSet::from(["IBuffer".into()]),
        &HashSet::new(),
    );
    assert!(!implementation.contains("def from_bytes"));
    assert!(!implementation.contains("def to_bytes"));
}
