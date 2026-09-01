// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python naming and identifier helpers.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::meta::InterfaceMeta;
use crate::types::{TypeIdentity, TypeIdentityKind, TypeKind, TypeMeta, TypeRef};

pub type PythonTypeIdentity = TypeIdentity;

#[derive(Clone, Debug)]
struct PythonProjection {
    implementation_module: String,
    projected_name: String,
    public_module: String,
    reference_name: String,
}

const MAX_PYTHON_MODULE_COMPONENT_LENGTH: usize = 120;
const MODULE_HASH_HEX_LENGTH: usize = 16;

fn stable_module_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn shorten_module_component_with_hash_input(value: &str, hash_input: &str) -> String {
    if value.chars().count() <= MAX_PYTHON_MODULE_COMPONENT_LENGTH {
        return value.to_string();
    }
    let prefix_length = MAX_PYTHON_MODULE_COMPONENT_LENGTH - MODULE_HASH_HEX_LENGTH - 1;
    let prefix = value
        .chars()
        .take(prefix_length)
        .collect::<String>()
        .trim_end_matches('_')
        .to_string();
    format!("{prefix}_{:016x}", stable_module_hash(hash_input))
}

fn shorten_module_component(value: &str) -> String {
    shorten_module_component_with_hash_input(value, value)
}

fn identity_kind_name(kind: TypeIdentityKind) -> &'static str {
    match kind {
        TypeIdentityKind::Class => "class",
        TypeIdentityKind::Delegate => "delegate",
        TypeIdentityKind::Enum => "enum",
        TypeIdentityKind::Interface => "interface",
        TypeIdentityKind::Struct => "struct",
    }
}

fn legacy_projected_name(identity: &PythonTypeIdentity) -> String {
    match identity {
        TypeIdentity::Primitive { name } | TypeIdentity::Named { name, .. } => name.clone(),
        TypeIdentity::ClosedGeneric {
            name, arguments, ..
        } => format!(
            "{}_{}",
            name.split('`').next().unwrap_or(name),
            arguments
                .iter()
                .map(legacy_projected_name)
                .collect::<Vec<_>>()
                .join("_")
        ),
        TypeIdentity::Array { element } => {
            format!("Array_{}", legacy_projected_name(element))
        }
        TypeIdentity::AsyncAction => "IAsyncAction".to_string(),
        TypeIdentity::AsyncActionWithProgress { progress } => {
            format!(
                "IAsyncActionWithProgress_{}",
                legacy_projected_name(progress)
            )
        }
        TypeIdentity::AsyncOperation { result } => {
            format!("IAsyncOperation_{}", legacy_projected_name(result))
        }
        TypeIdentity::AsyncOperationWithProgress { result, progress } => format!(
            "IAsyncOperationWithProgress_{}_{}",
            legacy_projected_name(result),
            legacy_projected_name(progress)
        ),
    }
}

fn sanitize_symbol_component(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut previous_underscore = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character);
            previous_underscore = false;
        } else if !previous_underscore {
            result.push('_');
            previous_underscore = true;
        }
    }
    let result = result.trim_matches('_');
    if result.is_empty() {
        return "Type".to_string();
    }
    if result.as_bytes()[0].is_ascii_digit() {
        format!("_{result}")
    } else {
        result.to_string()
    }
}

fn semantic_qualifier(identity: &PythonTypeIdentity) -> String {
    match identity {
        TypeIdentity::Primitive { name } => sanitize_symbol_component(name),
        TypeIdentity::Named {
            kind,
            namespace,
            name,
        } => sanitize_symbol_component(&format!(
            "{}_{}_{}",
            namespace,
            name,
            identity_kind_name(*kind)
        )),
        TypeIdentity::ClosedGeneric {
            kind,
            namespace,
            name,
            arguments,
        } => sanitize_symbol_component(&format!(
            "{}_{}_{}_{}",
            namespace,
            name,
            identity_kind_name(*kind),
            arguments
                .iter()
                .map(semantic_qualifier)
                .collect::<Vec<_>>()
                .join("_")
        )),
        TypeIdentity::Array { element } => {
            format!("Array_{}", semantic_qualifier(element))
        }
        TypeIdentity::AsyncAction => "IAsyncAction".to_string(),
        TypeIdentity::AsyncActionWithProgress { progress } => {
            format!("IAsyncActionWithProgress_{}", semantic_qualifier(progress))
        }
        TypeIdentity::AsyncOperation { result } => {
            format!("IAsyncOperation_{}", semantic_qualifier(result))
        }
        TypeIdentity::AsyncOperationWithProgress { result, progress } => format!(
            "IAsyncOperationWithProgress_{}_{}",
            semantic_qualifier(result),
            semantic_qualifier(progress)
        ),
    }
}

