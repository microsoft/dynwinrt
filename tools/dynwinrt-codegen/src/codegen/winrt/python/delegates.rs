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

use super::naming::{PythonProjectionContext, to_snake_case};
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
/// `Invoke` inputs. `None` when the signature is unknown or has outputs.
fn callback_params<'a>(
    typ: &TypeMeta,
    context: &'a PythonProjectionContext,
) -> Option<&'a [ParamMeta]> {
    let invoke = context.delegate_invoke(typ)?;
    invoke
        .params
        .iter()
        .all(|param| param.direction == ParamDirection::In)
        .then_some(invoke.params.as_slice())
}

/// Annotation of one argument passed to a Python callback.
///
/// WinRT passes null delegate arguments only for `Object` and `IReference<T>`.
/// Every callback-argument annotation goes through this function so a
/// position-aware output-nullability policy can take it over.
pub(crate) fn py_delegate_argument_type(
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> String {
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
    if context.is_delegate_type(typ) {
        return format!("(lambda value: None if value.is_null() else value)({expr})");
    }
    py_convert_return(expr, Some(typ), typ.is_async(), context)
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
/// native delegate value.
pub(crate) fn py_delegate_param_type(typ: &TypeMeta, context: &PythonProjectionContext) -> String {
    let sig = py_delegate_callable_type(typ, context);
    format!("{sig} | 'DynWinRTValue'")
}

/// `lambda <native args>: callback(<projected args>)`, adapting a Python
/// callable named `callback` to the delegate's native arguments. `None` when
/// there is nothing to project.
fn py_callback_adapter(typ: &TypeMeta, context: &PythonProjectionContext) -> Option<String> {
    let params = callback_params(typ, context)?;
    if params.is_empty() {
        return None;
    }
    let mut names = Vec::<String>::new();
    for (index, param) in params.iter().enumerate() {
        let name = format!("__{}__", to_snake_case(&param.name));
        names.push(if param.name.is_empty() || names.contains(&name) {
            format!("__arg{index}__")
        } else {
            name
        });
    }
    let arguments = params
        .iter()
        .zip(&names)
        .map(|(param, name)| py_delegate_argument(name, &param.typ, context))
        .collect::<Vec<_>>();
    Some(format!(
        "lambda {}: callback({})",
        names.join(", "),
        arguments.join(", ")
    ))
}

/// Build a Python callback signature + wrapper expression for an event delegate.
///
/// Returns `(signature, wrapper)`:
/// - `signature` is a Python type annotation (e.g., `Callable[['Foo', 'Bar'], object]`).
/// - `wrapper` is an expression that produces the ABI-facing callable, projecting
///   raw `DynWinRTValue` arguments into Python values before invoking the user's
///   `callback`.
///
/// The wrapper falls back to a passthrough (`callback`) when the delegate
/// signature is unknown.
pub(crate) fn py_event_callback(
    typ: Option<&TypeMeta>,
    context: &PythonProjectionContext,
) -> (String, String) {
    let Some(typ) = typ else {
        return ("Callable[..., object]".to_string(), "callback".to_string());
    };
    let wrapper = py_callback_adapter(typ, context).map_or_else(
        || "callback".to_string(),
        |adapter| format!("(lambda callback=callback: ({adapter}))()"),
    );
    (py_delegate_callable_type(typ, context), wrapper)
}

/// Convert a delegate-typed method, static, or setter argument: an existing
/// native delegate passes through; a Python callable becomes a new delegate
/// whose arguments are projected like event arguments.
pub(crate) fn py_delegate_input_arg(
    name: &str,
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> Option<String> {
    let abi = delegate_abi(typ, context)?;
    Some(match py_callback_adapter(typ, context) {
        Some(adapter) => format!(
            "_dynwinrt_delegate({name}, {}, {}, lambda callback: ({adapter}))",
            abi.iid, abi.param_types
        ),
        None => format!(
            "_dynwinrt_delegate({name}, {}, {})",
            abi.iid, abi.param_types
        ),
    })
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
        let (signature, wrapper) = py_event_callback(Some(&handler), &context);
        assert_eq!(signature, py_delegate_callable_type(&handler, &context));
        assert_eq!(
            wrapper,
            "(lambda callback=callback: (lambda __sender__, __e__: callback(\
             (lambda value: None if value.is_null() else value)(__sender__), \
             (lambda value: None if value.is_null() else \
             _dynwinrt_symbol('contoso__clicked_event_args', 'ClickedEventArgs')._from_native(value))(__e__))))()"
        );
        assert_eq!(
            py_delegate_input_arg("handler", &handler, &context).unwrap(),
            format!(
                "_dynwinrt_delegate(handler, \
                 _dynwinrt_symbol('clicked_handler', 'IID_ClickedHandler'), \
                 _dynwinrt_symbol('clicked_handler', 'ClickedHandler_PARAM_TYPES'), \
                 lambda callback: ({}))",
                wrapper
                    .strip_prefix("(lambda callback=callback: (")
                    .and_then(|body| body.strip_suffix("))()"))
                    .unwrap()
            )
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
        assert_eq!(py_event_callback(Some(&empty), &context).1, "callback");
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
        assert_eq!(py_event_callback(Some(&unknown), &context).1, "callback");
        assert!(py_delegate_input_arg("handler", &unknown, &context).is_none());
    }
}
