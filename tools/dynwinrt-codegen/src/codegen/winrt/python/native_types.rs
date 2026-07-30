// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::types::TypeMeta;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FoundationType {
    DateTime,
    TimeSpan,
}

pub(crate) fn foundation_type(typ: &TypeMeta) -> Option<FoundationType> {
    match typ {
        TypeMeta::Struct {
            namespace, name, ..
        } if namespace == "Windows.Foundation" && name == "DateTime" => {
            Some(FoundationType::DateTime)
        }
        TypeMeta::Struct {
            namespace, name, ..
        } if namespace == "Windows.Foundation" && name == "TimeSpan" => {
            Some(FoundationType::TimeSpan)
        }
        _ => None,
    }
}