pub fn python_identity_display_name(identity: &PythonTypeIdentity) -> String {
    match identity {
        TypeIdentity::Primitive { name } => name.clone(),
        TypeIdentity::Named {
            kind,
            namespace,
            name,
        } => format!("{namespace}.{name} ({})", identity_kind_name(*kind)),
        TypeIdentity::ClosedGeneric {
            kind,
            namespace,
            name,
            arguments,
        } => format!(
            "{}.{}<{}> ({})",
            namespace,
            name,
            arguments
                .iter()
                .map(python_identity_display_name)
                .collect::<Vec<_>>()
                .join(", "),
            identity_kind_name(*kind)
        ),
        TypeIdentity::Array { element } => {
            format!("{}[]", python_identity_display_name(element))
        }
        TypeIdentity::AsyncAction => "Windows.Foundation.IAsyncAction".to_string(),
        TypeIdentity::AsyncActionWithProgress { progress } => format!(
            "Windows.Foundation.IAsyncActionWithProgress<{}>",
            python_identity_display_name(progress)
        ),
        TypeIdentity::AsyncOperation { result } => format!(
            "Windows.Foundation.IAsyncOperation<{}>",
            python_identity_display_name(result)
        ),
        TypeIdentity::AsyncOperationWithProgress { result, progress } => format!(
            "Windows.Foundation.IAsyncOperationWithProgress<{}, {}>",
            python_identity_display_name(result),
            python_identity_display_name(progress)
        ),
    }
}

fn collect_named_identity_counts(
    identity: &PythonTypeIdentity,
    identities: &mut HashMap<String, HashSet<PythonTypeIdentity>>,
) {
    match identity {
        TypeIdentity::Named { name, .. } => {
            identities
                .entry(name.clone())
                .or_default()
                .insert(identity.clone());
        }
        TypeIdentity::ClosedGeneric { arguments, .. } => {
            for argument in arguments {
                collect_named_identity_counts(argument, identities);
            }
        }
        TypeIdentity::Array { element } => collect_named_identity_counts(element, identities),
        TypeIdentity::AsyncActionWithProgress { progress } => {
            collect_named_identity_counts(progress, identities)
        }
        TypeIdentity::AsyncOperation { result } => {
            collect_named_identity_counts(result, identities)
        }
        TypeIdentity::AsyncOperationWithProgress { result, progress } => {
            collect_named_identity_counts(result, identities);
            collect_named_identity_counts(progress, identities);
        }
        TypeIdentity::Primitive { .. } | TypeIdentity::AsyncAction => {}
    }
}

fn qualified_argument_name(
    identity: &PythonTypeIdentity,
    ambiguous_named_types: &HashSet<String>,
) -> String {
    match identity {
        TypeIdentity::Named {
            kind,
            namespace,
            name,
        } if ambiguous_named_types.contains(name) => sanitize_symbol_component(&format!(
            "{}_{}_{}",
            namespace,
            name,
            identity_kind_name(*kind)
        )),
        TypeIdentity::ClosedGeneric {
            name, arguments, ..
        } => format!(
            "{}_{}",
            name,
            arguments
                .iter()
                .map(|argument| qualified_argument_name(argument, ambiguous_named_types))
                .collect::<Vec<_>>()
                .join("_")
        ),
        TypeIdentity::Array { element } => format!(
            "Array_{}",
            qualified_argument_name(element, ambiguous_named_types)
        ),
        _ => legacy_projected_name(identity),
    }
}

