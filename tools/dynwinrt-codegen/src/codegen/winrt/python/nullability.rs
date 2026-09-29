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
use crate::meta::{ElementAccess, MethodMeta};
use crate::types::TypeMeta;

use super::collections::CollectionKind;
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

/// The collection holding an element. Whether a reference-type element admits
/// `None` depends only on this; see [`element_admits_none`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ElementContainer {
    /// `IIterable`, `IIterator`, `IVectorView`, `IMapView` or `IKeyValuePair`.
    View,
    /// `IVector`, `IMap` or their observable forms.
    Mutable,
    /// A map key or `IKeyValuePair.Key`. Reference keys can be null; string
    /// and value-type keys cannot.
    MapKey,
    /// An array returned by a member that does not read collection elements.
    Array,
}

impl ElementContainer {
    pub(crate) fn of(kind: CollectionKind) -> Self {
        match kind {
            CollectionKind::MutableSequence | CollectionKind::MutableMapping => Self::Mutable,
            CollectionKind::Iterable
            | CollectionKind::Iterator
            | CollectionKind::Sequence
            | CollectionKind::Mapping
            | CollectionKind::KeyValuePair => Self::View,
        }
    }
}

impl From<ElementAccess> for ElementContainer {
    fn from(access: ElementAccess) -> Self {
        match access {
            ElementAccess::ReadOnly => Self::View,
            ElementAccess::Mutable => Self::Mutable,
            ElementAccess::MapKey => Self::MapKey,
        }
    }
}

/// The collection element rule: a WinRT collection interface does not carry
/// the provenance needed to prove that a reference-type element is non-null.
/// In particular, a view or iterator obtained from a mutable collection can
/// expose a null slot. All WinRT collection element reads therefore admit
/// `None`. Returned arrays also preserve null reference slots at runtime, so
/// their reference elements follow the same rule while the array itself
/// remains non-null.
pub(crate) fn element_admits_none(container: ElementContainer) -> bool {
    match container {
        ElementContainer::View | ElementContainer::Mutable | ElementContainer::Array => true,
        ElementContainer::MapKey => false,
    }
}

/// A position plus the facts about the member producing the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputSite {
    pub(crate) position: OutputPosition,
    /// The member follows the `Try*` pattern, so a null result is part of
    /// its contract ("not found", "could not parse").
    pub(crate) try_method: bool,
    /// The Windows SDK documentation says the member's result can be null.
    pub(crate) documented_null: bool,
    /// The native member contract marks this exact logical output as an
    /// optional interface pointer (`IMapView.Split`'s two halves).
    pub(crate) contract_nullable: bool,
    /// The collection holding the value: set on collection elements, and on
    /// the results of members that read elements of the collection declaring
    /// them (`get_at`, `lookup`, `current`, ...).
    pub(crate) container: Option<ElementContainer>,
}

impl OutputSite {
    pub(crate) fn of(position: OutputPosition) -> Self {
        Self {
            position,
            try_method: false,
            documented_null: false,
            contract_nullable: false,
            container: None,
        }
    }

    pub(crate) fn for_method(method: &MethodMeta, position: OutputPosition) -> Self {
        Self {
            position,
            try_method: is_try_method(method),
            documented_null: method.documented_null_result,
            contract_nullable: false,
            container: method.element_access.map(ElementContainer::from),
        }
    }

    /// A specific logical method output, carrying per-output native contract
    /// facts rather than widening every result of the member.
    pub(crate) fn for_method_output(
        method: &MethodMeta,
        position: OutputPosition,
        index: usize,
    ) -> Self {
        Self {
            contract_nullable: method.contract_nullable_outputs.contains(&index),
            ..Self::for_method(method, position)
        }
    }

    /// An element read from a collection of the given kind.
    pub(crate) fn element_of(container: ElementContainer) -> Self {
        Self::of(OutputPosition::CollectionElement).element_in(container)
    }

    /// The site of a value nested in this one, such as an async result.
    /// Member facts carry over; the policy decides where they apply.
    pub(crate) fn nested(self, position: OutputPosition) -> Self {
        Self { position, ..self }
    }

