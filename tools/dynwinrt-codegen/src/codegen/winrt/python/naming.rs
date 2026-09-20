// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python naming and identifier helpers.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::super::shared::implementation_symbols::{
    HelperOwner, ImplementationHelper, allocate_helpers, interface_helpers,
};
use crate::meta::InterfaceMeta;
use crate::types::{TypeIdentity, TypeIdentityKind, TypeKind, TypeMeta, TypeRef};

pub type PythonTypeIdentity = TypeIdentity;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PythonSupportSymbol {
    ObjectInput,
}

impl PythonSupportSymbol {
    fn name(self) -> &'static str {
        match self {
            Self::ObjectInput => "_DynWinRTObject",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PythonSymbol {
    Type,
    Like,
    Identity,
    Pack,
    Unpack,
    TypeConstant,
    PrivatePack,
    PrivateUnpack,
    PrivateTypeConstant,
    Registration,
    ActivationFactoryRegistration,
}

impl PythonSymbol {
    fn named(self, name: &str) -> String {
        match self {
            Self::Type => name.into(),
            Self::Like => format!("{name}Like"),
            Self::Identity => format!("_{name}Identity"),
            Self::Pack => format!("pack_{}", to_snake_case(name)),
            Self::Unpack => format!("unpack_{}", to_snake_case(name)),
            Self::TypeConstant => format!("{name}_TYPE"),
            Self::PrivatePack => format!("_pack_{}", to_snake_case(name)),
            Self::PrivateUnpack => format!("_unpack_{}", to_snake_case(name)),
            Self::PrivateTypeConstant => format!("_{name}_TYPE"),
            Self::Registration => format!("_{name}"),
            Self::ActivationFactoryRegistration => "_IActivationFactory".into(),
        }
    }

    fn is_registration(self) -> bool {
        matches!(
            self,
            Self::Registration | Self::ActivationFactoryRegistration
        )
    }
}

pub(crate) const STRUCT_SYMBOLS: [PythonSymbol; 7] = [
    PythonSymbol::Type,
    PythonSymbol::TypeConstant,
    PythonSymbol::Pack,
    PythonSymbol::Unpack,
    PythonSymbol::PrivateTypeConstant,
    PythonSymbol::PrivatePack,
    PythonSymbol::PrivateUnpack,
];

pub(super) fn py_struct_export_names(typ: &TypeMeta) -> Vec<String> {
    let TypeMeta::Struct { name, .. } = typ else {
        return Vec::new();
    };
    STRUCT_SYMBOLS[..4]
        .iter()
        .filter(|role| {
            **role != PythonSymbol::Type || super::native_types::foundation_type(typ).is_none()
        })
        .map(|role| role.named(name))
        .collect()
}

pub(super) fn format_py_type_import(
    context: &PythonProjectionContext,
    namespace: &str,
    name: &str,
    kind: TypeKind,
) -> String {
    let identity_kind = match kind {
        TypeKind::Class => TypeIdentityKind::Class,
        TypeKind::Enum => TypeIdentityKind::Enum,
        TypeKind::Interface => TypeIdentityKind::Interface,
    };
    let identity = TypeIdentity::named(identity_kind, namespace, name);
    let module = context.implementation_module(&identity);
    let type_import = context.symbol_import(&identity, PythonSymbol::Type);
    let names = match kind {
        TypeKind::Interface => {
            let declaration = format!("IID_{}", context.projected_name(&identity));
            let reference = format!("IID_{}", context.reference_name(&identity));
            let iid = if declaration == reference {
                declaration
            } else {
                format!("{declaration} as {reference}")
            };
            format!("{iid}, {type_import}")
        }
        TypeKind::Class => format!(
            "{type_import}, {}",
            context.symbol_import(&identity, PythonSymbol::Like)
        ),
        TypeKind::Enum => type_import,
    };
    format!("from .{module} import {names}  # noqa: F401\n")
}

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
    let namespace = identity.namespace().unwrap_or_default();
    let qualified = qualified_module_name(identity, namespace, projected_name);
    let prefix = python_namespace_segments(namespace).join("__");
    if prefix.is_empty() {
        return qualified;
    }
    // Budget the namespace as well as the basename. Otherwise a readable flat
    // module can have a facade path that Windows Python cannot open.
    qualified
        .strip_prefix(&format!("{prefix}__"))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "type_{:016x}",
                stable_module_hash(&identity.canonical_key())
            )
        })
}