fn projected_names(
    identities: &BTreeSet<PythonTypeIdentity>,
    externally_ambiguous_named_types: &HashSet<String>,
) -> HashMap<PythonTypeIdentity, String> {
    let mut named_identity_counts = HashMap::<String, HashSet<PythonTypeIdentity>>::new();
    for identity in identities {
        collect_named_identity_counts(identity, &mut named_identity_counts);
    }
    let mut ambiguous_named_types = named_identity_counts
        .into_iter()
        .filter(|(_, identities)| identities.len() > 1)
        .map(|(name, _)| name)
        .collect::<HashSet<_>>();
    ambiguous_named_types.extend(externally_ambiguous_named_types.iter().cloned());

    let mut groups = BTreeMap::<String, Vec<&PythonTypeIdentity>>::new();
    for identity in identities {
        groups
            .entry(legacy_projected_name(identity))
            .or_default()
            .push(identity);
    }

    let mut result = HashMap::new();
    for (legacy_name, group) in groups {
        let intrinsic_names = group
            .iter()
            .map(|identity| {
                (
                    *identity,
                    if matches!(identity, TypeIdentity::ClosedGeneric { .. }) {
                        qualified_argument_name(identity, &ambiguous_named_types)
                    } else {
                        legacy_projected_name(identity)
                    },
                )
            })
            .collect::<Vec<_>>();
        let intrinsic_names_are_unique = intrinsic_names
            .iter()
            .map(|(_, name)| to_snake_case(name))
            .collect::<HashSet<_>>()
            .len()
            == intrinsic_names.len();
        if intrinsic_names_are_unique
            && intrinsic_names.iter().any(|(_, name)| name != &legacy_name)
        {
            for (identity, name) in intrinsic_names {
                result.insert(identity.clone(), name);
            }
            continue;
        }

        let namespaces = group
            .iter()
            .filter_map(|identity| identity.namespace())
            .collect::<HashSet<_>>();
        let safely_separated_by_namespace = namespaces.len() == group.len()
            && group.iter().all(|identity| identity.namespace().is_some());
        if group.len() == 1 || safely_separated_by_namespace {
            for identity in group {
                result.insert(identity.clone(), legacy_name.clone());
            }
            continue;
        }

        let mut candidate_owners = HashMap::<String, &PythonTypeIdentity>::new();
        for identity in group {
            let mut candidate = format!("{}_{}", legacy_name, semantic_qualifier(identity));
            let normalized = to_snake_case(&candidate);
            if let Some(existing) = candidate_owners.get(&normalized)
                && *existing != identity
            {
                candidate.push_str(&format!(
                    "_{:016x}",
                    stable_module_hash(&identity.canonical_key())
                ));
            }
            candidate_owners.insert(to_snake_case(&candidate), identity);
            result.insert(identity.clone(), candidate);
        }
    }
    result
}

fn qualified_module_name(
    identity: &PythonTypeIdentity,
    namespace: &str,
    projected_name: &str,
) -> String {
    let namespace = python_namespace_segments(namespace).join("__");
    let candidate = if namespace.is_empty() {
        to_snake_case(projected_name)
    } else {
        format!("{namespace}__{}", to_snake_case(projected_name))
    };
    shorten_module_component_with_hash_input(&candidate, &identity.canonical_key())
}

fn public_module_name(identity: &PythonTypeIdentity, projected_name: &str) -> String {
    shorten_module_component_with_hash_input(
        &to_snake_case(projected_name),
        &identity.canonical_key(),
    )
}

/// Explicit, immutable naming and lookup state for one Python projection.
#[derive(Clone, Debug, Default)]
pub struct PythonProjectionContext {
    packaged: bool,
    projections: HashMap<PythonTypeIdentity, PythonProjection>,
    aliases: HashMap<PythonTypeIdentity, PythonTypeIdentity>,
    compatibility_counts: HashMap<String, usize>,
}

impl PythonProjectionContext {
    pub fn new(
        identities: impl IntoIterator<Item = PythonTypeIdentity>,
        packaged: bool,
    ) -> Result<Self, String> {
        Self::new_with_ambiguities(identities, packaged, std::iter::empty())
    }

