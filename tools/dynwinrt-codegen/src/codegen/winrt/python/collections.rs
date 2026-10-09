// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::meta::{
    ClassMeta, CollectionInputRole, InterfaceMeta, WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
};
use crate::types::{TypeIdentity, TypeIdentityKind, TypeMeta};

use super::naming::{PythonProjectionContext, PythonSupportSymbol};
use super::nullability::{AnnotationSurface, ElementContainer, may_project_none};
use super::type_helpers::{
    py_collection_input_type, py_collection_item_type, py_optional_type, py_param_type_safe,
};

pub(crate) const IITERABLE_PIID: &str = "faa585ea-6214-4217-afda-7f46de5869b3";
pub(crate) const IITERATOR_PIID: &str = "6a79e863-4300-459a-9966-cbb660963ee1";
pub(crate) const IVECTOR_PIID: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";
pub(crate) const IVECTOR_VIEW_PIID: &str = "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56";
pub(crate) const IOBSERVABLE_VECTOR_PIID: &str = "5917eb53-50b4-4a0d-b309-65862b3f1dbc";
pub(crate) const IMAP_PIID: &str = "3c2925fe-8519-45c1-aa79-197b6718c1c1";
pub(crate) const IOBSERVABLE_MAP_PIID: &str = "65df2bf5-bf39-41b5-aebc-5a9d865e472b";
pub(crate) const IMAP_VIEW_PIID: &str = "e480ce40-a338-4ada-adcf-272272e48cb9";
pub(crate) const IKEY_VALUE_PAIR_PIID: &str = "02b51929-c1c4-4a7e-8940-0312b5c18500";

#[derive(Clone, Copy)]
pub(crate) struct NonNullJsonCollection {
    pub(crate) class_name: &'static str,
    pub(crate) class_iid: &'static str,
}

const JSON_ARRAY: NonNullJsonCollection = NonNullJsonCollection {
    class_name: "Windows.Data.Json.JsonArray",
    class_iid: "08c1ddb6-0cbd-4a9a-b5d3-2f852dc37e81",
};
const JSON_OBJECT: NonNullJsonCollection = NonNullJsonCollection {
    class_name: "Windows.Data.Json.JsonObject",
    class_iid: "064e24dd-29c2-4f83-9ac1-9ee11578beb3",
};

fn is_json_value(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Interface {
            namespace,
            name,
            iid,
        } if namespace == "Windows.Data.Json"
            && name == "IJsonValue"
            && iid.eq_ignore_ascii_case("a3219ecb-f0b3-4dcd-beee-19d48cd3ed1e")
    )
}

pub(crate) fn non_null_json_input(
    role: CollectionInputRole,
    typ: &TypeMeta,
) -> Option<NonNullJsonCollection> {
    let element = match typ {
        TypeMeta::Array(element) => element.as_ref(),
        element => element,
    };
    if !is_json_value(element) {
        return None;
    }
    match role {
        CollectionInputRole::Element => Some(JSON_ARRAY),
        CollectionInputRole::Value => Some(JSON_OBJECT),
        CollectionInputRole::Key => None,
    }
}

pub(crate) fn non_null_json_collection(
    kind: CollectionKind,
    args: &[TypeMeta],
) -> Option<NonNullJsonCollection> {
    match (kind, args) {
        (CollectionKind::MutableSequence, [value]) if is_json_value(value) => Some(JSON_ARRAY),
        (CollectionKind::MutableMapping, [TypeMeta::String, value]) if is_json_value(value) => {
            Some(JSON_OBJECT)
        }
        _ => None,
    }
}

