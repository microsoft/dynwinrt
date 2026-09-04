// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) enum UnsupportedReason {
    UnknownNativeType,
    UnknownPointerMeaning,
    UnknownLayout,
    UnknownOwnership,
    UnknownCleanup,
    MissingInterfaceIid,
    MissingCountRelationship,
    UnsupportedArray,
    UnsupportedFunctionPointer,
    UnsupportedVariantShape,
    UnsupportedSafeArrayElement,
    IncompleteInheritedVtable,
    Other(String),
}

impl fmt::Display for UnsupportedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNativeType => f.write_str("unknown native type"),
            Self::UnknownPointerMeaning => f.write_str("unknown pointer meaning"),
            Self::UnknownLayout => f.write_str("unknown native layout"),
            Self::UnknownOwnership => f.write_str("unknown ownership"),
            Self::UnknownCleanup => f.write_str("unknown cleanup"),
            Self::MissingInterfaceIid => f.write_str("missing interface IID"),
            Self::MissingCountRelationship => f.write_str("missing count relationship"),
            Self::UnsupportedArray => f.write_str("unsupported array"),
            Self::UnsupportedFunctionPointer => f.write_str("unsupported function pointer"),
            Self::UnsupportedVariantShape => f.write_str("unsupported VARIANT shape"),
            Self::UnsupportedSafeArrayElement => f.write_str("unsupported SAFEARRAY element"),
            Self::IncompleteInheritedVtable => f.write_str("incomplete inherited vtable"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) enum ModelError {
    CapacityExceeded(&'static str),
    InvalidName(String),
    UnknownId {
        kind: &'static str,
        index: usize,
    },
    DuplicateDefinition {
        kind: &'static str,
        index: usize,
    },
    IncompleteDefinition {
        kind: &'static str,
        index: usize,
    },
    Unsupported(UnsupportedReason),
    InvalidLayout(String),
    InvalidOwnership(String),
    InvalidContract(String),
    Context {
        context: String,
        source: Box<ModelError>,
    },
}

impl ModelError {
    pub(super) fn context(self, context: impl Into<String>) -> Self {
        Self::Context {
            context: context.into(),
            source: Box::new(self),
        }
    }

    pub(super) fn unsupported_reason(&self) -> Option<&UnsupportedReason> {
        match self {
            Self::Unsupported(reason) => Some(reason),
            Self::Context { source, .. } => source.unsupported_reason(),
            _ => None,
        }
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityExceeded(kind) => write!(f, "{kind} table exceeds u32 capacity"),
            Self::InvalidName(name) => write!(f, "invalid qualified name `{name}`"),
            Self::UnknownId { kind, index } => write!(f, "unknown {kind} id {index}"),
            Self::DuplicateDefinition { kind, index } => {
                write!(f, "{kind} id {index} is already defined")
            }
            Self::IncompleteDefinition { kind, index } => {
                write!(f, "{kind} id {index} is reserved but not defined")
            }
            Self::Unsupported(reason) => write!(f, "unsupported COM semantic: {reason}"),
            Self::InvalidLayout(message) => write!(f, "invalid native layout: {message}"),
            Self::InvalidOwnership(message) => write!(f, "invalid ownership: {message}"),
            Self::InvalidContract(message) => write!(f, "invalid COM contract: {message}"),
            Self::Context { context, source } => write!(f, "{context}: {source}"),
        }
    }
}

impl std::error::Error for ModelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Context { source, .. } => Some(source),
            _ => None,
        }
    }
}
