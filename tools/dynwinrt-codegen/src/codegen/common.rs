// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Backward-compatible exports retained for external callers.

// `to_snake_case_filename` is consumed by `main.rs` via the crate's public
// `codegen::common` path, so re-export it publicly in addition to the
// `pub(crate)` glob above.
pub use super::python::naming::to_snake_case_filename;

#[cfg(test)]
mod tests {
    use crate::codegen::javascript::naming::*;
    use crate::codegen::javascript::signature::*;
    use crate::codegen::javascript::structs::*;
    use crate::codegen::python::naming::*;
    use crate::codegen::python::signature::*;
    use crate::codegen::python::structs::*;
    use crate::codegen::shared::imports::*;
    use crate::meta::{MethodMeta, ParamDirection, ParamMeta};
    use crate::types::TypeMeta;
    use std::collections::HashSet;

    #[test]
    fn to_camel_case_basic() {
        assert_eq!(to_camel_case("GetValue"), "getValue");
        assert_eq!(to_camel_case("X"), "x");
        assert_eq!(to_camel_case("already"), "already");
        assert_eq!(to_camel_case(""), "");
    }

    #[test]
    fn capitalize_basic() {
        assert_eq!(capitalize("hello"), "Hello");
        assert_eq!(capitalize("H"), "H");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("already"), "Already");
    }

    #[test]
    fn ts_struct_field_type_mappings() {
        assert_eq!(ts_struct_field_type(&TypeMeta::Bool), "boolean");
        assert_eq!(ts_struct_field_type(&TypeMeta::String), "string");
        assert_eq!(ts_struct_field_type(&TypeMeta::Guid), "string");
        assert_eq!(ts_struct_field_type(&TypeMeta::I32), "number");
        assert_eq!(ts_struct_field_type(&TypeMeta::F64), "number");
        assert_eq!(
            ts_struct_field_type(&TypeMeta::Struct {
                namespace: "N".into(),
                name: "MyStruct".into(),
                fields: vec![],
            }),
            "MyStruct"
        );
        assert_eq!(
            ts_struct_field_type(&TypeMeta::Struct {
                namespace: "N".into(),
                name: "HResult".into(),
                fields: vec![],
            }),
            "number"
        );
    }

    #[test]
    fn struct_field_getter_expressions() {
        assert_eq!(struct_field_getter(&TypeMeta::Bool, 0), "s.getU8(0) !== 0");
        assert_eq!(struct_field_getter(&TypeMeta::I32, 2), "s.getI32(2)");
        assert_eq!(struct_field_getter(&TypeMeta::String, 1), "s.getHstring(1)");
        assert_eq!(struct_field_getter(&TypeMeta::F64, 3), "s.getF64(3)");
    }

    #[test]
    fn struct_field_setter_expressions() {
        assert_eq!(
            struct_field_setter(&TypeMeta::Bool, 0, "v"),
            "s.setU8(0, v ? 1 : 0)"
        );
        assert_eq!(
            struct_field_setter(&TypeMeta::I32, 1, "x"),
            "s.setI32(1, x)"
        );
        assert_eq!(
            struct_field_setter(&TypeMeta::String, 2, "s"),
            "s.setHstring(2, s)"
        );
    }

    #[test]
    fn ts_dynwinrt_type_primitives() {
        assert_eq!(ts_dynwinrt_type(&TypeMeta::Bool), "DynWinRtType.boolType()");
        assert_eq!(ts_dynwinrt_type(&TypeMeta::I32), "DynWinRtType.i32()");
        assert_eq!(
            ts_dynwinrt_type(&TypeMeta::String),
            "DynWinRtType.hstring()"
        );
        assert_eq!(ts_dynwinrt_type(&TypeMeta::Guid), "DynWinRtType.guidType()");
        assert_eq!(ts_dynwinrt_type(&TypeMeta::F64), "DynWinRtType.f64()");
        assert_eq!(ts_dynwinrt_type(&TypeMeta::Object), "DynWinRtType.object()");
    }

    #[test]
    fn ts_dynwinrt_type_async() {
        assert_eq!(
            ts_dynwinrt_type(&TypeMeta::AsyncAction),
            "DynWinRtType.iAsyncAction()"
        );
        assert_eq!(
            ts_dynwinrt_type(&TypeMeta::AsyncOperation(Box::new(TypeMeta::String))),
            "DynWinRtType.iAsyncOperation(DynWinRtType.hstring())"
        );
    }

    #[test]
    fn ts_dynwinrt_type_array() {
        assert_eq!(
            ts_dynwinrt_type(&TypeMeta::Array(Box::new(TypeMeta::I32))),
            "DynWinRtType.arrayType(DynWinRtType.i32())"
        );
    }

    #[test]
    fn ts_dynwinrt_type_struct() {
        let s = TypeMeta::Struct {
            namespace: "N".into(),
            name: "Rect".into(),
            fields: vec![
                crate::types::FieldMeta {
                    name: "X".into(),
                    typ: TypeMeta::F32,
                },
                crate::types::FieldMeta {
                    name: "Y".into(),
                    typ: TypeMeta::F32,
                },
            ],
        };
        assert_eq!(
            ts_dynwinrt_type(&s),
            "DynWinRtType.structType('N.Rect', [DynWinRtType.f32(), DynWinRtType.f32()])"
        );
    }

    #[test]
    fn ts_dynwinrt_type_hresult_struct() {
        // HResult is exposed in WinRT metadata as a struct wrapping an i32, but
        // the runtime treats it as its own kind (WinRTValue::HResult). The
        // codegen must register methods with DynWinRtType.hresult() so the
        // value comes back as HResult — calling .toNumber() on a plain
        // WinRTValue::Struct would panic the napi binding.
        let s = TypeMeta::Struct {
            namespace: "Windows.Foundation".into(),
            name: "HResult".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::I32,
            }],
        };
        assert_eq!(ts_dynwinrt_type(&s), "DynWinRtType.hresult()");
    }

    #[test]
    fn py_dynwinrt_type_hresult_struct() {
        let s = TypeMeta::Struct {
            namespace: "Windows.Foundation".into(),
            name: "HResult".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::I32,
            }],
        };
        assert_eq!(py_dynwinrt_type(&s), "DynWinRTType.hresult()");
    }

    #[test]
    fn build_method_sig_empty() {
        let m = MethodMeta {
            name: "DoSomething".into(),
            vtable_index: 6,
            params: vec![],
            return_type: None,
            is_property_getter: false,
            is_property_setter: false,
            is_event_add: false,
            is_event_remove: false,
            ..Default::default()
        };
        assert_eq!(build_method_sig(&m), "new DynWinRtMethodSig()");
    }

    #[test]
    fn build_method_sig_with_params_and_return() {
        let m = MethodMeta {
            name: "GetValue".into(),
            vtable_index: 7,
            params: vec![ParamMeta {
                name: "key".into(),
                typ: TypeMeta::String,
                direction: ParamDirection::In,
            }],
            return_type: Some(TypeMeta::I32),
            is_property_getter: false,
            is_property_setter: false,
            is_event_add: false,
            is_event_remove: false,
            ..Default::default()
        };
        let sig = build_method_sig(&m);
        assert!(sig.contains(".addIn(DynWinRtType.hstring())"));
        assert!(sig.contains(".addOut(DynWinRtType.i32())"));
    }

    #[test]
    fn wrap_arg_types() {
        assert_eq!(wrap_arg("s", &TypeMeta::String), "DynWinRtValue.hstring(s)");
        assert_eq!(wrap_arg("b", &TypeMeta::Bool), "DynWinRtValue.boolValue(b)");
        assert_eq!(wrap_arg("n", &TypeMeta::I32), "DynWinRtValue.i32(n)");
        assert_eq!(wrap_arg("n", &TypeMeta::I64), "DynWinRtValue.i64(n)");
        assert_eq!(wrap_arg("f", &TypeMeta::F64), "DynWinRtValue.f64(f)");
        assert_eq!(
            wrap_arg("o", &TypeMeta::Object),
            "(o == null ? DynWinRtValue.nullValue() : _unwrap(o))"
        );
    }

    #[test]
    fn wraps_javascript_ireference_inputs() {
        let reference = |inner| TypeMeta::Parameterized {
            namespace: "Windows.Foundation".into(),
            name: "IReference".into(),
            piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
            args: vec![inner],
        };

        assert!(
            wrap_arg("value", &reference(TypeMeta::String))
                .contains("boxReference(DynWinRtValue.hstring(value), DynWinRtType.hstring())")
        );
        assert!(
            wrap_arg(
                "value",
                &reference(TypeMeta::Struct {
                    namespace: "Windows.Foundation".into(),
                    name: "Point".into(),
                    fields: vec![],
                })
            )
            .contains("_packPoint(value).toValue()")
        );
        assert!(
            wrap_arg(
                "value",
                &reference(TypeMeta::Enum {
                    namespace: "Test".into(),
                    name: "Kind".into(),
                    underlying: Box::new(TypeMeta::I32),
                    members: vec![],
                    is_flags: false,
                    doc: None,
                    deprecated: None,
                })
            )
            .contains("DynWinRtValue.enumValue(")
        );
    }

    #[test]
    fn convert_return_basic() {
        let known = HashSet::new();
        let deferred = HashSet::new();
        assert_eq!(
            convert_return("r", Some(&TypeMeta::String), false, &known, &deferred),
            "r.toString()"
        );
        assert_eq!(
            convert_return("r", Some(&TypeMeta::I32), false, &known, &deferred),
            "r.toNumber()"
        );
        assert_eq!(
            convert_return("r", Some(&TypeMeta::Bool), false, &known, &deferred),
            "r.toBool()"
        );
        assert_eq!(convert_return("r", None, false, &known, &deferred), "r");
    }

    #[test]
    fn convert_return_with_known_class() {
        let mut known = HashSet::new();
        known.insert("Uri".to_string());
        let deferred = HashSet::new();
        let rt = TypeMeta::RuntimeClass {
            namespace: "Windows.Foundation".into(),
            name: "Uri".into(),
            default_interface: Some(Box::new(TypeMeta::Interface {
                namespace: "Windows.Foundation".into(),
                name: "IUriRuntimeClass".into(),
                iid: "abc".into(),
            })),
        };
        // Refs come out as `__DWRT_REF__<name>__`; render layer rewrites.
        assert_eq!(
            convert_return("r", Some(&rt), false, &known, &deferred),
            "((v) => v.isNull() ? null : __DWRT_REF__Uri__._fromNative(v))(r)"
        );
    }

    #[test]
    fn convert_return_with_unknown_class_preserves_non_null_raw_values() {
        let rt = TypeMeta::RuntimeClass {
            namespace: "Windows.Foundation".into(),
            name: "Uri".into(),
            default_interface: Some(Box::new(TypeMeta::Interface {
                namespace: "Windows.Foundation".into(),
                name: "IUriRuntimeClass".into(),
                iid: "abc".into(),
            })),
        };

        assert_eq!(
            convert_return("r", Some(&rt), false, &HashSet::new(), &HashSet::new()),
            "((v) => v.isNull() ? null : v)(r)"
        );
    }

    #[test]
    fn get_in_params_filters_correctly() {
        let m = MethodMeta {
            name: "Test".into(),
            vtable_index: 6,
            params: vec![
                ParamMeta {
                    name: "a".into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "b".into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::Out,
                },
                ParamMeta {
                    name: "c".into(),
                    typ: TypeMeta::Array(Box::new(TypeMeta::U8)),
                    direction: ParamDirection::OutFill,
                },
            ],
            return_type: None,
            is_property_getter: false,
            is_property_setter: false,
            is_event_add: false,
            is_event_remove: false,
            ..Default::default()
        };
        let in_params = get_in_params(&m);
        assert_eq!(in_params.len(), 2); // In + OutFill
        assert_eq!(in_params[0].name, "a");
        assert_eq!(in_params[1].name, "c");
    }

    #[test]
    fn collect_type_imports_skips_self() {
        let class = crate::meta::ClassMeta {
            name: "MyClass".into(),
            namespace: "N".into(),
            full_name: "N.MyClass".into(),
            default_interface: Some(crate::meta::InterfaceMeta {
                name: "IMyClass".into(),
                namespace: "N".into(),
                iid: "abc".into(),
                methods: vec![MethodMeta {
                    name: "GetSelf".into(),
                    vtable_index: 6,
                    params: vec![],
                    return_type: Some(TypeMeta::RuntimeClass {
                        namespace: "N".into(),
                        name: "MyClass".into(),
                        default_interface: Some(Box::new(TypeMeta::Interface {
                            namespace: "N".into(),
                            name: "IMyClass".into(),
                            iid: "def".into(),
                        })),
                    }),
                    is_property_getter: false,
                    is_property_setter: false,
                    is_event_add: false,
                    is_event_remove: false,
                    ..Default::default()
                }],
                generic_piid: None,
                generic_args: vec![],
                ..Default::default()
            }),
            required_interfaces: vec![],
            factory_interfaces: vec![],
            static_interfaces: vec![],
            has_default_constructor: false,
            ..Default::default()
        };
        let imports = collect_type_imports(&class);
        // Should not include MyClass itself
        assert!(!imports.iter().any(|r| r.name == "MyClass"));
    }

    // ------------------------------------------------------------------
    // Python-specific helper tests
    // ------------------------------------------------------------------

    #[test]
    fn to_snake_case_basic() {
        assert_eq!(to_snake_case("AbsoluteUri"), "absolute_uri");
        assert_eq!(to_snake_case("GetValue"), "get_value");
        assert_eq!(to_snake_case("createUri"), "create_uri");
        assert_eq!(to_snake_case("already"), "already");
        assert_eq!(to_snake_case(""), "");
        assert_eq!(to_snake_case("X"), "x");
        assert_eq!(to_snake_case("Port"), "port");
    }

    #[test]
    fn to_snake_case_acronyms() {
        assert_eq!(to_snake_case("IIDComponent"), "iid_component");
        assert_eq!(to_snake_case("HTMLParser"), "html_parser");
    }

    #[test]
    fn to_snake_case_reserved() {
        assert_eq!(to_snake_case("import"), "import_");
        assert_eq!(to_snake_case("class"), "class_");
        assert_eq!(to_snake_case("for"), "for_");
    }

    #[test]
    fn py_dynwinrt_type_primitives() {
        assert_eq!(
            py_dynwinrt_type(&TypeMeta::Bool),
            "DynWinRTType.bool_type()"
        );
        assert_eq!(py_dynwinrt_type(&TypeMeta::I32), "DynWinRTType.i32_type()");
        assert_eq!(
            py_dynwinrt_type(&TypeMeta::String),
            "DynWinRTType.hstring()"
        );
        assert_eq!(
            py_dynwinrt_type(&TypeMeta::Guid),
            "DynWinRTType.guid_type()"
        );
        assert_eq!(py_dynwinrt_type(&TypeMeta::F64), "DynWinRTType.f64_type()");
        assert_eq!(py_dynwinrt_type(&TypeMeta::Object), "DynWinRTType.object()");
    }

    #[test]
    fn py_dynwinrt_type_async() {
        assert_eq!(
            py_dynwinrt_type(&TypeMeta::AsyncAction),
            "DynWinRTType.i_async_action()"
        );
        assert_eq!(
            py_dynwinrt_type(&TypeMeta::AsyncOperation(Box::new(TypeMeta::String))),
            "DynWinRTType.i_async_operation(DynWinRTType.hstring())"
        );
    }

    #[test]
    fn py_wrap_arg_types() {
        assert_eq!(
            py_wrap_arg("s", &TypeMeta::String),
            "DynWinRTValue.from_hstring(s)"
        );
        assert_eq!(
            py_wrap_arg("b", &TypeMeta::Bool),
            "DynWinRTValue.from_bool(b)"
        );
        assert_eq!(
            py_wrap_arg("n", &TypeMeta::I32),
            "DynWinRTValue.from_i32(n)"
        );
        assert_eq!(
            py_wrap_arg("n", &TypeMeta::I64),
            "DynWinRTValue.from_i64(n)"
        );
        assert_eq!(
            py_wrap_arg("f", &TypeMeta::F64),
            "DynWinRTValue.from_f64(f)"
        );
    }

    #[test]
    fn wraps_python_ireference_inputs() {
        let reference = |inner| TypeMeta::Parameterized {
            namespace: "Windows.Foundation".into(),
            name: "IReference".into(),
            piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
            args: vec![inner],
        };

        assert!(
            py_wrap_arg("value", &reference(TypeMeta::String))
                .contains("lambda value: DynWinRTValue.from_hstring(value)")
        );
        assert!(
            py_wrap_arg(
                "value",
                &reference(TypeMeta::Struct {
                    namespace: "Windows.Foundation".into(),
                    name: "Point".into(),
                    fields: vec![],
                })
            )
            .contains("lambda value: _pack_point(value).to_value()")
        );
        assert!(
            py_wrap_arg(
                "value",
                &reference(TypeMeta::Enum {
                    namespace: "Test".into(),
                    name: "Kind".into(),
                    underlying: Box::new(TypeMeta::I32),
                    members: vec![],
                    is_flags: false,
                    doc: None,
                    deprecated: None,
                })
            )
            .contains("lambda value: DynWinRTValue.enum_value(")
        );
    }

    #[test]
    fn py_convert_return_basic() {
        let known = HashSet::new();
        assert_eq!(
            py_convert_return("r", Some(&TypeMeta::String), false, &known),
            "r.to_string()"
        );
        assert_eq!(
            py_convert_return("r", Some(&TypeMeta::I32), false, &known),
            "r.to_number()"
        );
        assert_eq!(
            py_convert_return("r", Some(&TypeMeta::Bool), false, &known),
            "r.to_bool()"
        );
        assert_eq!(py_convert_return("r", None, false, &known), "r");
    }

    #[test]
    fn py_convert_return_with_known_class() {
        let mut known = HashSet::new();
        known.insert("Uri".to_string());
        let rt = TypeMeta::RuntimeClass {
            namespace: "Windows.Foundation".into(),
            name: "Uri".into(),
            default_interface: Some(Box::new(TypeMeta::Interface {
                namespace: "Windows.Foundation".into(),
                name: "IUriRuntimeClass".into(),
                iid: "abc".into(),
            })),
        };
        assert_eq!(
            py_convert_return("r", Some(&rt), false, &known),
            "_dynwinrt_symbol('uri', 'Uri')._from_native(r)"
        );
        assert_eq!(
            py_convert_array_return("r", &rt, &known),
            "_dynwinrt_wrap_values('uri', 'Uri', r.to_values())"
        );
    }

    #[test]
    fn py_convert_return_with_enum() {
        let en = TypeMeta::Enum {
            namespace: "Windows.Globalization".into(),
            name: "DayOfWeek".into(),
            underlying: Box::new(TypeMeta::I32),
            members: Vec::new(),
            is_flags: false,
            doc: None,
            deprecated: None,
        };
        assert_eq!(
            py_convert_return("r", Some(&en), false, &HashSet::new()),
            "r.to_number()"
        );

        let known = HashSet::from(["DayOfWeek".to_string()]);
        assert_eq!(
            py_convert_return("r", Some(&en), false, &known),
            "_dynwinrt_enum('day_of_week', 'DayOfWeek', r.to_number())"
        );
    }

    #[test]
    fn py_build_method_sig_empty() {
        let m = MethodMeta {
            name: "DoSomething".into(),
            vtable_index: 6,
            params: vec![],
            return_type: None,
            is_property_getter: false,
            is_property_setter: false,
            is_event_add: false,
            is_event_remove: false,
            ..Default::default()
        };
        assert_eq!(py_build_method_sig(&m), "DynWinRTMethodSig()");
    }

    #[test]
    fn py_build_method_sig_with_params_and_return() {
        let m = MethodMeta {
            name: "GetValue".into(),
            vtable_index: 7,
            params: vec![ParamMeta {
                name: "key".into(),
                typ: TypeMeta::String,
                direction: ParamDirection::In,
            }],
            return_type: Some(TypeMeta::I32),
            is_property_getter: false,
            is_property_setter: false,
            is_event_add: false,
            is_event_remove: false,
            ..Default::default()
        };
        let sig = py_build_method_sig(&m);
        assert!(sig.contains(".add_in(DynWinRTType.hstring())"));
        assert!(sig.contains(".add_out(DynWinRTType.i32_type())"));
    }

    #[test]
    fn py_struct_field_getter_expressions() {
        assert_eq!(
            py_struct_field_getter(&TypeMeta::Bool, 0),
            "s.get_u8(0) != 0"
        );
        assert_eq!(py_struct_field_getter(&TypeMeta::I32, 2), "s.get_i32(2)");
        assert_eq!(
            py_struct_field_getter(&TypeMeta::String, 1),
            "s.get_hstring(1)"
        );
        assert_eq!(py_struct_field_getter(&TypeMeta::F64, 3), "s.get_f64(3)");
    }

    #[test]
    fn py_struct_field_setter_expressions() {
        assert_eq!(
            py_struct_field_setter(&TypeMeta::Bool, 0, "v"),
            "s.set_u8(0, 1 if v else 0)"
        );
        assert_eq!(
            py_struct_field_setter(&TypeMeta::I32, 1, "x"),
            "s.set_i32(1, x)"
        );
        assert_eq!(
            py_struct_field_setter(&TypeMeta::String, 2, "s_"),
            "s.set_hstring(2, s_)"
        );
    }

    #[test]
    fn py_struct_field_type_mappings() {
        assert_eq!(py_struct_field_type(&TypeMeta::Bool), "bool");
        assert_eq!(py_struct_field_type(&TypeMeta::String), "str");
        assert_eq!(py_struct_field_type(&TypeMeta::I32), "int");
        assert_eq!(py_struct_field_type(&TypeMeta::F64), "float");
    }
}