pub(crate) fn stock_json_class_contract(class: &ClassMeta) -> Option<NonNullJsonCollection> {
    let iface = class_interface(class)?;
    let contract = non_null_json_collection(interface_kind(iface)?, &iface.generic_args)?;
    (class.full_name == contract.class_name).then_some(contract)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CollectionKind {
    Iterable,
    Iterator,
    Sequence,
    MutableSequence,
    Mapping,
    MutableMapping,
    KeyValuePair,
}

pub(crate) fn kind_from_piid(piid: &str) -> Option<CollectionKind> {
    match piid {
        IITERABLE_PIID => Some(CollectionKind::Iterable),
        IITERATOR_PIID => Some(CollectionKind::Iterator),
        IVECTOR_PIID => Some(CollectionKind::MutableSequence),
        IOBSERVABLE_VECTOR_PIID => Some(CollectionKind::MutableSequence),
        IVECTOR_VIEW_PIID => Some(CollectionKind::Sequence),
        IMAP_PIID => Some(CollectionKind::MutableMapping),
        IMAP_VIEW_PIID => Some(CollectionKind::Mapping),
        IKEY_VALUE_PAIR_PIID => Some(CollectionKind::KeyValuePair),
        _ => None,
    }
}

pub(crate) fn interface_kind(iface: &InterfaceMeta) -> Option<CollectionKind> {
    iface.generic_piid.as_deref().and_then(kind_from_piid)
}

/// Collection protocol of an interface's own Python projection. An
/// `IObservableMap<K, V>` projection extends its `IMap<K, V>` companion.
pub(crate) fn projected_interface_kind(iface: &InterfaceMeta) -> Option<CollectionKind> {
    interface_kind(iface)
        .or_else(|| observable_map_identity(iface).map(|_| CollectionKind::MutableMapping))
}

pub(crate) fn class_interface(class: &ClassMeta) -> Option<&InterfaceMeta> {
    class
        .default_interface
        .iter()
        .chain(class.required_interfaces.iter())
        .filter_map(|iface| interface_kind(iface).map(|kind| (collection_rank(kind), iface)))
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, iface)| iface)
}

fn collection_rank(kind: CollectionKind) -> u8 {
    match kind {
        CollectionKind::MutableMapping => 0,
        CollectionKind::Mapping => 1,
        CollectionKind::MutableSequence => 2,
        CollectionKind::Sequence => 3,
        CollectionKind::Iterator => 4,
        CollectionKind::Iterable => 5,
        CollectionKind::KeyValuePair => 6,
    }
}

pub(crate) fn type_kind(typ: &TypeMeta) -> Option<CollectionKind> {
    match typ {
        TypeMeta::Parameterized { piid, .. } => kind_from_piid(piid),
        _ => None,
    }
}

pub(crate) fn runtime_mixin(kind: CollectionKind) -> Option<&'static str> {
    match kind {
        CollectionKind::Iterable => Some("_WinRTIterableMixin"),
        CollectionKind::Iterator => Some("_WinRTIteratorMixin"),
        CollectionKind::Sequence => Some("_WinRTSequenceMixin"),
        CollectionKind::MutableSequence => Some("_WinRTMutableSequenceMixin"),
        CollectionKind::Mapping => Some("_WinRTMappingMixin"),
        CollectionKind::MutableMapping => Some("_WinRTMutableMappingMixin"),
        CollectionKind::KeyValuePair => None,
    }
}

pub(crate) fn abc_name(kind: CollectionKind) -> Option<&'static str> {
    match kind {
        CollectionKind::Iterable => Some("Iterable"),
        CollectionKind::Iterator => Some("Iterator"),
        CollectionKind::Sequence => Some("Sequence"),
        CollectionKind::MutableSequence => Some("MutableSequence"),
        CollectionKind::Mapping => Some("Mapping"),
        CollectionKind::MutableMapping => Some("MutableMapping"),
        CollectionKind::KeyValuePair => None,
    }
}

pub(crate) fn map_iterable_identity(args: &[TypeMeta]) -> Option<TypeIdentity> {
    if args.len() != 2 {
        return None;
    }
    let pair = TypeIdentity::closed_generic(
        TypeIdentityKind::Interface,
        WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
        "IKeyValuePair",
        args.iter().map(TypeMeta::type_identity),
    );
    Some(TypeIdentity::closed_generic(
        TypeIdentityKind::Interface,
        WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
        "IIterable",
        [pair],
    ))
}

pub(crate) fn observable_vector_identity(iface: &InterfaceMeta) -> Option<TypeIdentity> {
    (iface.generic_piid.as_deref() == Some(IOBSERVABLE_VECTOR_PIID)
        && iface.generic_args.len() == 1)
        .then(|| {
            TypeIdentity::closed_generic(
                TypeIdentityKind::Interface,
                WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
                "IVector",
                iface.generic_args.iter().map(TypeMeta::type_identity),
            )
        })
}

