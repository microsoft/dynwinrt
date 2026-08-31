// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

pub(crate) mod docs;
pub mod generator;
pub mod ir;
pub(crate) mod method;
pub(crate) mod naming;
pub mod project;
pub mod render;
pub(crate) mod signature;
pub(crate) mod structs;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

use crate::meta::{ClassMeta, InterfaceMeta, MethodMeta};
use crate::types::{TypeKind, TypeMeta, TypeRef};
use serde::{Deserialize, Serialize};

use self::ir::ProjectedFile;
use super::shared::structs::{collect_used_structs_from_class, collect_used_structs_from_iface};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JavaScriptTypeKind {
    Class,
    Interface,
    Delegate,
    Enum,
}

impl JavaScriptTypeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Delegate => "delegate",
            Self::Enum => "enum",
        }
    }
}

impl From<TypeKind> for JavaScriptTypeKind {
    fn from(value: TypeKind) -> Self {
        match value {
            TypeKind::Class => Self::Class,
            TypeKind::Interface => Self::Interface,
            TypeKind::Enum => Self::Enum,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct JavaScriptTypeIdentity {
    pub namespace: String,
    pub name: String,
    pub kind: JavaScriptTypeKind,
    pub variant: String,
}

impl JavaScriptTypeIdentity {
    pub fn new(namespace: &str, name: &str, kind: JavaScriptTypeKind) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            kind,
            variant: String::new(),
        }
    }

    pub fn with_variant(
        namespace: &str,
        name: &str,
        kind: JavaScriptTypeKind,
        variant: &str,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            kind,
            variant: variant.into(),
        }
    }

    pub fn to_inventory_line(&self) -> String {
        format!("{}|{}|{}", self.kind.as_str(), self.namespace, self.name)
    }

