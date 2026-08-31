// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/// The kind of a named WinRT type reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TypeKind {
    Class,
    Enum,
    Interface,
}

/// A reference to a named WinRT type (namespace + name + kind).
/// Replaces raw `(String, String, &str)` tuples for type-safe dependency tracking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeRef {
    pub namespace: String,
    pub name: String,
    pub kind: TypeKind,
}

/// Describes a WinRT type as extracted from WinMD metadata.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeMeta {
    // Primitives
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Char16,
    String, // HSTRING
    Guid,

    // Reference types
    Object, // IInspectable / unknown type
    Interface {
        namespace: String,
        name: String,
        iid: String,
    },
    RuntimeClass {
        namespace: String,
        name: String,
        default_interface: Option<Box<TypeMeta>>,
    },
    Delegate {
        namespace: String,
        name: String,
        iid: String,
    },

    // Async patterns
    AsyncAction,
    AsyncActionWithProgress(Box<TypeMeta>),
    AsyncOperation(Box<TypeMeta>),
    AsyncOperationWithProgress(Box<TypeMeta>, Box<TypeMeta>),

    // Parameterized interface instantiation: e.g. IVector<String>, IMap<String, Object>
    Parameterized {
        namespace: String,
        name: String,
        piid: String,
        args: Vec<TypeMeta>,
    },

    // Composite
    Array(Box<TypeMeta>),
    Struct {
        namespace: String,
        name: String,
        fields: Vec<FieldMeta>,
    },
    Enum {
        namespace: String,
        name: String,
        underlying: Box<TypeMeta>,
        members: Vec<EnumMember>,
        is_flags: bool,
        /// XML doc summary for the enum itself (populated from sibling .xml).
        doc: Option<String>,
        /// Deprecation text if marked `[Deprecated(...)]`.
        deprecated: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldMeta {
    pub name: String,
    pub typ: TypeMeta,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumMember {
    pub name: String,
    pub value: i32,
    /// XML doc summary (populated from sibling .xml).
    pub doc: Option<String>,
}

impl TypeMeta {
    /// Returns true if this type represents an async operation.
    pub fn is_async(&self) -> bool {
        matches!(
            self,
            TypeMeta::AsyncAction
                | TypeMeta::AsyncActionWithProgress(_)
                | TypeMeta::AsyncOperation(_)
                | TypeMeta::AsyncOperationWithProgress(_, _)
        )
    }

    /// For async types, return the result type (if any).
    pub fn async_result_type(&self) -> Option<&TypeMeta> {
        match self {
            TypeMeta::AsyncOperation(t) | TypeMeta::AsyncOperationWithProgress(t, _) => Some(t),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_async_returns_true_for_async_types() {
        assert!(TypeMeta::AsyncAction.is_async());
        assert!(TypeMeta::AsyncOperation(Box::new(TypeMeta::String)).is_async());
        assert!(TypeMeta::AsyncActionWithProgress(Box::new(TypeMeta::I32)).is_async());
        assert!(
            TypeMeta::AsyncOperationWithProgress(
                Box::new(TypeMeta::String),
                Box::new(TypeMeta::U32),
            )
            .is_async()
        );
    }

    #[test]
    fn is_async_returns_false_for_non_async_types() {
        assert!(!TypeMeta::Bool.is_async());
        assert!(!TypeMeta::String.is_async());
        assert!(!TypeMeta::I32.is_async());
        assert!(!TypeMeta::Object.is_async());
        assert!(
            !TypeMeta::Interface {
                namespace: "N".into(),
                name: "I".into(),
                iid: "".into(),
            }
            .is_async()
        );
    }

    #[test]
    fn async_result_type_extracts_inner() {
        let inner = TypeMeta::String;
        let op = TypeMeta::AsyncOperation(Box::new(inner.clone()));
        assert_eq!(op.async_result_type(), Some(&inner));

        let progress = TypeMeta::U32;
        let op_wp =
            TypeMeta::AsyncOperationWithProgress(Box::new(inner.clone()), Box::new(progress));
        assert_eq!(op_wp.async_result_type(), Some(&inner));
    }

    #[test]
    fn async_result_type_returns_none_for_non_operations() {
        assert_eq!(TypeMeta::AsyncAction.async_result_type(), None);
        assert_eq!(
            TypeMeta::AsyncActionWithProgress(Box::new(TypeMeta::I32)).async_result_type(),
            None
        );
        assert_eq!(TypeMeta::String.async_result_type(), None);
    }

    #[test]
    fn type_ref_equality_and_hash() {
        let r1 = TypeRef {
            namespace: "A".into(),
            name: "B".into(),
            kind: TypeKind::Class,
        };
        let r2 = TypeRef {
            namespace: "A".into(),
            name: "B".into(),
            kind: TypeKind::Class,
        };
        let r3 = TypeRef {
            namespace: "A".into(),
            name: "B".into(),
            kind: TypeKind::Interface,
        };
        assert_eq!(r1, r2);
        assert_ne!(r1, r3);

        let mut set = std::collections::HashSet::new();
        set.insert(r1.clone());
        assert!(set.contains(&r2));
        assert!(!set.contains(&r3));
    }
}
