// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python type annotations and method documentation helpers.

use crate::codegen::winrt::shared::docs::{DocText, find_param_doc};
use crate::codegen::winrt::shared::imports::{
    fill_array_uses_retval_count, ireference_inner_type, method_abi_output_count,
};
use crate::meta::MethodMeta;
use crate::types::TypeMeta;

use super::collections::{CollectionKind, is_mapping_input, type_kind};
use super::docs::format_pydoc;
use super::naming::to_snake_case;
use super::naming::{PythonProjectionContext, PythonSupportSymbol, PythonSymbol};
use super::native_types::{FoundationType, foundation_type};

/// Build the Python docstring for a method body. Uses snake_case param display
/// names (matching the generated signature). Returns an empty string when no
/// doc fields are populated, preserving byte-identity for metadata without
/// sibling .xml files.
pub(super) fn method_pydoc(method: &MethodMeta, in_params: &[&crate::meta::ParamMeta]) -> String {
    method_pydoc_with_indent(method, in_params, "        ")
}

pub(super) fn method_pydoc_with_indent(
    method: &MethodMeta,
    in_params: &[&crate::meta::ParamMeta],
    indent: &str,
) -> String {
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
    format_pydoc(&doc, indent)
}

// ======================================================================
// Python type annotation helpers
// ======================================================================

pub(crate) fn py_optional_type(typ: String) -> String {
    let unquoted = typ
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .unwrap_or(&typ);
    if unquoted.split('|').any(|part| part.trim() == "None") {
        return unquoted.to_string();
    }
    format!("{} | None", unquoted)
}

fn is_nullable_reference_type(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Object
            | TypeMeta::Delegate { .. }
            | TypeMeta::RuntimeClass { .. }
            | TypeMeta::Interface { .. }
            | TypeMeta::Parameterized { .. }
    )
}

fn py_param_type(typ: &TypeMeta, context: &PythonProjectionContext) -> String {
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
        TypeMeta::RuntimeClass { .. } => {
            format!(
                "'{}'",
                context.symbol_reference(&typ.type_identity(), PythonSymbol::Like)
            )
        }
        TypeMeta::Enum { .. } | TypeMeta::Interface { .. } => {
            format!("'{}'", context.reference_name_for_type(typ))
        }
        TypeMeta::Parameterized { .. } => {
            format!("'{}'", context.reference_name_for_type(typ))
        }
        TypeMeta::Array(inner) => py_array_param_type(inner, context),
        TypeMeta::Object => format!(
            "'DynWinRTValue | {}'",
            context.support_symbol_reference(PythonSupportSymbol::ObjectInput)
        ),
        TypeMeta::Delegate { .. } => "'DynWinRTValue'".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => "datetime".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => "timedelta".to_string(),
        TypeMeta::Struct { .. } => format!("'{}'", context.reference_name_for_type(typ)),
        _ => "object".to_string(),
    }
}

pub(crate) fn py_param_type_safe(typ: &TypeMeta, context: &PythonProjectionContext) -> String {
    if let Some(inner) = ireference_inner_type(typ) {
        let native = py_optional_type(py_return_type_safe(Some(inner), context));
        let wrapper = context.reference_name_for_type(typ);
        return format!("{} | {}", native, wrapper);
    }

    if let Some(annotation) = py_collection_param_type(typ, context) {
        return annotation;
    }
    if let TypeMeta::Array(inner) = typ {
        return py_array_param_type(inner, context);
    }

    match typ {
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Interface { name, .. }
            if !context.is_known_type(typ) =>
        {
            "'DynWinRTValue'".to_string()
        }
        _ => py_param_type(typ, context),
    }
}