/// The mutable `IMap<K, V>` an `IObservableMap<K, V>` projection extends.
pub(crate) fn observable_map_identity(iface: &InterfaceMeta) -> Option<TypeIdentity> {
    (iface.generic_piid.as_deref() == Some(IOBSERVABLE_MAP_PIID) && iface.generic_args.len() == 2)
        .then(|| {
            TypeIdentity::closed_generic(
                TypeIdentityKind::Interface,
                WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
                "IMap",
                iface.generic_args.iter().map(TypeMeta::type_identity),
            )
        })
}

/// Observable collections extend their mutable companion in Python, so an
/// event sender still behaves as a sequence or mapping.
pub(crate) fn observable_collection_identity(iface: &InterfaceMeta) -> Option<TypeIdentity> {
    observable_vector_identity(iface).or_else(|| observable_map_identity(iface))
}

pub(crate) fn is_mapping_input(kind: CollectionKind, args: &[TypeMeta]) -> bool {
    matches!(
        kind,
        CollectionKind::Mapping | CollectionKind::MutableMapping
    ) || (kind == CollectionKind::Iterable
        && args.len() == 1
        && matches!(
            &args[0],
            TypeMeta::Parameterized { piid, .. } if piid == IKEY_VALUE_PAIR_PIID
        ))
}