    pub fn new_with_ambiguities(
        identities: impl IntoIterator<Item = PythonTypeIdentity>,
        packaged: bool,
        ambiguous_named_types: impl IntoIterator<Item = String>,
    ) -> Result<Self, String> {
        let identities = identities.into_iter().collect::<BTreeSet<_>>();
        let ambiguous_named_types = ambiguous_named_types.into_iter().collect::<HashSet<_>>();
        let projected_names = projected_names(&identities, &ambiguous_named_types);
        let mut projected_name_counts = HashMap::<String, usize>::new();
        for name in projected_names.values() {
            *projected_name_counts.entry(name.clone()).or_default() += 1;
        }
        let mut projections = HashMap::new();
        let mut aliases = HashMap::new();
        let mut compatibility_counts = HashMap::new();
        let mut module_owners = HashMap::<String, PythonTypeIdentity>::new();
        let mut public_module_owners = HashMap::<String, PythonTypeIdentity>::new();

        for identity in &identities {
            *compatibility_counts
                .entry(legacy_projected_name(identity))
                .or_default() += 1;
            if identity.kind() == Some(TypeIdentityKind::Delegate) {
                aliases.insert(
                    identity.with_kind(TypeIdentityKind::Interface),
                    identity.clone(),
                );
            }
        }

        for identity in identities {
            let namespace = identity.namespace().ok_or_else(|| {
                format!(
                    "Python generated type identity must be named: {}",
                    python_identity_display_name(&identity)
                )
            })?;
            let projected_name = projected_names[&identity].clone();
            let public_module = public_module_name(&identity, &projected_name);
            let compatibility_name = legacy_projected_name(&identity);
            let reference_name = if projected_name_counts
                .get(&projected_name)
                .copied()
                .unwrap_or_default()
                > 1
                || (ambiguous_named_types.contains(&compatibility_name)
                    && matches!(identity, TypeIdentity::Named { .. }))
            {
                semantic_qualifier(&identity)
            } else {
                projected_name.clone()
            };
            let implementation_module = if packaged
                || matches!(
                    identity,
                    TypeIdentity::Named {
                        kind: TypeIdentityKind::Class
                            | TypeIdentityKind::Enum
                            | TypeIdentityKind::Interface
                            | TypeIdentityKind::Struct,
                        ..
                    }
                ) {
                qualified_module_name(&identity, namespace, &projected_name)
            } else {
                public_module.clone()
            };

            if let Some(existing) =
                module_owners.insert(implementation_module.clone(), identity.clone())
                && existing != identity
            {
                return Err(format!(
                    "Python implementation module collision: `{}` and `{}` both normalize to \
                     `{implementation_module}.py`",
                    python_identity_display_name(&existing),
                    python_identity_display_name(&identity)
                ));
            }
            let public_key = format!(
                "{}/{}",
                python_namespace_segments(namespace).join("/"),
                public_module
            );
            if let Some(existing) =
                public_module_owners.insert(public_key.clone(), identity.clone())
                && existing != identity
            {
                return Err(format!(
                    "Python public module collision: `{}` and `{}` both normalize to \
                     `{public_key}.py`",
                    python_identity_display_name(&existing),
                    python_identity_display_name(&identity)
                ));
            }

            projections.insert(
                identity,
                PythonProjection {
                    implementation_module,
                    projected_name,
                    public_module,
                    reference_name,
                },
            );
        }

        Ok(Self {
            packaged,
            projections,
            aliases,
            compatibility_counts,
        })
    }

    pub fn packaged(
        identities: impl IntoIterator<Item = PythonTypeIdentity>,
    ) -> Result<Self, String> {
        Self::new(identities, true)
    }

    pub fn packaged_with_ambiguities(
        identities: impl IntoIterator<Item = PythonTypeIdentity>,
        ambiguous_named_types: impl IntoIterator<Item = String>,
    ) -> Result<Self, String> {
        Self::new_with_ambiguities(identities, true, ambiguous_named_types)
    }

    pub fn standalone(
        identities: impl IntoIterator<Item = PythonTypeIdentity>,
    ) -> Result<Self, String> {
        Self::new(identities, false)
    }

    pub fn is_packaged(&self) -> bool {
        self.packaged
    }

    pub fn normalize_identity(&self, identity: &PythonTypeIdentity) -> PythonTypeIdentity {
        let identity = match identity {
            TypeIdentity::ClosedGeneric {
                kind,
                namespace,
                name,
                arguments,
            } => TypeIdentity::closed_generic(
                *kind,
                namespace.clone(),
                name.clone(),
                arguments
                    .iter()
                    .map(|argument| self.normalize_identity(argument)),
            ),
            TypeIdentity::Array { element } => TypeIdentity::Array {
                element: Box::new(self.normalize_identity(element)),
            },
            TypeIdentity::AsyncActionWithProgress { progress } => {
                TypeIdentity::AsyncActionWithProgress {
                    progress: Box::new(self.normalize_identity(progress)),
                }
            }
            TypeIdentity::AsyncOperation { result } => TypeIdentity::AsyncOperation {
                result: Box::new(self.normalize_identity(result)),
            },
            TypeIdentity::AsyncOperationWithProgress { result, progress } => {
                TypeIdentity::AsyncOperationWithProgress {
                    result: Box::new(self.normalize_identity(result)),
                    progress: Box::new(self.normalize_identity(progress)),
                }
            }
            _ => identity.clone(),
        };
        self.aliases.get(&identity).cloned().unwrap_or(identity)
    }

