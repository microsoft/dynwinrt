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

use super::collections::{CollectionKind, is_mapping_input, type_kind};
use super::docs::format_pydoc;
use super::naming::to_snake_case;
use super::native_types::{FoundationType, foundation_type};

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
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64 => "int".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::String => "str".to_string(),
        TypeMeta::Char16 => "str".to_string(),
        TypeMeta::Guid => "UUID".to_string(),
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. } => format!("'{}'", name),
        TypeMeta::Parameterized { name, args, .. } => {
            format!("'{}'", crate::meta::make_parameterized_name(name, args))
        }
        TypeMeta::Array(inner) => py_array_param_type(inner, &HashSet::new()),
        TypeMeta::Object | TypeMeta::Delegate { .. } => "'DynWinRTValue'".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => "datetime".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => "timedelta".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{}'", name),
        _ => "object".to_string(),
    }
}

pub(crate) fn py_param_type_safe(typ: &TypeMeta, known: &HashSet<String>) -> String {
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

    if let Some(annotation) = py_collection_param_type(typ, known) {
        return annotation;
    }
    if let TypeMeta::Array(inner) = typ {
        return py_array_param_type(inner, known);
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

pub(crate) fn py_return_type_safe(typ: Option<&TypeMeta>, known: &HashSet<String>) -> String {
    if let Some(inner) = typ.and_then(ireference_inner_type) {
        return py_optional_type(py_return_type_safe(Some(inner), known));
    }
    if let Some(async_type) = typ.and_then(|typ| py_async_return_type(typ, known)) {
        return async_type;
    }
    if let Some(annotation) = typ.and_then(|typ| py_collection_return_type(typ, known)) {
        return annotation;
    }

    match typ {
        Some(TypeMeta::RuntimeClass { name, .. })
        | Some(TypeMeta::Enum { name, .. })
        | Some(TypeMeta::Interface { name, .. })
            if !known.contains(name) =>
        {
            "'DynWinRTValue'".to_string()
        }
        Some(TypeMeta::Array(inner)) => py_array_return_type(inner, known),
        _ => py_return_type(typ),
    }
}

fn py_async_return_type_with_result(
    typ: &TypeMeta,
    result_override: Option<String>,
    known: &HashSet<String>,
) -> Option<String> {
    match typ {
        TypeMeta::AsyncAction => Some("WinRTAsync[None]".to_string()),
        TypeMeta::AsyncOperation(result) => Some(format!(
            "WinRTAsync[{}]",
            result_override.unwrap_or_else(|| py_return_type_safe(Some(result), known))
        )),
        TypeMeta::AsyncActionWithProgress(progress) => Some(format!(
            "WinRTAsyncWithProgress[None, {}]",
            py_return_type_safe(Some(progress), known)
        )),
        TypeMeta::AsyncOperationWithProgress(result, progress) => Some(format!(
            "WinRTAsyncWithProgress[{}, {}]",
            result_override.unwrap_or_else(|| py_return_type_safe(Some(result), known)),
            py_return_type_safe(Some(progress), known)
        )),
        _ => None,
    }
}

pub(super) fn py_async_return_type(typ: &TypeMeta, known: &HashSet<String>) -> Option<String> {
    py_async_return_type_with_result(typ, None, known)
}

pub(super) fn py_factory_return_type(
    class_name: &str,
    method: &MethodMeta,
    known: &HashSet<String>,
) -> String {
    method
        .return_type
        .as_ref()
        .and_then(|typ| {
            py_async_return_type_with_result(typ, Some(format!("'{}'", class_name)), known)
        })
        .unwrap_or_else(|| format!("'{}'", class_name))
}

pub(super) fn methods_have_async_output<'a>(
    methods: impl IntoIterator<Item = &'a MethodMeta>,
) -> bool {
    methods.into_iter().any(|method| {
        method.return_type.as_ref().is_some_and(TypeMeta::is_async)
            || method.params.iter().any(|param| {
                param.direction != crate::meta::ParamDirection::In && param.typ.is_async()
            })
    })
}

pub(super) fn py_method_abi_output_count(method: &MethodMeta) -> usize {
    method_abi_output_count(method)
}

pub(super) fn py_method_outputs(method: &MethodMeta) -> Vec<(usize, &TypeMeta)> {
    let mut result_index = 0;
    let mut outputs = Vec::new();

    for param in &method.params {
        match param.direction {
            crate::meta::ParamDirection::Out
            | crate::meta::ParamDirection::OutStringBuffer { .. } => {
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
        Some(TypeMeta::String) => "str".to_string(),
        Some(TypeMeta::Guid) => "UUID".to_string(),
        Some(TypeMeta::Bool) => "bool".to_string(),
        Some(
            TypeMeta::I8
            | TypeMeta::U8
            | TypeMeta::I16
            | TypeMeta::U16
            | TypeMeta::I32
            | TypeMeta::U32
            | TypeMeta::I64
            | TypeMeta::U64,
        ) => "int".to_string(),
        Some(TypeMeta::Char16) => "str".to_string(),
        Some(TypeMeta::F32 | TypeMeta::F64) => "float".to_string(),
        Some(TypeMeta::RuntimeClass { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Enum { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Interface { name, .. }) => format!("'{}'", name),
        Some(TypeMeta::Parameterized { name, args, .. }) => {
            format!("'{}'", crate::meta::make_parameterized_name(name, args))
        }
        Some(TypeMeta::AsyncOperation(inner)) => {
            format!("WinRTAsync[{}]", py_return_type(Some(inner)))
        }
        Some(TypeMeta::AsyncOperationWithProgress(result, progress)) => format!(
            "WinRTAsyncWithProgress[{}, {}]",
            py_return_type(Some(result)),
            py_return_type(Some(progress))
        ),
        Some(TypeMeta::AsyncAction) => "WinRTAsync[None]".to_string(),
        Some(TypeMeta::AsyncActionWithProgress(progress)) => format!(
            "WinRTAsyncWithProgress[None, {}]",
            py_return_type(Some(progress))
        ),
        Some(TypeMeta::Array(inner)) => py_array_return_type(inner, &HashSet::new()),
        Some(TypeMeta::Object) | Some(TypeMeta::Delegate { .. }) => "'DynWinRTValue'".to_string(),
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => "int".to_string(),
        Some(typ) if foundation_type(typ) == Some(FoundationType::DateTime) => {
            "datetime".to_string()
        }
        Some(typ) if foundation_type(typ) == Some(FoundationType::TimeSpan) => {
            "timedelta".to_string()
        }
        Some(TypeMeta::Struct { name, .. }) => format!("'{}'", name),
        None => "None".to_string(),
    }
}

fn py_native_element_type(inner: &TypeMeta, known_types: &HashSet<String>) -> String {
    match inner {
        TypeMeta::Bool => "bool".to_string(),
        TypeMeta::String => "str".to_string(),
        TypeMeta::Guid => "UUID".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64 => "int".to_string(),
        TypeMeta::Char16 => "str".to_string(),
        TypeMeta::Enum { name, .. } if known_types.contains(name) => format!("'{name}'"),
        TypeMeta::Enum { .. } => "int".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => "datetime".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => "timedelta".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{name}'"),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => {
            format!("'{name}'")
        }
        TypeMeta::Interface { name, .. } if known_types.contains(name) => {
            format!("'{name}'")
        }
        TypeMeta::Parameterized { name, args, .. } => {
            let concrete = crate::meta::make_parameterized_name(name, args);
            if known_types.contains(&concrete) {
                format!("'{concrete}'")
            } else {
                "'DynWinRTValue'".to_string()
            }
        }
        _ => "'DynWinRTValue'".to_string(),
    }
}

fn py_array_param_type(inner: &TypeMeta, known_types: &HashSet<String>) -> String {
    let element = py_native_element_type(inner, known_types);
    if matches!(inner, TypeMeta::U8) {
        format!("DynWinRTArray | bytes | bytearray | Sequence[{element}]")
    } else {
        format!("DynWinRTArray | Sequence[{element}]")
    }
}

fn py_array_return_type(inner: &TypeMeta, known_types: &HashSet<String>) -> String {
    if matches!(inner, TypeMeta::U8) {
        "bytes".to_string()
    } else {
        format!("list[{}]", py_native_element_type(inner, known_types))
    }
}

fn py_collection_param_type(typ: &TypeMeta, known_types: &HashSet<String>) -> Option<String> {
    let TypeMeta::Parameterized { args, .. } = typ else {
        return None;
    };
    let kind = type_kind(typ)?;
    if is_mapping_input(kind, args) {
        let (key, value) = if matches!(
            kind,
            CollectionKind::Mapping | CollectionKind::MutableMapping
        ) {
            (args.first()?, args.get(1)?)
        } else {
            match args.first()? {
                TypeMeta::Parameterized {
                    args: pair_args, ..
                } => (pair_args.first()?, pair_args.get(1)?),
                _ => return None,
            }
        };
        return Some(format!(
            "Mapping[{}, {}]",
            py_native_element_type(key, known_types),
            py_native_element_type(value, known_types)
        ));
    }
    let element = args.first()?;
    match kind {
        CollectionKind::Iterable
        | CollectionKind::Iterator
        | CollectionKind::Sequence
        | CollectionKind::MutableSequence => Some(format!(
            "{}[{}]",
            if kind == CollectionKind::Iterator {
                "Iterator"
            } else if kind == CollectionKind::Iterable {
                "Iterable"
            } else {
                "Sequence"
            },
            py_native_element_type(element, known_types)
        )),
        _ => None,
    }
}

fn py_collection_return_type(typ: &TypeMeta, known_types: &HashSet<String>) -> Option<String> {
    let TypeMeta::Parameterized { args, .. } = typ else {
        return None;
    };
    let kind = type_kind(typ)?;
    let abc = super::collections::abc_name(kind)?;
    let types = args
        .iter()
        .map(|arg| py_native_element_type(arg, known_types))
        .collect::<Vec<_>>();
    Some(format!("{abc}[{}]", types.join(", ")))
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
                TypeMeta::Delegate { .. } => "Callable[..., object] | 'DynWinRTValue'".to_string(),
                TypeMeta::Interface { name, .. } if delegate_type_names.contains(name) => {
                    "Callable[..., object] | 'DynWinRTValue'".to_string()
                }
                TypeMeta::Parameterized { name, args, .. }
                    if delegate_type_names
                        .contains(&crate::meta::make_parameterized_name(name, args)) =>
                {
                    "Callable[..., object] | 'DynWinRTValue'".to_string()
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
            py_array_return_type(&TypeMeta::Object, &HashSet::new()),
            "list['DynWinRTValue']"
        );
    }

    #[test]
    fn delegate_inputs_accept_callables_and_runtime_values() {
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
            "handler: Callable[..., object] | 'DynWinRTValue'"
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
