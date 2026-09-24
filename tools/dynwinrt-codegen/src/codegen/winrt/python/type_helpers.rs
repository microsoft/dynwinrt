// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python type annotations and method documentation helpers.

use crate::codegen::winrt::shared::docs::{DocText, find_param_doc};
use crate::codegen::winrt::shared::imports::{
    fill_array_uses_retval_count, ireference_inner_type, method_abi_output_count,
};
use crate::meta::MethodMeta;
use crate::types::TypeMeta;

use super::collections::{CollectionKind, abc_name, is_mapping_input, type_kind};
use super::docs::format_pydoc;
use super::naming::to_snake_case;
use super::naming::{PythonProjectionContext, PythonSupportSymbol, PythonSymbol};
use super::native_types::{FoundationType, foundation_type};
use super::nullability::{
    AnnotationSurface, OutputPosition, OutputSite, may_project_none, output_admits_none,
};

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
    let unquoted = unquoted(&typ);
    if unquoted.split('|').any(|part| part.trim() == "None") {
        return unquoted.to_string();
    }
    format!("{} | None", unquoted)
}

fn unquoted(typ: &str) -> &str {
    typ.strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .unwrap_or(typ)
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
    if may_project_none(typ) {
        py_optional_type(input)
    } else {
        input
    }
}

// ======================================================================
// Output annotations
//
// Rendering and nullability are separate layers: the `spell_*` functions
// produce the non-null type expression for a position, and
// `nullability::output_admits_none` alone decides whether `| None` is added.
// ======================================================================

/// Spelling rules of the rendering layer. They predate the nullability policy
/// and are preserved byte-for-byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spelling {
    /// Method, out and property results: delegates are raw `DynWinRTValue`
    /// handles, also inside async results and arrays.
    Member,
    /// A standalone value, such as the item type of a collection class.
    Value,
    /// An element of a returned collection or array: nested generics are
    /// named by their projected class.
    Element,
}

impl Spelling {
    fn at(position: OutputPosition) -> Self {
        match position {
            OutputPosition::CollectionElement | OutputPosition::CallbackParam => Self::Value,
            OutputPosition::Return
            | OutputPosition::OutParam
            | OutputPosition::Property
            | OutputPosition::AsyncResult
            | OutputPosition::AsyncProgress
            | OutputPosition::Activation => Self::Member,
        }
    }
}