    pub fn identity_for_type(&self, typ: &TypeMeta) -> PythonTypeIdentity {
        self.normalize_identity(&typ.type_identity())
    }

    pub fn contains_identity(&self, identity: &PythonTypeIdentity) -> bool {
        self.projections
            .contains_key(&self.normalize_identity(identity))
    }

    pub fn is_known_type(&self, typ: &TypeMeta) -> bool {
        self.contains_identity(&typ.type_identity())
    }

    pub fn known_full_names(&self) -> HashSet<String> {
        self.projections
            .keys()
            .filter_map(|identity| {
                Some(format!(
                    "{}.{}",
                    identity.namespace()?,
                    identity.definition_name()?
                ))
            })
            .collect()
    }

    pub fn is_known_ref(&self, reference: &TypeRef) -> bool {
        let kind = match reference.kind {
            TypeKind::Class => TypeIdentityKind::Class,
            TypeKind::Enum => TypeIdentityKind::Enum,
            TypeKind::Interface => TypeIdentityKind::Interface,
        };
        self.contains_identity(&TypeIdentity::named(
            kind,
            reference.namespace.clone(),
            reference.name.clone(),
        ))
    }

    pub fn is_delegate_type(&self, typ: &TypeMeta) -> bool {
        self.identity_for_type(typ).kind() == Some(TypeIdentityKind::Delegate)
    }

    pub fn projected_name(&self, identity: &PythonTypeIdentity) -> String {
        let identity = self.normalize_identity(identity);
        self.projections
            .get(&identity)
            .map(|projection| projection.projected_name.clone())
            .unwrap_or_else(|| legacy_projected_name(&identity))
    }

    pub fn compatibility_name(&self, identity: &PythonTypeIdentity) -> String {
        legacy_projected_name(&self.normalize_identity(identity))
    }

    pub fn projected_name_for_type(&self, typ: &TypeMeta) -> String {
        self.projected_name(&self.identity_for_type(typ))
    }

    pub fn projected_name_for_interface(&self, interface: &InterfaceMeta) -> String {
        self.projected_name(&interface.type_identity())
    }

    pub fn reference_name(&self, identity: &PythonTypeIdentity) -> String {
        let identity = self.normalize_identity(identity);
        self.projections
            .get(&identity)
            .map(|projection| projection.reference_name.clone())
            .unwrap_or_else(|| self.projected_name(&identity))
    }

    pub fn reference_name_for_type(&self, typ: &TypeMeta) -> String {
        self.reference_name(&self.identity_for_type(typ))
    }

    pub fn implementation_module(&self, identity: &PythonTypeIdentity) -> String {
        let identity = self.normalize_identity(identity);
        self.projections
            .get(&identity)
            .map(|projection| projection.implementation_module.clone())
            .unwrap_or_else(|| {
                let projected_name = legacy_projected_name(&identity);
                if self.packaged
                    || matches!(
                        identity,
                        TypeIdentity::Named {
                            kind: TypeIdentityKind::Class
                                | TypeIdentityKind::Enum
                                | TypeIdentityKind::Interface
                                | TypeIdentityKind::Struct,
                            ..
                        }
                    )
                {
                    qualified_module_name(
                        &identity,
                        identity.namespace().unwrap_or_default(),
                        &projected_name,
                    )
                } else {
                    public_module_name(&identity, &projected_name)
                }
            })
    }

    pub fn implementation_module_for_type(&self, typ: &TypeMeta) -> String {
        self.implementation_module(&self.identity_for_type(typ))
    }

    pub fn implementation_module_for_named(
        &self,
        kind: TypeIdentityKind,
        namespace: &str,
        name: &str,
    ) -> String {
        self.implementation_module(&TypeIdentity::named(kind, namespace, name))
    }

