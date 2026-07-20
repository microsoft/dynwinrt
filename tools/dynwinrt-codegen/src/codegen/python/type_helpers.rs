// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python type annotations and method documentation helpers.

use std::collections::HashSet;

use crate::codegen::shared::docs::{DocText, find_param_doc};
use crate::codegen::shared::imports::{
    fill_array_uses_retval_count, ireference_inner_type, method_abi_output_count,
};
use crate::meta::MethodMeta;
use crate::types::TypeMeta;

use super::docs::format_pydoc;
use super::naming::to_snake_case;

/// Build the Python docstring for a method body. Uses snake_case param display
/// names (matching the generated signature). Returns an empty string when no
/// doc fields are populated, preserving byte-identity for metadata without
/// sibling .xml files.
pub(super) fn method_pydoc(method: &MethodMeta, in_params: &[&crate::meta::ParamMeta]) -> String {
    if method.doc.is_none()
        && method.deprecated.is_none()
        && method.returns_doc.is_none()
        && method.param_docs.is_empty()
    {
        return String::new();
    }
    let params_snake: Vec<(String, &str)> = in_params
        .iter()
        .filter_map(|p| {
            find_param_doc(&method.param_docs, &p.name).map(|d| (to_snake_case(&p.name), d))
        })
        .collect();
    let params_refs: Vec<(&str, &str)> =
        params_snake.iter().map(|(n, d)| (n.as_str(), *d)).collect();
    let doc = DocText {
        summary: method.doc.as_deref(),
        deprecated: method.deprecated.as_deref(),
        returns: method.returns_doc.as_deref(),
        params: params_refs,
    };
    format_pydoc(&doc, "        ")
}

// ======================================================================
// Python type annotation helpers
// ======================================================================

fn py_optional_type(typ: String) -> String {
    let unquoted = typ
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .unwrap_or(&typ);
    format!("{} | None", unquoted)
}

fn py_param_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "bool".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64 => "int".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::String | TypeMeta::Guid => "str".to_string(),
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. } => format!("'{}'", name),
        TypeMeta::Parameterized { name, args, .. } => {
            format!("'{}'", crate::meta::make_parameterized_name(name, args))
        }
        TypeMeta::Array(_) => "'DynWinRTArray'".to_string(),
        TypeMeta::Object | TypeMeta::Delegate { .. } => "'DynWinRTValue'".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{}'", name),
        _ => "object".to_string(),
    }
}

pub(super) fn py_param_type_safe(typ: &TypeMeta, known: &HashSet<String>) -> String {
    if let Some(inner) = ireference_inner_type(typ) {
        let native = py_optional_type(py_return_type_safe(Some(inner), known));
        let wrapper = match typ {
            TypeMeta::Parameterized { name, args, .. } => {
                crate::meta::make_parameterized_name(name, args)
            }
            _ => unreachable!(),
        };
        return format!("{} | {}", native, wrapper);
    }

    match typ {
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. }
            if !known.contains(name) =>
        {
            "'DynWinRTValue'".to_string()
        }
        _ => py_param_type(typ),
    }
}

pub(super) fn py_return_type_safe(typ: Option<&TypeMeta>, known: &HashSet<String>) -> String {
    if let Some(inner) = typ.and_then(ireference_inner_type) {
        return py_optional_type(py_return_type_safe(Some(inner), known));
    }

    match typ {
        Some(TypeMeta::RuntimeClass { name, .. })
        | Some(TypeMeta::Enum { name, .. })
        | Some(TypeMeta::Interface { name, .. })
            if !known.contains(name) =>
        {
            "'DynWinRTValue'".to_string()
        }
        Some(TypeMeta::AsyncOperation(inner)) => py_return_type_safe(Some(inner), known),
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => {
            py_return_type_safe(Some(result), known)
        }
        Some(TypeMeta::AsyncActionWithProgress(_)) | Some(TypeMeta::AsyncAction) => {
            "None".to_string()
        }
        Some(TypeMeta::Array(inner)) => py_array_element_type(inner, known),
        _ => py_return_type(typ),
    }
}

pub(super) fn py_method_abi_output_count(method: &MethodMeta) -> usize {
    method_abi_output_count(method)
}

