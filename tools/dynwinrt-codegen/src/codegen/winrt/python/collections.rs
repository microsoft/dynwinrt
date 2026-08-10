// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::meta::{ClassMeta, InterfaceMeta, make_parameterized_name};
use crate::types::TypeMeta;

pub(crate) const IITERABLE_PIID: &str = "faa585ea-6214-4217-afda-7f46de5869b3";
pub(crate) const IITERATOR_PIID: &str = "6a79e863-4300-459a-9966-cbb660963ee1";
pub(crate) const IVECTOR_PIID: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";
pub(crate) const IVECTOR_VIEW_PIID: &str = "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56";
pub(crate) const IOBSERVABLE_VECTOR_PIID: &str = "5917eb53-50b4-4a0d-b309-65862b3f1dbc";
pub(crate) const IMAP_PIID: &str = "3c2925fe-8519-45c1-aa79-197b6718c1c1";
pub(crate) const IMAP_VIEW_PIID: &str = "e480ce40-a338-4ada-adcf-272272e48cb9";
pub(crate) const IKEY_VALUE_PAIR_PIID: &str = "02b51929-c1c4-4a7e-8940-0312b5c18500";

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

pub(crate) fn map_iterable_name(args: &[TypeMeta]) -> Option<String> {
    if args.len() != 2 {
        return None;
    }
    let pair = make_parameterized_name("IKeyValuePair", args);
    Some(format!("IIterable_{pair}"))
}

pub(crate) fn observable_vector_name(iface: &InterfaceMeta) -> Option<String> {
    (iface.generic_piid.as_deref() == Some(IOBSERVABLE_VECTOR_PIID)
        && iface.generic_args.len() == 1)
        .then(|| iface.name.replacen("IObservableVector", "IVector", 1))
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

#[cfg(test)]
mod tests {
    use super::*;

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