    fn is_safe(&self) -> bool {
        !self.namespace.is_empty()
            && self
                .namespace
                .split('.')
                .all(|segment| !segment.is_empty() && is_metadata_identifier(segment))
            && is_metadata_identifier(&self.name)
            && self
                .variant
                .chars()
                .all(|character| character.is_ascii_graphic() && character != '|')
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JavaScriptTypeLayoutRecord {
    pub identity: JavaScriptTypeIdentity,
    pub projected_name: String,
    pub implementation_name: String,
    pub abi_identity: String,
    #[serde(default)]
    pub compatibility_aliases: BTreeSet<String>,
}

impl JavaScriptTypeLayoutRecord {
    pub fn new(
        identity: JavaScriptTypeIdentity,
        projected_name: impl Into<String>,
        abi_identity: impl Into<String>,
    ) -> Self {
        let projected_name = projected_name.into();
        Self {
            identity,
            implementation_name: projected_name.clone(),
            projected_name,
            abi_identity: abi_identity.into(),
            compatibility_aliases: BTreeSet::new(),
        }
    }

    pub fn with_compatibility_aliases(mut self, aliases: impl IntoIterator<Item = String>) -> Self {
        self.compatibility_aliases.extend(aliases);
        self
    }

    pub fn with_implementation_name(mut self, implementation_name: impl Into<String>) -> Self {
        self.implementation_name = implementation_name.into();
        self
    }

    fn is_safe(&self) -> bool {
        self.identity.is_safe()
            && is_metadata_identifier(&self.projected_name)
            && is_metadata_identifier(&self.implementation_name)
            && !self.abi_identity.is_empty()
            && self
                .abi_identity
                .chars()
                .all(|character| character.is_ascii_graphic() && character != '|')
            && self
                .compatibility_aliases
                .iter()
                .all(|alias| is_metadata_identifier(alias))
    }
}

fn is_metadata_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[derive(Clone, Debug)]
pub struct JavaScriptOutputTarget {
    pub identity: JavaScriptTypeIdentity,
    pub projected_name: String,
    pub canonical_module: String,
    pub collides: bool,
    pub compatibility_aliases: BTreeSet<String>,
}

#[derive(Default)]
struct JavaScriptModuleLayout {
    targets: BTreeMap<JavaScriptTypeIdentity, JavaScriptOutputTarget>,
    by_projected_name: HashMap<String, JavaScriptTypeIdentity>,
}

/// Immutable naming and import configuration for one JavaScript WinRT projection.
///
/// A context owns the complete native-identity-to-module mapping. Keeping it
/// explicit makes independent projections deterministic and prevents nested or
/// concurrent generation from observing process-global layout state.
pub struct JavaScriptProjectionContext {
    layout: JavaScriptModuleLayout,
    runtime_import_name: String,
}

impl Default for JavaScriptProjectionContext {
    fn default() -> Self {
        create_javascript_projection_context([])
            .expect("an empty JavaScript projection context is valid")
    }
}

impl JavaScriptProjectionContext {
    pub fn runtime_import_name(&self) -> &str {
        &self.runtime_import_name
    }

    pub fn identities(&self) -> impl Iterator<Item = &JavaScriptTypeIdentity> {
        self.layout.targets.keys()
    }

    pub fn output_targets(&self) -> impl Iterator<Item = &JavaScriptOutputTarget> {
        self.layout.targets.values()
    }

    pub fn target_for_identity(
        &self,
        identity: &JavaScriptTypeIdentity,
    ) -> Option<&JavaScriptOutputTarget> {
        self.layout.targets.get(identity)
    }

    pub fn output_target(&self, projected_name: &str) -> Option<&JavaScriptOutputTarget> {
        let identity = self.layout.by_projected_name.get(projected_name)?;
        self.layout.targets.get(identity)
    }

    pub fn metadata_type_name<'a>(&'a self, namespace: &str, output_name: &'a str) -> &'a str {
        self.layout
            .by_projected_name
            .get(output_name)
            .filter(|identity| identity.namespace == namespace)
            .map_or(output_name, |identity| identity.name.as_str())
    }

    pub fn projected_name(&self, namespace: &str, name: &str, kind: JavaScriptTypeKind) -> String {
        self.target_for_identity(&JavaScriptTypeIdentity::new(namespace, name, kind))
            .or_else(|| {
                (kind == JavaScriptTypeKind::Interface)
                    .then(|| {
                        self.target_for_identity(&JavaScriptTypeIdentity::new(
                            namespace,
                            name,
                            JavaScriptTypeKind::Delegate,
                        ))
                    })
                    .flatten()
            })
            .map_or_else(|| name.into(), |target| target.projected_name.clone())
    }

    pub fn projected_parameterized_name(
        &self,
        namespace: &str,
        generic_name: &str,
        piid: &str,
        args: &[TypeMeta],
    ) -> String {
        let output_name =
            parameterized_name_with_context(self, namespace, generic_name, piid, args);
        let variant = parameterized_reference_identity_with_context(self, piid, args);
        self.target_for_identity(&JavaScriptTypeIdentity::with_variant(
            namespace,
            &output_name,
            JavaScriptTypeKind::Interface,
            &variant,
        ))
        .or_else(|| {
            self.target_for_identity(&JavaScriptTypeIdentity::with_variant(
                namespace,
                &output_name,
                JavaScriptTypeKind::Delegate,
                &variant,
            ))
        })
        .map_or(output_name, |target| target.projected_name.clone())
    }

    pub fn configure_projected_file(
        &self,
        file: &mut ProjectedFile,
    ) -> Option<JavaScriptOutputTarget> {
        let target = self.output_target(&file.name)?.clone();
        for import in &mut file.imports {
            if import.is_runtime_package {
                if (import.from.starts_with("./") || import.from.starts_with("../"))
                    && let Some(module) = import
                        .from
                        .strip_suffix(".js")
                        .or(Some(import.from.as_str()))
                {
                    import.from =
                        format!("{}.js", relative_module(&target.canonical_module, module));
                }
                continue;
            }
            let Some(module) = import
                .from
                .strip_prefix("./")
                .and_then(|path| path.strip_suffix(".js"))
            else {
                continue;
            };
            let target_module = self.output_target(module).map_or_else(
                || module.to_string(),
                |target| target.canonical_module.clone(),
            );
            import.from = format!(
                "{}.js",
                relative_module(&target.canonical_module, &target_module)
            );
        }
        Some(target)
    }
}

fn pascal_segment(segment: &str) -> String {
    let mut result = String::new();
    let mut uppercase_next = true;
    for character in segment.chars() {
        if !character.is_ascii_alphanumeric() {
            uppercase_next = true;
            continue;
        }
        if uppercase_next {
            result.push(character.to_ascii_uppercase());
            uppercase_next = false;
        } else {
            result.push(character);
        }
    }
    result
}

fn namespace_qualifier(namespace: &str) -> String {
    let segments = namespace
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let skip = if segments.starts_with(&["Microsoft", "UI"])
        || segments.starts_with(&["Microsoft", "Windows"])
    {
        2
    } else if segments.first() == Some(&"Windows") {
        1
    } else {
        0
    };
    let selected = if skip < segments.len() {
        &segments[skip..]
    } else {
        &segments[..]
    };
    selected
        .iter()
        .map(|segment| pascal_segment(segment))
        .collect()
}

fn fnv1a64(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn bounded_identifier(candidate: String, identity: &JavaScriptTypeIdentity) -> String {
    const MAX_IDENTIFIER_LENGTH: usize = 120;
    if candidate.chars().count() <= MAX_IDENTIFIER_LENGTH {
        return candidate;
    }
    let key = format!(
        "{}|{}|{}|{}",
        identity.kind.as_str(),
        identity.namespace,
        identity.name,
        identity.variant,
    );
    let suffix = format!("_{:08x}", fnv1a64(&key) as u32);
    let keep = MAX_IDENTIFIER_LENGTH - suffix.len();
    format!(
        "{}{}",
        candidate.chars().take(keep).collect::<String>(),
        suffix
    )
}

fn normalized_guid(value: &str) -> String {
    value
        .trim_matches(|character| character == '{' || character == '}')
        .to_ascii_lowercase()
}

#[derive(Clone, Copy)]
enum SemanticPrimitive {
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
    String,
    Char16,
    Guid,
    Object,
}

#[derive(Clone, Copy)]
enum SemanticNamedKind {
    Interface,
    RuntimeClass,
    Delegate,
    Struct,
    Enum,
}

#[derive(Clone)]
enum JavaScriptSemanticTypeIdentity {
    Primitive(SemanticPrimitive),
    Named {
        kind: SemanticNamedKind,
        namespace: String,
        native_name: String,
        iid: Option<String>,
        default_interface: Option<Box<Self>>,
        fields: Vec<(String, Self)>,
        enum_underlying: Option<Box<Self>>,
    },
    Parameterized {
        namespace: String,
        native_name: String,
        piid: String,
        args: Vec<Self>,
    },
    Array(Box<Self>),
    AsyncAction,
    AsyncActionWithProgress(Box<Self>),
    AsyncOperation(Box<Self>),
    AsyncOperationWithProgress(Box<Self>, Box<Self>),
}

#[derive(Clone, Copy)]
enum SemanticFormat {
    ReadableToken,
    StableIdentity,
    AbiIdentity,
}

impl JavaScriptSemanticTypeIdentity {
    fn from_type(typ: &TypeMeta, context: Option<&JavaScriptProjectionContext>) -> Self {
        let native = |namespace: &str, name: &str| {
            context
                .map(|context| context.metadata_type_name(namespace, name))
                .unwrap_or(name)
                .to_string()
        };
        match typ {
            TypeMeta::Bool => Self::Primitive(SemanticPrimitive::Bool),
            TypeMeta::I8 => Self::Primitive(SemanticPrimitive::I8),
            TypeMeta::U8 => Self::Primitive(SemanticPrimitive::U8),
            TypeMeta::I16 => Self::Primitive(SemanticPrimitive::I16),
            TypeMeta::U16 => Self::Primitive(SemanticPrimitive::U16),
            TypeMeta::I32 => Self::Primitive(SemanticPrimitive::I32),
            TypeMeta::U32 => Self::Primitive(SemanticPrimitive::U32),
            TypeMeta::I64 => Self::Primitive(SemanticPrimitive::I64),
            TypeMeta::U64 => Self::Primitive(SemanticPrimitive::U64),
            TypeMeta::F32 => Self::Primitive(SemanticPrimitive::F32),
            TypeMeta::F64 => Self::Primitive(SemanticPrimitive::F64),
            TypeMeta::String => Self::Primitive(SemanticPrimitive::String),
            TypeMeta::Char16 => Self::Primitive(SemanticPrimitive::Char16),
            TypeMeta::Guid => Self::Primitive(SemanticPrimitive::Guid),
            TypeMeta::Object => Self::Primitive(SemanticPrimitive::Object),
            TypeMeta::Interface {
                namespace,
                name,
                iid,
            } => Self::named(
                SemanticNamedKind::Interface,
                namespace,
                native(namespace, name),
                Some(iid.clone()),
                None,
                Vec::new(),
                None,
            ),
            TypeMeta::RuntimeClass {
                namespace,
                name,
                default_interface,
            } => Self::named(
                SemanticNamedKind::RuntimeClass,
                namespace,
                native(namespace, name),
                None,
                default_interface
                    .as_deref()
                    .map(|typ| Box::new(Self::from_type(typ, context))),
                Vec::new(),
                None,
            ),
            TypeMeta::Delegate {
                namespace,
                name,
                iid,
            } => Self::named(
                SemanticNamedKind::Delegate,
                namespace,
                native(namespace, name),
                Some(iid.clone()),
                None,
                Vec::new(),
                None,
            ),
            TypeMeta::Struct {
                namespace,
                name,
                fields,
            } => Self::named(
                SemanticNamedKind::Struct,
                namespace,
                native(namespace, name),
                None,
                None,
                fields
                    .iter()
                    .map(|field| (field.name.clone(), Self::from_type(&field.typ, context)))
                    .collect(),
                None,
            ),
            TypeMeta::Enum {
                namespace,
                name,
                underlying,
                ..
            } => Self::named(
                SemanticNamedKind::Enum,
                namespace,
                native(namespace, name),
                None,
                None,
                Vec::new(),
                Some(Box::new(Self::from_type(underlying, context))),
            ),
            TypeMeta::Parameterized {
                namespace,
                name,
                piid,
                args,
            } => Self::Parameterized {
                namespace: namespace.clone(),
                native_name: name.clone(),
                piid: piid.clone(),
                args: args
                    .iter()
                    .map(|typ| Self::from_type(typ, context))
                    .collect(),
            },
            TypeMeta::Array(inner) => Self::Array(Box::new(Self::from_type(inner, context))),
            TypeMeta::AsyncAction => Self::AsyncAction,
            TypeMeta::AsyncActionWithProgress(inner) => {
                Self::AsyncActionWithProgress(Box::new(Self::from_type(inner, context)))
            }
            TypeMeta::AsyncOperation(inner) => {
                Self::AsyncOperation(Box::new(Self::from_type(inner, context)))
            }
            TypeMeta::AsyncOperationWithProgress(result, progress) => {
                Self::AsyncOperationWithProgress(
                    Box::new(Self::from_type(result, context)),
                    Box::new(Self::from_type(progress, context)),
                )
            }
        }
    }

    fn named(
        kind: SemanticNamedKind,
        namespace: &str,
        native_name: String,
        iid: Option<String>,
        default_interface: Option<Box<Self>>,
        fields: Vec<(String, Self)>,
        enum_underlying: Option<Box<Self>>,
    ) -> Self {
        Self::Named {
            kind,
            namespace: namespace.into(),
            native_name,
            iid,
            default_interface,
            fields,
            enum_underlying,
        }
    }

    fn is_primitive(&self) -> bool {
        matches!(self, Self::Primitive(_))
    }

    fn format(&self, format: SemanticFormat) -> String {
        match self {
            Self::Primitive(primitive) => match format {
                SemanticFormat::ReadableToken => match primitive {
                    SemanticPrimitive::Bool => "Boolean",
                    SemanticPrimitive::I8 => "Int8",
                    SemanticPrimitive::U8 => "UInt8",
                    SemanticPrimitive::I16 => "Int16",
                    SemanticPrimitive::U16 => "UInt16",
                    SemanticPrimitive::I32 => "Int32",
                    SemanticPrimitive::U32 => "UInt32",
                    SemanticPrimitive::I64 => "Int64",
                    SemanticPrimitive::U64 => "UInt64",
                    SemanticPrimitive::F32 => "Single",
                    SemanticPrimitive::F64 => "Double",
                    SemanticPrimitive::String => "String",
                    SemanticPrimitive::Char16 => "Char16",
                    SemanticPrimitive::Guid => "Guid",
                    SemanticPrimitive::Object => "Object",
                }
                .into(),
                SemanticFormat::StableIdentity | SemanticFormat::AbiIdentity => match primitive {
                    SemanticPrimitive::Bool => "bool",
                    SemanticPrimitive::I8 => "i8",
                    SemanticPrimitive::U8 => "u8",
                    SemanticPrimitive::I16 => "i16",
                    SemanticPrimitive::U16 => "u16",
                    SemanticPrimitive::I32 => "i32",
                    SemanticPrimitive::U32 => "u32",
                    SemanticPrimitive::I64 => "i64",
                    SemanticPrimitive::U64 => "u64",
                    SemanticPrimitive::F32 => "f32",
                    SemanticPrimitive::F64 => "f64",
                    SemanticPrimitive::String => "string",
                    SemanticPrimitive::Char16 => "char16",
                    SemanticPrimitive::Guid => "guid",
                    SemanticPrimitive::Object => "object",
                }
                .into(),
            },
            Self::Named {
                kind,
                namespace,
                native_name,
                iid,
                default_interface,
                fields,
                enum_underlying,
            } => match format {
                SemanticFormat::ReadableToken => format!(
                    "{}{}",
                    namespace
                        .split('.')
                        .filter(|segment| !segment.is_empty())
                        .map(pascal_segment)
                        .collect::<String>(),
                    native_name
                ),
                SemanticFormat::StableIdentity => format!(
                    "{}:{namespace}.{native_name}",
                    match kind {
                        SemanticNamedKind::Interface => "interface",
                        SemanticNamedKind::RuntimeClass => "class",
                        SemanticNamedKind::Delegate => "delegate",
                        SemanticNamedKind::Struct => "struct",
                        SemanticNamedKind::Enum => "enum",
                    }
                ),
                SemanticFormat::AbiIdentity => match kind {
                    SemanticNamedKind::Interface => format!(
                        "interface({namespace}.{native_name},{})",
                        normalized_guid(iid.as_deref().unwrap_or_default())
                    ),
                    SemanticNamedKind::RuntimeClass => format!(
                        "class({namespace}.{native_name};default={})",
                        default_interface.as_deref().map_or_else(
                            || "none".into(),
                            |typ| typ.format(SemanticFormat::AbiIdentity)
                        )
                    ),
                    SemanticNamedKind::Delegate => format!(
                        "delegate({namespace}.{native_name},{})",
                        normalized_guid(iid.as_deref().unwrap_or_default())
                    ),
                    SemanticNamedKind::Struct => format!(
                        "struct({namespace}.{native_name};fields=[{}])",
                        fields
                            .iter()
                            .map(|(name, typ)| format!(
                                "{name}:{}",
                                typ.format(SemanticFormat::AbiIdentity)
                            ))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    SemanticNamedKind::Enum => format!(
                        "enum({namespace}.{native_name};underlying={})",
                        enum_underlying
                            .as_deref()
                            .expect("canonical enum identity always has an underlying type")
                            .format(SemanticFormat::AbiIdentity)
                    ),
                },
            },
            Self::Parameterized {
                namespace,
                native_name,
                piid,
                args,
            } => match format {
                SemanticFormat::ReadableToken => format!(
                    "{}{}",
                    namespace
                        .split('.')
                        .filter(|segment| !segment.is_empty())
                        .map(pascal_segment)
                        .collect::<String>(),
                    parameterized_name_for_identity(namespace, native_name, piid, args)
                ),
                SemanticFormat::StableIdentity => format!(
                    "generic:{namespace}.{native_name}<{}>",
                    Self::format_list(args, SemanticFormat::StableIdentity)
                ),
                SemanticFormat::AbiIdentity => format!(
                    "parameterized({namespace}.{native_name},{},[{}])",
                    normalized_guid(piid),
                    Self::format_list(args, SemanticFormat::AbiIdentity)
                ),
            },
            Self::Array(inner) => match format {
                SemanticFormat::ReadableToken => {
                    format!("Array{}", inner.format(SemanticFormat::ReadableToken))
                }
                SemanticFormat::StableIdentity => {
                    format!("array:{}", inner.format(SemanticFormat::StableIdentity))
                }
                SemanticFormat::AbiIdentity => {
                    format!("array({})", inner.format(SemanticFormat::AbiIdentity))
                }
            },
            Self::AsyncAction => match format {
                SemanticFormat::ReadableToken => "AsyncAction",
                SemanticFormat::StableIdentity | SemanticFormat::AbiIdentity => "async-action",
            }
            .into(),
            Self::AsyncActionWithProgress(progress) => match format {
                SemanticFormat::ReadableToken => format!(
                    "AsyncActionWithProgress{}",
                    progress.format(SemanticFormat::ReadableToken)
                ),
                SemanticFormat::StableIdentity => format!(
                    "async-action-progress:{}",
                    progress.format(SemanticFormat::StableIdentity)
                ),
                SemanticFormat::AbiIdentity => format!(
                    "async-action-with-progress({})",
                    progress.format(SemanticFormat::AbiIdentity)
                ),
            },
            Self::AsyncOperation(result) => match format {
                SemanticFormat::ReadableToken => format!(
                    "AsyncOperation{}",
                    result.format(SemanticFormat::ReadableToken)
                ),
                SemanticFormat::StableIdentity => format!(
                    "async-operation:{}",
                    result.format(SemanticFormat::StableIdentity)
                ),
                SemanticFormat::AbiIdentity => format!(
                    "async-operation({})",
                    result.format(SemanticFormat::AbiIdentity)
                ),
            },
            Self::AsyncOperationWithProgress(result, progress) => match format {
                SemanticFormat::ReadableToken => format!(
                    "AsyncOperationWithProgress{}{}",
                    result.format(SemanticFormat::ReadableToken),
                    progress.format(SemanticFormat::ReadableToken)
                ),
                SemanticFormat::StableIdentity => format!(
                    "async-operation-progress:{},{}",
                    result.format(SemanticFormat::StableIdentity),
                    progress.format(SemanticFormat::StableIdentity)
                ),
                SemanticFormat::AbiIdentity => format!(
                    "async-operation-with-progress({},{})",
                    result.format(SemanticFormat::AbiIdentity),
                    progress.format(SemanticFormat::AbiIdentity)
                ),
            },
        }
    }

    fn format_list(values: &[Self], format: SemanticFormat) -> String {
        values
            .iter()
            .map(|value| value.format(format))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn canonical_types(
    args: &[TypeMeta],
    context: Option<&JavaScriptProjectionContext>,
) -> Vec<JavaScriptSemanticTypeIdentity> {
    args.iter()
        .map(|typ| JavaScriptSemanticTypeIdentity::from_type(typ, context))
        .collect()
}

pub fn parameterized_abi_identity(piid: &str, iid: &str, args: &[TypeMeta]) -> String {
    parameterized_abi_identity_impl(None, piid, iid, args)
}

fn parameterized_abi_identity_impl(
    context: Option<&JavaScriptProjectionContext>,
    piid: &str,
    iid: &str,
    args: &[TypeMeta],
) -> String {
    format!(
        "piid:{}:iid:{}:args:[{}]",
        normalized_guid(piid),
        normalized_guid(iid),
        JavaScriptSemanticTypeIdentity::format_list(
            &canonical_types(args, context),
            SemanticFormat::AbiIdentity,
        )
    )
}

pub fn parameterized_reference_identity(piid: &str, args: &[TypeMeta]) -> String {
    parameterized_reference_identity_impl(None, piid, args)
}

fn parameterized_reference_identity_with_context(
    context: &JavaScriptProjectionContext,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    parameterized_reference_identity_impl(Some(context), piid, args)
}

fn parameterized_reference_identity_impl(
    context: Option<&JavaScriptProjectionContext>,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    format!(
        "piid:{}:args:[{}]",
        normalized_guid(piid),
        JavaScriptSemanticTypeIdentity::format_list(
            &canonical_types(args, context),
            SemanticFormat::AbiIdentity,
        )
    )
}

pub fn interface_abi_identity(interface: &InterfaceMeta) -> String {
    match &interface.generic_piid {
        Some(piid) => parameterized_abi_identity(piid, &interface.iid, &interface.generic_args),
        None => format!("iid:{}", normalized_guid(&interface.iid)),
    }
}

pub fn parameterized_name(
    namespace: &str,
    generic_name: &str,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    parameterized_name_impl(None, namespace, generic_name, piid, args)
}

fn parameterized_name_with_context(
    context: &JavaScriptProjectionContext,
    namespace: &str,
    generic_name: &str,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    parameterized_name_impl(Some(context), namespace, generic_name, piid, args)
}

fn parameterized_name_impl(
    context: Option<&JavaScriptProjectionContext>,
    namespace: &str,
    generic_name: &str,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    let identities = canonical_types(args, context);
    parameterized_name_for_identity(namespace, generic_name, piid, &identities)
}

fn parameterized_name_for_identity(
    namespace: &str,
    generic_name: &str,
    piid: &str,
    args: &[JavaScriptSemanticTypeIdentity],
) -> String {
    let base = generic_name.split('`').next().unwrap_or(generic_name);
    let readable = format!(
        "{}_{}",
        base,
        args.iter()
            .map(|typ| typ.format(SemanticFormat::ReadableToken))
            .collect::<Vec<_>>()
            .join("_")
    );
    let signature = format!(
        "{namespace}.{base}:{}<{}>",
        normalized_guid(piid),
        args.iter()
            .map(|typ| typ.format(SemanticFormat::StableIdentity))
            .collect::<Vec<_>>()
            .join(",")
    );
    let requires_identity_suffix = !matches!(
        namespace,
        "Windows.Foundation" | "Windows.Foundation.Collections"
    ) || args.iter().any(|argument| !argument.is_primitive());
    if !requires_identity_suffix {
        return readable;
    }
    let suffix = format!("_g{:016x}", fnv1a64(&signature));
    const MAX_IDENTIFIER_LENGTH: usize = 120;
    let keep = MAX_IDENTIFIER_LENGTH - suffix.len();
    format!(
        "{}{}",
        readable.chars().take(keep).collect::<String>(),
        suffix
    )
}

pub fn parameterized_interface_base_name(interface_name: &str, args: &[TypeMeta]) -> String {
    let legacy_suffix = crate::meta::make_parameterized_name("", args);
    interface_name
        .strip_suffix(&legacy_suffix)
        .or_else(|| interface_name.split_once('_').map(|(base, _)| base))
        .unwrap_or(interface_name)
        .into()
}

pub fn parameterized_interface_name(
    namespace: &str,
    interface_name: &str,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    parameterized_name(
        namespace,
        &parameterized_interface_base_name(interface_name, args),
        piid,
        args,
    )
}

fn parameterized_interface_name_with_context(
    context: &JavaScriptProjectionContext,
    namespace: &str,
    interface_name: &str,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    parameterized_name_with_context(
        context,
        namespace,
        &parameterized_interface_base_name(interface_name, args),
        piid,
        args,
    )
}

pub fn projected_parameterized_name(
    context: &JavaScriptProjectionContext,
    namespace: &str,
    generic_name: &str,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    context.projected_parameterized_name(namespace, generic_name, piid, args)
}

fn namespace_path(namespace: &str) -> String {
    namespace
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut result = String::new();
            let chars = segment.chars().collect::<Vec<_>>();
            for (index, character) in chars.iter().copied().enumerate() {
                if !character.is_ascii_alphanumeric() {
                    if !result.ends_with('-') {
                        result.push('-');
                    }
                    continue;
                }
                let previous_is_lower_or_digit = index > 0
                    && (chars[index - 1].is_ascii_lowercase() || chars[index - 1].is_ascii_digit());
                let next_is_lower = chars
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_lowercase());
                if character.is_ascii_uppercase()
                    && !result.is_empty()
                    && (previous_is_lower_or_digit || next_is_lower)
                    && !result.ends_with('-')
                {
                    result.push('-');
                }
                result.push(character.to_ascii_lowercase());
            }
            result.trim_matches('-').to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn qualified_projected_name(identity: &JavaScriptTypeIdentity) -> String {
    let qualifier = namespace_qualifier(&identity.namespace);
    let candidate = if qualifier.is_empty() {
        format!(
            "{}{}",
            pascal_segment(identity.kind.as_str()),
            identity.name
        )
    } else {
        format!("{qualifier}{}", identity.name)
    };
    bounded_identifier(candidate, identity)
}

fn disambiguated_projected_name(identity: &JavaScriptTypeIdentity) -> String {
    let namespace = identity
        .namespace
        .split('.')
        .map(pascal_segment)
        .collect::<String>();
    bounded_identifier(
        format!(
            "{namespace}{}{}",
            identity.name,
            pascal_segment(identity.kind.as_str())
        ),
        identity,
    )
}

fn hashed_projected_name(candidate: String, identity: &JavaScriptTypeIdentity) -> String {
    let suffix = format!(
        "_{:08x}",
        fnv1a64(&format!(
            "{}|{}|{}|{}",
            identity.kind.as_str(),
            identity.namespace,
            identity.name,
            identity.variant,
        )) as u32
    );
    let keep = 120usize.saturating_sub(suffix.len());
    format!(
        "{}{}",
        candidate.chars().take(keep).collect::<String>(),
        suffix
    )
}

pub fn create_javascript_projection_context(
    identities: impl IntoIterator<Item = JavaScriptTypeIdentity>,
) -> Result<JavaScriptProjectionContext, String> {
    create_javascript_projection_context_with_records(
        identities,
        std::iter::empty::<JavaScriptTypeLayoutRecord>(),
        "@microsoft/dynwinrt",
    )
}

pub fn create_javascript_projection_context_with_records(
    identities: impl IntoIterator<Item = JavaScriptTypeIdentity>,
    previous_records: impl IntoIterator<Item = JavaScriptTypeLayoutRecord>,
    runtime_import_name: impl Into<String>,
) -> Result<JavaScriptProjectionContext, String> {
    let identities = identities.into_iter().collect::<BTreeSet<_>>();
    if let Some(identity) = identities.iter().find(|identity| !identity.is_safe()) {
        return Err(format!(
            "Unsafe JavaScript type identity `{}`",
            identity.to_inventory_line()
        ));
    }
    let mut counts = HashMap::<String, usize>::new();
    for identity in &identities {
        *counts.entry(identity.name.clone()).or_default() += 1;
    }

    let mut previous_by_identity = BTreeMap::new();
    for record in previous_records {
        if !record.is_safe() || !identities.contains(&record.identity) {
            return Err(format!(
                "Invalid previous JavaScript module layout record `{:?}`",
                record
            ));
        }
        let identity = record.identity.clone();
        if previous_by_identity
            .insert(identity.clone(), record)
            .is_some()
        {
            return Err(format!(
                "Duplicate previous JavaScript module layout record for `{}.{}`",
                identity.namespace, identity.name,
            ));
        }
    }

    let mut projected_names = BTreeMap::new();
    let mut normalized_projected_owners = HashMap::<String, JavaScriptTypeIdentity>::new();

    // Assign collision-derived names first so a unique native short name can
    // never claim another type's natural qualified name in a fresh layout.
    for colliding_pass in [true, false] {
        for identity in identities
            .iter()
            .filter(|identity| (counts[&identity.name] > 1) == colliding_pass)
        {
            if projected_names.contains_key(identity) {
                continue;
            }
            let mut projected = if colliding_pass {
                qualified_projected_name(identity)
            } else {
                identity.name.clone()
            };
            if normalized_projected_owners.contains_key(&projected.to_ascii_lowercase()) {
                projected = disambiguated_projected_name(identity);
            }
            if normalized_projected_owners.contains_key(&projected.to_ascii_lowercase()) {
                projected = hashed_projected_name(projected, identity);
            }
            let normalized = projected.to_ascii_lowercase();
            if let Some(existing) = normalized_projected_owners.get(&normalized) {
                return Err(format!(
                    "JavaScript projected name collision: `{}.{}` and `{}.{}` both map to `{projected}`",
                    existing.namespace, existing.name, identity.namespace, identity.name
                ));
            }
            normalized_projected_owners.insert(normalized, identity.clone());
            projected_names.insert(identity.clone(), projected);
        }
    }

    let mut targets = BTreeMap::new();
    let mut module_owners = HashMap::<String, JavaScriptTypeIdentity>::new();
    for identity in identities {
        let projected_name = projected_names[&identity].clone();
        let namespace = namespace_path(&identity.namespace);
        let canonical_module = if namespace.is_empty() {
            identity.name.clone()
        } else {
            format!("{namespace}/{}", identity.name)
        };
        let normalized_module = canonical_module.to_ascii_lowercase();
        if let Some(existing) = module_owners.insert(normalized_module, identity.clone()) {
            return Err(format!(
                "JavaScript canonical module collision: `{}.{}` and `{}.{}` both map to `{canonical_module}`",
                existing.namespace, existing.name, identity.namespace, identity.name
            ));
        }
        let mut compatibility_aliases = previous_by_identity
            .get(&identity)
            .map(|record| record.compatibility_aliases.clone())
            .unwrap_or_default();
        if let Some(previous) = previous_by_identity.get(&identity)
            && previous.projected_name != projected_name
        {
            compatibility_aliases.insert(previous.projected_name.clone());
        }
        compatibility_aliases.remove(&projected_name);
        compatibility_aliases.remove(&identity.name);
        let target = JavaScriptOutputTarget {
            identity: identity.clone(),
            projected_name: projected_name.clone(),
            canonical_module,
            collides: counts[&identity.name] > 1 || projected_name != identity.name,
            compatibility_aliases,
        };
        targets.insert(identity, target);
    }
    let projected_owners = projected_names
        .into_iter()
        .map(|(identity, projected)| (projected, identity))
        .collect();

    Ok(JavaScriptProjectionContext {
        layout: JavaScriptModuleLayout {
            targets,
            by_projected_name: projected_owners,
        },
        runtime_import_name: runtime_import_name.into(),
    })
}

fn apply_projected_type_names(context: &JavaScriptProjectionContext, typ: &mut TypeMeta) {
    match typ {
        TypeMeta::Interface {
            namespace, name, ..
        } => *name = context.projected_name(namespace, name, JavaScriptTypeKind::Interface),
        TypeMeta::Delegate {
            namespace, name, ..
        } => *name = context.projected_name(namespace, name, JavaScriptTypeKind::Delegate),
        TypeMeta::RuntimeClass {
            namespace,
            name,
            default_interface,
        } => {
            *name = context.projected_name(namespace, name, JavaScriptTypeKind::Class);
            if let Some(default_interface) = default_interface {
                apply_projected_type_names(context, default_interface);
            }
        }
        TypeMeta::AsyncActionWithProgress(inner)
        | TypeMeta::AsyncOperation(inner)
        | TypeMeta::Array(inner) => apply_projected_type_names(context, inner),
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            apply_projected_type_names(context, result);
            apply_projected_type_names(context, progress);
        }
        TypeMeta::Parameterized { args, .. } => {
            for argument in args {
                apply_projected_type_names(context, argument);
            }
        }
        TypeMeta::Struct { fields, .. } => {
            for field in fields {
                apply_projected_type_names(context, &mut field.typ);
            }
        }
        TypeMeta::Enum {
            namespace,
            name,
            underlying,
            ..
        } => {
            *name = context.projected_name(namespace, name, JavaScriptTypeKind::Enum);
            apply_projected_type_names(context, underlying);
        }
        _ => {}
    }
}

fn apply_projected_method_names(context: &JavaScriptProjectionContext, method: &mut MethodMeta) {
    for parameter in &mut method.params {
        apply_projected_type_names(context, &mut parameter.typ);
    }
    if let Some(return_type) = &mut method.return_type {
        apply_projected_type_names(context, return_type);
    }
}

fn apply_projected_interface_names(
    context: &JavaScriptProjectionContext,
    interface: &mut InterfaceMeta,
) {
    let kind = if interface
        .methods
        .iter()
        .any(|method| method.name == ".ctor")
        && interface
            .methods
            .iter()
            .any(|method| method.name == "Invoke")
    {
        JavaScriptTypeKind::Delegate
    } else {
        JavaScriptTypeKind::Interface
    };
    interface.name = if let Some(piid) = &interface.generic_piid {
        let output_name = parameterized_interface_name_with_context(
            context,
            &interface.namespace,
            &interface.name,
            piid,
            &interface.generic_args,
        );
        let variant =
            parameterized_reference_identity_with_context(context, piid, &interface.generic_args);
        context
            .target_for_identity(&JavaScriptTypeIdentity::with_variant(
                &interface.namespace,
                &output_name,
                kind,
                &variant,
            ))
            .map_or(output_name, |target| target.projected_name.clone())
    } else {
        context.projected_name(&interface.namespace, &interface.name, kind)
    };
    for base in &mut interface.base_interfaces {
        apply_projected_type_names(context, base);
    }
    for argument in &mut interface.generic_args {
        apply_projected_type_names(context, argument);
    }
    for method in &mut interface.methods {
        apply_projected_method_names(context, method);
    }
}

fn apply_projected_type_ref_name(context: &JavaScriptProjectionContext, reference: &mut TypeRef) {
    reference.name =
        context.projected_name(&reference.namespace, &reference.name, reference.kind.into());
}

fn apply_projected_class_names(context: &JavaScriptProjectionContext, class: &mut ClassMeta) {
    class.name = context.projected_name(&class.namespace, &class.name, JavaScriptTypeKind::Class);
    if let Some(base_class) = &mut class.base_class {
        apply_projected_type_ref_name(context, base_class);
    }
    if let Some(default_interface) = &mut class.default_interface {
        apply_projected_interface_names(context, default_interface);
    }
    for interface in class
        .required_interfaces
        .iter_mut()
        .chain(class.overridable_interfaces.iter_mut())
        .chain(class.factory_interfaces.iter_mut())
        .chain(class.static_interfaces.iter_mut())
    {
        apply_projected_interface_names(context, interface);
    }
    for constructor in &mut class.constructors {
        if let Some(factory_interface) = &mut constructor.factory_interface {
            apply_projected_type_ref_name(context, factory_interface);
        }
    }
}

pub fn apply_javascript_projected_names(
    context: &JavaScriptProjectionContext,
    classes: &mut [ClassMeta],
    interfaces: &mut [InterfaceMeta],
    enums: &mut [TypeMeta],
) {
    for class in classes {
        apply_projected_class_names(context, class);
    }
    for interface in interfaces {
        apply_projected_interface_names(context, interface);
    }
    for en in enums {
        apply_projected_type_names(context, en);
    }
}

pub fn validate_struct_helper_identities(
    classes: &[ClassMeta],
    interfaces: &[InterfaceMeta],
) -> Result<BTreeMap<String, String>, String> {
    fn validate(
        owner: &str,
        structs: Vec<TypeMeta>,
        identities: &mut HashMap<String, (String, String)>,
    ) -> Result<(), String> {
        for structure in structs {
            let TypeMeta::Struct {
                namespace, name, ..
            } = structure
            else {
                continue;
            };
            let identity = format!("{namespace}.{name}");
            if let Some((existing, existing_owner)) =
                identities.insert(name.clone(), (identity.clone(), owner.to_string()))
                && existing != identity
            {
                return Err(format!(
                    "JavaScript struct helper collision: `{existing}` from `{existing_owner}` and \
                     `{identity}` from `{owner}` both require `{name}_Type`/`pack{name}`. \
                     Generate these bindings into separate output directories."
                ));
            }
        }
        Ok(())
    }

    let mut identities = HashMap::new();
    for class in classes {
        validate(
            &class.full_name,
            collect_used_structs_from_class(class),
            &mut identities,
        )?;
    }
    for interface in interfaces {
        validate(
            &format!("{}.{}", interface.namespace, interface.name),
            collect_used_structs_from_iface(interface),
            &mut identities,
        )?;
    }
    Ok(identities
        .into_iter()
        .map(|(name, (identity, _))| (name, identity))
        .collect())
}

fn relative_module(from_module: &str, to_module: &str) -> String {
    let from_parent = Path::new(from_module)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let from = from_parent.components().collect::<Vec<_>>();
    let to = Path::new(to_module).components().collect::<Vec<_>>();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut result = PathBuf::new();
    for _ in common..from.len() {
        result.push("..");
    }
    for component in &to[common..] {
        match component {
            Component::Normal(value) => result.push(value),
            Component::ParentDir => result.push(".."),
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => {}
        }
    }
    let result = result.to_string_lossy().replace('\\', "/");
    if result.starts_with("../") {
        result
    } else {
        format!("./{result}")
    }
}

pub fn root_relative_module(from_module: &str, root_module: &str) -> String {
    relative_module(from_module, root_module)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{ParamDirection, ParamMeta};

    fn runtime_class(namespace: &str, name: &str) -> TypeMeta {
        TypeMeta::RuntimeClass {
            namespace: namespace.into(),
            name: name.into(),
            default_interface: Some(Box::new(TypeMeta::Interface {
                namespace: namespace.into(),
                name: format!("I{name}"),
                iid: "11111111-1111-1111-1111-111111111111".into(),
            })),
        }
    }

    #[test]
    fn layout_qualifies_collisions_and_preserves_native_identities() {
        let identities = [
            JavaScriptTypeIdentity::new(
                "Microsoft.UI.Input.DragDrop",
                "DragUIOverride",
                JavaScriptTypeKind::Class,
            ),
            JavaScriptTypeIdentity::new(
                "Microsoft.UI.Xaml",
                "DragUIOverride",
                JavaScriptTypeKind::Class,
            ),
            JavaScriptTypeIdentity::new(
                "Microsoft.UI.Composition",
                "AnimationDirection",
                JavaScriptTypeKind::Enum,
            ),
            JavaScriptTypeIdentity::new(
                "Microsoft.UI.Xaml.Controls.Primitives",
                "AnimationDirection",
                JavaScriptTypeKind::Enum,
            ),
        ];
        let context = create_javascript_projection_context(identities).unwrap();
        let mut classes = vec![
            ClassMeta {
                namespace: "Microsoft.UI.Input.DragDrop".into(),
                name: "DragUIOverride".into(),
                full_name: "Microsoft.UI.Input.DragDrop.DragUIOverride".into(),
                default_interface: Some(InterfaceMeta {
                    name: "IDragUIOverride".into(),
                    methods: vec![MethodMeta {
                        params: vec![ParamMeta {
                            name: "value".into(),
                            typ: runtime_class("Microsoft.UI.Input.DragDrop", "DragUIOverride"),
                            direction: ParamDirection::In,
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            },
            ClassMeta {
                namespace: "Microsoft.UI.Xaml".into(),
                name: "DragUIOverride".into(),
                full_name: "Microsoft.UI.Xaml.DragUIOverride".into(),
                ..Default::default()
            },
        ];
        let mut enums = vec![TypeMeta::Enum {
            namespace: "Microsoft.UI.Xaml.Controls.Primitives".into(),
            name: "AnimationDirection".into(),
            underlying: Box::new(TypeMeta::I32),
            members: Vec::new(),
            is_flags: false,
            doc: None,
            deprecated: None,
        }];

        apply_javascript_projected_names(&context, &mut classes, &mut [], &mut enums);

        assert_eq!(classes[0].name, "InputDragDropDragUIOverride");
        assert_eq!(classes[1].name, "XamlDragUIOverride");
        assert_eq!(
            classes[0].full_name,
            "Microsoft.UI.Input.DragDrop.DragUIOverride"
        );
        let parameter = &classes[0].default_interface.as_ref().unwrap().methods[0].params[0].typ;
        assert!(matches!(
            parameter,
            TypeMeta::RuntimeClass { name, .. } if name == "InputDragDropDragUIOverride"
        ));
        assert!(
            signature::ts_dynwinrt_type(&context, parameter)
                .contains("Microsoft.UI.Input.DragDrop.DragUIOverride")
        );
        assert!(matches!(
            enums[0],
            TypeMeta::Enum { ref name, .. } if name == "XamlControlsPrimitivesAnimationDirection"
        ));
        assert_eq!(
            context
                .metadata_type_name("Microsoft.UI.Input.DragDrop", "InputDragDropDragUIOverride"),
            "DragUIOverride"
        );
    }

    #[test]
    fn layout_keeps_unique_short_names() {
        let identity = JavaScriptTypeIdentity::new(
            "Microsoft.UI.Xaml.Controls",
            "Button",
            JavaScriptTypeKind::Class,
        );
        let context = create_javascript_projection_context([identity.clone()]).unwrap();
        let target = context.target_for_identity(&identity).unwrap();

        assert_eq!(target.projected_name, "Button");
        assert_eq!(target.canonical_module, "microsoft/ui/xaml/controls/Button");
        assert!(!target.collides);
    }

    #[test]
    fn independent_contexts_do_not_share_layout_state() {
        let button = JavaScriptTypeIdentity::new(
            "Microsoft.UI.Xaml.Controls",
            "Button",
            JavaScriptTypeKind::Class,
        );
        let uri =
            JavaScriptTypeIdentity::new("Windows.Foundation", "Uri", JavaScriptTypeKind::Class);
        let outer = create_javascript_projection_context([button.clone()]).unwrap();
        let inner = create_javascript_projection_context([uri.clone()]).unwrap();
        assert!(outer.target_for_identity(&button).is_some());
        assert!(outer.target_for_identity(&uri).is_none());
        assert!(inner.target_for_identity(&button).is_none());
        assert!(inner.target_for_identity(&uri).is_some());
    }

    #[test]
    fn parameterized_names_include_complete_named_argument_identity() {
        let windows_pointer = TypeMeta::RuntimeClass {
            namespace: "Windows.UI.Input".into(),
            name: "PointerPoint".into(),
            default_interface: None,
        };
        let winui_pointer = TypeMeta::RuntimeClass {
            namespace: "Microsoft.UI.Input".into(),
            name: "PointerPoint".into(),
            default_interface: None,
        };

        let windows_name = parameterized_name(
            "Windows.Foundation.Collections",
            "IVector`1",
            "vector",
            &[windows_pointer],
        );
        let winui_name = parameterized_name(
            "Windows.Foundation.Collections",
            "IVector",
            "vector",
            &[winui_pointer],
        );
        assert!(
            windows_name.starts_with("IVector_WindowsUIInputPointerPoint_g"),
            "{windows_name}"
        );
        assert!(
            winui_name.starts_with("IVector_MicrosoftUIInputPointerPoint_g"),
            "{winui_name}"
        );
        assert_ne!(windows_name, winui_name);
        assert_eq!(
            parameterized_name(
                "Windows.Foundation.Collections",
                "IMap",
                "map",
                &[TypeMeta::String, TypeMeta::Object],
            ),
            "IMap_String_Object"
        );
        assert_eq!(
            windows_name,
            "IVector_WindowsUIInputPointerPoint_gc328f0b29477ee07"
        );
        assert_eq!(
            parameterized_reference_identity(
                "vector",
                &[TypeMeta::RuntimeClass {
                    namespace: "Windows.UI.Input".into(),
                    name: "PointerPoint".into(),
                    default_interface: None,
                }],
            ),
            "piid:vector:args:[class(Windows.UI.Input.PointerPoint;default=none)]"
        );
    }

    #[test]
    fn parameterized_names_recursively_encode_nested_generics() {
        let nested = TypeMeta::Parameterized {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IVector`1".into(),
            piid: "vector".into(),
            args: vec![TypeMeta::RuntimeClass {
                namespace: "Contoso.Models".into(),
                name: "Widget".into(),
                default_interface: None,
            }],
        };
        let name = parameterized_name(
            "Windows.Foundation.Collections",
            "IIterable`1",
            "iterable",
            &[nested],
        );

        assert!(
            name.starts_with("IIterable_WindowsFoundationCollectionsIVector_ContosoModelsWidget_g"),
            "{name}"
        );
    }

    #[test]
    fn parameterized_identity_suffix_disambiguates_namespace_boundaries() {
        let first = TypeMeta::RuntimeClass {
            namespace: "A.B".into(),
            name: "C".into(),
            default_interface: None,
        };
        let second = TypeMeta::RuntimeClass {
            namespace: "A".into(),
            name: "BC".into(),
            default_interface: None,
        };
        let first_name = parameterized_name(
            "Windows.Foundation.Collections",
            "IVector",
            "vector",
            &[first],
        );
        let second_name = parameterized_name(
            "Windows.Foundation.Collections",
            "IVector",
            "vector",
            &[second],
        );

        assert!(first_name.starts_with("IVector_ABC_g"), "{first_name}");
        assert!(second_name.starts_with("IVector_ABC_g"), "{second_name}");
        assert_ne!(first_name, second_name);
    }

    #[test]
    fn generic_abi_identity_includes_runtime_and_struct_signatures() {
        let runtime = |iid: &str| TypeMeta::RuntimeClass {
            namespace: "Contoso".into(),
            name: "Widget".into(),
            default_interface: Some(Box::new(TypeMeta::Interface {
                namespace: "Contoso".into(),
                name: "IWidget".into(),
                iid: iid.into(),
            })),
        };
        let structure = |field_type| TypeMeta::Struct {
            namespace: "Contoso".into(),
            name: "Payload".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: field_type,
            }],
        };

        assert_ne!(
            parameterized_abi_identity("piid", "piid", &[runtime("iid-one")]),
            parameterized_abi_identity("piid", "piid", &[runtime("iid-two")])
        );
        assert_ne!(
            parameterized_abi_identity("piid", "piid", &[structure(TypeMeta::I32)]),
            parameterized_abi_identity("piid", "piid", &[structure(TypeMeta::I64)])
        );
    }

    #[test]
    fn clean_and_incremental_layouts_use_the_same_second_order_collision_owner() {
        let windows =
            JavaScriptTypeIdentity::new("Windows.Foundation", "Widget", JavaScriptTypeKind::Class);
        let contoso = JavaScriptTypeIdentity::new("Contoso", "Widget", JavaScriptTypeKind::Class);
        let old_context =
            create_javascript_projection_context([windows.clone(), contoso.clone()]).unwrap();
        let records = old_context
            .output_targets()
            .into_iter()
            .map(|target| {
                JavaScriptTypeLayoutRecord::new(
                    target.identity.clone(),
                    target.projected_name.clone(),
                    "type",
                )
            })
            .collect::<Vec<_>>();

        let microsoft = JavaScriptTypeIdentity::new(
            "Microsoft.UI.Foundation",
            "Widget",
            JavaScriptTypeKind::Class,
        );
        let identities = [windows.clone(), contoso.clone(), microsoft.clone()];
        let incremental_context = create_javascript_projection_context_with_records(
            identities.clone(),
            records,
            "@microsoft/dynwinrt",
        )
        .unwrap();
        assert!(
            incremental_context
                .target_for_identity(&windows)
                .unwrap()
                .compatibility_aliases
                .contains("FoundationWidget")
        );
        let incremental = identities
            .iter()
            .map(|identity| {
                (
                    identity.clone(),
                    incremental_context
                        .target_for_identity(identity)
                        .unwrap()
                        .projected_name
                        .clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let clean_context = create_javascript_projection_context(identities.clone()).unwrap();
        let clean = identities
            .iter()
            .map(|identity| {
                (
                    identity.clone(),
                    clean_context
                        .target_for_identity(identity)
                        .unwrap()
                        .projected_name
                        .clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();

        assert_eq!(incremental, clean);
        assert_eq!(clean[&microsoft], "FoundationWidget");
        assert_eq!(clean[&windows], "WindowsFoundationWidgetClass");
    }

    #[test]
    fn namespace_distinct_struct_helpers_fail_closed_before_projection() {
        let structure = |namespace: &str, typ| TypeMeta::Struct {
            namespace: namespace.into(),
            name: "Payload".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ,
            }],
        };
        let class = ClassMeta {
            name: "Widget".into(),
            namespace: "Contoso".into(),
            full_name: "Contoso.Widget".into(),
            default_interface: Some(InterfaceMeta {
                name: "IWidget".into(),
                namespace: "Contoso".into(),
                iid: "11111111-1111-1111-1111-111111111111".into(),
                methods: vec![MethodMeta {
                    name: "UsePayloads".into(),
                    params: vec![
                        ParamMeta {
                            name: "alpha".into(),
                            typ: structure("Alpha", TypeMeta::I32),
                            direction: ParamDirection::In,
                        },
                        ParamMeta {
                            name: "beta".into(),
                            typ: structure("Beta", TypeMeta::I64),
                            direction: ParamDirection::In,
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let error = validate_struct_helper_identities(&[class], &[])
            .expect_err("namespace-distinct short struct names must fail closed");

        assert!(error.contains("Alpha.Payload"), "{error}");
        assert!(error.contains("Beta.Payload"), "{error}");
        assert!(error.contains("packPayload"), "{error}");
    }

    #[test]
    fn relative_modules_point_from_namespace_to_root_and_siblings() {
        assert_eq!(
            relative_module("microsoft/ui/xaml/controls/Button", "lifetime"),
            "../../../../lifetime"
        );
        assert_eq!(
            relative_module(
                "microsoft/ui/xaml/controls/Button",
                "microsoft/ui/xaml/Control"
            ),
            "../Control"
        );
        assert_eq!(
            relative_module("windows/foundation/Uri", "../runtime"),
            "../../../runtime"
        );
    }

    #[test]
    fn qualified_names_use_stable_bounded_hash_suffixes() {
        let name = "VeryLongProjectedTypeName".repeat(8);
        let first = JavaScriptTypeIdentity::new(
            "Contoso.Extremely.Long.Namespace.For.Generated.Bindings",
            &name,
            JavaScriptTypeKind::Class,
        );
        let second = JavaScriptTypeIdentity::new(
            "Fabrikam.Extremely.Long.Namespace.For.Generated.Bindings",
            &name,
            JavaScriptTypeKind::Class,
        );

        let first_name = qualified_projected_name(&first);
        let second_name = qualified_projected_name(&second);

        assert_eq!(first_name.len(), 120);
        assert_eq!(second_name.len(), 120);
        assert_ne!(first_name, second_name);
        assert_eq!(first_name, qualified_projected_name(&first));
    }

    #[test]
    fn layout_rejects_unsafe_inventory_identities() {
        let identity =
            JavaScriptTypeIdentity::new("Contoso", "../outside/Widget", JavaScriptTypeKind::Class);

        let error = create_javascript_projection_context([identity])
            .err()
            .expect("path-like metadata names must be rejected");

        assert!(error.contains("Unsafe JavaScript type identity"), "{error}");
    }
}
