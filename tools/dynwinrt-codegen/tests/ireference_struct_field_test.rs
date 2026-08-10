// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};

use dynwinrt_codegen::codegen::winrt::javascript::project;
use dynwinrt_codegen::codegen::winrt::python;
use dynwinrt_codegen::codegen::{python_stub, render_dts, render_js};
use dynwinrt_codegen::meta::{
    ClassMeta, InterfaceMeta, MethodMeta, parse_class, resolve_dependencies,
};
use dynwinrt_codegen::types::{FieldMeta, TypeMeta};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
const PIID_IREFERENCE: &str = "61c17706-2d65-11e0-9ae8-d48564015472";

fn point() -> TypeMeta {
    TypeMeta::Struct {
        namespace: "Windows.Foundation".into(),
        name: "Point".into(),
        fields: vec![
            FieldMeta {
                name: "X".into(),
                typ: TypeMeta::F32,
            },
            FieldMeta {
                name: "Y".into(),
                typ: TypeMeta::F32,
            },
        ],
    }
}

fn ireference(inner: TypeMeta) -> TypeMeta {
    TypeMeta::Parameterized {
        namespace: "Windows.Foundation".into(),
        name: "IReference`1".into(),
        piid: PIID_IREFERENCE.into(),
        args: vec![inner],
    }
}

