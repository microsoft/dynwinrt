// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python projection of WinRT delegates.
//!
//! This module owns every conversion between a Python callable and a native
//! delegate. The delegate's `Invoke` signature, with generic arguments
//! substituted, drives both the `Callable[...]` annotation offered to callers
//! and the adapter that projects native arguments before the callable runs.

use crate::codegen::winrt::shared::imports::ireference_inner_type;
use crate::meta::{ParamDirection, ParamMeta};
use crate::types::TypeMeta;

use super::naming::PythonProjectionContext;
use super::signature::{py_convert_return, py_runtime_symbol};
use super::type_helpers::{py_output_type, py_return_type, py_return_type_safe};

/// Generated symbols that describe a delegate's native ABI.
pub(crate) struct DelegateAbi {
    /// Expression evaluating to the delegate IID.
    pub(crate) iid: String,
    /// Expression evaluating to the delegate's `Invoke` parameter types.
    pub(crate) param_types: String,
}

/// Resolve the generated IID and parameter-type symbols of a delegate type.
pub(crate) fn delegate_abi(
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> Option<DelegateAbi> {
    if !context.is_delegate_type(typ) {
        return None;
    }
    let identity = context.identity_for_type(typ);
    let projected_name = context.projected_name(&identity);
    Some(DelegateAbi {
        iid: py_runtime_symbol(context, &identity, &format!("IID_{projected_name}")),
        param_types: py_runtime_symbol(
            context,
            &identity,
            &format!("{projected_name}_PARAM_TYPES"),
        ),
    })
}

/// The native arguments a Python callable receives for a delegate: its
/// `Invoke` inputs. `None` when the type is not a generated delegate, or its
/// signature is unknown or has outputs.
fn callback_params<'a>(
    typ: &TypeMeta,
    context: &'a PythonProjectionContext,
) -> Option<&'a [ParamMeta]> {
    if !context.is_delegate_type(typ) {
        return None;
    }
    let invoke = context.delegate_invoke(typ)?;
    invoke
        .params
        .iter()
        .all(|param| param.direction == ParamDirection::In)
        .then_some(invoke.params.as_slice())
}