pub(super) fn py_collection_input_type(
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> String {
    let input = py_param_type_safe(typ, context);
    // Keep the existing nullable ABC contract; only widen its projected inputs.
    if is_nullable_reference_type(typ) {
        py_optional_type(input)
    } else {
        input
    }
}

pub(crate) fn py_return_type_safe(
    typ: Option<&TypeMeta>,
    context: &PythonProjectionContext,
) -> String {
    if let Some(inner) = typ.and_then(ireference_inner_type) {
        return py_optional_type(py_return_type_safe(Some(inner), context));
    }
    if let Some(async_type) = typ.and_then(|typ| py_async_return_type(typ, context)) {
        return async_type;
    }
    if let Some(annotation) = typ.and_then(|typ| py_collection_return_type(typ, context)) {
        return py_optional_type(annotation);
    }

    match typ {
        Some(typ @ TypeMeta::Enum { .. }) if !context.is_known_type(typ) => "int".to_string(),
        Some(typ @ (TypeMeta::RuntimeClass { .. } | TypeMeta::Interface { .. }))
            if !context.is_known_type(typ) =>
        {
            "DynWinRTValue | None".to_string()
        }
        Some(TypeMeta::Array(inner)) => py_array_return_type(inner, context),
        Some(typ) if is_nullable_reference_type(typ) => {
            py_optional_type(py_return_type(Some(typ), context))
        }
        _ => py_return_type(typ, context),
    }
}

fn py_async_return_type_with_result(
    typ: &TypeMeta,
    result_override: Option<String>,
    context: &PythonProjectionContext,
) -> Option<String> {
    match typ {
        TypeMeta::AsyncAction => Some("WinRTCoroutine[None]".to_string()),
        TypeMeta::AsyncOperation(result) => Some(format!(
            "WinRTCoroutine[{}]",
            result_override.unwrap_or_else(|| py_return_type_safe(Some(result), context))
        )),
        TypeMeta::AsyncActionWithProgress(progress) => Some(format!(
            "WinRTCoroutineWithProgress[None, {}]",
            py_return_type_safe(Some(progress), context)
        )),
        TypeMeta::AsyncOperationWithProgress(result, progress) => Some(format!(
            "WinRTCoroutineWithProgress[{}, {}]",
            result_override.unwrap_or_else(|| py_return_type_safe(Some(result), context)),
            py_return_type_safe(Some(progress), context)
        )),
        _ => None,
    }
}

pub(super) fn py_async_return_type(
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> Option<String> {
    py_async_return_type_with_result(typ, None, context)
}

pub(super) fn py_factory_return_type(
    class_name: &str,
    method: &MethodMeta,
    context: &PythonProjectionContext,
) -> String {
    method
        .return_type
        .as_ref()
        .and_then(|typ| {
            py_async_return_type_with_result(typ, Some(format!("'{}'", class_name)), context)
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

fn is_delegate_output(typ: &TypeMeta, context: &PythonProjectionContext) -> bool {
    context.is_delegate_type(typ)
}

pub(super) fn py_output_type(typ: &TypeMeta, context: &PythonProjectionContext) -> String {
    match typ {
        _ if is_delegate_output(typ, context) => "DynWinRTValue | None".to_string(),
        TypeMeta::Array(inner) if is_delegate_output(inner, context) => {
            "list[DynWinRTValue | None]".to_string()
        }
        TypeMeta::AsyncOperation(inner) => {
            format!("WinRTCoroutine[{}]", py_output_type(inner, context))
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => format!(
            "WinRTCoroutineWithProgress[{}, {}]",
            py_output_type(result, context),
            py_output_type(progress, context)
        ),
        TypeMeta::AsyncActionWithProgress(progress) => format!(
            "WinRTCoroutineWithProgress[None, {}]",
            py_output_type(progress, context)
        ),
        _ => py_return_type_safe(Some(typ), context),
    }
}

pub(super) fn py_method_return_type(
    method: &MethodMeta,
    context: &PythonProjectionContext,
) -> String {
    let outputs = py_method_outputs(method);
    match outputs.as_slice() {
        [] => "None".to_string(),
        [(_, typ)] => py_output_type(typ, context),
        _ => format!(
            "tuple[{}]",
            outputs
                .iter()
                .map(|(_, typ)| py_output_type(typ, context))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub(super) fn py_return_type(typ: Option<&TypeMeta>, context: &PythonProjectionContext) -> String {
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
        Some(typ @ TypeMeta::RuntimeClass { .. })
        | Some(typ @ TypeMeta::Enum { .. })
        | Some(typ @ TypeMeta::Interface { .. }) => {
            format!("'{}'", context.reference_name_for_type(typ))
        }
        Some(typ @ TypeMeta::Parameterized { .. }) => {
            format!("'{}'", context.reference_name_for_type(typ))
        }
        Some(TypeMeta::AsyncOperation(inner)) => {
            format!("WinRTCoroutine[{}]", py_return_type(Some(inner), context))
        }
        Some(TypeMeta::AsyncOperationWithProgress(result, progress)) => format!(
            "WinRTCoroutineWithProgress[{}, {}]",
            py_return_type(Some(result), context),
            py_return_type(Some(progress), context)
        ),
        Some(TypeMeta::AsyncAction) => "WinRTCoroutine[None]".to_string(),
        Some(TypeMeta::AsyncActionWithProgress(progress)) => format!(
            "WinRTCoroutineWithProgress[None, {}]",
            py_return_type(Some(progress), context)
        ),
        Some(TypeMeta::Array(inner)) => py_array_return_type(inner, context),
        Some(TypeMeta::Object) | Some(TypeMeta::Delegate { .. }) => "'DynWinRTValue'".to_string(),
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => "int".to_string(),
        Some(typ) if foundation_type(typ) == Some(FoundationType::DateTime) => {
            "datetime".to_string()
        }
        Some(typ) if foundation_type(typ) == Some(FoundationType::TimeSpan) => {
            "timedelta".to_string()
        }
        Some(typ @ TypeMeta::Struct { .. }) => {
            format!("'{}'", context.reference_name_for_type(typ))
        }
        None => "None".to_string(),
    }
}

fn py_native_element_type(inner: &TypeMeta, context: &PythonProjectionContext) -> String {
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
        TypeMeta::Enum { .. } if context.is_known_type(inner) => {
            format!("'{}'", context.reference_name_for_type(inner))
        }
        TypeMeta::Enum { .. } => "int".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => "datetime".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => "timedelta".to_string(),
        TypeMeta::Struct { .. } => format!("'{}'", context.reference_name_for_type(inner)),
        TypeMeta::RuntimeClass { .. } if context.is_known_type(inner) => {
            format!("'{}'", context.reference_name_for_type(inner))
        }
        TypeMeta::Interface { .. } if context.is_known_type(inner) => {
            format!("'{}'", context.reference_name_for_type(inner))
        }
        TypeMeta::Parameterized { .. } => {
            let concrete = context.reference_name_for_type(inner);
            if context.is_known_type(inner) {
                format!("'{concrete}'")
            } else {
                "'DynWinRTValue'".to_string()
            }
        }
        _ => "'DynWinRTValue'".to_string(),
    }
}

fn py_array_param_type(inner: &TypeMeta, context: &PythonProjectionContext) -> String {
    let element = py_native_param_element_type(inner, context);
    if matches!(inner, TypeMeta::U8) {
        format!("DynWinRTArray | bytes | bytearray | Sequence[{element}]")
    } else {
        format!("DynWinRTArray | Sequence[{element}]")
    }
}

fn py_native_param_element_type(inner: &TypeMeta, context: &PythonProjectionContext) -> String {
    match inner {
        TypeMeta::Object => py_param_type(inner, context),
        TypeMeta::RuntimeClass { name, .. } if context.is_known_type(inner) => {
            format!(
                "'{}'",
                context.symbol_reference(&inner.type_identity(), PythonSymbol::Like)
            )
        }
        _ => py_native_element_type(inner, context),
    }
}

fn py_array_return_type(inner: &TypeMeta, context: &PythonProjectionContext) -> String {
    if matches!(inner, TypeMeta::U8) {
        "bytes".to_string()
    } else {
        let element = py_native_element_type(inner, context);
        let element = if is_nullable_reference_type(inner) {
            py_optional_type(element)
        } else {
            element
        };
        format!("list[{element}]")
    }
}

fn py_collection_param_type(typ: &TypeMeta, context: &PythonProjectionContext) -> Option<String> {
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
            py_native_param_element_type(key, context),
            py_native_param_element_type(value, context)
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
            py_native_param_element_type(element, context)
        )),
        _ => None,
    }
}

fn py_collection_return_type(typ: &TypeMeta, context: &PythonProjectionContext) -> Option<String> {
    let TypeMeta::Parameterized { args, .. } = typ else {
        return None;
    };
    let kind = type_kind(typ)?;
    let abc = super::collections::abc_name(kind)?;
    let types = args
        .iter()
        .map(|arg| {
            let element = py_native_element_type(arg, context);
            if is_nullable_reference_type(arg) {
                py_optional_type(element)
            } else {
                element
            }
        })
        .collect::<Vec<_>>();
    Some(format!("{abc}[{}]", types.join(", ")))
}

pub(super) fn py_param_list(
    in_params: &[&crate::meta::ParamMeta],
    context: &PythonProjectionContext,
) -> String {
    in_params
        .iter()
        .map(|p| {
            let param_type = match &p.typ {
                typ if context.is_delegate_type(typ) => {
                    super::delegates::py_delegate_param_type(typ, context)
                }
                _ => py_param_type_safe(&p.typ, context),
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
            py_method_return_type(&method, &PythonProjectionContext::default()),
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
            py_method_return_type(&method, &PythonProjectionContext::default()),
            "list[str]"
        );
        assert_eq!(
            crate::codegen::winrt::python::signature::py_build_method_sig(&method),
            "DynWinRTMethodSig().add_in(DynWinRTType.u32_type()).add_out_fill(DynWinRTType.array_type(DynWinRTType.hstring())).add_out(DynWinRTType.u32_type())"
        );
        assert_eq!(
            crate::codegen::winrt::javascript::signature::build_method_sig(
                &crate::codegen::winrt::javascript::create_javascript_projection_context([])
                    .unwrap(),
                &method,
            ),
            "new DynWinRtMethodSig().addIn(DynWinRtType.u32()).addOutFill(DynWinRtType.arrayType(DynWinRtType.hstring())).addOut(DynWinRtType.u32())"
        );
    }

    #[test]
    fn object_arrays_return_typed_runtime_values() {
        assert_eq!(
            py_array_return_type(&TypeMeta::Object, &PythonProjectionContext::default()),
            "list[DynWinRTValue | None]"
        );
    }

    #[test]
    fn object_inputs_accept_native_wrappers_without_widening_outputs() {
        let context = PythonProjectionContext::default();
        assert_eq!(
            py_param_type_safe(&TypeMeta::Object, &context),
            "'DynWinRTValue | _DynWinRTObject'"
        );
        assert_eq!(
            py_array_param_type(&TypeMeta::Object, &context),
            "DynWinRTArray | Sequence['DynWinRTValue | _DynWinRTObject']"
        );
        for (name, piid, args, expected) in [
            (
                "IIterable`1",
                "faa585ea-6214-4217-afda-7f46de5869b3",
                vec![TypeMeta::Object],
                "Iterable['DynWinRTValue | _DynWinRTObject']",
            ),
            (
                "IMap`2",
                "3c2925fe-8519-45c1-aa79-197b6718c1c1",
                vec![TypeMeta::String, TypeMeta::Object],
                "Mapping[str, 'DynWinRTValue | _DynWinRTObject']",
            ),
        ] {
            let typ = TypeMeta::Parameterized {
                namespace: "Windows.Foundation.Collections".into(),
                name: name.into(),
                piid: piid.into(),
                args,
            };
            assert_eq!(py_param_type_safe(&typ, &context), expected);
        }
        assert_eq!(
            py_return_type_safe(Some(&TypeMeta::Object), &context),
            "DynWinRTValue | None"
        );
        assert_eq!(
            py_collection_input_type(&TypeMeta::Object, &context),
            "DynWinRTValue | _DynWinRTObject | None"
        );
        assert_eq!(py_collection_input_type(&TypeMeta::I32, &context), "int");
    }

    #[test]
    fn object_input_aliases_reach_collection_inputs_without_changing_outputs() {
        let typ = TypeMeta::Struct {
            namespace: "Audit".into(),
            name: "_DynWinRTObject".into(),
            fields: vec![],
        };
        let context = PythonProjectionContext::packaged([typ.type_identity()]).unwrap();
        let context = context.for_struct_module(&typ, std::slice::from_ref(&typ));
        let mapping = TypeMeta::Parameterized {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IMap`2".into(),
            piid: "3c2925fe-8519-45c1-aa79-197b6718c1c1".into(),
            args: vec![TypeMeta::String, TypeMeta::Object],
        };
        assert_eq!(
            py_param_type_safe(&mapping, &context),
            "Mapping[str, 'DynWinRTValue | _DynWinRTObject_2']"
        );
        assert_eq!(
            py_collection_input_type(&TypeMeta::Object, &context),
            "DynWinRTValue | _DynWinRTObject_2 | None"
        );
        assert_eq!(
            py_return_type_safe(Some(&TypeMeta::Object), &context),
            "DynWinRTValue | None"
        );
        assert_eq!(
            py_array_return_type(&TypeMeta::Object, &context),
            "list[DynWinRTValue | None]"
        );
    }

    #[test]
    fn reference_returns_are_annotated_as_nullable() {
        let runtime_class = TypeMeta::RuntimeClass {
            namespace: "Contoso".into(),
            name: "Widget".into(),
            default_interface: None,
        };
        let interface = TypeMeta::Interface {
            namespace: "Contoso".into(),
            name: "IWidget".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
        };
        let context = PythonProjectionContext::standalone([
            runtime_class.type_identity(),
            interface.type_identity(),
        ])
        .unwrap();

        assert_eq!(
            py_return_type_safe(Some(&runtime_class), &context),
            "Widget | None"
        );
        assert_eq!(
            py_return_type_safe(Some(&interface), &context),
            "IWidget | None"
        );
        assert_eq!(
            py_return_type_safe(Some(&TypeMeta::Object), &context),
            "DynWinRTValue | None"
        );
        assert_eq!(
            py_array_return_type(&runtime_class, &context),
            "list[Widget | None]"
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
        let context = PythonProjectionContext::standalone([param
            .typ
            .type_identity()
            .with_kind(crate::types::TypeIdentityKind::Delegate)])
        .unwrap();
        assert_eq!(
            py_param_list(&[&param], &context),
            "handler: Callable[..., object] | 'DynWinRTValue | DynWinRtDelegate'"
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
            py_return_type_safe(Some(&reference), &PythonProjectionContext::default()),
            "int | None"
        );
        assert_eq!(
            py_param_type_safe(&reference, &PythonProjectionContext::default()),
            "int | None | IReference_UInt32"
        );
        assert_eq!(py_optional_type("'DayOfWeek'".into()), "DayOfWeek | None");
    }
}