    /// The site of an element held by `container`, nested in this value.
    pub(crate) fn element_in(self, container: ElementContainer) -> Self {
        Self {
            position: OutputPosition::CollectionElement,
            container: Some(container),
            ..self
        }
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
    use OutputPosition::{Activation, AsyncResult, CollectionElement, OutParam, Property, Return};

    if site.container == Some(ElementContainer::MapKey) {
        // output_admits_none only reaches this branch after
        // may_project_none(typ), which deliberately covers the Python
        // projection's supported COM-pointer shapes (not async wrappers).
        return true;
    }
    // `Object` positions are frequently null, e.g. the arguments of a
    // `TypedEventHandler<T, Object>`.
    if matches!(typ, TypeMeta::Object) {
        return true;
    }
    // Delegate-typed values are raw handles that are null while unset.
    if context.is_delegate_type(typ) {
        return true;
    }
    // `Try*` members report "not found" through a null result, and the
    // Windows SDK documentation names the other members that return null.
    let member_result = matches!(
        site.position,
        Return | OutParam | Property | AsyncResult | Activation
    );
    if member_result && (site.try_method || site.documented_null || site.contract_nullable) {
        return true;
    }
    // Collection elements, including the results of `get_at`, `lookup` and
    // `current`, follow the collection holding them.
    matches!(site.position, CollectionElement | Return | Property)
        && site.container.is_some_and(element_admits_none)
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
                documented_null: false,
                contract_nullable: false,
                container: None,
            }
        );
        assert_eq!(
            site.element_in(ElementContainer::Array),
            OutputSite {
                position: OutputPosition::CollectionElement,
                try_method: true,
                documented_null: false,
                contract_nullable: false,
                container: Some(ElementContainer::Array),
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
            assert!(
                admits(&handler(), site, AnnotationSurface::Stub),
                "{position:?}"
            );
        }
    }

    const MEMBER_RESULTS: [OutputPosition; 5] = [
        OutputPosition::Return,
        OutputPosition::OutParam,
        OutputPosition::Property,
        OutputPosition::AsyncResult,
        OutputPosition::Activation,
    ];

    #[test]
    fn try_members_keep_none_on_their_results_only() {
        let try_get = method("TryGetItemAsync");
        for position in POSITIONS {
            assert_eq!(
                admits(
                    &widget(),
                    OutputSite::for_method(&try_get, position),
                    AnnotationSurface::Stub
                ),
                MEMBER_RESULTS.contains(&position),
                "{position:?}"
            );
        }
        assert!(!admits(
            &TypeMeta::Bool,
            OutputSite::for_method(&try_get, OutputPosition::Return),
            AnnotationSurface::Stub
        ));
    }

    #[test]
    fn documented_null_members_keep_none_on_their_results_only() {
        let get_default = MethodMeta {
            documented_null_result: true,
            ..method("GetDefault")
        };
        for position in POSITIONS {
            assert_eq!(
                admits(
                    &widget(),
                    OutputSite::for_method(&get_default, position),
                    AnnotationSurface::Stub
                ),
                MEMBER_RESULTS.contains(&position),
                "{position:?}"
            );
        }
        let site = OutputSite::for_method(&get_default, OutputPosition::Return);
        assert!(admits(
            &widget(),
            site.element_in(ElementContainer::View),
            AnnotationSurface::Stub
        ));
    }

    #[test]
    fn reference_collection_elements_are_nullable_regardless_of_provenance() {
        let stub = AnnotationSurface::Stub;
        for container in [ElementContainer::View, ElementContainer::Mutable] {
            let site = OutputSite::element_of(container);
            assert!(admits(&widget(), site, stub), "{container:?}");
            assert!(admits(&TypeMeta::Object, site, stub));
            assert!(admits(&nullable_u32(), site, stub));
            assert!(!admits(&TypeMeta::String, site, stub));
        }
        let array = OutputSite::element_of(ElementContainer::Array);
        assert!(admits(&widget(), array, stub));
        assert!(admits(&TypeMeta::Object, array, stub));
        assert!(admits(&nullable_u32(), array, stub));
        assert!(!admits(&TypeMeta::String, array, stub));
        assert!(!admits(&TypeMeta::Guid, array, stub));
        let key = OutputSite::element_of(ElementContainer::MapKey);
        assert!(admits(&widget(), key, stub));
        assert!(admits(&TypeMeta::Object, key, stub));
        assert!(admits(&nullable_u32(), key, stub));
        assert!(!admits(&TypeMeta::String, key, stub));
        assert!(!admits(&TypeMeta::Guid, key, stub));
        // Async projection shapes are not ordinary collection values in the
        // current Python codegen, so this scope stays fail closed.
        assert!(!admits(&TypeMeta::AsyncAction, key, stub));

        for access in [ElementAccess::Mutable, ElementAccess::ReadOnly] {
            let get_at = MethodMeta {
                element_access: Some(access),
                ..method("GetAt")
            };
            for position in [OutputPosition::Return, OutputPosition::Property] {
                assert!(
                    admits(&widget(), OutputSite::for_method(&get_at, position), stub),
                    "{access:?} at {position:?}"
                );
            }
            assert!(!admits(
                &widget(),
                OutputSite::for_method(&get_at, OutputPosition::AsyncProgress),
                stub
            ));
        }
        assert_eq!(
            ElementContainer::of(CollectionKind::MutableSequence),
            ElementContainer::Mutable
        );
        assert_eq!(
            ElementContainer::of(CollectionKind::KeyValuePair),
            ElementContainer::View
        );
        let key = MethodMeta {
            element_access: Some(ElementAccess::MapKey),
            ..method("get_Key")
        };
        assert!(admits(
            &widget(),
            OutputSite::for_method(&key, OutputPosition::Property),
            stub
        ));
    }

    #[test]
    fn native_contract_nullability_is_per_logical_output() {
        let split = MethodMeta {
            contract_nullable_outputs: vec![0, 1],
            ..method("Split")
        };
        for (index, expected) in [(0, true), (1, true), (2, false)] {
            assert_eq!(
                admits(
                    &widget(),
                    OutputSite::for_method_output(&split, OutputPosition::OutParam, index),
                    AnnotationSurface::Stub
                ),
                expected,
                "logical output {index}"
            );
        }
        assert!(!admits(
            &widget(),
            OutputSite::for_method(&split, OutputPosition::Return),
            AnnotationSurface::Stub
        ));
    }
}
