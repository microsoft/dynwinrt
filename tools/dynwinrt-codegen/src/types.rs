// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/// The kind of a named WinRT type reference.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TypeKind {
    Class,
    Enum,
    Interface,
}

/// The semantic kind of a projected WinRT type.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TypeIdentityKind {
    Class,
    Delegate,
    Enum,
    Interface,
    Struct,
}

/// Canonical identity for a WinRT type, including recursively closed generics.
///
/// Projection names are deliberately not part of this model. Language
/// projections may derive readable symbols and paths from this identity without
/// weakening the namespace, kind, or nested generic argument identity used for
/// lookup and collision handling.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum TypeIdentity {
    Primitive {
        name: String,
    },
    Named {
        kind: TypeIdentityKind,
        namespace: String,
        name: String,
    },
    ClosedGeneric {
        kind: TypeIdentityKind,
        namespace: String,
        name: String,
        arguments: Vec<TypeIdentity>,
    },
    Array {
        element: Box<TypeIdentity>,
    },
    AsyncAction,
    AsyncActionWithProgress {
        progress: Box<TypeIdentity>,
    },
    AsyncOperation {
        result: Box<TypeIdentity>,
    },
    AsyncOperationWithProgress {
        result: Box<TypeIdentity>,
        progress: Box<TypeIdentity>,
    },
}

impl TypeIdentity {
    pub fn named(
        kind: TypeIdentityKind,
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self::Named {
            kind,
            namespace: namespace.into(),
            name: name.into(),
        }
    }

    pub fn closed_generic(
        kind: TypeIdentityKind,
        namespace: impl Into<String>,
        name: impl Into<String>,
        arguments: impl IntoIterator<Item = TypeIdentity>,
    ) -> Self {
        Self::ClosedGeneric {
            kind,
            namespace: namespace.into(),
            name: name
                .into()
                .split('`')
                .next()
                .unwrap_or_default()
                .to_string(),
            arguments: arguments.into_iter().collect(),
        }
    }

    pub fn kind(&self) -> Option<TypeIdentityKind> {
        match self {
            Self::Named { kind, .. } | Self::ClosedGeneric { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    pub fn namespace(&self) -> Option<&str> {
        match self {
            Self::Named { namespace, .. } | Self::ClosedGeneric { namespace, .. } => {
                Some(namespace)
            }
            _ => None,
        }
    }

    pub fn definition_name(&self) -> Option<&str> {
        match self {
            Self::Named { name, .. } | Self::ClosedGeneric { name, .. } => Some(name),
            _ => None,
        }
    }

    pub fn with_kind(&self, kind: TypeIdentityKind) -> Self {
        match self {
            Self::Named {
                namespace, name, ..
            } => Self::named(kind, namespace.clone(), name.clone()),
            Self::ClosedGeneric {
                namespace,
                name,
                arguments,
                ..
            } => Self::closed_generic(
                kind,
                namespace.clone(),
                name.clone(),
                arguments.iter().cloned(),
            ),
            _ => self.clone(),
        }
    }

    /// Stable, unambiguous text used only as hash input and diagnostics.
    pub fn canonical_key(&self) -> String {
        fn atom(value: &str) -> String {
            format!("{}:{value}", value.len())
        }

        fn append(identity: &TypeIdentity, out: &mut String) {
            match identity {
                TypeIdentity::Primitive { name } => {
                    out.push_str("p");
                    out.push_str(&atom(name));
                }
                TypeIdentity::Named {
                    kind,
                    namespace,
                    name,
                } => {
                    out.push_str("n");
                    out.push_str(&format!("{kind:?}:"));
                    out.push_str(&atom(namespace));
                    out.push_str(&atom(name));
                }
                TypeIdentity::ClosedGeneric {
                    kind,
                    namespace,
                    name,
                    arguments,
                } => {
                    out.push_str("g");
                    out.push_str(&format!("{kind:?}:"));
                    out.push_str(&atom(namespace));
                    out.push_str(&atom(name));
                    out.push_str(&format!("{}:", arguments.len()));
                    for argument in arguments {
                        append(argument, out);
                    }
                }
                TypeIdentity::Array { element } => {
                    out.push('a');
                    append(element, out);
                }
                TypeIdentity::AsyncAction => out.push_str("async-action"),
                TypeIdentity::AsyncActionWithProgress { progress } => {
                    out.push_str("async-action-progress");
                    append(progress, out);
                }
                TypeIdentity::AsyncOperation { result } => {
                    out.push_str("async-operation");
                    append(result, out);
                }
                TypeIdentity::AsyncOperationWithProgress { result, progress } => {
                    out.push_str("async-operation-progress");
                    append(result, out);
                    append(progress, out);
                }
            }
        }

        let mut result = String::new();
        append(self, &mut result);
        result
    }
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
    /// Return the canonical semantic identity represented by this metadata type.
    pub fn type_identity(&self) -> TypeIdentity {
        let primitive = |name: &str| TypeIdentity::Primitive {
            name: name.to_string(),
        };
        match self {
            Self::Bool => primitive("Boolean"),
            Self::I8 => primitive("Int8"),
            Self::U8 => primitive("UInt8"),
            Self::I16 => primitive("Int16"),
            Self::U16 => primitive("UInt16"),
            Self::I32 => primitive("Int32"),
            Self::U32 => primitive("UInt32"),
            Self::I64 => primitive("Int64"),
            Self::U64 => primitive("UInt64"),
            Self::F32 => primitive("Single"),
            Self::F64 => primitive("Double"),
            Self::Char16 => primitive("Char16"),
            Self::String => primitive("String"),
            Self::Guid => primitive("Guid"),
            Self::Object => primitive("Object"),
            Self::Interface {
                namespace, name, ..
            } => TypeIdentity::named(TypeIdentityKind::Interface, namespace, name),
            Self::RuntimeClass {
                namespace, name, ..
            } => TypeIdentity::named(TypeIdentityKind::Class, namespace, name),
            Self::Delegate {
                namespace, name, ..
            } => TypeIdentity::named(TypeIdentityKind::Delegate, namespace, name),
            Self::Parameterized {
                namespace,
                name,
                args,
                ..
            } => TypeIdentity::closed_generic(
                TypeIdentityKind::Interface,
                namespace,
                name,
                args.iter().map(TypeMeta::type_identity),
            ),
            Self::Array(element) => TypeIdentity::Array {
                element: Box::new(element.type_identity()),
            },
            Self::Struct {
                namespace, name, ..
            } => TypeIdentity::named(TypeIdentityKind::Struct, namespace, name),
            Self::Enum {
                namespace, name, ..
            } => TypeIdentity::named(TypeIdentityKind::Enum, namespace, name),
            Self::AsyncAction => TypeIdentity::AsyncAction,
            Self::AsyncActionWithProgress(progress) => TypeIdentity::AsyncActionWithProgress {
                progress: Box::new(progress.type_identity()),
            },
            Self::AsyncOperation(result) => TypeIdentity::AsyncOperation {
                result: Box::new(result.type_identity()),
            },
            Self::AsyncOperationWithProgress(result, progress) => {
                TypeIdentity::AsyncOperationWithProgress {
                    result: Box::new(result.type_identity()),
                    progress: Box::new(progress.type_identity()),
                }
            }
        }
    }

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