    pub fn implementation_module_for_interface(&self, interface: &InterfaceMeta) -> String {
        self.implementation_module(&interface.type_identity())
    }

    pub fn public_qualified_module(&self, identity: &PythonTypeIdentity) -> String {
        let identity = self.normalize_identity(identity);
        let mut segments = python_namespace_segments(identity.namespace().unwrap_or_default());
        let module = self
            .projections
            .get(&identity)
            .map(|projection| projection.public_module.clone())
            .unwrap_or_else(|| public_module_name(&identity, &legacy_projected_name(&identity)));
        segments.push(module);
        segments.join(".")
    }

    pub fn public_module(&self, identity: &PythonTypeIdentity) -> String {
        self.public_qualified_module(identity)
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_string()
    }

    pub fn root_name_is_unambiguous(&self, identity: &PythonTypeIdentity) -> bool {
        self.compatibility_counts
            .get(&legacy_projected_name(identity))
            .copied()
            .unwrap_or_default()
            == 1
    }
}

pub fn python_namespace_segments(namespace: &str) -> Vec<String> {
    namespace
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(|segment| shorten_module_component(&to_snake_case(segment)))
        .collect()
}

pub fn python_public_module_name(name: &str) -> String {
    shorten_module_component(&to_snake_case(name))
}

pub fn python_public_qualified_module_name(namespace: &str, name: &str) -> String {
    let mut segments = python_namespace_segments(namespace);
    segments.push(python_public_module_name(name));
    segments.join(".")
}

fn is_winrt_uint_suffix(token: &str) -> bool {
    matches!(token, "int8" | "int16" | "int32" | "int64")
}

fn collapse_winrt_uint_tokens(name: &str) -> String {
    let tokens: Vec<_> = name.split('_').collect();
    let mut normalized = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index] == "u"
            && index + 1 < tokens.len()
            && is_winrt_uint_suffix(tokens[index + 1])
        {
            normalized.push(format!("u{}", tokens[index + 1]));
            index += 2;
        } else {
            normalized.push(tokens[index].to_string());
            index += 1;
        }
    }
    normalized.join("_")
}

/// Convert PascalCase / camelCase to snake_case.
pub(crate) fn to_snake_case(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                let prev_lower_or_digit =
                    chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit();
                let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
                if prev_lower_or_digit || (next_lower && chars[i - 1].is_uppercase()) {
                    result.push('_');
                }
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    let result = collapse_winrt_uint_tokens(result.trim_start_matches('_'));
    if is_py_reserved(&result) {
        format!("{}_", result)
    } else {
        result
    }
}

pub(crate) fn is_py_reserved(s: &str) -> bool {
    matches!(
        s,
        "False"
            | "True"
            | "None"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    )
}