fn synthetic_class() -> ClassMeta {
    let holder = TypeMeta::Struct {
        namespace: "Synthetic".into(),
        name: "OptionalPointHolder".into(),
        fields: vec![FieldMeta {
            name: "Value".into(),
            typ: ireference(point()),
        }],
    };
    ClassMeta {
        name: "UsesOptionalPoint".into(),
        namespace: "Synthetic".into(),
        full_name: "Synthetic.UsesOptionalPoint".into(),
        default_interface: Some(InterfaceMeta {
            name: "IUsesOptionalPoint".into(),
            namespace: "Synthetic".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![MethodMeta {
                name: "GetHolder".into(),
                raw_name: "GetHolder".into(),
                vtable_index: 6,
                return_type: Some(holder),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn class_with_struct(name: &str, structure: TypeMeta) -> ClassMeta {
    ClassMeta {
        name: name.into(),
        namespace: "Synthetic".into(),
        full_name: format!("Synthetic.{name}"),
        default_interface: Some(InterfaceMeta {
            name: format!("I{name}"),
            namespace: "Synthetic".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            methods: vec![MethodMeta {
                name: "GetValue".into(),
                raw_name: "GetValue".into(),
                vtable_index: 6,
                return_type: Some(structure),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn generate_javascript(class: &ClassMeta, known: &HashSet<String>) -> (String, String) {
    let projected = project::project_class(
        class,
        known,
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    (
        render_js::render(&projected),
        render_dts::render(&projected),
    )
}

#[test]
fn synthetic_ireference_point_field_uses_native_optional_projection() {
    let class = synthetic_class();
    let known = HashSet::from([
        "UsesOptionalPoint".to_string(),
        "Point".to_string(),
        "IReference_Point".to_string(),
    ]);
    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());
    let (js, dts) = generate_javascript(&class, &known);

    let constructor = "def __init__(self, value: Point | None | IReference_Point = None):";
    let getter = "def value(self) -> Point | None:";
    let setter = "def value(self, value: Point | None | IReference_Point) -> None:";
    assert!(py.contains(constructor), "{py}");
    assert!(py.contains(getter) && py.contains(setter), "{py}");
    assert!(py.contains("self._value = _dynwinrt_unbox_reference(value)"));
    assert!(py.contains("_dynwinrt_symbol('i_reference_point', 'IReference_Point')(value).value"));
    assert!(
        py.contains(
            "s.set_object(0, _dynwinrt_box_reference(v.value, DynWinRTType.struct_type('Windows.Foundation.Point', [DynWinRTType.f32_type(), DynWinRTType.f32_type()]), lambda value: _pack_point(value).to_value()))"
        ),
        "{py}"
    );
    assert!(py.contains("from .i_reference_point import IReference_Point"));
    assert!(!getter.contains("DynWinRTValue"));

    assert!(
        pyi.contains(
            "def __init__(self, value: Point | None | IReference_Point = ...) -> None: ..."
        ),
        "{pyi}"
    );
    assert!(pyi.contains(&format!("{getter} ...")), "{pyi}");
    assert!(pyi.contains(&format!("{setter} ...")), "{pyi}");
    assert!(!pyi.contains("value(self) -> DynWinRTValue"));

    assert!(
        dts.contains("value: Point | null | IReference_Point;"),
        "{dts}"
    );
    assert!(
        js.contains("value.isNull() ? null") && js.contains(".value)(s.getObject(0))"),
        "{js}"
    );
    assert!(
        js.contains(
            "DynWinRtValue.boxReference(_packPoint(value).toValue(), DynWinRtType.structType('Windows.Foundation.Point', [DynWinRtType.f32(), DynWinRtType.f32()]))"
        ),
        "{js}"
    );
    assert!(
        js.contains("s.setObject(0, ((value) => value == null ? DynWinRtValue.nullValue()"),
        "{js}"
    );
    assert!(
        dts.contains("import { IReference_Point } from './IReference_Point.js';"),
        "{dts}"
    );

    if std::path::Path::new(WINDOWS_WINMD).exists() {
        let dependencies = resolve_dependencies(WINDOWS_WINMD, &[class], &[], &[]);
        let wrapper = dependencies
            .interfaces
            .iter()
            .find(|interface| interface.name == "IReference_Point")
            .expect("IReference<Point> used only by a struct field must be registered");
        assert_eq!(wrapper.generic_args, [point()]);
        assert!(
            wrapper
                .methods
                .iter()
                .any(|method| method.name == "get_Value")
        );
    }
}

#[test]
fn scalar_ireference_fields_preserve_native_python_types() {
    let mode = TypeMeta::Enum {
        namespace: "Synthetic".into(),
        name: "Mode".into(),
        underlying: Box::new(TypeMeta::I32),
        members: Vec::new(),
        is_flags: false,
        doc: None,
        deprecated: None,
    };
    let holder = TypeMeta::Struct {
        namespace: "Synthetic".into(),
        name: "OptionalScalars".into(),
        fields: vec![
            FieldMeta {
                name: "Count".into(),
                typ: ireference(TypeMeta::U32),
            },
            FieldMeta {
                name: "Label".into(),
                typ: ireference(TypeMeta::String),
            },
            FieldMeta {
                name: "Mode".into(),
                typ: ireference(mode),
            },
        ],
    };
    let class = class_with_struct("UsesOptionalScalars", holder);
    let known = HashSet::from([
        "UsesOptionalScalars".to_string(),
        "Mode".to_string(),
        "IReference_UInt32".to_string(),
        "IReference_String".to_string(),
        "IReference_Mode".to_string(),
    ]);
    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());
    let (js, dts) = generate_javascript(&class, &known);

    for output in [&py, &pyi] {
        assert!(output.contains("def count(self) -> int | None:"));
        assert!(output.contains("count(self, value: int | None | IReference_UInt32)"));
        assert!(output.contains("def label(self) -> str | None:"));
        assert!(output.contains("label(self, value: str | None | IReference_String)"));
        assert!(output.contains("def mode(self) -> Mode | None:"));
        assert!(output.contains("mode(self, value: Mode | None | IReference_Mode)"));
        assert!(!output.contains("def count(self) -> DynWinRTValue"));
    }
    assert!(py.contains("lambda value: DynWinRTValue.from_u32(value)"));
    assert!(py.contains("lambda value: DynWinRTValue.from_hstring(value)"));
    assert!(py.contains("lambda value: DynWinRTValue.enum_value("));
    assert!(
        dts.contains("count: number | null | IReference_UInt32;")
            && dts.contains("label: string | null | IReference_String;")
            && dts.contains("mode: Mode | null | IReference_Mode;"),
        "{dts}"
    );
    assert!(js.contains("DynWinRtValue.u32(value)"), "{js}");
    assert!(js.contains("DynWinRtValue.hstring(value)"), "{js}");
    assert!(js.contains("DynWinRtValue.enumValue("), "{js}");
}

#[test]
fn nested_struct_defaults_and_enum_fields_are_python_native() {
    let mode = TypeMeta::Enum {
        namespace: "Synthetic".into(),
        name: "Mode".into(),
        underlying: Box::new(TypeMeta::U32),
        members: vec![],
        is_flags: false,
        doc: None,
        deprecated: None,
    };
    let inner = TypeMeta::Struct {
        namespace: "Synthetic".into(),
        name: "Inner".into(),
        fields: vec![FieldMeta {
            name: "Count".into(),
            typ: TypeMeta::U32,
        }],
    };
    let outer = TypeMeta::Struct {
        namespace: "Synthetic".into(),
        name: "Outer".into(),
        fields: vec![
            FieldMeta {
                name: "Mode".into(),
                typ: mode,
            },
            FieldMeta {
                name: "Inner".into(),
                typ: inner,
            },
        ],
    };
    let class = class_with_struct("UsesNestedStruct", outer);
    let known = HashSet::from([
        "UsesNestedStruct".to_string(),
        "Mode".to_string(),
        "Inner".to_string(),
        "Outer".to_string(),
    ]);
    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(
        py.contains("def __init__(self, mode: 'Mode' = _dynwinrt_enum('mode', 'Mode', 0), inner: Inner | None = None):"),
        "{py}"
    );
    assert!(!py.contains("inner: 'Inner' | None"), "{py}");
    assert!(
        py.contains("self.inner = Inner() if inner is None else inner"),
        "{py}"
    );
    assert!(
        py.contains("mode=_dynwinrt_enum('mode', 'Mode', s.get_u32(0))"),
        "{py}"
    );
    assert!(py.contains("s.set_u32(0, int(v.mode))"), "{py}");
    assert!(py.contains("s.set_struct(1, _pack_inner(v.inner))"), "{py}");

    assert!(
        pyi.contains("def __init__(self, mode: 'Mode' = ..., inner: 'Inner' = ...) -> None: ..."),
        "{pyi}"
    );
    assert!(pyi.contains("mode: 'Mode'"), "{pyi}");
    assert!(pyi.contains("inner: 'Inner'"), "{pyi}");
}

#[test]
fn sdk_http_progress_ireference_u64_fields_are_native_optional_values() {
    if !std::path::Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let class = parse_class(WINDOWS_WINMD, "Windows.Web.Http", "HttpClient").unwrap();
    let known = HashSet::from(["HttpClient".to_string(), "HttpProgressStage".to_string()]);
    let py = python::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let pyi = python_stub::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());
    let (js, dts) = generate_javascript(&class, &known);

    for output in [&py, &pyi] {
        assert!(output.contains("total_bytes_to_send: int | None | IReference_UInt64"));
        assert!(output.contains("def total_bytes_to_send(self) -> int | None:"));
        assert!(output.contains(
            "def total_bytes_to_send(self, value: int | None | IReference_UInt64) -> None:"
        ));
    }
    assert!(py.contains(
        "s.set_object(2, _dynwinrt_box_reference(v.total_bytes_to_send, DynWinRTType.u64_type(), lambda value: DynWinRTValue.from_u64(value)))"
    ));
    assert!(py.contains(
        "None if value.is_null() else _dynwinrt_symbol('i_reference_u_int64', 'IReference_UInt64')(value).value"
    ));
    assert!(
        dts.contains("totalBytesToSend: bigint | null | IReference_UInt64;")
            && dts.contains("totalBytesToReceive: bigint | null | IReference_UInt64;"),
        "{dts}"
    );
    assert!(
        js.contains(".value)(s.getObject(2))")
            && js.contains(".value)(s.getObject(4))")
            && js.contains(
                "DynWinRtValue.boxReference(DynWinRtValue.u64(value), DynWinRtType.u64())"
            ),
        "{js}"
    );
}