/// Annotation of one argument passed to a Python callback.
///
/// Callback arguments are annotated as non-null, except WinRT `Object` and
/// `IReference<T>`. WinMD metadata does not record nullability, so this is an
/// optimistic policy shared with method outputs: the runtime still passes
/// `None` for a null reference. Every callback-argument annotation goes through
/// this function so a position-aware output-nullability policy can take it
/// over.
pub(crate) fn py_delegate_argument_type(
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> String {
    if typ.is_async() {
        return "DynWinRTValue".to_string();
    }
    if context.is_delegate_type(typ) {
        return py_output_type(typ, context);
    }
    let unknown = matches!(
        typ,
        TypeMeta::RuntimeClass { .. }
            | TypeMeta::Interface { .. }
            | TypeMeta::Enum { .. }
            | TypeMeta::Parameterized { .. }
    ) && !context.is_known_type(typ);
    if unknown || matches!(typ, TypeMeta::Object) || ireference_inner_type(typ).is_some() {
        py_return_type_safe(Some(typ), context)
    } else {
        py_return_type(Some(typ), context)
    }
}

/// Project one native callback argument the way a method return is projected.
fn py_delegate_argument(expr: &str, typ: &TypeMeta, context: &PythonProjectionContext) -> String {
    // An awaitable wrapper would take over the operation's completion and
    // cancel it on release, so async arguments stay raw values.
    if typ.is_async() {
        return expr.to_string();
    }
    if context.is_delegate_type(typ) {
        return format!("(lambda value: None if value.is_null() else value)({expr})");
    }
    py_convert_return(expr, Some(typ), false, context)
}

/// `Callable[[...], object]` derived from the delegate's `Invoke` signature, or
/// `Callable[..., object]` when the signature is unavailable.
pub(crate) fn py_delegate_callable_type(
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> String {
    let Some(params) = callback_params(typ, context) else {
        return "Callable[..., object]".to_string();
    };
    let arguments = params
        .iter()
        .map(|param| py_delegate_argument_type(&param.typ, context))
        .collect::<Vec<_>>();
    format!("Callable[[{}], object]", arguments.join(", "))
}

/// Annotation for a delegate-typed input: a Python callable or an existing
/// native delegate object/value.
pub(crate) fn py_delegate_param_type(typ: &TypeMeta, context: &PythonProjectionContext) -> String {
    let sig = py_delegate_callable_type(typ, context);
    format!("{sig} | 'DynWinRTValue | DynWinRtDelegate'")
}

/// Constructor overload resolution cannot distinguish a raw delegate value
/// from the one-argument native-wrapper shortcut in `__new__`, so constructor
/// stubs advertise callable or delegate-object inputs only.
pub(crate) fn py_delegate_constructor_param_type(
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> String {
    let sig = py_delegate_callable_type(typ, context);
    format!("{sig} | 'DynWinRtDelegate'")
}

/// `lambda <native args>: (<projected args>)`, projecting a delegate's native
/// arguments for a Python callable. `None` when no argument needs projection.
fn py_callback_projection(typ: &TypeMeta, context: &PythonProjectionContext) -> Option<String> {
    let params = callback_params(typ, context)?;
    // Positional names stay distinct even when metadata names normalize alike.
    let names = (0..params.len())
        .map(|index| format!("__p{index}__"))
        .collect::<Vec<_>>();
    let arguments = params
        .iter()
        .zip(&names)
        .map(|(param, name)| py_delegate_argument(name, &param.typ, context))
        .collect::<Vec<_>>();
    if arguments
        .iter()
        .zip(&names)
        .all(|(argument, name)| argument == name)
    {
        return None;
    }
    let tuple = if arguments.len() == 1 {
        format!("({},)", arguments[0])
    } else {
        format!("({})", arguments.join(", "))
    };
    Some(format!("lambda {}: {tuple}", names.join(", ")))
}

/// Convert a delegate-typed input: an existing native delegate passes through;
/// a Python callable becomes a new delegate whose arguments are projected.
pub(crate) fn py_delegate_input_arg(
    name: &str,
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> Option<String> {
    let abi = delegate_abi(typ, context)?;
    Some(match py_callback_projection(typ, context) {
        Some(projection) => format!(
            "_dynwinrt_delegate({name}, {}, {}, {projection})",
            abi.iid, abi.param_types
        ),
        None => format!(
            "_dynwinrt_delegate({name}, {}, {})",
            abi.iid, abi.param_types
        ),
    })
}

/// The delegate value an `on_<event>` method registers for `callback`.
pub(crate) fn py_event_handler_arg(
    name: &str,
    typ: Option<&TypeMeta>,
    context: &PythonProjectionContext,
) -> String {
    typ.and_then(|typ| py_delegate_input_arg(name, typ, context))
        .unwrap_or_else(|| {
            format!(
                "_dynwinrt_delegate({name}, DynWinRTType.object().iid(), \
                 [DynWinRTType.object(), DynWinRTType.object()])"
            )
        })
}

/// Reject native delegates in `once_<event>`, which must wrap a Python
/// callable to remove the subscription after its first invocation.
pub(crate) fn py_once_callback_check(name: &str, event: &str, indent: &str) -> String {
    format!(
        "{indent}if not callable({name}) or isinstance(getattr({name}, '_obj', {name}), DynWinRTValue):\n\
         {indent}    raise TypeError('once_{event} requires a Python callable; \
         use on_{event} or subscribe_{event} for native delegates')\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{ImplementationDelegateMeta, InterfaceMeta, MethodMeta};
    use crate::types::{TypeIdentity, TypeIdentityKind};

    fn named(kind: TypeIdentityKind, name: &str) -> TypeIdentity {
        TypeIdentity::named(kind, "Contoso", name)
    }

    fn class(name: &str) -> TypeMeta {
        TypeMeta::RuntimeClass {
            namespace: "Contoso".into(),
            name: name.into(),
            default_interface: None,
        }
    }

    fn delegate(name: &str) -> TypeMeta {
        TypeMeta::Interface {
            namespace: "Contoso".into(),
            name: name.into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
        }
    }

    fn input(name: &str, typ: TypeMeta) -> ParamMeta {
        ParamMeta {
            name: name.into(),
            typ,
            direction: ParamDirection::In,
        }
    }

    /// A context that generates `types` and knows the `Invoke` signature of
    /// every `(delegate, inputs)` pair.
    fn context(
        types: &[TypeIdentity],
        delegates: Vec<(TypeMeta, Vec<ParamMeta>)>,
    ) -> PythonProjectionContext {
        let mut identities = types.to_vec();
        identities.extend(
            delegates
                .iter()
                .map(|(typ, _)| typ.type_identity().with_kind(TypeIdentityKind::Delegate)),
        );
        let mut context = PythonProjectionContext::standalone(identities).unwrap();
        let owner = InterfaceMeta {
            implementation_metadata: crate::meta::InterfaceImplementationMetadata {
                delegates: delegates
                    .into_iter()
                    .map(|(typ, params)| ImplementationDelegateMeta {
                        typ,
                        invoke: MethodMeta {
                            name: "Invoke".into(),
                            params,
                            ..Default::default()
                        },
                    })
                    .collect(),
                ..Default::default()
            },
            ..Default::default()
        };
        context.register_delegate_invokes([&owner]);
        context
    }

    #[test]
    fn invoke_signature_types_and_projects_bespoke_delegates() {
        let handler = delegate("ClickedHandler");
        let context = context(
            &[named(TypeIdentityKind::Class, "ClickedEventArgs")],
            vec![(
                handler.clone(),
                vec![
                    input("sender", TypeMeta::Object),
                    input("e", class("ClickedEventArgs")),
                ],
            )],
        );

        assert_eq!(
            py_delegate_callable_type(&handler, &context),
            "Callable[[DynWinRTValue | None, 'ClickedEventArgs'], object]"
        );
        let handler_arg = "_dynwinrt_delegate(callback, \
             _dynwinrt_symbol('clicked_handler', 'IID_ClickedHandler'), \
             _dynwinrt_symbol('clicked_handler', 'ClickedHandler_PARAM_TYPES'), \
             lambda __p0__, __p1__: (\
             (lambda value: None if value.is_null() else value)(__p0__), \
             (lambda value: None if value.is_null() else \
             _dynwinrt_symbol('contoso__clicked_event_args', 'ClickedEventArgs')._from_native(value))(__p1__)))";
        assert_eq!(
            py_delegate_input_arg("callback", &handler, &context).unwrap(),
            handler_arg
        );
        assert_eq!(
            py_event_handler_arg("callback", Some(&handler), &context),
            handler_arg
        );
    }

    #[test]
    fn projection_parameters_are_positional_when_metadata_names_collide() {
        let handler = delegate("PairHandler");
        let context = context(
            &[],
            vec![(
                handler.clone(),
                vec![input("arg1", TypeMeta::I32), input("_arg1", TypeMeta::I32)],
            )],
        );

        let delegate = py_delegate_input_arg("handler", &handler, &context).unwrap();
        assert!(
            delegate.ends_with("lambda __p0__, __p1__: (__p0__.to_number(), __p1__.to_number()))"),
            "{delegate}"
        );
    }

    #[test]
    fn async_arguments_stay_raw_values() {
        let completed = delegate("AsyncActionCompletedHandler");
        let work = delegate("WorkItemHandler");
        let status = TypeMeta::Enum {
            namespace: "Contoso".into(),
            name: "AsyncStatus".into(),
            underlying: Box::new(TypeMeta::I32),
            members: Vec::new(),
            is_flags: false,
            doc: None,
            deprecated: None,
        };
        let context = context(
            &[named(TypeIdentityKind::Enum, "AsyncStatus")],
            vec![
                (
                    completed.clone(),
                    vec![
                        input("asyncInfo", TypeMeta::AsyncAction),
                        input("asyncStatus", status),
                    ],
                ),
                (
                    work.clone(),
                    vec![input(
                        "operation",
                        TypeMeta::AsyncOperationWithProgress(
                            Box::new(TypeMeta::U32),
                            Box::new(TypeMeta::U32),
                        ),
                    )],
                ),
            ],
        );

        assert_eq!(
            py_delegate_callable_type(&completed, &context),
            "Callable[[DynWinRTValue, 'AsyncStatus'], object]"
        );
        assert!(
            py_delegate_input_arg("handler", &completed, &context)
                .unwrap()
                .ends_with(
                    "lambda __p0__, __p1__: (__p0__, \
                     _dynwinrt_enum('contoso__async_status', 'AsyncStatus', __p1__.to_number())))"
                )
        );
        assert_eq!(
            py_delegate_callable_type(&work, &context),
            "Callable[[DynWinRTValue], object]"
        );
        assert_eq!(
            py_delegate_input_arg("handler", &work, &context).unwrap(),
            "_dynwinrt_delegate(handler, \
             _dynwinrt_symbol('work_item_handler', 'IID_WorkItemHandler'), \
             _dynwinrt_symbol('work_item_handler', 'WorkItemHandler_PARAM_TYPES'))"
        );
    }

    #[test]
    fn argument_annotations_are_non_null_except_object_and_references() {
        let reference = TypeMeta::Parameterized {
            namespace: "Windows.Foundation".into(),
            name: "IReference`1".into(),
            piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
            args: vec![TypeMeta::I32],
        };
        let mode = TypeMeta::Enum {
            namespace: "Contoso".into(),
            name: "Mode".into(),
            underlying: Box::new(TypeMeta::I32),
            members: Vec::new(),
            is_flags: false,
            doc: None,
            deprecated: None,
        };
        let handler = delegate("ChangedHandler");
        let context = context(
            &[
                named(TypeIdentityKind::Class, "Widget"),
                named(TypeIdentityKind::Enum, "Mode"),
                reference.type_identity(),
            ],
            vec![(
                handler.clone(),
                vec![
                    input("sender", class("Widget")),
                    input("mode", mode),
                    input("value", reference),
                    input("peer", class("Unknown")),
                ],
            )],
        );

        assert_eq!(
            py_delegate_callable_type(&handler, &context),
            "Callable[['Widget', 'Mode', int | None, DynWinRTValue | None], object]"
        );
    }

    #[test]
    fn delegates_without_arguments_or_signatures_pass_callables_through() {
        let empty = delegate("DispatchedHandler");
        let unknown = delegate("UnregisteredHandler");
        let context = context(&[], vec![(empty.clone(), Vec::new())]);

        assert_eq!(
            py_delegate_callable_type(&empty, &context),
            "Callable[[], object]"
        );
        assert_eq!(
            py_delegate_input_arg("handler", &empty, &context).unwrap(),
            "_dynwinrt_delegate(handler, \
             _dynwinrt_symbol('dispatched_handler', 'IID_DispatchedHandler'), \
             _dynwinrt_symbol('dispatched_handler', 'DispatchedHandler_PARAM_TYPES'))"
        );
        assert_eq!(
            py_delegate_callable_type(&unknown, &context),
            "Callable[..., object]"
        );
        assert!(py_delegate_input_arg("handler", &unknown, &context).is_none());
        assert_eq!(
            py_event_handler_arg("callback", Some(&unknown), &context),
            "_dynwinrt_delegate(callback, DynWinRTType.object().iid(), \
             [DynWinRTType.object(), DynWinRTType.object()])"
        );
    }

    #[test]
    fn once_rejects_native_delegates_before_subscribing() {
        assert_eq!(
            py_once_callback_check("callback", "changed", "        "),
            "        if not callable(callback) or isinstance(getattr(callback, '_obj', callback), DynWinRTValue):\n\
             \x20           raise TypeError('once_changed requires a Python callable; \
             use on_changed or subscribe_changed for native delegates')\n"
        );
    }
}
