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

use std::cell::RefCell;
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

thread_local! {
    static MODULE_LAYOUT: RefCell<Option<JavaScriptModuleLayout>> = const { RefCell::new(None) };
}

pub struct JavaScriptModuleLayoutGuard {
    previous: Option<JavaScriptModuleLayout>,
}

impl Drop for JavaScriptModuleLayoutGuard {
    fn drop(&mut self) {
        MODULE_LAYOUT.with(|layout| {
            *layout.borrow_mut() = self.previous.take();
        });
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

fn named_identity_token(namespace: &str, name: &str) -> String {
    let native_name = metadata_type_name(namespace, name);
    let namespace_token = namespace
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(pascal_segment)
        .collect::<String>();
    format!("{namespace_token}{native_name}")
}

fn generic_type_token(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "Boolean".into(),
        TypeMeta::I8 => "Int8".into(),
        TypeMeta::U8 => "UInt8".into(),
        TypeMeta::I16 => "Int16".into(),
        TypeMeta::U16 => "UInt16".into(),
        TypeMeta::I32 => "Int32".into(),
        TypeMeta::U32 => "UInt32".into(),
        TypeMeta::I64 => "Int64".into(),
        TypeMeta::U64 => "UInt64".into(),
        TypeMeta::F32 => "Single".into(),
        TypeMeta::F64 => "Double".into(),
        TypeMeta::String => "String".into(),
        TypeMeta::Char16 => "Char16".into(),
        TypeMeta::Guid => "Guid".into(),
        TypeMeta::Object => "Object".into(),
        TypeMeta::Interface {
            namespace, name, ..
        }
        | TypeMeta::RuntimeClass {
            namespace, name, ..
        }
        | TypeMeta::Delegate {
            namespace, name, ..
        }
        | TypeMeta::Struct {
            namespace, name, ..
        }
        | TypeMeta::Enum {
            namespace, name, ..
        } => named_identity_token(namespace, name),
        TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
            ..
        } => format!(
            "{}{}",
            namespace
                .split('.')
                .filter(|segment| !segment.is_empty())
                .map(pascal_segment)
                .collect::<String>(),
            parameterized_name(namespace, name, piid, args)
        ),
        TypeMeta::Array(inner) => format!("Array{}", generic_type_token(inner)),
        TypeMeta::AsyncAction => "AsyncAction".into(),
        TypeMeta::AsyncActionWithProgress(progress) => {
            format!("AsyncActionWithProgress{}", generic_type_token(progress))
        }
        TypeMeta::AsyncOperation(result) => {
            format!("AsyncOperation{}", generic_type_token(result))
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => format!(
            "AsyncOperationWithProgress{}{}",
            generic_type_token(result),
            generic_type_token(progress)
        ),
    }
}

fn generic_identity_signature(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "bool".into(),
        TypeMeta::I8 => "i8".into(),
        TypeMeta::U8 => "u8".into(),
        TypeMeta::I16 => "i16".into(),
        TypeMeta::U16 => "u16".into(),
        TypeMeta::I32 => "i32".into(),
        TypeMeta::U32 => "u32".into(),
        TypeMeta::I64 => "i64".into(),
        TypeMeta::U64 => "u64".into(),
        TypeMeta::F32 => "f32".into(),
        TypeMeta::F64 => "f64".into(),
        TypeMeta::String => "string".into(),
        TypeMeta::Char16 => "char16".into(),
        TypeMeta::Guid => "guid".into(),
        TypeMeta::Object => "object".into(),
        TypeMeta::Interface {
            namespace, name, ..
        } => format!(
            "interface:{namespace}.{}",
            metadata_type_name(namespace, name)
        ),
        TypeMeta::RuntimeClass {
            namespace, name, ..
        } => format!("class:{namespace}.{}", metadata_type_name(namespace, name)),
        TypeMeta::Delegate {
            namespace, name, ..
        } => format!(
            "delegate:{namespace}.{}",
            metadata_type_name(namespace, name)
        ),
        TypeMeta::Struct {
            namespace, name, ..
        } => format!("struct:{namespace}.{}", metadata_type_name(namespace, name)),
        TypeMeta::Enum {
            namespace, name, ..
        } => format!("enum:{namespace}.{}", metadata_type_name(namespace, name)),
        TypeMeta::Parameterized {
            namespace,
            name,
            args,
            ..
        } => format!(
            "generic:{namespace}.{name}<{}>",
            args.iter()
                .map(generic_identity_signature)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeMeta::Array(inner) => format!("array:{}", generic_identity_signature(inner)),
        TypeMeta::AsyncAction => "async-action".into(),
        TypeMeta::AsyncActionWithProgress(progress) => {
            format!(
                "async-action-progress:{}",
                generic_identity_signature(progress)
            )
        }
        TypeMeta::AsyncOperation(result) => {
            format!("async-operation:{}", generic_identity_signature(result))
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => format!(
            "async-operation-progress:{},{}",
            generic_identity_signature(result),
            generic_identity_signature(progress)
        ),
    }
}

fn normalized_guid(value: &str) -> String {
    value
        .trim_matches(|character| character == '{' || character == '}')
        .to_ascii_lowercase()
}

fn type_abi_signature(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "bool".into(),
        TypeMeta::I8 => "i8".into(),
        TypeMeta::U8 => "u8".into(),
        TypeMeta::I16 => "i16".into(),
        TypeMeta::U16 => "u16".into(),
        TypeMeta::I32 => "i32".into(),
        TypeMeta::U32 => "u32".into(),
        TypeMeta::I64 => "i64".into(),
        TypeMeta::U64 => "u64".into(),
        TypeMeta::F32 => "f32".into(),
        TypeMeta::F64 => "f64".into(),
        TypeMeta::Char16 => "char16".into(),
        TypeMeta::String => "string".into(),
        TypeMeta::Guid => "guid".into(),
        TypeMeta::Object => "object".into(),
        TypeMeta::Interface {
            namespace,
            name,
            iid,
        } => format!(
            "interface({namespace}.{},{})",
            metadata_type_name(namespace, name),
            normalized_guid(iid)
        ),
        TypeMeta::RuntimeClass {
            namespace,
            name,
            default_interface,
        } => format!(
            "class({namespace}.{};default={})",
            metadata_type_name(namespace, name),
            default_interface
                .as_deref()
                .map(type_abi_signature)
                .unwrap_or_else(|| "none".into())
        ),
        TypeMeta::Delegate {
            namespace,
            name,
            iid,
        } => format!(
            "delegate({namespace}.{},{})",
            metadata_type_name(namespace, name),
            normalized_guid(iid)
        ),
        TypeMeta::AsyncAction => "async-action".into(),
        TypeMeta::AsyncActionWithProgress(progress) => {
            format!(
                "async-action-with-progress({})",
                type_abi_signature(progress)
            )
        }
        TypeMeta::AsyncOperation(result) => {
            format!("async-operation({})", type_abi_signature(result))
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => format!(
            "async-operation-with-progress({},{})",
            type_abi_signature(result),
            type_abi_signature(progress)
        ),
        TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        } => format!(
            "parameterized({namespace}.{name},{},[{}])",
            normalized_guid(piid),
            args.iter()
                .map(type_abi_signature)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeMeta::Array(element) => format!("array({})", type_abi_signature(element)),
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } => format!(
            "struct({namespace}.{};fields=[{}])",
            metadata_type_name(namespace, name),
            fields
                .iter()
                .map(|field| format!("{}:{}", field.name, type_abi_signature(&field.typ)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeMeta::Enum {
            namespace,
            name,
            underlying,
            ..
        } => format!(
            "enum({namespace}.{};underlying={})",
            metadata_type_name(namespace, name),
            type_abi_signature(underlying)
        ),
    }
}

pub fn parameterized_abi_identity(piid: &str, iid: &str, args: &[TypeMeta]) -> String {
    format!(
        "piid:{}:iid:{}:args:[{}]",
        normalized_guid(piid),
        normalized_guid(iid),
        args.iter()
            .map(type_abi_signature)
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub fn parameterized_reference_identity(piid: &str, args: &[TypeMeta]) -> String {
    format!(
        "piid:{}:args:[{}]",
        normalized_guid(piid),
        args.iter()
            .map(type_abi_signature)
            .collect::<Vec<_>>()
            .join(",")
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
    let base = generic_name.split('`').next().unwrap_or(generic_name);
    let readable = format!(
        "{}_{}",
        base,
        args.iter()
            .map(generic_type_token)
            .collect::<Vec<_>>()
            .join("_")
    );
    let signature = format!(
        "{namespace}.{base}:{}<{}>",
        normalized_guid(piid),
        args.iter()
            .map(generic_identity_signature)
            .collect::<Vec<_>>()
            .join(",")
    );
    let requires_identity_suffix = !matches!(
        namespace,
        "Windows.Foundation" | "Windows.Foundation.Collections"
    ) || args.iter().any(|argument| {
        !matches!(
            argument,
            TypeMeta::Bool
                | TypeMeta::I8
                | TypeMeta::U8
                | TypeMeta::I16
                | TypeMeta::U16
                | TypeMeta::I32
                | TypeMeta::U32
                | TypeMeta::I64
                | TypeMeta::U64
                | TypeMeta::F32
                | TypeMeta::F64
                | TypeMeta::String
                | TypeMeta::Char16
                | TypeMeta::Guid
                | TypeMeta::Object
        )
    });
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

pub fn projected_parameterized_name(
    namespace: &str,
    generic_name: &str,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    let output_name = parameterized_name(namespace, generic_name, piid, args);
    let variant = parameterized_reference_identity(piid, args);
    target_for_identity(&JavaScriptTypeIdentity::with_variant(
        namespace,
        &output_name,
        JavaScriptTypeKind::Interface,
        &variant,
    ))
    .or_else(|| {
        target_for_identity(&JavaScriptTypeIdentity::with_variant(
            namespace,
            &output_name,
            JavaScriptTypeKind::Delegate,
            &variant,
        ))
    })
    .map_or(output_name, |target| target.projected_name)
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

pub fn install_javascript_module_layout(
    identities: impl IntoIterator<Item = JavaScriptTypeIdentity>,
) -> Result<JavaScriptModuleLayoutGuard, String> {
    install_javascript_module_layout_with_records(
        identities,
        std::iter::empty::<JavaScriptTypeLayoutRecord>(),
    )
}

pub fn install_javascript_module_layout_with_records(
    identities: impl IntoIterator<Item = JavaScriptTypeIdentity>,
    previous_records: impl IntoIterator<Item = JavaScriptTypeLayoutRecord>,
) -> Result<JavaScriptModuleLayoutGuard, String> {
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

    let previous = MODULE_LAYOUT.with(|layout| {
        layout.borrow_mut().replace(JavaScriptModuleLayout {
            targets,
            by_projected_name: projected_owners,
        })
    });
    Ok(JavaScriptModuleLayoutGuard { previous })
}

pub fn javascript_module_layout_installed() -> bool {
    MODULE_LAYOUT.with(|layout| layout.borrow().is_some())
}

pub fn javascript_layout_identities() -> Vec<JavaScriptTypeIdentity> {
    MODULE_LAYOUT.with(|layout| {
        layout
            .borrow()
            .as_ref()
            .map(|layout| layout.targets.keys().cloned().collect())
            .unwrap_or_default()
    })
}

pub fn javascript_output_targets() -> Vec<JavaScriptOutputTarget> {
    MODULE_LAYOUT.with(|layout| {
        layout
            .borrow()
            .as_ref()
            .map(|layout| layout.targets.values().cloned().collect())
            .unwrap_or_default()
    })
}

fn target_for_identity(identity: &JavaScriptTypeIdentity) -> Option<JavaScriptOutputTarget> {
    MODULE_LAYOUT.with(|layout| {
        layout
            .borrow()
            .as_ref()
            .and_then(|layout| layout.targets.get(identity).cloned())
    })
}

pub fn javascript_output_target(projected_name: &str) -> Option<JavaScriptOutputTarget> {
    MODULE_LAYOUT.with(|layout| {
        let layout = layout.borrow();
        let layout = layout.as_ref()?;
        let identity = layout.by_projected_name.get(projected_name)?;
        layout.targets.get(identity).cloned()
    })
}

fn projected_name(namespace: &str, name: &str, kind: JavaScriptTypeKind) -> String {
    target_for_identity(&JavaScriptTypeIdentity::new(namespace, name, kind))
        .or_else(|| {
            if kind == JavaScriptTypeKind::Interface {
                target_for_identity(&JavaScriptTypeIdentity::new(
                    namespace,
                    name,
                    JavaScriptTypeKind::Delegate,
                ))
            } else {
                None
            }
        })
        .map_or_else(|| name.into(), |target| target.projected_name)
}

pub(crate) fn metadata_type_name(namespace: &str, output_name: &str) -> String {
    MODULE_LAYOUT.with(|layout| {
        layout
            .borrow()
            .as_ref()
            .and_then(|layout| {
                let identity = layout.by_projected_name.get(output_name)?;
                (identity.namespace == namespace).then(|| identity.name.clone())
            })
            .unwrap_or_else(|| output_name.into())
    })
}

fn apply_projected_type_names(typ: &mut TypeMeta) {
    match typ {
        TypeMeta::Interface {
            namespace, name, ..
        } => *name = projected_name(namespace, name, JavaScriptTypeKind::Interface),
        TypeMeta::Delegate {
            namespace, name, ..
        } => *name = projected_name(namespace, name, JavaScriptTypeKind::Delegate),
        TypeMeta::RuntimeClass {
            namespace,
            name,
            default_interface,
        } => {
            *name = projected_name(namespace, name, JavaScriptTypeKind::Class);
            if let Some(default_interface) = default_interface {
                apply_projected_type_names(default_interface);
            }
        }
        TypeMeta::AsyncActionWithProgress(inner)
        | TypeMeta::AsyncOperation(inner)
        | TypeMeta::Array(inner) => apply_projected_type_names(inner),
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            apply_projected_type_names(result);
            apply_projected_type_names(progress);
        }
        TypeMeta::Parameterized { args, .. } => {
            for argument in args {
                apply_projected_type_names(argument);
            }
        }
        TypeMeta::Struct { fields, .. } => {
            for field in fields {
                apply_projected_type_names(&mut field.typ);
            }
        }
        TypeMeta::Enum {
            namespace,
            name,
            underlying,
            ..
        } => {
            *name = projected_name(namespace, name, JavaScriptTypeKind::Enum);
            apply_projected_type_names(underlying);
        }
        _ => {}
    }
}

fn apply_projected_method_names(method: &mut MethodMeta) {
    for parameter in &mut method.params {
        apply_projected_type_names(&mut parameter.typ);
    }
    if let Some(return_type) = &mut method.return_type {
        apply_projected_type_names(return_type);
    }
}

fn apply_projected_interface_names(interface: &mut InterfaceMeta) {
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
        let output_name = parameterized_interface_name(
            &interface.namespace,
            &interface.name,
            piid,
            &interface.generic_args,
        );
        let variant = parameterized_reference_identity(piid, &interface.generic_args);
        target_for_identity(&JavaScriptTypeIdentity::with_variant(
            &interface.namespace,
            &output_name,
            kind,
            &variant,
        ))
        .map_or(output_name, |target| target.projected_name)
    } else {
        projected_name(&interface.namespace, &interface.name, kind)
    };
    for base in &mut interface.base_interfaces {
        apply_projected_type_names(base);
    }
    for argument in &mut interface.generic_args {
        apply_projected_type_names(argument);
    }
    for method in &mut interface.methods {
        apply_projected_method_names(method);
    }
}

fn apply_projected_type_ref_name(reference: &mut TypeRef) {
    reference.name = projected_name(&reference.namespace, &reference.name, reference.kind.into());
}

fn apply_projected_class_names(class: &mut ClassMeta) {
    class.name = projected_name(&class.namespace, &class.name, JavaScriptTypeKind::Class);
    if let Some(base_class) = &mut class.base_class {
        apply_projected_type_ref_name(base_class);
    }
    if let Some(default_interface) = &mut class.default_interface {
        apply_projected_interface_names(default_interface);
    }
    for interface in class
        .required_interfaces
        .iter_mut()
        .chain(class.overridable_interfaces.iter_mut())
        .chain(class.factory_interfaces.iter_mut())
        .chain(class.static_interfaces.iter_mut())
    {
        apply_projected_interface_names(interface);
    }
    for constructor in &mut class.constructors {
        if let Some(factory_interface) = &mut constructor.factory_interface {
            apply_projected_type_ref_name(factory_interface);
        }
    }
}

pub fn apply_javascript_projected_names(
    classes: &mut [ClassMeta],
    interfaces: &mut [InterfaceMeta],
    enums: &mut [TypeMeta],
) {
    for class in classes {
        apply_projected_class_names(class);
    }
    for interface in interfaces {
        apply_projected_interface_names(interface);
    }
    for en in enums {
        apply_projected_type_names(en);
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

pub fn configure_projected_file(file: &mut ProjectedFile) -> Option<JavaScriptOutputTarget> {
    let target = javascript_output_target(&file.name)?;
    for import in &mut file.imports {
        if import.is_runtime_package {
            if (import.from.starts_with("./") || import.from.starts_with("../"))
                && let Some(module) = import
                    .from
                    .strip_suffix(".js")
                    .or(Some(import.from.as_str()))
            {
                import.from = format!("{}.js", relative_module(&target.canonical_module, module));
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
        let target_module = javascript_output_target(module)
            .map_or_else(|| module.to_string(), |target| target.canonical_module);
        import.from = format!(
            "{}.js",
            relative_module(&target.canonical_module, &target_module)
        );
    }
    Some(target)
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
        let _layout = install_javascript_module_layout(identities).unwrap();
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

        apply_javascript_projected_names(&mut classes, &mut [], &mut enums);

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
            signature::ts_dynwinrt_type(parameter)
                .contains("Microsoft.UI.Input.DragDrop.DragUIOverride")
        );
        assert!(matches!(
            enums[0],
            TypeMeta::Enum { ref name, .. } if name == "XamlControlsPrimitivesAnimationDirection"
        ));
        assert_eq!(
            metadata_type_name("Microsoft.UI.Input.DragDrop", "InputDragDropDragUIOverride"),
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
        let _layout = install_javascript_module_layout([identity.clone()]).unwrap();
        let target = target_for_identity(&identity).unwrap();

        assert_eq!(target.projected_name, "Button");
        assert_eq!(target.canonical_module, "microsoft/ui/xaml/controls/Button");
        assert!(!target.collides);
    }

    #[test]
    fn nested_layout_guard_restores_previous_layout() {
        let button = JavaScriptTypeIdentity::new(
            "Microsoft.UI.Xaml.Controls",
            "Button",
            JavaScriptTypeKind::Class,
        );
        let uri =
            JavaScriptTypeIdentity::new("Windows.Foundation", "Uri", JavaScriptTypeKind::Class);
        let _outer = install_javascript_module_layout([button.clone()]).unwrap();
        assert!(target_for_identity(&button).is_some());
        {
            let _inner = install_javascript_module_layout([uri.clone()]).unwrap();
            assert!(target_for_identity(&button).is_none());
            assert!(target_for_identity(&uri).is_some());
        }
        assert!(target_for_identity(&button).is_some());
        assert!(target_for_identity(&uri).is_none());
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
        let old_layout =
            install_javascript_module_layout([windows.clone(), contoso.clone()]).unwrap();
        let records = javascript_output_targets()
            .into_iter()
            .map(|target| {
                JavaScriptTypeLayoutRecord::new(target.identity, target.projected_name, "type")
            })
            .collect::<Vec<_>>();
        drop(old_layout);

        let microsoft = JavaScriptTypeIdentity::new(
            "Microsoft.UI.Foundation",
            "Widget",
            JavaScriptTypeKind::Class,
        );
        let identities = [windows.clone(), contoso.clone(), microsoft.clone()];
        let incremental_layout =
            install_javascript_module_layout_with_records(identities.clone(), records).unwrap();
        assert!(
            target_for_identity(&windows)
                .unwrap()
                .compatibility_aliases
                .contains("FoundationWidget")
        );
        let incremental = identities
            .iter()
            .map(|identity| {
                (
                    identity.clone(),
                    target_for_identity(identity).unwrap().projected_name,
                )
            })
            .collect::<BTreeMap<_, _>>();
        drop(incremental_layout);

        let _clean_layout = install_javascript_module_layout(identities.clone()).unwrap();
        let clean = identities
            .iter()
            .map(|identity| {
                (
                    identity.clone(),
                    target_for_identity(identity).unwrap().projected_name,
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

        let error = install_javascript_module_layout([identity])
            .err()
            .expect("path-like metadata names must be rejected");

        assert!(error.contains("Unsafe JavaScript type identity"), "{error}");
    }
}