/// Explicit, immutable naming and lookup state for one Python projection.
#[derive(Clone, Debug, Default)]
pub struct PythonProjectionContext {
    packaged: bool,
    projections: HashMap<PythonTypeIdentity, PythonProjection>,
    aliases: HashMap<PythonTypeIdentity, PythonTypeIdentity>,
    compatibility_counts: HashMap<String, usize>,
    implementation_helpers: BTreeMap<String, Vec<ImplementationHelper>>,
    module_symbols: HashMap<(PythonTypeIdentity, PythonSymbol), String>,
    module_support_symbols: HashMap<PythonSupportSymbol, String>,
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
            implementation_helpers: BTreeMap::new(),
            module_symbols: HashMap::new(),
            module_support_symbols: HashMap::new(),
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

    fn imported_type_symbols(
        &self,
        references: impl IntoIterator<Item = TypeRef>,
        generics: impl IntoIterator<Item = PythonTypeIdentity>,
    ) -> Vec<(PythonTypeIdentity, PythonSymbol)> {
        let mut symbols = Vec::new();
        for reference in references {
            if !self.is_known_ref(&reference) {
                continue;
            }
            let kind = match reference.kind {
                TypeKind::Class => TypeIdentityKind::Class,
                TypeKind::Enum => TypeIdentityKind::Enum,
                TypeKind::Interface => TypeIdentityKind::Interface,
            };
            let identity = self.normalize_identity(&TypeIdentity::named(
                kind,
                reference.namespace,
                reference.name,
            ));
            if identity.kind() == Some(TypeIdentityKind::Delegate) {
                continue;
            }
            symbols.push((identity.clone(), PythonSymbol::Type));
            if kind == TypeIdentityKind::Class {
                symbols.push((identity, PythonSymbol::Like));
            }
        }
        for identity in generics {
            let identity = self.normalize_identity(&identity);
            if identity.kind() != Some(TypeIdentityKind::Delegate) {
                symbols.push((identity, PythonSymbol::Type));
            }
        }
        symbols
    }

