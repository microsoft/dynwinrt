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
    context: &PythonProjectionContext,
) -> bool {
    if ireference_inner_type(typ).is_some() {
        return true;
    }
    if !may_project_none(typ) {
        return false;
    }
    match surface {
        // Runtime annotations stay pessimistic: `typing.get_type_hints()` and
        // `--no-pyi` consumers see every reference output as optional.
        AnnotationSurface::Runtime => site.position != OutputPosition::Activation,
        AnnotationSurface::Stub => stub_output_admits_none(typ, site, context),
    }
}

/// Stubs are optimistic, like the JavaScript declarations: WinRT metadata has
/// no nullability, and most APIs raise instead of returning null.
fn stub_output_admits_none(
    typ: &TypeMeta,
    site: OutputSite,
    context: &PythonProjectionContext,
) -> bool {
    use OutputPosition::{Activation, AsyncResult, CallbackParam, OutParam, Return};

    // `Object` positions are frequently null, e.g. the arguments of a
    // `TypedEventHandler<T, Object>`.
    if matches!(typ, TypeMeta::Object) {
        return true;
    }
    // Delegate-typed values are raw handles that are null while unset.
    if context.is_delegate_type(typ) {
        return site.position != CallbackParam;
    }
    // `Try*` members report "not found" through a null result.
    site.try_method && matches!(site.position, Return | OutParam | AsyncResult | Activation)
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

    const POSITIONS: [OutputPosition; 8] = [
        OutputPosition::Return,
        OutputPosition::OutParam,
        OutputPosition::Property,
        OutputPosition::AsyncResult,
        OutputPosition::AsyncProgress,
        OutputPosition::CollectionElement,
        OutputPosition::CallbackParam,
        OutputPosition::Activation,
    ];

    fn widget() -> TypeMeta {
        TypeMeta::RuntimeClass {
            namespace: "Contoso".into(),
            name: "Widget".into(),
            default_interface: None,
        }
    }

    fn handler() -> TypeMeta {
        TypeMeta::Delegate {
            namespace: "Contoso".into(),
            name: "Handler".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
        }
    }

    fn nullable_u32() -> TypeMeta {
        TypeMeta::Parameterized {
            namespace: "Windows.Foundation".into(),
            name: "IReference`1".into(),
            piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
            args: vec![TypeMeta::U32],
        }
    }

    fn admits(typ: &TypeMeta, site: OutputSite, surface: AnnotationSurface) -> bool {
        output_admits_none(typ, site, surface, &PythonProjectionContext::default())
    }

    #[test]
    fn runtime_annotations_stay_pessimistic() {
        for position in POSITIONS {
            let site = OutputSite::of(position);
            let expected = position != OutputPosition::Activation;
            for typ in [widget(), handler(), TypeMeta::Object] {
                assert_eq!(
                    admits(&typ, site, AnnotationSurface::Runtime),
                    expected,
                    "{typ:?} at {position:?}"
                );
            }
            assert!(admits(&nullable_u32(), site, AnnotationSurface::Runtime));
            assert!(!admits(&TypeMeta::String, site, AnnotationSurface::Runtime));
        }
    }

    #[test]
    fn stubs_are_non_null_by_default() {
        for position in POSITIONS {
            let site = OutputSite::of(position);
            assert!(
                !admits(&widget(), site, AnnotationSurface::Stub),
                "{position:?}"
            );
            assert!(!admits(&TypeMeta::I32, site, AnnotationSurface::Stub));
        }
    }

    #[test]
    fn stub_exceptions_keep_none() {
        for position in POSITIONS {
            let site = OutputSite::of(position);
            assert!(admits(&nullable_u32(), site, AnnotationSurface::Stub));
            assert!(admits(&TypeMeta::Object, site, AnnotationSurface::Stub));
            assert_eq!(
                admits(&handler(), site, AnnotationSurface::Stub),
                position != OutputPosition::CallbackParam,
                "{position:?}"
            );
        }
    }

    #[test]
    fn try_members_keep_none_on_their_results_only() {
        let try_get = method("TryGetItemAsync");
        for position in POSITIONS {
            let expected = matches!(
                position,
                OutputPosition::Return
                    | OutputPosition::OutParam
                    | OutputPosition::AsyncResult
                    | OutputPosition::Activation
            );
            assert_eq!(
                admits(
                    &widget(),
                    OutputSite::for_method(&try_get, position),
                    AnnotationSurface::Stub
                ),
                expected,
                "{position:?}"
            );
        }
        assert!(!admits(
            &TypeMeta::Bool,
            OutputSite::for_method(&try_get, OutputPosition::Return),
            AnnotationSurface::Stub
        ));
    }
}