pub(super) fn py_method_outputs(method: &MethodMeta) -> Vec<(usize, &TypeMeta)> {
    let mut result_index = 0;
    let mut outputs = Vec::new();

    for param in &method.params {
        match param.direction {
            crate::meta::ParamDirection::Out => {
                outputs.push((result_index, &param.typ));
                result_index += 1;
            }
            crate::meta::ParamDirection::OutFill => {
                // The runtime allocates a distinct filled result buffer; the
                // caller-provided array supplies capacity and is not mutated.
                outputs.push((result_index, &param.typ));
                result_index += 1;
            }
            crate::meta::ParamDirection::In => {}
        }
    }

    if let Some(return_type) = method
        .return_type
        .as_ref()
        .filter(|_| !fill_array_uses_retval_count(method))
    {
        outputs.push((result_index, return_type));
    }

    outputs
}

fn py_output_type(
    typ: &TypeMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    match typ {
        TypeMeta::Delegate { .. } => "'DynWinRTValue'".to_string(),
        TypeMeta::Interface { name, .. } if delegate_type_names.contains(name) => {
            "'DynWinRTValue'".to_string()
        }
        _ => py_return_type_safe(Some(typ), known_types),
    }
}

pub(super) fn py_method_return_type(
    method: &MethodMeta,
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    let outputs = py_method_outputs(method);
    match outputs.as_slice() {
        [] => "None".to_string(),
        [(_, typ)] => py_output_type(typ, known_types, delegate_type_names),
        _ => format!(
            "tuple[{}]",
            outputs
                .iter()
                .map(|(_, typ)| py_output_type(typ, known_types, delegate_type_names))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn py_return_type(typ: Option<&TypeMeta>) -> String {
    match typ {
        Some(TypeMeta::String) | Some(TypeMeta::Guid) => "str".to_string(),
        Some(TypeMeta::Bool) => "bool".to_string(),
        Some(
            TypeMeta::I8
            | TypeMeta::U8
            | TypeMeta::I16
            | TypeMeta::U16
            | TypeMeta::Char16
            | TypeMeta::I32
            | TypeMeta::U32
            | TypeMeta::I64
            | TypeMeta::U64,
        ) => "int".to_string(),
        Some(TypeMeta::F32 | TypeMeta::F64) => "float".to_string(),
        Some(TypeMeta::RuntimeClass { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Enum { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Interface { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Parameterized { name, args, .. }) => {
            format!("'{}'", crate::meta::make_parameterized_name(name, args))
        }
        Some(TypeMeta::AsyncOperation(inner)) => py_return_type(Some(inner)),
        Some(TypeMeta::AsyncOperationWithProgress(result, _)) => py_return_type(Some(result)),
        Some(TypeMeta::AsyncAction) | Some(TypeMeta::AsyncActionWithProgress(_)) => {
            "None".to_string()
        }
        Some(TypeMeta::Array(inner)) => py_array_element_type(inner, &HashSet::new()),
        Some(TypeMeta::Object) | Some(TypeMeta::Delegate { .. }) => "'DynWinRTValue'".to_string(),
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => "int".to_string(),
        Some(TypeMeta::Struct { name, .. }) => format!("'{}'", name),
        None => "None".to_string(),
    }
}

pub(super) fn py_array_element_type(inner: &TypeMeta, known_types: &HashSet<String>) -> String {
    match inner {
        TypeMeta::Bool => "list[bool]".to_string(),
        TypeMeta::String | TypeMeta::Guid => "list[str]".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64
        | TypeMeta::Enum { .. } => "list[int]".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "list[float]".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "list[int]".to_string(),
        TypeMeta::Struct { name, .. } => format!("list['{}']", name),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => {
            format!("list['{}']", name)
        }
        TypeMeta::Interface { name, .. } if known_types.contains(name) => {
            format!("list['{}']", name)
        }
        _ => "list['DynWinRTValue']".to_string(),
    }
}

pub(super) fn py_param_list(
    in_params: &[&crate::meta::ParamMeta],
    known_types: &HashSet<String>,
    delegate_type_names: &HashSet<String>,
) -> String {
    in_params
        .iter()
        .map(|p| {
            let param_type = match &p.typ {
                TypeMeta::Delegate { .. } => "'DynWinRTValue'".to_string(),
                TypeMeta::Interface { name, .. } if delegate_type_names.contains(name) => {
                    "'DynWinRTValue'".to_string()
                }
                TypeMeta::Parameterized { name, args, .. }
                    if delegate_type_names
                        .contains(&crate::meta::make_parameterized_name(name, args)) =>
                {
                    "'DynWinRTValue'".to_string()
                }
                _ => py_param_type_safe(&p.typ, known_types),
            };
            format!("{}: {}", to_snake_case(&p.name), param_type)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{ParamDirection, ParamMeta};

    #[test]
    fn multi_out_returns_typed_tuple_in_abi_order() {
        let method = MethodMeta {
            name: "IndexOf".into(),
            params: vec![
                ParamMeta {
                    name: "value".into(),
                    typ: TypeMeta::String,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "index".into(),
                    typ: TypeMeta::U32,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(TypeMeta::Bool),
            ..Default::default()
        };

        assert_eq!(py_method_abi_output_count(&method), 2);
        let outputs = py_method_outputs(&method);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0], (0, &TypeMeta::U32));
        assert_eq!(outputs[1], (1, &TypeMeta::Bool));
        assert_eq!(
            py_method_return_type(&method, &HashSet::new(), &HashSet::new()),
            "tuple[int, bool]"
        );
    }

    #[test]
    fn fill_array_count_retval_is_not_registered_twice() {
        let method = MethodMeta {
            name: "GetMany".into(),
            params: vec![
                ParamMeta {
                    name: "startIndex".into(),
                    typ: TypeMeta::U32,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "items".into(),
                    typ: TypeMeta::Array(Box::new(TypeMeta::String)),
                    direction: ParamDirection::OutFill,
                },
            ],
            return_type: Some(TypeMeta::U32),
            ..Default::default()
        };

        assert_eq!(py_method_abi_output_count(&method), 2);
        let outputs = py_method_outputs(&method);
        assert_eq!(outputs.len(), 1);
        assert_eq!(
            outputs[0],
            (0, &TypeMeta::Array(Box::new(TypeMeta::String)))
        );
        assert_eq!(
            py_method_return_type(&method, &HashSet::new(), &HashSet::new()),
            "list[str]"
        );
        assert_eq!(
            crate::codegen::python::signature::py_build_method_sig(&method),
            "DynWinRTMethodSig().add_in(DynWinRTType.u32_type()).add_out_fill(DynWinRTType.array_type(DynWinRTType.hstring())).add_out(DynWinRTType.u32_type())"
        );
        assert_eq!(
            crate::codegen::javascript::signature::build_method_sig(&method),
            "new DynWinRtMethodSig().addIn(DynWinRtType.u32()).addOutFill(DynWinRtType.arrayType(DynWinRtType.hstring())).addOut(DynWinRtType.u32())"
        );
    }

    #[test]
    fn object_arrays_return_typed_runtime_values() {
        assert_eq!(
            py_array_element_type(&TypeMeta::Object, &HashSet::new()),
            "list['DynWinRTValue']"
        );
    }

    #[test]
    fn delegate_interfaces_are_typed_as_runtime_values() {
        let param = ParamMeta {
            name: "handler".into(),
            typ: TypeMeta::Interface {
                namespace: "Test".into(),
                name: "Handler".into(),
                iid: "00000000-0000-0000-0000-000000000000".into(),
            },
            direction: ParamDirection::In,
        };
        assert_eq!(
            py_param_list(
                &[&param],
                &HashSet::from(["Handler".into()]),
                &HashSet::from(["Handler".into()])
            ),
            "handler: 'DynWinRTValue'"
        );
    }

    #[test]
    fn ireference_returns_are_projected_as_optional_values() {
        let reference = TypeMeta::Parameterized {
            namespace: "Windows.Foundation".into(),
            name: "IReference".into(),
            piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
            args: vec![TypeMeta::U32],
        };

        assert_eq!(
            py_return_type_safe(Some(&reference), &HashSet::new()),
            "int | None"
        );
        assert_eq!(
            py_param_type_safe(&reference, &HashSet::new()),
            "int | None | IReference_UInt32"
        );
        assert_eq!(py_optional_type("'DayOfWeek'".into()), "DayOfWeek | None");
    }
}
