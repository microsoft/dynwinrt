// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::types::TypeMeta;

/// Collection reference inputs accept native null, independently of their elements
/// and of whether a particular API permits an absent collection.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CollectionInput<'a> {
    Vector(&'a TypeMeta),
    Map(&'a TypeMeta, &'a TypeMeta),
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
            [key, value]
                if [
                    "3c2925fe-8519-45c1-aa79-197b6718c1c1",
                    "e480ce40-a338-4ada-adcf-272272e48cb9",
                ]
                .iter()
                .any(|expected| piid.eq_ignore_ascii_case(expected)) =>
            {
                Some(Self::Map(key, value))
            }
            _ => None,
        }
    }
}
