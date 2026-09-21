// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::types::TypeMeta;

/// Collection reference inputs accept native null, independently of their elements
/// and of whether a particular API permits an absent collection.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CollectionInput<'a> {
    Vector(&'a TypeMeta),
    Map(&'a TypeMeta, &'a TypeMeta),
    MapView(&'a TypeMeta, &'a TypeMeta),
}

impl<'a> CollectionInput<'a> {
    pub(crate) fn from_type(typ: &'a TypeMeta) -> Option<Self> {
        let TypeMeta::Parameterized { piid, args, .. } = typ else {
            return None;
        };
        match args.as_slice() {
            [element]
                if [
                    "913337e9-11a1-4345-a3a2-4e7f956e222d",
                    "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56",
                    "faa585ea-6214-4217-afda-7f46de5869b3",
                ]
                .iter()
                .any(|expected| piid.eq_ignore_ascii_case(expected)) =>
            {
                Some(Self::Vector(element))
            }
            [key, value] if piid.eq_ignore_ascii_case("3c2925fe-8519-45c1-aa79-197b6718c1c1") => {
                Some(Self::Map(key, value))
            }
            [key, value] if piid.eq_ignore_ascii_case("e480ce40-a338-4ada-adcf-272272e48cb9") => {
                Some(Self::MapView(key, value))
            }
            _ => None,
        }
    }

    pub(crate) fn map_view_source(typ: &'a TypeMeta) -> Option<TypeMeta> {
        let Self::MapView(key, value) = Self::from_type(typ)? else {
            return None;
        };
        Some(TypeMeta::Parameterized {
            namespace: crate::meta::WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE.into(),
            name: "IMap".into(),
            piid: "3c2925fe-8519-45c1-aa79-197b6718c1c1".into(),
            args: vec![key.clone(), value.clone()],
        })
    }
}