/// Convert a PascalCase name to a snake_case Python filename (without extension).
pub fn to_snake_case_filename(name: &str) -> String {
    shorten_module_component(&to_snake_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_keeps_winrt_uint_tokens_together() {
        assert_eq!(to_snake_case("UInt8"), "uint8");
        assert_eq!(to_snake_case("UInt16"), "uint16");
        assert_eq!(to_snake_case("UInt32"), "uint32");
        assert_eq!(to_snake_case("UInt64"), "uint64");
        assert_eq!(to_snake_case("CreateUInt8"), "create_uint8");
        assert_eq!(to_snake_case("CreateUInt32Value"), "create_uint32_value");
        assert_eq!(to_snake_case("IReference_UInt32"), "i_reference_uint32");
        assert_eq!(
            to_snake_case_filename("IReference_UInt32"),
            "i_reference_uint32"
        );
    }

    #[test]
    fn public_qualified_module_uses_namespace_facades() {
        assert_eq!(
            python_public_qualified_module_name("Microsoft.UI.Xaml.Controls", "Button"),
            "microsoft.ui.xaml.controls.button"
        );
    }

    #[test]
    fn long_module_names_use_stable_hash_suffixes() {
        let name = "TypedEventHandler_MediaPlaybackCommandManager_MediaPlaybackCommandManagerAutoRepeatModeReceivedEventArgsAdditionalCompatibilitySuffix";
        let other = format!("{name}2");
        let shortened = python_public_module_name(name);
        assert_eq!(
            shortened.chars().count(),
            MAX_PYTHON_MODULE_COMPONENT_LENGTH
        );
        assert_eq!(shortened, python_public_module_name(name));
        assert_ne!(shortened, python_public_module_name(&other));
        assert!(shortened.starts_with("typed_event_handler_media_playback_command_manager"));
    }

    #[test]
    fn projection_context_shortens_implementation_and_public_modules_consistently() {
        let name = "TypedEventHandler_MediaPlaybackCommandManager_MediaPlaybackCommandManagerAutoRepeatModeReceivedEventArgsAdditionalCompatibilitySuffix";
        let identity = TypeIdentity::named(TypeIdentityKind::Delegate, "Windows.Foundation", name);
        let context = PythonProjectionContext::packaged([identity.clone()]).unwrap();
        let implementation = context.implementation_module(&identity);
        assert!(implementation.chars().count() <= MAX_PYTHON_MODULE_COMPONENT_LENGTH);
        assert_eq!(implementation, context.implementation_module(&identity));
        assert!(
            context.public_module(&identity).chars().count() <= MAX_PYTHON_MODULE_COMPONENT_LENGTH
        );
    }

    #[test]
    fn snake_case_only_collapses_uint_word_boundaries() {
        assert_eq!(to_snake_case("MenuInt8"), "menu_int8");
        assert_eq!(to_snake_case("GpuInt32"), "gpu_int32");
        assert_eq!(to_snake_case("MenuUInt8"), "menu_uint8");
    }

    #[test]
    fn snake_case_preserves_acronym_regressions() {
        assert_eq!(to_snake_case("GUID"), "guid");
        assert_eq!(to_snake_case("IIDComponent"), "iid_component");
        assert_eq!(to_snake_case("HTMLParser"), "html_parser");
    }

    #[test]
    fn projection_context_collision_detection_uses_normalized_names() {
        let err = PythonProjectionContext::packaged([
            TypeIdentity::named(TypeIdentityKind::Interface, "Example", "UInt32"),
            TypeIdentity::named(TypeIdentityKind::Interface, "Example", "Uint32"),
        ])
        .err()
        .expect("normalized module name collision should fail");

        assert!(err.contains("Example.UInt32"), "{err}");
        assert!(err.contains("Example.Uint32"), "{err}");
        assert!(err.contains("example__uint32.py"), "{err}");
    }

    #[test]
    fn missing_context_identity_keeps_namespace_qualification() {
        let context = PythonProjectionContext::packaged([TypeIdentity::named(
            TypeIdentityKind::Class,
            "Microsoft.UI.Dispatching",
            "Other",
        )])
        .unwrap();
        assert_eq!(
            context.implementation_module(&TypeIdentity::named(
                TypeIdentityKind::Class,
                "Windows.System",
                "DispatcherQueue",
            )),
            "windows__system__dispatcher_queue"
        );
    }

    #[test]
    fn same_short_named_types_keep_namespace_facade_names_and_distinct_modules() {
        let left = TypeIdentity::named(TypeIdentityKind::Interface, "Example.Left", "IValue");
        let right = TypeIdentity::named(TypeIdentityKind::Interface, "Example.Right", "IValue");
        let context = PythonProjectionContext::packaged([left.clone(), right.clone()]).unwrap();

        assert_eq!(context.projected_name(&left), "IValue");
        assert_eq!(context.projected_name(&right), "IValue");
        assert_ne!(
            context.implementation_module(&left),
            context.implementation_module(&right)
        );
        assert!(!context.root_name_is_unambiguous(&left));
        assert!(!context.root_name_is_unambiguous(&right));
    }

    #[test]
    fn same_short_closed_generics_use_distinct_reference_aliases() {
        let closed = |namespace: &str| {
            TypeIdentity::closed_generic(
                TypeIdentityKind::Interface,
                namespace,
                "IBox",
                [TypeIdentity::Primitive {
                    name: "String".into(),
                }],
            )
        };
        let left = closed("Example.Left");
        let right = closed("Example.Right");
        let context = PythonProjectionContext::packaged([left.clone(), right.clone()]).unwrap();

        assert_eq!(context.projected_name(&left), "IBox_String");
        assert_eq!(context.projected_name(&right), "IBox_String");
        assert_ne!(
            context.reference_name(&left),
            context.reference_name(&right)
        );
    }

    #[test]
    fn closed_generic_names_preserve_nested_semantic_identity() {
        let point = |namespace| TypeIdentity::named(TypeIdentityKind::Struct, namespace, "Point");
        let vector = |argument| {
            TypeIdentity::closed_generic(
                TypeIdentityKind::Interface,
                "Windows.Foundation.Collections",
                "IVector",
                [argument],
            )
        };
        let left = vector(point("Example.Left"));
        let right = vector(point("Example.Right"));
        let nested_left = vector(left.clone());
        let nested_right = vector(right.clone());
        let context = PythonProjectionContext::packaged([
            left.clone(),
            right.clone(),
            nested_left.clone(),
            nested_right.clone(),
        ])
        .unwrap();

        assert_ne!(
            context.projected_name(&left),
            context.projected_name(&right)
        );
        assert_ne!(
            context.projected_name(&nested_left),
            context.projected_name(&nested_right)
        );
        assert_ne!(
            context.implementation_module(&nested_left),
            context.implementation_module(&nested_right)
        );
    }

    #[test]
    fn metadata_ambiguity_keeps_phased_generic_names_stable() {
        let closed = |namespace: &str| {
            TypeIdentity::closed_generic(
                TypeIdentityKind::Interface,
                "Example.Collections",
                "IBox",
                [TypeIdentity::named(
                    TypeIdentityKind::Struct,
                    namespace,
                    "Point",
                )],
            )
        };
        let left = closed("Example.Left");
        let right = closed("Example.Right");
        let left_phase = PythonProjectionContext::packaged_with_ambiguities(
            [left.clone()],
            ["Point".to_string()],
        )
        .unwrap();
        let right_phase = PythonProjectionContext::packaged_with_ambiguities(
            [right.clone()],
            ["Point".to_string()],
        )
        .unwrap();
        let combined = PythonProjectionContext::packaged_with_ambiguities(
            [left.clone(), right.clone()],
            ["Point".to_string()],
        )
        .unwrap();

        assert_eq!(
            left_phase.projected_name(&left),
            combined.projected_name(&left)
        );
        assert_eq!(
            right_phase.projected_name(&right),
            combined.projected_name(&right)
        );
        assert_ne!(
            left_phase.projected_name(&left),
            right_phase.projected_name(&right)
        );
    }

    #[test]
    fn ordinary_closed_generic_names_remain_compatible() {
        let reference = TypeIdentity::closed_generic(
            TypeIdentityKind::Interface,
            "Windows.Foundation",
            "IReference",
            [TypeIdentity::Primitive {
                name: "UInt32".into(),
            }],
        );
        let context = PythonProjectionContext::packaged([reference.clone()]).unwrap();

        assert_eq!(context.projected_name(&reference), "IReference_UInt32");
        assert_eq!(
            context.implementation_module(&reference),
            "windows__foundation__i_reference_uint32"
        );
    }

    #[test]
    fn canonical_hash_input_is_stable_and_kind_sensitive() {
        let interface = TypeIdentity::named(TypeIdentityKind::Interface, "Example", "Value");
        let runtime_class = TypeIdentity::named(TypeIdentityKind::Class, "Example", "Value");

        assert_eq!(interface.canonical_key(), interface.clone().canonical_key());
        assert_ne!(interface.canonical_key(), runtime_class.canonical_key());
        assert_ne!(interface, runtime_class);
    }

    #[test]
    fn closed_generic_projection_identity_does_not_replace_abi_identity() {
        let point = |namespace: &str| TypeMeta::Struct {
            namespace: namespace.into(),
            name: "Point".into(),
            fields: vec![],
        };
        let interface = |argument: TypeMeta| InterfaceMeta {
            name: "IBox_Point".into(),
            namespace: "Example.Collections".into(),
            iid: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
            generic_piid: Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into()),
            generic_name: Some("IBox`1".into()),
            generic_args: vec![argument],
            ..Default::default()
        };
        let left = interface(point("Example.Left"));
        let right = interface(point("Example.Right"));

        assert_ne!(left.type_identity(), right.type_identity());
        let left_abi =
            crate::codegen::winrt::python::signature::py_interface_iid_expr(&left).unwrap();
        let right_abi =
            crate::codegen::winrt::python::signature::py_interface_iid_expr(&right).unwrap();
        assert_ne!(left_abi, right_abi);
        assert!(left_abi.contains("Example.Left.Point"));
        assert!(right_abi.contains("Example.Right.Point"));
    }
}
