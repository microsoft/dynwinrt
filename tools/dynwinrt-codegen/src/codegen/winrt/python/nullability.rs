// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The one policy deciding whether a projected output annotation admits
//! `None`.
//!
//! WinRT metadata carries no nullability, so whether a consumer-facing value
//! is annotated `T | None` depends on the value's type, the position it is
//! received at, facts about the member producing it, and the generated file
//! the annotation is rendered into. Renderers in `type_helpers` spell the
//! non-null type expression; only [`output_admits_none`] decides whether
//! `| None` is appended.

use crate::codegen::winrt::shared::imports::ireference_inner_type;
use crate::meta::MethodMeta;
use crate::types::TypeMeta;

use super::naming::PythonProjectionContext;

/// The generated artifact an annotation is rendered into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnnotationSurface {
    /// Inline annotations of runtime `.py` modules. `typing.get_type_hints()`
    /// exposes them, and checkers read them when stubs are not generated.
    Runtime,
    /// `.pyi` stubs read by type checkers.
    Stub,
}

/// Where the consumer receives a value from the projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputPosition {
    /// Return value of a method.
    Return,
    /// Out value in a method's result tuple.
    OutParam,
    /// Value read from a property getter.
    Property,
    /// Completed value of an async operation.
    AsyncResult,
    /// Progress value reported by an async operation.
    AsyncProgress,
    /// Element, key or value read from a collection or an array.
    CollectionElement,
    /// Sender or argument the runtime passes to a consumer callback.
    CallbackParam,
    /// Instance created by an activation factory. Activation reports failure
    /// by raising, so the instance is never null unless the factory follows
    /// the `Try*` pattern.
    Activation,
}

/// A position plus the facts about the member producing the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputSite {
    pub(crate) position: OutputPosition,
    /// The member follows the `Try*` pattern, so a null result is part of
    /// its contract ("not found", "could not parse").
    pub(crate) try_method: bool,
}

impl OutputSite {
    pub(crate) fn of(position: OutputPosition) -> Self {
        Self {
            position,
            try_method: false,
        }
    }

    pub(crate) fn for_method(method: &MethodMeta, position: OutputPosition) -> Self {
        Self {
            position,
            try_method: is_try_method(method),
        }
    }

    /// The site of a value nested in this one, such as an async result or a
    /// collection element. Member facts carry over; the policy decides where
    /// they apply.
    pub(crate) fn nested(self, position: OutputPosition) -> Self {
        Self { position, ..self }
    }
}

/// `TryParse`, `TryGetItemAsync`, ...: the CLR name is `Try` followed by an
/// uppercase letter.
pub(crate) fn is_try_method(method: &MethodMeta) -> bool {
    method
        .raw_name
        .strip_prefix("Try")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|next| next.is_ascii_uppercase())
}

/// Whether the runtime converts a null ABI value of `typ` to `None`:
/// interface pointers (objects, classes, interfaces, delegates and
/// parameterized interfaces, including `IReference<T>`). Value types,
/// strings, arrays and async operations never project as `None`.
pub(crate) fn may_project_none(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Object
            | TypeMeta::Delegate { .. }
            | TypeMeta::RuntimeClass { .. }
            | TypeMeta::Interface { .. }
            | TypeMeta::Parameterized { .. }
    )
}

/// Whether the annotation of `typ` received at `site` admits `None` on
/// `surface`. This is the only place that makes that decision for outputs.
pub(crate) fn output_admits_none(
    typ: &TypeMeta,
    site: OutputSite,
    surface: AnnotationSurface,
    _context: &PythonProjectionContext,
) -> bool {
    if ireference_inner_type(typ).is_some() {
        return true;
    }
    if !may_project_none(typ) {
        return false;
    }
    match (surface, site.position) {
        (_, OutputPosition::Activation) => false,
        (AnnotationSurface::Runtime, _) => true,
        (AnnotationSurface::Stub, _) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(raw_name: &str) -> MethodMeta {
        MethodMeta {
            name: raw_name.into(),
            raw_name: raw_name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn try_methods_require_an_uppercase_continuation() {
        assert!(is_try_method(&method("TryParse")));
        assert!(is_try_method(&method("TryGetItemAsync")));
        assert!(!is_try_method(&method("Try")));
        assert!(!is_try_method(&method("Trying")));
        assert!(!is_try_method(&method("GetTryCount")));
        assert!(!is_try_method(&method("get_TryCount")));
    }

    #[test]
    fn nested_sites_keep_member_facts() {
        let site = OutputSite::for_method(&method("TryGetItemAsync"), OutputPosition::Return);
        assert_eq!(
            site.nested(OutputPosition::AsyncResult),
            OutputSite {
                position: OutputPosition::AsyncResult,
                try_method: true,
            }
        );
    }
}