/// Renders the annotation of a value the consumer receives at `site`.
///
/// Nullability never changes how the base type is spelled: a value that may
/// project as `None` renders the same unquoted expression with or without
/// ` | None`.
pub(crate) fn py_output_annotation(
    typ: &TypeMeta,
    site: OutputSite,
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> String {
    render_output(typ, Spelling::at(site.position), site, surface, context)
}

fn render_output(
    typ: &TypeMeta,
    spelling: Spelling,
    site: OutputSite,
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> String {
    let base = match spelling {
        Spelling::Member => spell_member(typ, site, surface, context),
        Spelling::Value => spell_value(typ, site, surface, context),
        Spelling::Element => py_native_element_type(typ, context),
    };
    if !may_project_none(typ) {
        return base;
    }
    if output_admits_none(typ, site, surface, context) {
        py_optional_type(base)
    } else {
        unquoted(&base).to_string()
    }
}

fn spell_member(
    typ: &TypeMeta,
    site: OutputSite,
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> String {
    if context.is_delegate_type(typ) {
        return "DynWinRTValue".to_string();
    }
    let nested = |typ: &TypeMeta, position: OutputPosition| {
        render_output(
            typ,
            Spelling::Member,
            site.nested(position),
            surface,
            context,
        )
    };
    match typ {
        TypeMeta::Array(inner) if context.is_delegate_type(inner) => {
            format!("list[{}]", nested(inner, OutputPosition::CollectionElement))
        }
        TypeMeta::AsyncOperation(result) => {
            format!(
                "WinRTCoroutine[{}]",
                nested(result, OutputPosition::AsyncResult)
            )
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => format!(
            "WinRTCoroutineWithProgress[{}, {}]",
            nested(result, OutputPosition::AsyncResult),
            nested(progress, OutputPosition::AsyncProgress)
        ),
        TypeMeta::AsyncActionWithProgress(progress) => format!(
            "WinRTCoroutineWithProgress[None, {}]",
            nested(progress, OutputPosition::AsyncProgress)
        ),
        _ => spell_value(typ, site, surface, context),
    }
}

fn spell_value(
    typ: &TypeMeta,
    site: OutputSite,
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> String {
    if let Some(inner) = ireference_inner_type(typ) {
        return render_output(inner, Spelling::Value, site, surface, context);
    }
    if let Some(collection) = spell_collection(typ, site, surface, context) {
        return collection;
    }
    let nested = |typ: &TypeMeta, spelling: Spelling, position: OutputPosition| {
        render_output(typ, spelling, site.nested(position), surface, context)
    };
    match typ {
        TypeMeta::AsyncAction => "WinRTCoroutine[None]".to_string(),
        TypeMeta::AsyncOperation(result) => format!(
            "WinRTCoroutine[{}]",
            nested(result, Spelling::Value, OutputPosition::AsyncResult)
        ),
        TypeMeta::AsyncActionWithProgress(progress) => format!(
            "WinRTCoroutineWithProgress[None, {}]",
            nested(progress, Spelling::Value, OutputPosition::AsyncProgress)
        ),
        TypeMeta::AsyncOperationWithProgress(result, progress) => format!(
            "WinRTCoroutineWithProgress[{}, {}]",
            nested(result, Spelling::Value, OutputPosition::AsyncResult),
            nested(progress, Spelling::Value, OutputPosition::AsyncProgress)
        ),
        TypeMeta::Enum { .. } if !context.is_known_type(typ) => "int".to_string(),
        TypeMeta::RuntimeClass { .. } | TypeMeta::Interface { .. }
            if !context.is_known_type(typ) =>
        {
            "DynWinRTValue".to_string()
        }
        TypeMeta::Array(inner) if matches!(inner.as_ref(), TypeMeta::U8) => "bytes".to_string(),
        TypeMeta::Array(inner) => format!(
            "list[{}]",
            nested(inner, Spelling::Element, OutputPosition::CollectionElement)
        ),
        TypeMeta::String | TypeMeta::Char16 => "str".to_string(),
        TypeMeta::Guid => "UUID".to_string(),
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
        TypeMeta::RuntimeClass { .. }
        | TypeMeta::Enum { .. }
        | TypeMeta::Interface { .. }
        | TypeMeta::Parameterized { .. } => {
            format!("'{}'", context.reference_name_for_type(typ))
        }
        TypeMeta::Object | TypeMeta::Delegate { .. } => "'DynWinRTValue'".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => "datetime".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => "timedelta".to_string(),
        TypeMeta::Struct { .. } => format!("'{}'", context.reference_name_for_type(typ)),
    }
}

fn spell_collection(
    typ: &TypeMeta,
    site: OutputSite,
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> Option<String> {
    let TypeMeta::Parameterized { args, .. } = typ else {
        return None;
    };
    let abc = type_kind(typ).and_then(abc_name)?;
    let elements = args
        .iter()
        .map(|arg| {
            render_output(
                arg,
                Spelling::Element,
                site.nested(OutputPosition::CollectionElement),
                surface,
                context,
            )
        })
        .collect::<Vec<_>>();
    Some(format!("{abc}[{}]", elements.join(", ")))
}

/// Pessimistic rendering for callers outside the output policy (callback
/// parameters and the `IReference<T>` input arm): every value that may
/// project as `None` admits it, as on the runtime surface.
pub(crate) fn py_return_type_safe(
    typ: Option<&TypeMeta>,
    context: &PythonProjectionContext,
) -> String {
    typ.map(|typ| {
        render_output(
            typ,
            Spelling::Value,
            OutputSite::of(OutputPosition::CallbackParam),
            AnnotationSurface::Runtime,
            context,
        )
    })
    .unwrap_or_else(|| "None".to_string())
}

/// Annotation of a property getter's value.
pub(super) fn py_property_type(
    typ: &TypeMeta,
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> String {
    py_output_annotation(
        typ,
        OutputSite::of(OutputPosition::Property),
        surface,
        context,
    )
}

/// Item, key or value type of a projected collection class.
pub(super) fn py_collection_item_type(
    typ: &TypeMeta,
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> String {
    py_output_annotation(
        typ,
        OutputSite::of(OutputPosition::CollectionElement),
        surface,
        context,
    )
}

/// `Sequence[T]` / `Mapping[K, V]` base of a projected collection class.
pub(super) fn py_collection_base_type(
    abc: &str,
    args: &[TypeMeta],
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> Option<String> {
    let item = |typ| py_collection_item_type(typ, surface, context);
    match args {
        [element] => Some(format!("{abc}[{}]", item(element))),
        [key, value] => Some(format!("{abc}[{}, {}]", item(key), item(value))),
        _ => None,
    }
}

/// Result annotation of an activation factory creating `class_name`.
pub(super) fn py_factory_return_type(
    class_name: &str,
    method: &MethodMeta,
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> String {
    let instance = |typ: &TypeMeta| {
        let instance = format!("'{class_name}'");
        let site = OutputSite::for_method(method, OutputPosition::Activation);
        if output_admits_none(typ, site, surface, context) {
            py_optional_type(instance)
        } else {
            instance
        }
    };
    let progress = |typ: &TypeMeta| {
        render_output(
            typ,
            Spelling::Value,
            OutputSite::for_method(method, OutputPosition::AsyncProgress),
            surface,
            context,
        )
    };
    match method.return_type.as_ref() {
        None => format!("'{class_name}'"),
        Some(TypeMeta::AsyncAction) => "WinRTCoroutine[None]".to_string(),
        Some(TypeMeta::AsyncOperation(result)) => {
            format!("WinRTCoroutine[{}]", instance(result))
        }
        Some(TypeMeta::AsyncActionWithProgress(progress_type)) => {
            format!(
                "WinRTCoroutineWithProgress[None, {}]",
                progress(progress_type)
            )
        }
        Some(TypeMeta::AsyncOperationWithProgress(result, progress_type)) => format!(
            "WinRTCoroutineWithProgress[{}, {}]",
            instance(result),
            progress(progress_type)
        ),
        Some(typ) => instance(typ),
    }
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
    py_method_output_positions(method)
        .into_iter()
        .enumerate()
        .map(|(result_index, (typ, _))| (result_index, typ))
        .collect()
}

/// Logical outputs in result order: out values first, then the return value.
fn py_method_output_positions(method: &MethodMeta) -> Vec<(&TypeMeta, OutputPosition)> {
    let mut outputs = Vec::new();

    for param in &method.params {
        match param.direction {
            crate::meta::ParamDirection::Out => {
                outputs.push((&param.typ, OutputPosition::OutParam));
            }
            crate::meta::ParamDirection::OutFill => {
                // The runtime allocates a distinct filled result buffer; the
                // caller-provided array supplies capacity and is not mutated.
                outputs.push((&param.typ, OutputPosition::OutParam));
            }
            crate::meta::ParamDirection::In => {}
        }
    }

    if let Some(return_type) = method
        .return_type
        .as_ref()
        .filter(|_| !fill_array_uses_retval_count(method))
    {
        outputs.push((return_type, OutputPosition::Return));
    }

    outputs
}

pub(super) fn py_method_return_type(
    method: &MethodMeta,
    surface: AnnotationSurface,
    context: &PythonProjectionContext,
) -> String {
    let outputs = py_method_output_positions(method)
        .into_iter()
        .map(|(typ, position)| {
            py_output_annotation(
                typ,
                OutputSite::for_method(method, position),
                surface,
                context,
            )
        })
        .collect::<Vec<_>>();
    match outputs.as_slice() {
        [] => "None".to_string(),
        [output] => output.clone(),
        _ => format!("tuple[{}]", outputs.join(", ")),
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

pub(super) fn py_param_list(
    in_params: &[&crate::meta::ParamMeta],
    context: &PythonProjectionContext,
) -> String {
    in_params
        .iter()
        .map(|p| {
            let param_type = match &p.typ {
                typ if context.is_delegate_type(typ) => py_delegate_param_type(typ, context),
                _ => py_param_type_safe(&p.typ, context),
            };
            format!("{}: {}", to_snake_case(&p.name), param_type)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Produce a typed Python annotation for a delegate parameter, with
/// `TypedEventHandler` / `EventHandler` unwrapped. Bespoke non-parametric
/// delegates fall back to `Callable[..., object]`.
pub(crate) fn py_delegate_callable_type(
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> String {
    match typ {
        TypeMeta::Parameterized { name, args, .. }
            if name.split('`').next() == Some("TypedEventHandler") && args.len() == 2 =>
        {
            let sender = py_return_type_safe(Some(&args[0]), context);
            let arg = py_return_type_safe(Some(&args[1]), context);
            format!("Callable[[{}, {}], object]", sender, arg)
        }
        TypeMeta::Parameterized { name, args, .. }
            if name.split('`').next() == Some("EventHandler") && args.len() == 1 =>
        {
            let arg = py_return_type_safe(Some(&args[0]), context);
            format!("Callable[[object, {}], object]", arg)
        }
        TypeMeta::Parameterized { name, args, .. }
            if name.split('`').next() == Some("VectorChangedEventHandler") && args.len() == 1 =>
        {
            let observable_identity = crate::types::TypeIdentity::closed_generic(
                crate::types::TypeIdentityKind::Interface,
                crate::meta::WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
                "IObservableVector",
                args.iter().map(TypeMeta::type_identity),
            );
            let observable = context.reference_name(&observable_identity);
            format!(
                "Callable[['{}', 'IVectorChangedEventArgs'], object]",
                observable
            )
        }
        _ => "Callable[..., object]".to_string(),
    }
}

fn py_delegate_param_type(typ: &TypeMeta, context: &PythonProjectionContext) -> String {
    let sig = py_delegate_callable_type(typ, context);
    format!("{sig} | 'DynWinRTValue'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{ParamDirection, ParamMeta};

    fn returned(
        typ: &TypeMeta,
        surface: AnnotationSurface,
        context: &PythonProjectionContext,
    ) -> String {
        py_output_annotation(
            typ,
            OutputSite::of(OutputPosition::Return),
            surface,
            context,
        )
    }

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
            py_method_return_type(
                &method,
                AnnotationSurface::Stub,
                &PythonProjectionContext::default()
            ),
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
            py_method_return_type(
                &method,
                AnnotationSurface::Stub,
                &PythonProjectionContext::default()
            ),
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
        let array = TypeMeta::Array(Box::new(TypeMeta::Object));
        for surface in [AnnotationSurface::Runtime, AnnotationSurface::Stub] {
            assert_eq!(
                returned(&array, surface, &PythonProjectionContext::default()),
                "list[DynWinRTValue | None]"
            );
        }
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
            returned(&TypeMeta::Object, AnnotationSurface::Stub, &context),
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
            returned(&TypeMeta::Object, AnnotationSurface::Stub, &context),
            "DynWinRTValue | None"
        );
        assert_eq!(
            returned(
                &TypeMeta::Array(Box::new(TypeMeta::Object)),
                AnnotationSurface::Stub,
                &context
            ),
            "list[DynWinRTValue | None]"
        );
    }

    #[test]
    fn runtime_reference_outputs_stay_nullable() {
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
        let runtime = AnnotationSurface::Runtime;

        assert_eq!(returned(&runtime_class, runtime, &context), "Widget | None");
        assert_eq!(returned(&interface, runtime, &context), "IWidget | None");
        assert_eq!(
            returned(&TypeMeta::Object, runtime, &context),
            "DynWinRTValue | None"
        );
        assert_eq!(
            returned(
                &TypeMeta::Array(Box::new(runtime_class.clone())),
                runtime,
                &context
            ),
            "list[Widget | None]"
        );
        assert_eq!(
            py_return_type_safe(Some(&runtime_class), &context),
            "Widget | None"
        );
    }

    #[test]
    fn stub_outputs_are_non_null_except_policy_exceptions() {
        let widget = TypeMeta::RuntimeClass {
            namespace: "Contoso".into(),
            name: "Widget".into(),
            default_interface: None,
        };
        let interface = TypeMeta::Interface {
            namespace: "Contoso".into(),
            name: "IWidget".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
        };
        let unknown = TypeMeta::RuntimeClass {
            namespace: "Contoso".into(),
            name: "NotGenerated".into(),
            default_interface: None,
        };
        let context = PythonProjectionContext::standalone([
            widget.type_identity(),
            interface.type_identity(),
        ])
        .unwrap();
        let widgets = TypeMeta::Parameterized {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IVectorView`1".into(),
            piid: crate::codegen::winrt::python::collections::IVECTOR_VIEW_PIID.into(),
            args: vec![widget.clone()],
        };
        let stub = AnnotationSurface::Stub;
        let async_of = |typ: &TypeMeta| TypeMeta::AsyncOperation(Box::new(typ.clone()));
        let method = |raw_name: &str, params: Vec<ParamMeta>, return_type: TypeMeta| MethodMeta {
            name: raw_name.into(),
            raw_name: raw_name.into(),
            params,
            return_type: Some(return_type),
            ..Default::default()
        };

        assert_eq!(returned(&widget, stub, &context), "Widget");
        assert_eq!(returned(&interface, stub, &context), "IWidget");
        assert_eq!(returned(&unknown, stub, &context), "DynWinRTValue");
        assert_eq!(
            returned(&TypeMeta::Array(Box::new(widget.clone())), stub, &context),
            "list[Widget]"
        );
        assert_eq!(
            returned(&async_of(&widget), stub, &context),
            "WinRTCoroutine[Widget]"
        );
        assert_eq!(
            returned(&async_of(&widgets), stub, &context),
            "WinRTCoroutine[Sequence[Widget]]"
        );
        assert_eq!(
            returned(&TypeMeta::Object, stub, &context),
            "DynWinRTValue | None"
        );
        assert_eq!(py_property_type(&widget, stub, &context), "Widget");
        assert_eq!(py_collection_item_type(&widget, stub, &context), "Widget");
        assert_eq!(
            py_collection_base_type("Sequence", std::slice::from_ref(&widget), stub, &context),
            Some("Sequence[Widget]".to_string())
        );

        let get_item = method("GetItemAsync", vec![], async_of(&widget));
        let try_get_item = method("TryGetItemAsync", vec![], async_of(&widget));
        let try_get_items = method("TryGetItemsAsync", vec![], async_of(&widgets));
        let try_parse = method(
            "TryParse",
            vec![
                ParamMeta {
                    name: "input".into(),
                    typ: TypeMeta::String,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "result".into(),
                    typ: widget.clone(),
                    direction: ParamDirection::Out,
                },
            ],
            TypeMeta::Bool,
        );
        for (method, stub_type, runtime_type) in [
            (
                &get_item,
                "WinRTCoroutine[Widget]",
                "WinRTCoroutine[Widget | None]",
            ),
            (
                &try_get_item,
                "WinRTCoroutine[Widget | None]",
                "WinRTCoroutine[Widget | None]",
            ),
            (
                &try_get_items,
                "WinRTCoroutine[Sequence[Widget] | None]",
                "WinRTCoroutine[Sequence[Widget | None] | None]",
            ),
            (
                &try_parse,
                "tuple[Widget | None, bool]",
                "tuple[Widget | None, bool]",
            ),
        ] {
            assert_eq!(py_method_return_type(method, stub, &context), stub_type);
            assert_eq!(
                py_method_return_type(method, AnnotationSurface::Runtime, &context),
                runtime_type
            );
        }

        let create = method("CreateWidget", vec![], widget.clone());
        let try_create = method("TryCreateWidget", vec![], widget.clone());
        for surface in [AnnotationSurface::Runtime, stub] {
            assert_eq!(
                py_factory_return_type("Widget", &create, surface, &context),
                "'Widget'"
            );
        }
        assert_eq!(
            py_factory_return_type("Widget", &try_create, stub, &context),
            "Widget | None"
        );
        assert_eq!(
            py_factory_return_type("Widget", &try_create, AnnotationSurface::Runtime, &context),
            "'Widget'"
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

        for surface in [AnnotationSurface::Runtime, AnnotationSurface::Stub] {
            assert_eq!(
                returned(&reference, surface, &PythonProjectionContext::default()),
                "int | None"
            );
        }
        assert_eq!(
            py_param_type_safe(&reference, &PythonProjectionContext::default()),
            "int | None | IReference_UInt32"
        );
        assert_eq!(py_optional_type("'DayOfWeek'".into()), "DayOfWeek | None");
    }
}