    pub(super) fn for_class_module(
        &self,
        class: &crate::meta::ClassMeta,
        structs: &[TypeMeta],
    ) -> Cow<'_, Self> {
        use crate::codegen::winrt::shared::imports::{
            collect_class_type_imports_by_identity, collect_used_generic_identities_from_class,
        };
        let mut imports = self.imported_type_symbols(
            collect_class_type_imports_by_identity(class),
            collect_used_generic_identities_from_class(class),
        );
        // Required views are visible either as peer imports or inline wrappers.
        imports.extend(
            class
                .required_interfaces
                .iter()
                .filter(|interface| !interface.iid.is_empty())
                .map(|interface| (interface.type_identity(), PythonSymbol::Type)),
        );
        if let Some(base) = class
            .base_class
            .as_ref()
            .filter(|base| self.is_known_ref(base))
        {
            imports.push((
                TypeIdentity::named(TypeIdentityKind::Class, &base.namespace, &base.name),
                PythonSymbol::Identity,
            ));
        }
        let owner = TypeIdentity::named(TypeIdentityKind::Class, &class.namespace, &class.name);
        let registrations = class
            .all_interfaces()
            .map(|interface| (interface.type_identity(), PythonSymbol::Registration))
            .chain(
                class
                    .has_default_activation()
                    .then(|| (owner.clone(), PythonSymbol::ActivationFactoryRegistration)),
            );
        self.with_local_types(Some(owner.clone()), structs, imports, registrations)
    }

    pub(super) fn for_interface_module(
        &self,
        interface: &InterfaceMeta,
        structs: &[TypeMeta],
    ) -> Cow<'_, Self> {
        use crate::codegen::winrt::shared::imports::{
            collect_iface_type_imports_by_identity, collect_used_generic_identities_from_methods,
        };
        let mut generics = collect_used_generic_identities_from_methods(&interface.methods);
        for delegate in &interface.implementation_metadata.delegates {
            generics.extend(collect_used_generic_identities_from_methods(
                std::slice::from_ref(&delegate.invoke),
            ));
        }
        self.with_local_types(
            Some(interface.type_identity()),
            structs,
            self.imported_type_symbols(collect_iface_type_imports_by_identity(interface), generics),
            (!interface.is_delegate())
                .then(|| (interface.type_identity(), PythonSymbol::Registration)),
        )
    }

    pub(super) fn for_struct_module(&self, typ: &TypeMeta, structs: &[TypeMeta]) -> Cow<'_, Self> {
        use crate::codegen::winrt::shared::imports::{
            collect_struct_field_type_imports, collect_used_generic_identities_from_type,
        };
        self.with_local_types(
            Some(typ.type_identity()),
            structs,
            self.imported_type_symbols(
                collect_struct_field_type_imports(typ),
                collect_used_generic_identities_from_type(typ),
            ),
            [],
        )
    }

    /// Bind local declarations and imported conversion helpers in one module.
    fn with_local_types(
        &self,
        owner: Option<PythonTypeIdentity>,
        structs: &[TypeMeta],
        imports: impl IntoIterator<Item = (PythonTypeIdentity, PythonSymbol)>,
        registrations: impl IntoIterator<Item = (PythonTypeIdentity, PythonSymbol)>,
    ) -> Cow<'_, Self> {
        let declarations = owner
            .iter()
            .map(|identity| (identity.clone(), self.declaration_name(identity)))
            .chain(structs.iter().filter_map(|typ| match typ {
                TypeMeta::Struct { name, .. }
                    if !self.is_packaged()
                        && owner.as_ref().and_then(TypeIdentity::kind)
                            != Some(TypeIdentityKind::Struct) =>
                {
                    Some((typ.type_identity(), name.clone()))
                }
                _ => None,
            }));
        let mut context = Cow::Borrowed(self);
        for (identity, declaration_name) in declarations {
            let identity = self.normalize_identity(&identity);
            if self.reference_name(&identity) != declaration_name
                && let Some(projection) = context.to_mut().projections.get_mut(&identity)
            {
                projection.reference_name = declaration_name;
            }
        }
        let mut symbols = BTreeMap::new();
        for typ in structs {
            let identity = self.identity_for_type(typ);
            for role in STRUCT_SYMBOLS {
                if role != PythonSymbol::Type || super::native_types::foundation_type(typ).is_none()
                {
                    symbols.insert(
                        (identity.clone(), role),
                        context.symbol_reference(&identity, role),
                    );
                }
            }
        }
        for (identity, role) in imports {
            let identity = self.normalize_identity(&identity);
            if owner.as_ref() != Some(&identity) {
                symbols.insert(
                    (identity.clone(), role),
                    context.symbol_reference(&identity, role),
                );
            }
        }
        if let Some(identity) = &owner
            && identity.kind() != Some(TypeIdentityKind::Struct)
        {
            let mut roles = vec![PythonSymbol::Type];
            if identity.kind() == Some(TypeIdentityKind::Class) {
                roles.extend([PythonSymbol::Like, PythonSymbol::Identity]);
            } else if identity.kind() == Some(TypeIdentityKind::Interface) {
                roles.push(PythonSymbol::Identity);
            }
            for role in roles {
                symbols.insert(
                    (identity.clone(), role),
                    context.symbol_reference(identity, role),
                );
            }
        }
        for (identity, role) in registrations {
            let identity = self.normalize_identity(&identity);
            symbols.insert(
                (identity.clone(), role),
                context.symbol_reference(&identity, role),
            );
        }
        let mut groups = BTreeMap::<String, Vec<(PythonTypeIdentity, PythonSymbol)>>::new();
        for (key, name) in &symbols {
            groups.entry(name.clone()).or_default().push(key.clone());
        }
        let mut reserved = groups.keys().cloned().collect::<HashSet<_>>();
        if groups.values().any(|group| group.len() > 1) {
            // Freeze each visible role before changing a type alias: its Like and
            // identity imports may keep their existing, independently valid names.
            context.to_mut().module_symbols.extend(symbols);
        }
        for group in groups.values().filter(|group| group.len() > 1) {
            let can_alias_helper = |identity: &TypeIdentity, role: PythonSymbol| {
                identity.kind() == Some(TypeIdentityKind::Struct)
                    && role != PythonSymbol::Type
                    && owner.as_ref() != Some(identity)
            };
            let fixed_symbol_count = group
                .iter()
                .filter(|(identity, role)| !can_alias_helper(identity, *role))
                .count();
            let fixed_type_count = group
                .iter()
                .filter(|(identity, role)| {
                    !can_alias_helper(identity, *role) && !role.is_registration()
                })
                .count();
            for (identity, role) in group {
                let rename = if role.is_registration() {
                    fixed_symbol_count > 1
                } else {
                    owner.as_ref() != Some(identity)
                        && (can_alias_helper(identity, *role) || fixed_type_count > 1)
                };
                if !rename {
                    continue;
                }
                let candidate = role.named(&semantic_qualifier(identity));
                let mut name = candidate.clone();
                let mut index = 0;
                while !reserved.insert(name.clone()) {
                    name = format!(
                        "{candidate}_{:016x}_{index}",
                        stable_module_hash(&identity.canonical_key())
                    );
                    index += 1;
                }
                context
                    .to_mut()
                    .module_symbols
                    .insert((identity.clone(), *role), name);
            }
        }
        // Support imports yield to metadata declarations and their allocated roles.
        for helper in [PythonSupportSymbol::ObjectInput] {
            let preferred = helper.name();
            let mut name = preferred.to_string();
            let mut index = 2;
            while !reserved.insert(name.clone()) {
                name = format!("{preferred}_{index}");
                index += 1;
            }
            if name != preferred {
                context.to_mut().module_support_symbols.insert(helper, name);
            }
        }
        context
    }

    pub(crate) fn support_symbol_reference(&self, symbol: PythonSupportSymbol) -> &str {
        self.module_support_symbols
            .get(&symbol)
            .map_or(symbol.name(), String::as_str)
    }

    pub(crate) fn support_symbol_import(&self, symbol: PythonSupportSymbol) -> String {
        let declaration = symbol.name();
        let reference = self.support_symbol_reference(symbol);
        if declaration == reference {
            declaration.to_string()
        } else {
            format!("{declaration} as {reference}")
        }
    }

    pub(crate) fn declaration_name(&self, identity: &PythonTypeIdentity) -> String {
        if identity.kind() == Some(TypeIdentityKind::Struct) {
            legacy_projected_name(identity)
        } else {
            self.projected_name(identity)
        }
    }

    pub(crate) fn symbol_declaration(
        &self,
        identity: &PythonTypeIdentity,
        role: PythonSymbol,
    ) -> String {
        role.named(&self.declaration_name(identity))
    }

    pub(crate) fn symbol_reference(
        &self,
        identity: &PythonTypeIdentity,
        role: PythonSymbol,
    ) -> String {
        let identity = self.normalize_identity(identity);
        if let Some(symbol) = self.module_symbols.get(&(identity.clone(), role)) {
            return symbol.clone();
        }
        let name =
            if identity.kind() == Some(TypeIdentityKind::Struct) && role != PythonSymbol::Type {
                self.declaration_name(&identity)
            } else {
                self.reference_name(&identity)
            };
        role.named(&name)
    }

    pub(crate) fn symbol_import(
        &self,
        identity: &PythonTypeIdentity,
        role: PythonSymbol,
    ) -> String {
        let declaration = self.symbol_declaration(identity, role);
        let reference = self.symbol_reference(identity, role);
        if declaration == reference {
            declaration
        } else {
            format!("{declaration} as {reference}")
        }
    }

    pub(crate) fn struct_symbol(&self, typ: &TypeMeta, role: PythonSymbol) -> String {
        self.symbol_reference(&self.identity_for_type(typ), role)
    }

    pub(crate) fn registration_symbol(&self, interface: &InterfaceMeta) -> String {
        self.symbol_reference(&interface.type_identity(), PythonSymbol::Registration)
    }

    pub(crate) fn activation_factory_symbol(&self, class: &crate::meta::ClassMeta) -> String {
        self.symbol_reference(
            &TypeIdentity::named(TypeIdentityKind::Class, &class.namespace, &class.name),
            PythonSymbol::ActivationFactoryRegistration,
        )
    }

    pub(crate) fn registration_call_name(&self, variable: &str) -> String {
        self.module_symbols
            .iter()
            .find(|((_, role), name)| *role == PythonSymbol::Registration && *name == variable)
            .map_or_else(
                || variable.trim_start_matches('_').to_string(),
                |((identity, _), _)| {
                    self.unaliased_reference_name(identity)
                        .trim_start_matches('_')
                        .to_string()
                },
            )
    }

    pub fn configure_implementation_helpers(
        &mut self,
        owners: impl IntoIterator<Item = (PythonTypeIdentity, Vec<ImplementationHelper>)>,
    ) {
        let owners = owners
            .into_iter()
            .map(|(identity, helpers)| HelperOwner {
                identity: identity.canonical_key(),
                projected_name: self.projected_name(&identity),
                qualified_name: semantic_qualifier(&identity),
                helpers,
            })
            .collect::<Vec<_>>();
        let reserved = self.projections.iter().flat_map(|(identity, projection)| {
            [
                projection.projected_name.clone(),
                projection.reference_name.clone(),
                legacy_projected_name(identity),
                format!("IID_{}", projection.projected_name),
            ]
        });
        self.implementation_helpers = allocate_helpers(owners, reserved, false);
    }

    pub fn implementation_helpers(&self, identity: &PythonTypeIdentity) -> &[ImplementationHelper] {
        self.implementation_helpers
            .get(&self.normalize_identity(identity).canonical_key())
            .map_or(&[], Vec::as_slice)
    }

    pub(crate) fn implementation_helper_name(
        &self,
        interface: &InterfaceMeta,
        key: &str,
        suffix: &str,
    ) -> String {
        let identity = self
            .projections
            .iter()
            .find(|(identity, projection)| {
                identity.namespace() == Some(interface.namespace.as_str())
                    && projection.projected_name == interface.name
            })
            .map_or_else(
                || interface.type_identity(),
                |(identity, _)| identity.clone(),
            );
        if let Some(helper) = self
            .implementation_helpers(&identity)
            .iter()
            .find(|helper| helper.key == key)
        {
            return helper.name.clone();
        }
        let names = allocate_helpers(
            [HelperOwner {
                identity: identity.canonical_key(),
                projected_name: interface.name.clone(),
                qualified_name: semantic_qualifier(&identity),
                helpers: interface_helpers(interface, true),
            }],
            self.projections
                .values()
                .map(|projection| projection.projected_name.clone()),
            false,
        );
        names
            .get(&identity.canonical_key())
            .and_then(|helpers| helpers.iter().find(|helper| helper.key == key))
            .map_or_else(
                || format!("{}{suffix}", interface.name),
                |helper| helper.name.clone(),
            )
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

    pub(crate) fn class_name(&self, class: &crate::meta::ClassMeta) -> String {
        self.projected_name(&TypeIdentity::named(
            TypeIdentityKind::Class,
            &class.namespace,
            &class.name,
        ))
    }

    pub fn reference_name(&self, identity: &PythonTypeIdentity) -> String {
        let identity = self.normalize_identity(identity);
        if let Some(name) = self
            .module_symbols
            .get(&(identity.clone(), PythonSymbol::Type))
        {
            return name.clone();
        }
        self.unaliased_reference_name(&identity)
    }

    fn unaliased_reference_name(&self, identity: &PythonTypeIdentity) -> String {
        self.projections
            .get(identity)
            .map(|projection| projection.reference_name.clone())
            .unwrap_or_else(|| self.projected_name(identity))
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

    pub fn public_qualified_module_for_export(
        &self,
        identity: &PythonTypeIdentity,
        projected_name: &str,
    ) -> String {
        let identity = self.normalize_identity(identity);
        let mut segments = python_namespace_segments(identity.namespace().unwrap_or_default());
        segments.push(public_module_name(&identity, projected_name));
        segments.join(".")
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
    fn object_input_helper_yields_to_visible_roles_without_renaming_metadata() {
        let helper = PythonSupportSymbol::ObjectInput;
        for kind in [
            TypeIdentityKind::Class,
            TypeIdentityKind::Interface,
            TypeIdentityKind::Struct,
        ] {
            for packaged in [false, true] {
                let owner = TypeIdentity::named(kind, "Audit", "_DynWinRTObject");
                let peer =
                    TypeIdentity::named(TypeIdentityKind::Enum, "Audit", "_DynWinRTObject_2");
                let unused =
                    TypeIdentity::named(TypeIdentityKind::Enum, "Unused", "_DynWinRTObject_3");
                let context =
                    PythonProjectionContext::new([owner.clone(), peer.clone(), unused], packaged)
                        .unwrap();
                let structs = if kind == TypeIdentityKind::Struct {
                    vec![TypeMeta::Struct {
                        namespace: "Audit".into(),
                        name: "_DynWinRTObject".into(),
                        fields: vec![],
                    }]
                } else {
                    vec![]
                };
                let module = context.with_local_types(
                    Some(owner.clone()),
                    &structs,
                    [(peer.clone(), PythonSymbol::Type)],
                    [],
                );
                assert_eq!(module.reference_name(&owner), "_DynWinRTObject");
                assert_eq!(module.reference_name(&peer), "_DynWinRTObject_2");
                assert_eq!(module.support_symbol_reference(helper), "_DynWinRTObject_3");
                assert_eq!(
                    module.support_symbol_import(helper),
                    "_DynWinRTObject as _DynWinRTObject_3"
                );
                let isolated = context.with_local_types(Some(owner), &structs, [], []);
                assert_eq!(
                    isolated.support_symbol_reference(helper),
                    "_DynWinRTObject_2"
                );
                let control = context.with_local_types(Some(peer), &[], [], []);
                assert_eq!(control.support_symbol_import(helper), "_DynWinRTObject");
                assert_eq!(context.support_symbol_import(helper), "_DynWinRTObject");
            }
        }
    }

    #[test]
    fn companion_aliases_freeze_roles_and_use_only_visible_symbols() {
        let owner = TypeIdentity::named(TypeIdentityKind::Class, "Audit", "Widget");
        let peer = TypeIdentity::named(TypeIdentityKind::Class, "Audit", "WidgetLike");
        let occupied =
            TypeIdentity::named(TypeIdentityKind::Enum, "Other", "Audit_WidgetLike_class");
        let identities = [owner.clone(), peer.clone(), occupied.clone()];
        let imports = [
            (peer.clone(), PythonSymbol::Type),
            (peer.clone(), PythonSymbol::Like),
            (occupied.clone(), PythonSymbol::Type),
        ];
        for packaged in [false, true] {
            let context = PythonProjectionContext::new(identities.clone(), packaged).unwrap();
            let module = context.with_local_types(Some(owner.clone()), &[], imports.clone(), []);
            for role in [
                PythonSymbol::Type,
                PythonSymbol::Like,
                PythonSymbol::Identity,
            ] {
                assert_eq!(
                    module.symbol_reference(&owner, role),
                    context.symbol_declaration(&owner, role)
                );
            }
            let alias = module.reference_name(&peer);
            assert!(alias.starts_with("Audit_WidgetLike_class_"), "{alias}");
            assert_eq!(
                module.symbol_reference(&peer, PythonSymbol::Like),
                "WidgetLikeLike"
            );
            assert_eq!(module.reference_name(&occupied), "Audit_WidgetLike_class");
            let reversed =
                PythonProjectionContext::new(identities.clone().into_iter().rev(), packaged)
                    .unwrap();
            assert_eq!(
                module.module_symbols,
                reversed
                    .with_local_types(
                        Some(owner.clone()),
                        &[],
                        imports.clone().into_iter().rev(),
                        []
                    )
                    .module_symbols
            );
            let unused = context.with_local_types(Some(owner.clone()), &[], [], []);
            assert!(unused.module_symbols.is_empty());
            let no_fallback = context.with_local_types(
                Some(owner.clone()),
                &[],
                imports[..2].iter().cloned(),
                [],
            );
            assert_eq!(no_fallback.reference_name(&peer), "Audit_WidgetLike_class");

            let imported_like = context.with_local_types(
                Some(peer.clone()),
                &[],
                [
                    (owner.clone(), PythonSymbol::Type),
                    (owner.clone(), PythonSymbol::Like),
                ],
                [],
            );
            assert_eq!(imported_like.reference_name(&owner), "Widget");
            assert_eq!(
                imported_like.symbol_reference(&owner, PythonSymbol::Like),
                "Audit_Widget_classLike"
            );
        }
    }

    #[test]
    fn interface_identity_declarations_and_foreign_identity_imports_are_reserved() {
        let owner = TypeIdentity::named(TypeIdentityKind::Interface, "Audit", "IUse");
        let peer = TypeIdentity::named(TypeIdentityKind::Class, "Audit", "_IUseIdentity");
        let base = TypeIdentity::named(TypeIdentityKind::Class, "Audit", "Base");
        let foreign = TypeIdentity::named(TypeIdentityKind::Enum, "Audit", "_BaseIdentity");
        let context = PythonProjectionContext::packaged([
            owner.clone(),
            peer.clone(),
            base.clone(),
            foreign.clone(),
        ])
        .unwrap();
        let module = context.with_local_types(
            Some(owner.clone()),
            &[],
            [
                (peer.clone(), PythonSymbol::Type),
                (peer.clone(), PythonSymbol::Like),
                (base.clone(), PythonSymbol::Identity),
                (foreign.clone(), PythonSymbol::Type),
            ],
            [],
        );
        assert_eq!(
            module.symbol_reference(&owner, PythonSymbol::Identity),
            "_IUseIdentity"
        );
        assert_ne!(module.reference_name(&peer), "_IUseIdentity");
        assert_eq!(
            module.symbol_reference(&peer, PythonSymbol::Like),
            "_IUseIdentityLike"
        );
        assert_eq!(
            module.symbol_import(&base, PythonSymbol::Identity),
            "_BaseIdentity as _Audit_Base_classIdentity"
        );
        assert_ne!(module.reference_name(&foreign), "_BaseIdentity");
    }

    #[test]
    fn module_helper_aliases_preserve_isolated_exports_and_input_order() {
        let alpha = TypeMeta::Struct {
            namespace: "Alpha".into(),
            name: "URLValue".into(),
            fields: vec![],
        };
        let beta = TypeMeta::Struct {
            namespace: "Beta".into(),
            name: "UrlValue".into(),
            fields: vec![],
        };
        let context =
            PythonProjectionContext::packaged([alpha.type_identity(), beta.type_identity()])
                .unwrap();
        let forward = context
            .with_local_types(None, &[alpha.clone(), beta.clone()], [], [])
            .into_owned();
        let reverse = context
            .with_local_types(None, &[beta.clone(), alpha.clone()], [], [])
            .into_owned();
        for role in [
            PythonSymbol::Pack,
            PythonSymbol::Unpack,
            PythonSymbol::PrivatePack,
            PythonSymbol::PrivateUnpack,
        ] {
            assert_ne!(
                forward.struct_symbol(&alpha, role),
                forward.struct_symbol(&beta, role)
            );
            for typ in [&alpha, &beta] {
                assert_eq!(
                    forward.struct_symbol(typ, role),
                    reverse.struct_symbol(typ, role)
                );
                assert_eq!(
                    forward.symbol_import(&typ.type_identity(), role),
                    format!(
                        "{} as {}",
                        context.symbol_declaration(&typ.type_identity(), role),
                        forward.struct_symbol(typ, role),
                    )
                );
            }
        }
        assert_eq!(
            context.struct_symbol(&alpha, PythonSymbol::Pack),
            "pack_url_value"
        );
        assert_eq!(
            context.struct_symbol(&beta, PythonSymbol::Pack),
            "pack_url_value"
        );
        let owned = context
            .with_local_types(
                Some(alpha.type_identity()),
                &[alpha.clone(), beta.clone()],
                [],
                [],
            )
            .into_owned();
        assert_eq!(
            owned.struct_symbol(&alpha, PythonSymbol::Pack),
            "pack_url_value"
        );
        assert_ne!(
            owned.struct_symbol(&beta, PythonSymbol::Pack),
            "pack_url_value"
        );
    }

    #[test]
    fn cross_role_aliases_use_visible_imports_and_preserve_owned_struct_exports() {
        let typ = TypeMeta::Struct {
            namespace: "Alpha".into(),
            name: "URLValue".into(),
            fields: vec![],
        };
        let class = TypeIdentity::named(TypeIdentityKind::Class, "Kinds", "pack_url_value");
        let enum_type = TypeIdentity::named(TypeIdentityKind::Enum, "Kinds", "URLValue_TYPE");
        let context = PythonProjectionContext::packaged([
            typ.type_identity(),
            class.clone(),
            enum_type.clone(),
        ])
        .unwrap();
        let structs = std::slice::from_ref(&typ);
        let unrelated = context.with_local_types(None, structs, [], []);
        assert_eq!(
            unrelated.struct_symbol(&typ, PythonSymbol::Pack),
            "pack_url_value"
        );
        assert_eq!(
            unrelated.struct_symbol(&typ, PythonSymbol::TypeConstant),
            "URLValue_TYPE"
        );
        let imports = [
            (class.clone(), PythonSymbol::Type),
            (class.clone(), PythonSymbol::Like),
            (enum_type.clone(), PythonSymbol::Type),
        ];
        let consumer = context.with_local_types(None, structs, imports.clone(), []);
        assert_ne!(
            consumer.struct_symbol(&typ, PythonSymbol::Pack),
            "pack_url_value"
        );
        assert_ne!(
            consumer.struct_symbol(&typ, PythonSymbol::TypeConstant),
            "URLValue_TYPE"
        );
        assert_eq!(consumer.reference_name(&class), "pack_url_value");
        assert_eq!(consumer.reference_name(&enum_type), "URLValue_TYPE");

        let owned = context.with_local_types(Some(typ.type_identity()), structs, imports, []);
        assert_eq!(
            owned.struct_symbol(&typ, PythonSymbol::Pack),
            "pack_url_value"
        );
        assert_eq!(
            owned.struct_symbol(&typ, PythonSymbol::TypeConstant),
            "URLValue_TYPE"
        );
        assert_ne!(owned.reference_name(&class), "pack_url_value");
        assert_ne!(owned.reference_name(&enum_type), "URLValue_TYPE");
        assert_eq!(
            owned.symbol_reference(&class, PythonSymbol::Like),
            "pack_url_valueLike",
        );
        assert_eq!(
            owned.symbol_import(&class, PythonSymbol::Type),
            format!("pack_url_value as {}", owned.reference_name(&class)),
        );
    }

    #[test]
    fn registration_aliases_keep_independent_type_and_call_behavior_names() {
        let interface = InterfaceMeta {
            namespace: "Audit".into(),
            name: "IApplicationStatics".into(),
            ..Default::default()
        };
        let typ = TypeMeta::Struct {
            namespace: "Alpha".into(),
            name: "_IApplicationStatics".into(),
            fields: vec![],
        };
        let context =
            PythonProjectionContext::standalone([interface.type_identity(), typ.type_identity()])
                .unwrap();
        let module = context.for_interface_module(&interface, std::slice::from_ref(&typ));
        let registration = module.registration_symbol(&interface);
        assert_ne!(registration, "_IApplicationStatics");
        assert_eq!(module.reference_name_for_type(&typ), "_IApplicationStatics");
        assert_eq!(
            module.registration_call_name(&registration),
            "IApplicationStatics"
        );
        assert_eq!(
            module.registration_call_name(&registration),
            context.registration_call_name("_IApplicationStatics"),
        );
    }

    #[test]
    fn activation_registration_is_reserved_only_for_default_activation() {
        let mut class = crate::meta::ClassMeta {
            namespace: "Audit".into(),
            name: "Wrapper".into(),
            full_name: "Audit.Wrapper".into(),
            ..Default::default()
        };
        let typ = TypeMeta::Struct {
            namespace: "Alpha".into(),
            name: "_IActivationFactory".into(),
            fields: vec![],
        };
        let context = PythonProjectionContext::standalone([
            TypeIdentity::named(TypeIdentityKind::Class, &class.namespace, &class.name),
            typ.type_identity(),
        ])
        .unwrap();
        let structs = std::slice::from_ref(&typ);
        let inactive = context.for_class_module(&class, structs);
        assert_eq!(
            inactive.reference_name_for_type(&typ),
            "_IActivationFactory"
        );
        assert!(
            !inactive
                .module_symbols
                .keys()
                .any(|(_, role)| role.is_registration())
        );

        class.constructors.push(crate::meta::ConstructorMeta {
            kind: crate::meta::ConstructorKind::DefaultActivation,
            factory_interface: None,
        });
        let active = context.for_class_module(&class, structs);
        assert_eq!(active.reference_name_for_type(&typ), "_IActivationFactory");
        assert_ne!(
            active.activation_factory_symbol(&class),
            "_IActivationFactory"
        );
    }

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
        assert_eq!(
            context.public_module(&identity),
            implementation
                .strip_prefix("windows__foundation__")
                .unwrap()
        );
        assert!(
            context.public_qualified_module(&identity).chars().count()
                <= MAX_PYTHON_MODULE_COMPONENT_LENGTH
        );
    }

    #[test]
    fn background_event_delegate_facade_uses_the_qualified_module_budget() {
        let identity = TypeIdentity::closed_generic(
            TypeIdentityKind::Delegate,
            "Windows.Foundation",
            "TypedEventHandler",
            [
                TypeIdentity::named(
                    TypeIdentityKind::Class,
                    "Windows.ApplicationModel.Background",
                    "BackgroundTaskRegistrationGroup",
                ),
                TypeIdentity::named(
                    TypeIdentityKind::Class,
                    "Windows.ApplicationModel.Activation",
                    "BackgroundActivatedEventArgs",
                ),
            ],
        );
        let context = PythonProjectionContext::packaged_with_ambiguities(
            [identity.clone()],
            ["BackgroundActivatedEventArgs".to_string()],
        )
        .unwrap();
        let implementation = context.implementation_module(&identity);
        let public = context.public_module(&identity);
        assert_eq!(
            public,
            implementation
                .strip_prefix("windows__foundation__")
                .unwrap()
        );
        assert!(
            context.public_qualified_module(&identity).len() <= MAX_PYTHON_MODULE_COMPONENT_LENGTH
        );
        assert!(public.starts_with("typed_event_handler_background_task_registration_group"));
        assert_eq!(public, context.public_module(&identity));
        assert_ne!(
            public,
            context.public_module(&TypeIdentity::closed_generic(
                TypeIdentityKind::Delegate,
                "Windows.Foundation",
                "TypedEventHandler",
                [
                    TypeIdentity::Primitive {
                        name: "Object".into()
                    },
                    TypeIdentity::Primitive {
                        name: "Object".into()
                    }
                ],
            ))
        );
    }

    #[test]
    fn facade_shortening_keeps_a_valid_name_when_namespace_uses_the_budget() {
        let identity = TypeIdentity::named(
            TypeIdentityKind::Interface,
            "VeryLongNamespace".repeat(12),
            "IValue",
        );
        let context = PythonProjectionContext::packaged([identity.clone()]).unwrap();
        let public = context.public_module(&identity);
        assert!(public.starts_with("type_"), "{public}");
        assert_eq!(public.len(), 5 + MODULE_HASH_HEX_LENGTH);
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