/// Bulk helpers write the same inputs as item assignment, but return only the
/// read projection (or Self), never the caller's unconverted input.
pub(super) fn protocol_helper_methods(
    iface: &InterfaceMeta,
    owner_name: &str,
    context: &PythonProjectionContext,
    surface: AnnotationSurface,
    stock_json_receiver: bool,
    indent_spaces: usize,
) -> String {
    let indent = " ".repeat(indent_spaces);
    let input = |typ| {
        if stock_json_receiver {
            py_param_type_safe(typ, context)
        } else {
            py_collection_input_type(typ, context)
        }
    };
    let is_stub = surface == AnnotationSurface::Stub;
    match (
        projected_interface_kind(iface),
        iface.generic_args.as_slice(),
    ) {
        (Some(CollectionKind::MutableSequence), [element]) => {
            let element = input(element);
            if is_stub {
                format!(
                    "{indent}def extend(self, values: Iterable[{element}]) -> None: ...\n\
                         {indent}def __iadd__(self, values: Iterable[{element}]) -> Self: ...\n"
                )
            } else {
                format!(
                    "\n{indent}def extend(self, values: Iterable[{element}]) -> None:\n\
                         {indent}    super().extend(values)\n\
                         \n{indent}def __iadd__(self, values: Iterable[{element}]) -> Self:\n\
                         {indent}    self.extend(values)\n\
                         {indent}    return self\n"
                )
            }
        }
        (Some(CollectionKind::MutableMapping), [key, value]) => {
            let key_input = py_collection_input_type(key, context);
            let value_input = input(value);
            let value_read = if stock_json_receiver {
                py_param_type_safe(value, context)
            } else {
                py_collection_item_type(value, ElementContainer::Mutable, surface, context)
            };
            let supports = context.support_symbol_reference(PythonSupportSymbol::MappingInput);
            let key_variable = context.support_symbol_reference(PythonSupportSymbol::MappingKey);
            let key_reference = if is_stub {
                key_variable.to_string()
            } else {
                format!("{owner_name}.{key_variable}")
            };
            let key_bound = serde_json::to_string(super::type_helpers::unquoted(&key_input))
                .expect("serialize a Python type annotation");
            let mapping = format!("{supports}[{key_reference}, {value_input}]");
            let pairs = format!("Iterable[tuple[{key_input}, {value_input}]]");
            let keywords = if matches!(key, TypeMeta::String | TypeMeta::Char16) {
                format!(", **kwargs: {value_input}")
            } else {
                String::new()
            };
            let mut result = format!(
                "\n{indent}{key_variable} = TypeVar('{key_variable}', bound={key_bound})\n\
                     \n{indent}@overload\n\
                     {indent}def update(self, other: {mapping}, /{keywords}) -> None: ...\n\
                     {indent}@overload\n\
                     {indent}def update(self, other: {pairs} = (), /{keywords}) -> None: ...\n"
            );
            if !is_stub {
                // Keep runtime kwargs handling, including partial updates, in
                // the existing checked mixin even for non-string key types.
                result.push_str(&format!(
                        "{indent}def update(self, other: {mapping} | {pairs} = (), /, **kwargs: {value_input}) -> None:\n\
                         {indent}    super().update(other, **kwargs)\n"
                    ));
            }
            let nullable = may_project_none(value) && !stock_json_receiver;
            if is_stub {
                let default = if nullable { " = None" } else { "" };
                result.push_str(&format!(
                        "\n{indent}def setdefault(self, key: {key_input}, default: {value_input}{default}) -> {value_read}: ...\n"
                    ));
            } else {
                if !nullable {
                    result.push_str(&format!(
                            "\n{indent}@overload\n\
                             {indent}def setdefault(self, key: {key_input}, default: {value_input}, /) -> {value_read}: ...\n\
                             {indent}@overload\n\
                             {indent}def setdefault(self, key: {key_input}, *, default: {value_input}) -> {value_read}: ...\n"
                        ));
                }
                let default_input = py_optional_type(value_input);
                // An omitted/invalid default is still ignored for an existing
                // key. The overloads constrain values that may be inserted.
                result.push_str(&format!(
                        "\n{indent}def setdefault(self, key: {key_input}, default: {default_input} = None) -> {value_read}:\n\
                         {indent}    return super().setdefault(key, default)\n"
                    ));
            }
            result
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_null_contract_needs_the_exact_element_iid_and_native_class() {
        let json_value = TypeMeta::Interface {
            namespace: "Windows.Data.Json".into(),
            name: "IJsonValue".into(),
            iid: "a3219ecb-f0b3-4dcd-beee-19d48cd3ed1e".into(),
        };
        assert_eq!(
            non_null_json_input(CollectionInputRole::Element, &json_value)
                .unwrap()
                .class_name,
            "Windows.Data.Json.JsonArray"
        );
        assert_eq!(
            non_null_json_input(
                CollectionInputRole::Value,
                &TypeMeta::Array(Box::new(json_value.clone()))
            )
            .unwrap()
            .class_name,
            "Windows.Data.Json.JsonObject"
        );
        assert!(non_null_json_input(CollectionInputRole::Key, &json_value).is_none());
        assert!(
            non_null_json_collection(
                CollectionKind::MutableMapping,
                &[TypeMeta::String, json_value.clone()]
            )
            .is_some()
        );

        let iface = InterfaceMeta {
            generic_piid: Some(IVECTOR_PIID.into()),
            generic_args: vec![json_value.clone()],
            ..Default::default()
        };
        let custom = ClassMeta {
            full_name: "Contoso.CustomJsonVector".into(),
            default_interface: Some(iface.clone()),
            ..Default::default()
        };
        assert!(stock_json_class_contract(&custom).is_none());
        assert!(
            stock_json_class_contract(&ClassMeta {
                full_name: JSON_ARRAY.class_name.into(),
                default_interface: Some(iface),
                ..Default::default()
            })
            .is_some()
        );

        let TypeMeta::Interface {
            namespace,
            name,
            iid: _,
        } = json_value
        else {
            unreachable!()
        };
        let wrong_iid = TypeMeta::Interface {
            namespace,
            name,
            iid: "00000000-0000-0000-0000-000000000000".into(),
        };
        assert!(non_null_json_input(CollectionInputRole::Element, &wrong_iid).is_none());
        assert!(non_null_json_collection(CollectionKind::MutableSequence, &[wrong_iid]).is_none());
        assert!(non_null_json_input(CollectionInputRole::Value, &TypeMeta::Object).is_none());
    }

    #[test]
    fn map_piids_project_to_python_mapping_protocols() {
        assert_eq!(
            kind_from_piid(IMAP_VIEW_PIID),
            Some(CollectionKind::Mapping)
        );
        assert_eq!(
            runtime_mixin(CollectionKind::Mapping),
            Some("_WinRTMappingMixin")
        );
        assert_eq!(abc_name(CollectionKind::Mapping), Some("Mapping"));

        assert_eq!(
            kind_from_piid(IMAP_PIID),
            Some(CollectionKind::MutableMapping)
        );
        assert_eq!(
            runtime_mixin(CollectionKind::MutableMapping),
            Some("_WinRTMutableMappingMixin")
        );
        assert_eq!(
            abc_name(CollectionKind::MutableMapping),
            Some("MutableMapping")
        );
    }
}
