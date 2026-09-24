// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python projection of WinRT delegates.
//!
//! This module owns every conversion between a Python callable and a native
//! delegate: the `Callable[...]` annotation offered to callers and the adapter
//! that projects a delegate's native arguments before the callable runs.

use crate::types::{TypeIdentity, TypeIdentityKind, TypeMeta};

use super::naming::PythonProjectionContext;
use super::signature::{py_convert_return, py_runtime_named_symbol, py_runtime_symbol};
use super::type_helpers::py_return_type_safe;

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
            let observable_identity = TypeIdentity::closed_generic(
                TypeIdentityKind::Interface,
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

/// Annotation for a delegate-typed input: a Python callable or an existing
/// native delegate value.
pub(crate) fn py_delegate_param_type(typ: &TypeMeta, context: &PythonProjectionContext) -> String {
    let sig = py_delegate_callable_type(typ, context);
    format!("{sig} | 'DynWinRTValue'")
}

/// Build a Python callback signature + wrapper expression for an event delegate.
///
/// Returns `(signature, wrapper)`:
/// - `signature` is a Python type annotation (e.g., `Callable[['Foo', 'Bar'], object]`).
/// - `wrapper` is an expression that produces the ABI-facing callable, unwrapping
///   raw `DynWinRTValue` sender/args back into projected Python objects before
///   invoking the user's `callback`.
///
/// The wrapper falls back to a passthrough (`callback`) for unknown delegate shapes.
pub(crate) fn py_event_callback(
    typ: Option<&TypeMeta>,
    context: &PythonProjectionContext,
) -> (String, String) {
    match typ {
        Some(typ @ TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("TypedEventHandler") && args.len() == 2 =>
        {
            let sender_conv = py_convert_return("__sender__", Some(&args[0]), false, context);
            let args_conv = py_convert_return("__args__", Some(&args[1]), false, context);
            let sig = py_delegate_callable_type(typ, context);
            let wrapper = format!(
                "(lambda callback=callback: (lambda __sender__, __args__: callback({}, {})))()",
                sender_conv, args_conv
            );
            (sig, wrapper)
        }
        Some(typ @ TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("EventHandler") && args.len() == 1 =>
        {
            let args_conv = py_convert_return("__args__", Some(&args[0]), false, context);
            let sig = py_delegate_callable_type(typ, context);
            let wrapper = format!(
                "(lambda callback=callback: (lambda __sender__, __args__: callback(__sender__, {})))()",
                args_conv
            );
            (sig, wrapper)
        }
        Some(typ @ TypeMeta::Parameterized { name, args, .. })
            if name.split('`').next() == Some("VectorChangedEventHandler") && args.len() == 1 =>
        {
            let observable_identity = TypeIdentity::closed_generic(
                TypeIdentityKind::Interface,
                crate::meta::WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
                "IObservableVector",
                args.iter().map(TypeMeta::type_identity),
            );
            let observable_name = context.projected_name(&observable_identity);
            let sender = format!(
                "(lambda value: None if value.is_null() else {}(value))(__sender__)",
                py_runtime_symbol(context, &observable_identity, &observable_name)
            );
            let event_args = format!(
                "(lambda value: None if value.is_null() else {}(value))(__args__)",
                py_runtime_named_symbol(
                    context,
                    TypeIdentityKind::Interface,
                    crate::meta::WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
                    "IVectorChangedEventArgs",
                    "IVectorChangedEventArgs",
                )
            );
            let sig = py_delegate_callable_type(typ, context);
            let wrapper = format!(
                "(lambda callback=callback: (lambda __sender__, __args__: callback({}, {})))()",
                sender, event_args
            );
            (sig, wrapper)
        }
        _ => ("Callable[..., object]".to_string(), "callback".to_string()),
    }
}

/// Convert a delegate-typed method, static, or setter argument: an existing
/// native delegate passes through; a Python callable becomes a new delegate.
pub(crate) fn py_delegate_input_arg(
    name: &str,
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> Option<String> {
    let abi = delegate_abi(typ, context)?;
    Some(format!(
        "_dynwinrt_delegate({name}, {}, {})",
        abi.iid, abi.param_types
    ))
}
