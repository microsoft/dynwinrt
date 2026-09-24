// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python member plan: overload groups, public method names, private dispatch
//! names, and compatibility aliases.
//!
//! A plan is computed once per generated Python class from its WinRT
//! interfaces. The runtime (`.py`) and stub (`.pyi`) generators both render the
//! same plan, so they cannot disagree about which methods share a Python name,
//! the dispatch order of the overloads, the private name of each overload, or
//! which earlier names remain as aliases. Generators keep their own member
//! order and emit a group where its first method appears.
//!
//! All overloads of a WinRT method share one CLR (MethodDef) name, while
//! `[Overload]` gives each ABI slot a unique name. Python projects the
//! overloads of a CLR method as one dispatched method named after the CLR
//! name, so `IStorageFile.CopyAsync` is `copy_async` rather than
//! `copy_overload`, `copy_overload_default_options`, and so on. The
//! established suffix heuristics (`Foo2`, `FooOverload...`, `FooWithOptions`
//! next to `foo`) still merge on top of the CLR name.
//!
//! Names are planned per Python class namespace: a runtime class shares one
//! namespace between its static and instance members, properties, event
//! helpers, and generated members. The plan never removes a public name and
//! never changes which overload an existing name reaches. A CLR-name group
//! keeps its previous names when its new name would collide with another
//! member, or when an existing name would lose one of its overloads or gain an
//! overload that could take its calls. Every previously emitted method name
//! that is no longer a public method stays as a compatibility alias: of the
//! exact implementation it called when it was a standalone method, otherwise of
//! the dispatcher that reaches its overloads.
//!
//! Interface implementation handlers keep one name per ABI slot and do not use
//! this module.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::codegen::winrt::extensions::winui;
use crate::codegen::winrt::shared::imports::{get_in_params, ireference_inner_type};
use crate::meta::{
    ClassMeta, ConstructorKind, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta,
};
use crate::types::TypeMeta;

use super::collections::{
    class_interface, interface_kind, observable_vector_identity, runtime_mixin, type_kind,
};
use super::naming::{PythonProjectionContext, to_snake_case};
use super::native_types::{FoundationType, foundation_type};
use super::signature::py_dispatch_type_sort_key;

const ICLOSABLE_IID: &str = "30d5a829-7fa4-4026-83bb-d75bae4ea99e";

/// Members inherited from the `collections.abc` bases of generated collection mixins.
const COLLECTION_MIXIN_MEMBERS: &[&str] = &[
    "append",
    "clear",
    "count",
    "extend",
    "get",
    "index",
    "insert",
    "items",
    "keys",
    "pop",
    "popitem",
    "remove",
    "reverse",
    "setdefault",
    "update",
    "values",
];

/// One overload of a planned method group.
pub(crate) struct Candidate<'a> {
    pub(crate) interface: &'a InterfaceMeta,
    pub(crate) method: &'a MethodMeta,
    /// Attribute implementing this overload: the group name when it is the only
    /// candidate, otherwise its private dispatch name.
    pub(crate) attribute: String,
    /// Whether this group defines the implementation attribute. Compatibility
    /// dispatchers may also call an implementation defined by its canonical
    /// CLR-name group.
    pub(crate) define: bool,
}

/// Methods projected as one Python method, in dispatch order.
pub(crate) struct MethodGroup<'a> {
    pub(crate) name: String,
    pub(crate) candidates: Vec<Candidate<'a>>,
    /// The exact formerly standalone method to call, without type guards,
    /// when no typed overload candidate accepts the call.
    pub(crate) legacy_fallback: Option<LegacyFallback<'a>>,
}

pub(crate) struct LegacyFallback<'a> {
    pub(crate) method: &'a MethodMeta,
    pub(crate) attribute: String,
}

/// A previously emitted method name kept as a class attribute alias.
pub(crate) struct Alias<'a> {
    pub(crate) name: String,
    /// Attribute the alias is bound to.
    pub(crate) target: String,
    /// Methods whose signatures the stub declares for this name.
    pub(crate) signatures: Vec<&'a MethodMeta>,
}

/// A member of a Python class in generator order.
pub(crate) enum PlannedMember<'p, 'a> {
    /// Property accessor or event method, emitted by its own rules.
    Accessor(&'a InterfaceMeta, &'a MethodMeta),
    /// The first appearance of a method group.
    Group(&'p MethodGroup<'a>),
}

/// Member plan for one scope (static or instance members) of a Python class.
pub(crate) struct ScopePlan<'a> {
    groups: Vec<MethodGroup<'a>>,
    group_of: HashMap<*const MethodMeta, usize>,
    aliases: Vec<Alias<'a>>,
    previous_attributes: HashMap<*const MethodMeta, String>,
    #[cfg(test)]
    fallbacks: Vec<String>,
}

impl<'a> ScopePlan<'a> {
    /// Walk members in generator order, yielding each method group once, at its
    /// first method.
    pub(crate) fn members<'p>(
        &'p self,
        methods: impl IntoIterator<Item = (&'a InterfaceMeta, &'a MethodMeta)>,
    ) -> Vec<PlannedMember<'p, 'a>> {
        let mut emitted = HashSet::new();
        let mut members = Vec::new();
        for (interface, method) in methods {
            match self.group_of.get(&(method as *const MethodMeta)) {
                Some(&index) => {
                    if emitted.insert(index) {
                        members.push(PlannedMember::Group(&self.groups[index]));
                    }
                }
                None => {
                    assert!(
                        is_accessor(method),
                        "method {} is not part of this member plan",
                        method.name
                    );
                    members.push(PlannedMember::Accessor(interface, method));
                }
            }
        }
        members
    }

    /// The attribute implementing `method`, or `None` for accessors.
    pub(crate) fn attribute(&self, method: &MethodMeta) -> Option<&str> {
        let group = &self.groups[*self.group_of.get(&(method as *const MethodMeta))?];
        group
            .candidates
            .iter()
            .find(|candidate| std::ptr::eq(candidate.method, method))
            .map(|candidate| candidate.attribute.as_str())
    }

    /// The attribute that implemented `method` before CLR-name grouping. Used
    /// only where generated dispatch order depended on attribute names.
    pub(crate) fn previous_attribute(&self, method: &MethodMeta) -> Option<&str> {
        self.previous_attributes
            .get(&(method as *const MethodMeta))
            .map(String::as_str)
    }

    /// Previously emitted names kept as aliases, sorted by name.
    pub(crate) fn aliases(&self) -> &[Alias<'a>] {
        &self.aliases
    }

    pub(crate) fn has_legacy_fallback(&self) -> bool {
        self.groups
            .iter()
            .any(|group| group.legacy_fallback.is_some())
    }

    /// CLR names that kept their previous Python names because of a collision.
    #[cfg(test)]
    pub(crate) fn fallbacks(&self) -> &[String] {
        &self.fallbacks
    }
}

/// Member plans for a runtime class; its static and instance members share
/// one Python class namespace.
pub(crate) struct ClassMemberPlan<'a> {
    pub(crate) statics: ScopePlan<'a>,
    pub(crate) instance: ScopePlan<'a>,
}

impl<'a> ClassMemberPlan<'a> {
    pub(crate) fn new(class: &'a ClassMeta, context: &PythonProjectionContext) -> Self {
        let statics = class
            .factory_interfaces
            .iter()
            .chain(class.static_interfaces.iter())
            .collect();
        let instance = class_instance_interfaces(class).collect();
        let reserved = class_reserved_names(class, context);
        let mut scopes = plan_scopes(&[statics, instance], &reserved).into_iter();
        Self {
            statics: scopes.next().expect("static scope"),
            instance: scopes.next().expect("instance scope"),
        }
    }
}

/// Member plan for an interface wrapper class.
pub(crate) fn interface_member_plan(interface: &InterfaceMeta) -> ScopePlan<'_> {
    plan_scopes(&[vec![interface]], &interface_reserved_names(interface))
        .pop()
        .expect("interface scope")
}

/// Interfaces whose methods are projected as instance members of a runtime class.
pub(crate) fn class_instance_interfaces(class: &ClassMeta) -> impl Iterator<Item = &InterfaceMeta> {
    class
        .default_interface
        .iter()
        .chain(class.required_interfaces.iter())
        .filter(|interface| interface.iid != ICLOSABLE_IID)
}

pub(crate) fn is_accessor(method: &MethodMeta) -> bool {
    method.is_property_getter
        || method.is_property_setter
        || method.is_event_add
        || method.is_event_remove
}

/// Snake-case ABI name of a method: its public name before CLR-name grouping.
fn abi_name(method: &MethodMeta) -> String {
    to_snake_case(&method.name)
}

/// Snake-case CLR (MethodDef) name shared by every overload of a method.
fn clr_name(method: &MethodMeta) -> String {
    if method.raw_name.is_empty() {
        abi_name(method)
    } else {
        to_snake_case(&method.raw_name)
    }
}

/// Merge `foo2`, `foo_overload...`, and `foo_with_options` into an existing `foo`.
fn suffix_group_key(name: &str, names: &HashSet<String>) -> String {
    let mut candidates = Vec::new();
    if let Some((base, _)) = name.split_once("_overload") {
        candidates.push(base);
    }
    if let Some(base) = name.strip_suffix("_with_options") {
        candidates.push(base);
    }
    let numeric_base = name.trim_end_matches(|character: char| character.is_ascii_digit());
    if numeric_base.len() < name.len() {
        candidates.push(numeric_base);
    }
    candidates
        .into_iter()
        .find(|base| !base.is_empty() && names.contains(*base))
        .map(str::to_string)
        .unwrap_or_else(|| name.to_string())
}

pub(crate) fn private_overload_names<'a>(
    public_name: &str,
    methods: impl IntoIterator<Item = &'a MethodMeta>,
) -> Vec<String> {
    let base_names = methods
        .into_iter()
        .map(|method| format!("_{public_name}_{}", method.vtable_index))
        .collect::<Vec<_>>();
    base_names
        .iter()
        .enumerate()
        .map(|(index, base)| {
            if base_names
                .iter()
                .filter(|candidate| *candidate == base)
                .count()
                > 1
            {
                format!("{base}_{index}")
            } else {
                base.clone()
            }
        })
        .collect()
}

fn insert_accessor_names(method: &MethodMeta, is_static: bool, names: &mut HashSet<String>) {
    if is_static {
        if method.is_property_getter && get_in_params(method).is_empty() {
            let property = method.name.strip_prefix("get_").unwrap_or(&method.name);
            names.insert(format!("get_{}", to_snake_case(property)));
        } else if is_accessor(method) {
            names.insert(abi_name(method));
        }
    } else if method.is_property_getter {
        names.insert(to_snake_case(
            method.name.strip_prefix("get_").unwrap_or(&method.name),
        ));
    } else if method.is_property_setter {
        let property = to_snake_case(method.name.strip_prefix("put_").unwrap_or(&method.name));
        names.insert(format!("set_{property}"));
        names.insert(property);
    } else if method.is_event_add || method.is_event_remove {
        let event = to_snake_case(
            method
                .name
                .strip_prefix("add_")
                .or_else(|| method.name.strip_prefix("remove_"))
                .unwrap_or(&method.name),
        );
        for prefix in ["on", "off", "subscribe", "once"] {
            names.insert(format!("{prefix}_{event}"));
        }
    }
}

/// Non-method names in a runtime class namespace: accessors and generated members.
fn class_reserved_names(class: &ClassMeta, context: &PythonProjectionContext) -> HashSet<String> {
    let mut names = HashSet::from(["as_interface".to_string()]);
    for method in class_instance_interfaces(class).flat_map(|interface| interface.methods.iter()) {
        insert_accessor_names(method, false, &mut names);
    }
    for method in class
        .factory_interfaces
        .iter()
        .chain(class.static_interfaces.iter())
        .flat_map(|interface| interface.methods.iter())
    {
        insert_accessor_names(method, true, &mut names);
    }
    let factory_names = class
        .factory_interfaces
        .iter()
        .flat_map(|interface| interface.methods.iter())
        .map(abi_name)
        .collect::<Vec<_>>();
    if class.has_default_activation() {
        let default_constructor = if factory_names.iter().any(|name| name.starts_with("create")) {
            "create_default"
        } else {
            "create"
        };
        names.insert(default_constructor.to_string());
    } else if !factory_names.iter().any(|name| name == "create")
        && class.factory_interfaces.iter().any(|interface| {
            class.is_public_constructor_factory(interface)
                && interface.methods.iter().any(|method| {
                    method.name == "CreateInstance"
                        && get_in_params(method).is_empty()
                        && matches!(
                            method.return_type.as_ref(),
                            Some(TypeMeta::RuntimeClass { namespace, name, .. })
                                if namespace == &class.namespace && name == &class.name
                        )
                })
        })
    {
        names.insert("create".to_string());
    }
    if class
        .required_interfaces
        .iter()
        .any(|interface| interface.iid == ICLOSABLE_IID)
    {
        names.insert("close".to_string());
    }
    if crate::codegen::winrt::is_buffer_class(&class.namespace, &class.name) {
        names.extend(["from_bytes".to_string(), "to_bytes".to_string()]);
    }
    if winui::is_dispatcher_queue(class) {
        names.extend([
            "enqueue_async".to_string(),
            "enqueue_with_priority_async".to_string(),
        ]);
    }
    if winui::resolve_application_bootstrap(class, &context.known_full_names()).is_some() {
        names.extend([
            "create".to_string(),
            "create_with_metadata_provider".to_string(),
        ]);
    }
    if class
        .constructors
        .iter()
        .any(|constructor| constructor.kind == ConstructorKind::PublicComposition)
    {
        names.insert("register_xaml_runtime_class".to_string());
    }
    if class_interface(class)
        .and_then(interface_kind)
        .and_then(runtime_mixin)
        .is_some()
    {
        names.extend(COLLECTION_MIXIN_MEMBERS.iter().map(|name| name.to_string()));
    }
    names
}

/// Non-method names in an interface wrapper class: accessors and generated members.
fn interface_reserved_names(interface: &InterfaceMeta) -> HashSet<String> {
    let mut names = [
        "as_interface",
        "as_vector",
        "create",
        "from_bytes",
        "from_implementation",
        "from_value",
        "implement",
        "implementation",
        "release_callbacks",
        "to_bytes",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<HashSet<_>>();
    for method in &interface.methods {
        insert_accessor_names(method, false, &mut names);
    }
    if interface_kind(interface).and_then(runtime_mixin).is_some()
        || observable_vector_identity(interface).is_some()
    {
        names.extend(COLLECTION_MIXIN_MEMBERS.iter().map(|name| name.to_string()));
    }
    names
}

struct Entry<'a> {
    interface: &'a InterfaceMeta,
    method: &'a MethodMeta,
    scope: usize,
    abi: String,
    abi_key: String,
    clr_key: String,
}

type Groups = Vec<BTreeMap<String, Vec<usize>>>;

/// Plan the scopes of one Python class namespace; `reserved` holds its
/// non-method member names.
fn plan_scopes<'a>(
    scopes: &[Vec<&'a InterfaceMeta>],
    reserved: &HashSet<String>,
) -> Vec<ScopePlan<'a>> {
    let mut entries = Vec::new();
    for (scope, interfaces) in scopes.iter().enumerate() {
        let methods = interfaces
            .iter()
            .flat_map(|interface| {
                interface
                    .methods
                    .iter()
                    .map(move |method| (*interface, method))
            })
            .filter(|(_, method)| !is_accessor(method))
            .collect::<Vec<_>>();
        let abi_names = methods
            .iter()
            .map(|(_, method)| abi_name(method))
            .collect::<HashSet<_>>();
        let clr_names = methods
            .iter()
            .map(|(_, method)| clr_name(method))
            .collect::<HashSet<_>>();
        for (interface, method) in methods {
            let abi = abi_name(method);
            entries.push(Entry {
                interface,
                method,
                scope,
                abi_key: suffix_group_key(&abi, &abi_names),
                clr_key: suffix_group_key(&clr_name(method), &clr_names),
                abi,
            });
        }
    }
    let dispatch_order = |members: &[usize]| {
        let mut ordered = members.to_vec();
        ordered.sort_by(|left, right| {
            cmp_python_dispatch_methods(entries[*left].method, entries[*right].method)
        });
        ordered
    };
    let equivalent = |left: usize, right: usize| {
        left == right || equivalent_overloads(entries[left].method, entries[right].method)
    };
    // The overloads a group dispatched to, without duplicates that dispatch
    // always shadowed (the same method on two interfaces).
    let reachable = |members: &[usize]| {
        let mut kept = Vec::<usize>::new();
        for index in dispatch_order(members) {
            if !kept.iter().any(|&other| equivalent(other, index)) {
                kept.push(index);
            }
        }
        kept
    };

    // Names emitted before CLR-name grouping, mapped to the group each reached.
    let mut previous_groups: Groups = vec![BTreeMap::new(); scopes.len()];
    for (index, entry) in entries.iter().enumerate() {
        previous_groups[entry.scope]
            .entry(entry.abi_key.clone())
            .or_default()
            .push(index);
    }
    let mut existing: Vec<BTreeMap<String, String>> = vec![BTreeMap::new(); scopes.len()];
    for (scope, groups) in previous_groups.iter().enumerate() {
        for key in groups.keys() {
            existing[scope].insert(key.clone(), key.clone());
        }
    }
    for entry in &entries {
        existing[entry.scope]
            .entry(entry.abi.clone())
            .or_insert_with(|| entry.abi_key.clone());
    }

    let mut fallback = BTreeSet::<(usize, String)>::new();
    let groups = loop {
        let key_of = |index: usize| -> &String {
            let entry = &entries[index];
            if fallback.contains(&(entry.scope, entry.clr_key.clone())) {
                &entry.abi_key
            } else {
                &entry.clr_key
            }
        };
        let mut groups: Groups = vec![BTreeMap::new(); scopes.len()];
        for (index, entry) in entries.iter().enumerate() {
            groups[entry.scope]
                .entry(key_of(index).clone())
                .or_default()
                .push(index);
        }
        let mut blamed = BTreeSet::new();
        let can_fall_back = |members: &[usize]| {
            members.iter().any(|&index| {
                !fallback.contains(&(entries[index].scope, entries[index].clr_key.clone()))
            })
        };
        let mut blame = |members: &[usize]| {
            for &index in members {
                let key = (entries[index].scope, entries[index].clr_key.clone());
                if !fallback.contains(&key) {
                    blamed.insert(key);
                }
            }
        };
        let covers_exact = |members: &[usize], index: usize| members.contains(&index);
        let covers_shape = |members: &[usize], index: usize| {
            members.iter().any(|&member| equivalent(member, index))
        };
        // Whether a new overload could take a call that reached `expected`.
        let takes_calls = |members: &[usize], expected: &[usize]| {
            members.iter().any(|&member| {
                !covers_shape(expected, member)
                    && expected.iter().any(|&index| {
                        overloads_may_overlap(entries[member].method, entries[index].method)
                    })
            })
        };
        for scope in 0..scopes.len() {
            for (name, previous_key) in &existing[scope] {
                let previous_members = &previous_groups[scope][previous_key];
                let expected = reachable(previous_members);
                if let Some(members) = groups[scope].get(name) {
                    // An existing public method keeps every overload it reached
                    // and gains none that could take its calls.
                    for &index in &expected {
                        if covers_exact(members, index) || covers_shape(members, index) {
                            continue;
                        }
                        if can_fall_back(members) {
                            blame(members);
                        } else {
                            // The name already kept its previous meaning; return
                            // the overload that moved to another CLR-name group.
                            blame(&groups[scope][key_of(index)]);
                        }
                    }
                    if takes_calls(members, &expected) {
                        blame(members);
                    }
                } else if previous_members.len() > 1 {
                    // A former dispatcher becomes an alias of the dispatcher that
                    // now owns all of its overloads.
                    let targets = expected
                        .iter()
                        .map(|&index| key_of(index))
                        .collect::<BTreeSet<_>>();
                    for target in &targets {
                        let members = &groups[scope][*target];
                        if targets.len() > 1 || takes_calls(members, &expected) {
                            blame(members);
                        }
                    }
                }
            }
            for (name, members) in &groups[scope] {
                if existing[scope].contains_key(name) {
                    continue;
                }
                let collides = reserved.contains(name)
                    || (0..scopes.len()).any(|other| {
                        other != scope
                            && (existing[other].contains_key(name)
                                || groups[other].contains_key(name))
                    });
                if collides {
                    blame(members);
                }
            }
        }
        if blamed.is_empty() {
            break groups;
        }
        fallback.extend(blamed);
    };

    // An existing name can now be the documented name of a different,
    // identically shaped interface method. Keep its old dispatcher exact by
    // prepending the methods it previously dispatched to, while the same
    // implementations remain available from their canonical CLR-name group.
    let mut effective_groups = groups.clone();
    let mut compatibility_dispatchers = BTreeSet::new();
    for scope in 0..scopes.len() {
        for (name, previous_key) in &existing[scope] {
            let Some(members) = groups[scope].get(name) else {
                continue;
            };
            let expected = reachable(&previous_groups[scope][previous_key]);
            let current = reachable(members);
            let needs_compatibility_dispatcher = expected.iter().any(|&index| {
                !current.contains(&index) && members.iter().any(|&member| equivalent(member, index))
            });
            if !needs_compatibility_dispatcher {
                continue;
            }
            let mut combined = dispatch_order(&previous_groups[scope][previous_key]);
            let extras = dispatch_order(members)
                .into_iter()
                .filter(|index| !combined.contains(index))
                .collect::<Vec<_>>();
            combined.extend(extras);
            effective_groups[scope].insert(name.clone(), combined);
            compatibility_dispatchers.insert((scope, name.clone()));
        }
    }

    let previous_attributes = |name: &str, members: &[usize]| {
        let ordered = dispatch_order(members);
        let names = if ordered.len() == 1 {
            vec![name.to_string()]
        } else {
            private_overload_names(name, ordered.iter().map(|&index| entries[index].method))
        };
        ordered.into_iter().zip(names).collect::<Vec<_>>()
    };
    (0..scopes.len())
        .map(|scope| {
            let primary_group_of = groups[scope]
                .iter()
                .flat_map(|(name, members)| members.iter().map(move |&index| (index, name.clone())))
                .collect::<HashMap<_, _>>();
            let borrowed = effective_groups[scope]
                .iter()
                .flat_map(|(name, members)| {
                    members
                        .iter()
                        .copied()
                        .filter(|index| primary_group_of[index] != *name)
                })
                .collect::<HashSet<_>>();
            let mut attribute_of = HashMap::new();
            for (name, members) in &groups[scope] {
                let ordered = dispatch_order(members);
                let names = if effective_groups[scope][name].len() == 1
                    && members.iter().all(|index| !borrowed.contains(index))
                {
                    vec![name.clone()]
                } else {
                    private_overload_names(name, ordered.iter().map(|&index| entries[index].method))
                };
                attribute_of.extend(ordered.into_iter().zip(names));
            }

            let mut ordered_groups = effective_groups[scope].iter().collect::<Vec<_>>();
            ordered_groups.sort_by_key(|(name, _)| groups[scope][*name][0]);
            let mut plan_groups = Vec::with_capacity(ordered_groups.len());
            let mut group_of = HashMap::new();
            for (position, (name, members)) in ordered_groups.into_iter().enumerate() {
                let ordered = if compatibility_dispatchers.contains(&(scope, name.clone())) {
                    members.clone()
                } else {
                    dispatch_order(members)
                };
                let candidates = ordered
                    .into_iter()
                    .map(|index| {
                        let entry = &entries[index];
                        let define = primary_group_of[&index] == *name;
                        if define {
                            group_of.insert(entry.method as *const MethodMeta, position);
                        }
                        Candidate {
                            interface: entry.interface,
                            method: entry.method,
                            attribute: attribute_of[&index].clone(),
                            define,
                        }
                    })
                    .collect::<Vec<_>>();
                let legacy_fallback = existing[scope].get(name).and_then(|previous_key| {
                    let previous = &previous_groups[scope][previous_key];
                    (previous.len() == 1 && candidates.len() > 1).then(|| {
                        let index = previous[0];
                        LegacyFallback {
                            method: entries[index].method,
                            attribute: attribute_of[&index].clone(),
                        }
                    })
                });
                plan_groups.push(MethodGroup {
                    name: name.clone(),
                    candidates,
                    legacy_fallback,
                });
            }
            let previous_attributes = previous_groups[scope]
                .iter()
                .flat_map(|(name, members)| previous_attributes(name, members))
                .map(|(index, attribute)| (entries[index].method as *const MethodMeta, attribute))
                .collect();
            let aliases = existing[scope]
                .iter()
                .filter(|(name, _)| !groups[scope].contains_key(*name))
                .map(|(name, previous_key)| {
                    let previous_members = &previous_groups[scope][previous_key];
                    // A former standalone method has no dispatch guards; bind
                    // its name to the exact implementation it called.
                    let target = if previous_members.len() == 1 {
                        attribute_of[&previous_members[0]].clone()
                    } else {
                        let expected = reachable(previous_members);
                        let entry = &entries[expected[0]];
                        plan_groups[group_of[&(entry.method as *const MethodMeta)]]
                            .name
                            .clone()
                    };
                    // Stubs keep the signatures each name declared before.
                    let signatures = if name == previous_key {
                        dispatch_order(previous_members)
                    } else {
                        previous_members
                            .iter()
                            .copied()
                            .filter(|&index| &entries[index].abi == name)
                            .collect()
                    };
                    Alias {
                        name: name.clone(),
                        target,
                        signatures: signatures
                            .into_iter()
                            .map(|index| entries[index].method)
                            .collect(),
                    }
                })
                .collect();
            ScopePlan {
                groups: plan_groups,
                group_of,
                aliases,
                previous_attributes,
                #[cfg(test)]
                fallbacks: fallback
                    .iter()
                    .filter(|(fallback_scope, key)| {
                        *fallback_scope == scope
                            && entries.iter().any(|entry| {
                                entry.scope == scope
                                    && &entry.clr_key == key
                                    && &entry.abi_key != key
                            })
                    })
                    .map(|(_, key)| key.clone())
                    .collect(),
            }
        })
        .collect()
}

fn outputs(method: &MethodMeta) -> Vec<&TypeMeta> {
    method
        .params
        .iter()
        .filter(|param| param.direction == ParamDirection::Out)
        .map(|param| &param.typ)
        .chain(method.return_type.as_ref())
        .collect()
}

/// Overloads that bind and behave identically, such as the same projected
/// method on two interfaces (`INumberFormatter.FormatInt` and `INumberFormatter2.FormatInt`).
fn equivalent_overloads(left: &MethodMeta, right: &MethodMeta) -> bool {
    let left_params = get_in_params(left);
    let right_params = get_in_params(right);
    left_params.len() == right_params.len()
        && left_params.iter().zip(&right_params).all(|(left, right)| {
            to_snake_case(&left.name) == to_snake_case(&right.name) && left.typ == right.typ
        })
        && outputs(left) == outputs(right)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GuardDomain {
    Bool,
    Number,
    Enum,
    Text,
    Guid,
    DateTime,
    TimeSpan,
    Struct,
    Sequence,
    Collection,
    Object,
}

fn guard_domain(typ: &TypeMeta) -> GuardDomain {
    match typ {
        TypeMeta::Bool => GuardDomain::Bool,
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64
        | TypeMeta::F32
        | TypeMeta::F64 => GuardDomain::Number,
        TypeMeta::Enum { .. } => GuardDomain::Enum,
        TypeMeta::Char16 | TypeMeta::String => GuardDomain::Text,
        TypeMeta::Guid => GuardDomain::Guid,
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => GuardDomain::DateTime,
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => GuardDomain::TimeSpan,
        TypeMeta::Struct { .. } => GuardDomain::Struct,
        TypeMeta::Array(_) => GuardDomain::Sequence,
        typ if type_kind(typ).is_some() => GuardDomain::Collection,
        TypeMeta::Object
        | TypeMeta::Interface { .. }
        | TypeMeta::RuntimeClass { .. }
        | TypeMeta::Delegate { .. }
        | TypeMeta::Parameterized { .. }
        | TypeMeta::AsyncAction
        | TypeMeta::AsyncActionWithProgress(_)
        | TypeMeta::AsyncOperation(_)
        | TypeMeta::AsyncOperationWithProgress(_, _) => GuardDomain::Object,
    }
}

/// Whether one value could satisfy the dispatch guards of both parameter types.
///
/// Conservative: returns `true` unless the guards are provably disjoint. Enums
/// may use an integer guard when their type is not projected, and projected
/// objects are sequences or collections when they wrap WinRT collections.
fn guard_types_may_overlap(left: &TypeMeta, right: &TypeMeta) -> bool {
    if left == right {
        return true;
    }
    match (ireference_inner_type(left), ireference_inner_type(right)) {
        (Some(_), Some(_)) => return true,
        (Some(inner), None) => return guard_types_may_overlap(inner, right),
        (None, Some(inner)) => return guard_types_may_overlap(left, inner),
        (None, None) => {}
    }
    use GuardDomain::*;
    match (guard_domain(left), guard_domain(right)) {
        (Struct, Struct) => false,
        (left, right) if left == right => true,
        (Number, Enum) | (Enum, Number) => true,
        (Sequence | Collection | Object, Sequence | Collection | Object) => true,
        _ => false,
    }
}

/// Whether some positional call could satisfy the guards of both overloads.
fn overloads_may_overlap(left: &MethodMeta, right: &MethodMeta) -> bool {
    let left_params = get_in_params(left);
    let right_params = get_in_params(right);
    left_params.len() == right_params.len()
        && left_params
            .iter()
            .zip(&right_params)
            .all(|(left, right)| guard_types_may_overlap(&left.typ, &right.typ))
}

pub(crate) fn cmp_python_dispatch_methods(left: &MethodMeta, right: &MethodMeta) -> Ordering {
    cmp_python_dispatch_params(&get_in_params(left), &get_in_params(right))
        .then_with(|| left.raw_name.cmp(&right.raw_name))
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.vtable_index.cmp(&right.vtable_index))
}

pub(crate) fn cmp_python_dispatch_params(left: &[&ParamMeta], right: &[&ParamMeta]) -> Ordering {
    let sort_key = |params: &[&ParamMeta]| {
        params
            .iter()
            .map(|param| py_dispatch_type_sort_key(&param.typ))
            .collect::<Vec<_>>()
    };
    sort_key(left).cmp(&sort_key(right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{ParamDirection, ParamMeta};
    use crate::types::TypeMeta;

    fn method(name: &str, vtable_index: usize, typ: TypeMeta) -> MethodMeta {
        overload(name, name, vtable_index, &[("value", typ)])
    }

    fn overload(
        name: &str,
        raw_name: &str,
        vtable_index: usize,
        params: &[(&str, TypeMeta)],
    ) -> MethodMeta {
        MethodMeta {
            name: name.into(),
            raw_name: raw_name.into(),
            vtable_index,
            params: params
                .iter()
                .map(|(name, typ)| ParamMeta {
                    name: (*name).into(),
                    typ: typ.clone(),
                    direction: ParamDirection::In,
                })
                .collect(),
            return_type: Some(TypeMeta::String),
            ..Default::default()
        }
    }

    fn interface_type(name: &str) -> TypeMeta {
        TypeMeta::Interface {
            namespace: "Contoso".into(),
            name: name.into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
        }
    }

    fn enumeration(name: &str) -> TypeMeta {
        TypeMeta::Enum {
            namespace: "Contoso".into(),
            name: name.into(),
            underlying: Box::new(TypeMeta::I32),
            members: Vec::new(),
            is_flags: false,
            doc: None,
            deprecated: None,
        }
    }

    fn interface(name: &str, methods: Vec<MethodMeta>) -> InterfaceMeta {
        InterfaceMeta {
            name: name.into(),
            namespace: "Contoso".into(),
            methods,
            ..Default::default()
        }
    }

    /// Owned view of a scope plan for assertions.
    #[derive(Debug, Default, PartialEq)]
    struct Planned {
        /// `(ABI name, vtable) -> (group name, attribute)`.
        methods: BTreeMap<(String, usize), (String, String)>,
        aliases: Vec<(String, String)>,
        fallbacks: Vec<String>,
        legacy_fallbacks: BTreeMap<String, (String, usize, String)>,
    }

    impl Planned {
        fn group(&self, name: &str, vtable_index: usize) -> &str {
            &self.methods[&(name.to_string(), vtable_index)].0
        }

        fn attribute(&self, name: &str, vtable_index: usize) -> &str {
            &self.methods[&(name.to_string(), vtable_index)].1
        }
    }

    fn summarize(plan: &ScopePlan<'_>) -> Planned {
        Planned {
            methods: plan
                .groups
                .iter()
                .flat_map(|group| {
                    group.candidates.iter().map(|candidate| {
                        (
                            (candidate.method.name.clone(), candidate.method.vtable_index),
                            (group.name.clone(), candidate.attribute.clone()),
                        )
                    })
                })
                .collect(),
            aliases: plan
                .aliases()
                .iter()
                .map(|alias| (alias.name.clone(), alias.target.clone()))
                .collect(),
            fallbacks: plan.fallbacks().to_vec(),
            legacy_fallbacks: plan
                .groups
                .iter()
                .filter_map(|group| {
                    group.legacy_fallback.as_ref().map(|fallback| {
                        (
                            group.name.clone(),
                            (
                                fallback.method.name.clone(),
                                fallback.method.vtable_index,
                                fallback.attribute.clone(),
                            ),
                        )
                    })
                })
                .collect(),
        }
    }

    fn plan_scope(methods: Vec<MethodMeta>, reserved: &[&str]) -> Planned {
        let widget = interface("IWidget", methods);
        let reserved = reserved.iter().map(|name| name.to_string()).collect();
        summarize(&plan_scopes(&[vec![&widget]], &reserved)[0])
    }

    fn aliases(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, target)| (name.to_string(), target.to_string()))
            .collect()
    }

    #[test]
    fn python_numeric_overload_method_cmp_prefers_narrower_and_signed_ranges() {
        let i8 = method("Read", 6, TypeMeta::I8);
        let u8 = method("Read2", 7, TypeMeta::U8);
        let i16 = method("Read3", 8, TypeMeta::I16);

        assert_eq!(cmp_python_dispatch_methods(&i8, &i16), Ordering::Less);
        assert_eq!(cmp_python_dispatch_methods(&i8, &u8), Ordering::Less);
    }

    #[test]
    fn python_numeric_overload_method_cmp_prefers_char16_integer_and_f64() {
        let char16 = method("Pick", 6, TypeMeta::Char16);
        let string = method("Pick2", 7, TypeMeta::String);
        let int = method("Pick3", 8, TypeMeta::I32);
        let f64 = method("Pick4", 9, TypeMeta::F64);
        let f32 = method("Pick5", 10, TypeMeta::F32);

        assert_eq!(
            cmp_python_dispatch_methods(&char16, &string),
            Ordering::Less
        );
        assert_eq!(cmp_python_dispatch_methods(&int, &f64), Ordering::Less);
        assert_eq!(cmp_python_dispatch_methods(&f64, &f32), Ordering::Less);
    }

    #[test]
    fn python_overload_suffixes_merge_only_when_base_method_exists() {
        let planned = plan_scope(
            vec![
                method("CreateFileAsync", 6, TypeMeta::String),
                method("CreateFileAsyncOverloadDefaultOptions", 7, TypeMeta::String),
                method("RunEventLoopWithOptions", 8, TypeMeta::String),
            ],
            &[],
        );

        assert_eq!(planned.group("CreateFileAsync", 6), "create_file_async");
        assert_eq!(
            planned.group("CreateFileAsyncOverloadDefaultOptions", 7),
            "create_file_async"
        );
        assert_eq!(
            planned.group("RunEventLoopWithOptions", 8),
            "run_event_loop_with_options"
        );
        assert_eq!(
            planned.aliases,
            aliases(&[(
                "create_file_async_overload_default_options",
                "create_file_async"
            )]),
            "a former alias keeps aliasing the dispatcher"
        );
    }

    #[test]
    fn plan_orders_candidates_for_dispatch_and_names_private_overloads() {
        let first = interface("IFirst", vec![method("Register", 6, TypeMeta::String)]);
        let second = interface("ISecond", vec![method("Register", 6, TypeMeta::I32)]);
        let registered = summarize(&plan_scopes(&[vec![&first, &second]], &HashSet::new())[0]);
        let values = registered.methods.values().collect::<Vec<_>>();
        assert_eq!(
            values.len(),
            1,
            "same ABI slot on two interfaces: {registered:?}"
        );

        let plans = plan_scopes(&[vec![&first, &second]], &HashSet::new());
        let group = &plans[0].groups[0];
        let attributes = group
            .candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.method.params[0].typ.clone(),
                    candidate.attribute.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            attributes,
            [
                (TypeMeta::String, "_register_6_0"),
                (TypeMeta::I32, "_register_6_1"),
            ]
        );

        let read = plan_scope(
            vec![
                method("Read2", 7, TypeMeta::F64),
                method("Read", 6, TypeMeta::I8),
            ],
            &[],
        );
        assert_eq!(read.group("Read2", 7), "read");
        assert_eq!(read.attribute("Read", 6), "_read_6");
        assert_eq!(read.attribute("Read2", 7), "_read_7");
    }

    #[test]
    fn clr_name_groups_overloads_without_a_documented_python_name() {
        let folder = interface_type("IStorageFolder");
        let planned = plan_scope(
            vec![
                overload(
                    "CopyOverloadDefaultNameAndOptions",
                    "CopyAsync",
                    8,
                    &[("destination_folder", folder.clone())],
                ),
                overload(
                    "CopyOverloadDefaultOptions",
                    "CopyAsync",
                    9,
                    &[
                        ("destination_folder", folder.clone()),
                        ("desired_new_name", TypeMeta::String),
                    ],
                ),
                overload(
                    "CopyOverload",
                    "CopyAsync",
                    10,
                    &[
                        ("destination_folder", folder),
                        ("desired_new_name", TypeMeta::String),
                        ("option", enumeration("NameCollisionOption")),
                    ],
                ),
            ],
            &[],
        );

        for vtable_index in 8..=10 {
            let name = [
                "CopyOverloadDefaultNameAndOptions",
                "CopyOverloadDefaultOptions",
                "CopyOverload",
            ][vtable_index - 8];
            assert_eq!(planned.group(name, vtable_index), "copy_async");
        }
        assert_eq!(
            planned.aliases,
            aliases(&[
                ("copy_overload", "_copy_async_10"),
                ("copy_overload_default_name_and_options", "_copy_async_8"),
                ("copy_overload_default_options", "_copy_async_9"),
            ]),
            "former standalone methods alias the exact implementation they called"
        );
        assert!(planned.fallbacks.is_empty());
    }

    #[test]
    fn clr_name_renames_single_overloads_and_keeps_the_abi_name_as_alias() {
        let planned = plan_scope(
            vec![overload(
                "LaunchUriWithDataAsync",
                "LaunchUriAsync",
                8,
                &[("uri", TypeMeta::String)],
            )],
            &[],
        );

        assert_eq!(
            planned.group("LaunchUriWithDataAsync", 8),
            "launch_uri_async"
        );
        assert_eq!(
            planned.attribute("LaunchUriWithDataAsync", 8),
            "launch_uri_async"
        );
        assert_eq!(
            planned.aliases,
            aliases(&[("launch_uri_with_data_async", "launch_uri_async")])
        );
    }

    #[test]
    fn clr_name_keeps_real_methods_that_share_an_overload_name() {
        let format_int = interface(
            "INumberFormatter",
            vec![
                overload("FormatInt", "Format", 6, &[("value", TypeMeta::I64)]),
                overload("FormatUInt", "Format", 7, &[("value", TypeMeta::U64)]),
            ],
        );
        let real = interface(
            "INumberFormatter2",
            vec![
                overload("FormatInt", "FormatInt", 6, &[("value", TypeMeta::I64)]),
                overload("FormatUInt", "FormatUInt", 7, &[("value", TypeMeta::U64)]),
            ],
        );
        let plans = plan_scopes(&[vec![&format_int, &real]], &HashSet::new());
        let planned = summarize(&plans[0]);

        let groups = plans[0]
            .groups
            .iter()
            .map(|group| {
                (
                    group.name.as_str(),
                    group
                        .candidates
                        .iter()
                        .map(|candidate| candidate.interface.name.as_str())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            groups,
            [
                ("format", vec!["INumberFormatter", "INumberFormatter"]),
                ("format_int", vec!["INumberFormatter", "INumberFormatter2"]),
                (
                    "format_u_int",
                    vec!["INumberFormatter", "INumberFormatter2"]
                ),
            ]
        );
        for (name, canonical_attribute, real_attribute) in [
            ("format_int", "_format_6", "_format_int_6"),
            ("format_u_int", "_format_7", "_format_u_int_7"),
        ] {
            let group = plans[0]
                .groups
                .iter()
                .find(|group| group.name == name)
                .unwrap();
            assert_eq!(
                group
                    .candidates
                    .iter()
                    .map(|candidate| (candidate.attribute.as_str(), candidate.define))
                    .collect::<Vec<_>>(),
                [(canonical_attribute, false), (real_attribute, true)]
            );
        }
        assert!(planned.aliases.is_empty(), "{planned:?}");
        assert!(planned.fallbacks.is_empty(), "{planned:?}");
    }

    #[test]
    fn compatibility_dispatcher_keeps_the_exact_previously_selected_interface() {
        let first = interface(
            "IFirst",
            vec![overload(
                "Pick",
                "Choose",
                6,
                &[("value", TypeMeta::String)],
            )],
        );
        let second = interface(
            "ISecond",
            vec![overload("Pick", "Pick", 6, &[("value", TypeMeta::String)])],
        );
        let plans = plan_scopes(&[vec![&first, &second]], &HashSet::new());
        let choose = plans[0]
            .groups
            .iter()
            .find(|group| group.name == "choose")
            .unwrap();
        let pick = plans[0]
            .groups
            .iter()
            .find(|group| group.name == "pick")
            .unwrap();

        assert_eq!(
            choose
                .candidates
                .iter()
                .map(|candidate| {
                    (
                        candidate.interface.name.as_str(),
                        candidate.attribute.as_str(),
                        candidate.define,
                    )
                })
                .collect::<Vec<_>>(),
            [("IFirst", "_choose_6", true)]
        );
        assert_eq!(
            pick.candidates
                .iter()
                .map(|candidate| {
                    (
                        candidate.interface.name.as_str(),
                        candidate.attribute.as_str(),
                        candidate.define,
                    )
                })
                .collect::<Vec<_>>(),
            [("IFirst", "_choose_6", false), ("ISecond", "_pick_6", true),],
            "pick() must still call IFirst first, while ISecond.Pick remains projected"
        );
    }

    #[test]
    fn compatibility_dispatcher_pins_exact_target_within_a_canonical_group() {
        let first = interface(
            "IFirst",
            vec![overload("Foo", "Foo", 6, &[("value", TypeMeta::String)])],
        );
        let second = interface(
            "ISecond",
            vec![overload("Bar", "Foo", 6, &[("value", TypeMeta::String)])],
        );
        let plans = plan_scopes(&[vec![&first, &second]], &HashSet::new());
        let foo = plans[0]
            .groups
            .iter()
            .find(|group| group.name == "foo")
            .unwrap();

        assert_eq!(
            foo.candidates
                .iter()
                .map(|candidate| candidate.interface.name.as_str())
                .collect::<Vec<_>>(),
            ["IFirst", "ISecond"],
            "Foo must keep the exact IFirst target first even though Bar sorts before Foo"
        );
    }

    #[test]
    fn clr_name_does_not_split_distinct_overloads_of_an_existing_name() {
        // Same shape as INumberFormatter/INumberFormatter2, but the overloads
        // differ (parameter names), so `format_int` must keep reaching both.
        let clr = interface(
            "IFirst",
            vec![
                overload("FormatInt", "Format", 6, &[("value", TypeMeta::I64)]),
                overload("FormatDouble", "Format", 7, &[("value", TypeMeta::F64)]),
            ],
        );
        let abi = interface(
            "ISecond",
            vec![overload(
                "FormatInt",
                "FormatInt",
                6,
                &[("number", TypeMeta::I64)],
            )],
        );
        let plans = plan_scopes(&[vec![&clr, &abi]], &HashSet::new());
        let planned = summarize(&plans[0]);

        assert_eq!(planned.group("FormatDouble", 7), "format_double");
        let format_int = plans[0]
            .groups
            .iter()
            .find(|group| group.name == "format_int")
            .expect("format_int group");
        let interfaces = format_int
            .candidates
            .iter()
            .map(|candidate| candidate.interface.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(interfaces, ["IFirst", "ISecond"], "{planned:?}");
        assert_eq!(planned.fallbacks, ["format"]);
    }

    #[test]
    fn clr_name_grouping_keeps_existing_suffix_merges() {
        let planned = plan_scope(
            vec![
                overload("Read", "Read", 6, &[("value", TypeMeta::String)]),
                overload("Read2", "Read2", 7, &[("value", TypeMeta::I32)]),
                overload("ReadWithHint", "Read", 8, &[("value", TypeMeta::Bool)]),
            ],
            &[],
        );

        assert_eq!(planned.group("Read2", 7), "read");
        assert_eq!(planned.group("ReadWithHint", 8), "read");
        assert_eq!(planned.attribute("Read2", 7), "_read_7");
        assert_eq!(
            planned.aliases,
            aliases(&[("read2", "read"), ("read_with_hint", "_read_8")])
        );
    }

    #[test]
    fn clr_name_falls_back_when_the_name_is_a_property_or_generated_member() {
        let getter = MethodMeta {
            name: "get_Source".into(),
            raw_name: "get_Source".into(),
            vtable_index: 8,
            return_type: Some(TypeMeta::String),
            is_property_getter: true,
            ..Default::default()
        };
        let mut reserved = HashSet::from(["close".to_string()]);
        insert_accessor_names(&getter, false, &mut reserved);
        let widget = interface(
            "IWidget",
            vec![
                overload("CloseWithStatus", "Close", 6, &[("code", TypeMeta::U16)]),
                overload(
                    "SetSourceWithOptions",
                    "Source",
                    7,
                    &[("value", TypeMeta::String)],
                ),
                getter,
            ],
        );
        let plans = plan_scopes(&[vec![&widget]], &reserved);
        let planned = summarize(&plans[0]);

        assert_eq!(planned.group("CloseWithStatus", 6), "close_with_status");
        assert_eq!(
            planned.group("SetSourceWithOptions", 7),
            "set_source_with_options"
        );
        assert!(planned.aliases.is_empty());
        assert_eq!(planned.fallbacks, ["close", "source"]);
        let members = plans[0].members(widget.methods.iter().map(|method| (&widget, method)));
        assert!(
            matches!(members[2], PlannedMember::Accessor(_, method) if method.name == "get_Source")
        );
    }

    #[test]
    fn clr_name_falls_back_when_the_other_scope_owns_the_name() {
        let statics = interface(
            "IWidgetStatics",
            vec![
                overload("CopyFromAsync", "CopyAsync", 7, &[("value", TypeMeta::I32)]),
                overload("MergeWith", "Merge", 8, &[("value", TypeMeta::I32)]),
            ],
        );
        let instance = interface(
            "IWidget",
            vec![
                overload("CopyAsync", "CopyAsync", 6, &[("value", TypeMeta::String)]),
                overload("Merge", "Merge", 9, &[("value", TypeMeta::String)]),
            ],
        );
        let plans = plan_scopes(&[vec![&statics], vec![&instance]], &HashSet::new());
        let static_plan = summarize(&plans[0]);
        let instance_plan = summarize(&plans[1]);

        assert_eq!(static_plan.group("CopyFromAsync", 7), "copy_from_async");
        assert_eq!(static_plan.group("MergeWith", 8), "merge_with");
        assert_eq!(instance_plan.group("CopyAsync", 6), "copy_async");
        assert_eq!(instance_plan.group("Merge", 9), "merge");
        assert_eq!(static_plan.fallbacks, ["copy_async", "merge"]);
        assert!(static_plan.aliases.is_empty());
    }

    #[test]
    fn clr_name_falls_back_when_an_existing_name_would_change_meaning() {
        // `CreateUpdater()` is an overload name of `CreateUpdaterForUser`, while
        // `CreateUpdater(String)` is named `CreateUpdaterWithId`.
        let planned = plan_scope(
            vec![
                overload("CreateUpdater", "CreateUpdaterForUser", 6, &[]),
                overload(
                    "CreateUpdaterWithId",
                    "CreateUpdater",
                    7,
                    &[("id", TypeMeta::String)],
                ),
            ],
            &[],
        );

        assert_eq!(planned.group("CreateUpdater", 6), "create_updater_for_user");
        assert_eq!(
            planned.group("CreateUpdaterWithId", 7),
            "create_updater_with_id"
        );
        assert_eq!(
            planned.aliases,
            aliases(&[("create_updater", "create_updater_for_user")])
        );
        assert_eq!(planned.fallbacks, ["create_updater"]);
    }

    #[test]
    fn clr_name_falls_back_when_a_new_overload_could_take_existing_calls() {
        let by_interface = overload("Show", "Show", 6, &[("target", interface_type("ITarget"))]);
        let by_text = overload(
            "ShowText",
            "Show",
            8,
            &[("target", TypeMeta::String), ("mode", TypeMeta::I32)],
        );
        let planned = plan_scope(
            vec![
                by_interface.clone(),
                overload("ShowObject", "Show", 7, &[("target", TypeMeta::Object)]),
                by_text.clone(),
            ],
            &[],
        );
        assert_eq!(planned.group("Show", 6), "show");
        assert_eq!(planned.group("ShowObject", 7), "show_object");
        assert_eq!(planned.group("ShowText", 8), "show_text");
        assert_eq!(planned.fallbacks, ["show"]);

        let planned = plan_scope(
            vec![
                by_interface,
                overload("ShowKind", "Show", 9, &[("target", enumeration("Kind"))]),
                by_text,
            ],
            &[],
        );
        assert_eq!(planned.group("ShowKind", 9), "show");
        assert_eq!(planned.group("ShowText", 8), "show");
        assert_eq!(
            planned.aliases,
            aliases(&[("show_kind", "_show_9"), ("show_text", "_show_8")])
        );
        assert_eq!(
            planned.legacy_fallbacks["show"],
            ("Show".to_string(), 6, "_show_6".to_string())
        );
    }

    #[test]
    fn former_dispatchers_alias_the_dispatcher_that_owns_their_overloads() {
        let planned = plan_scope(
            vec![
                overload(
                    "TryUpdatePosition",
                    "TryUpdatePosition",
                    6,
                    &[("value", TypeMeta::F32)],
                ),
                overload(
                    "TryUpdatePositionWithOption",
                    "TryUpdatePosition",
                    7,
                    &[
                        ("value", TypeMeta::F32),
                        ("option", enumeration("Clamping")),
                    ],
                ),
                overload(
                    "TryUpdatePositionWithOption",
                    "TryUpdatePosition",
                    8,
                    &[
                        ("value", TypeMeta::F32),
                        ("option", enumeration("Clamping")),
                        ("update", enumeration("Update")),
                    ],
                ),
            ],
            &[],
        );

        assert_eq!(
            planned.group("TryUpdatePositionWithOption", 8),
            "try_update_position"
        );
        assert_eq!(
            planned.aliases,
            aliases(&[("try_update_position_with_option", "try_update_position")])
        );
    }

    #[test]
    fn previous_attributes_keep_pre_clr_names_for_ordering() {
        let widget = interface(
            "IWidget",
            vec![
                overload("CreateWithName", "Create", 6, &[("name", TypeMeta::String)]),
                overload("Create", "Create", 7, &[]),
            ],
        );
        let plan = &plan_scopes(&[vec![&widget]], &HashSet::new())[0];

        assert_eq!(plan.attribute(&widget.methods[0]), Some("_create_6"));
        assert_eq!(
            plan.previous_attribute(&widget.methods[0]),
            Some("create_with_name")
        );
        assert_eq!(plan.previous_attribute(&widget.methods[1]), Some("create"));
    }

    #[test]
    fn windows_corpus_marks_every_new_dispatcher_with_its_old_standalone_method() {
        use crate::codegen::winrt::python::naming::PythonProjectionContext;
        use crate::meta;
        use std::path::Path;

        const WINMD: &str =
            r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
        if !Path::new(WINMD).is_file() {
            eprintln!("Skipping: Windows.winmd not found");
            return;
        }

        fn assert_scope(interfaces: &[&InterfaceMeta], plan: &ScopePlan<'_>) -> usize {
            let methods = interfaces
                .iter()
                .flat_map(|interface| interface.methods.iter())
                .filter(|method| !is_accessor(method))
                .collect::<Vec<_>>();
            let names = methods.iter().map(|method| abi_name(method)).collect();
            let mut previous = BTreeMap::<String, Vec<&MethodMeta>>::new();
            for method in methods {
                previous
                    .entry(suffix_group_key(&abi_name(method), &names))
                    .or_default()
                    .push(method);
            }

            let mut count = 0;
            for group in &plan.groups {
                let Some(methods) = previous.get(&group.name) else {
                    continue;
                };
                if methods.len() != 1 || group.candidates.len() <= 1 {
                    continue;
                }
                let fallback = group.legacy_fallback.as_ref().unwrap_or_else(|| {
                    panic!("{} became a dispatcher without a legacy tier", group.name)
                });
                assert!(
                    std::ptr::eq(fallback.method, methods[0]),
                    "{} legacy tier changed its native method",
                    group.name
                );
                count += 1;
            }
            count
        }

        let context = PythonProjectionContext::default();
        let mut runtime_count = 0;
        let mut all_plan_sites = 0;
        for namespace in meta::list_namespaces(WINMD) {
            for class in meta::parse_namespace(WINMD, &namespace) {
                let statics = class
                    .factory_interfaces
                    .iter()
                    .chain(class.static_interfaces.iter())
                    .collect::<Vec<_>>();
                let instance = class_instance_interfaces(&class).collect::<Vec<_>>();
                let plan = ClassMemberPlan::new(&class, &context);
                let count =
                    assert_scope(&statics, &plan.statics) + assert_scope(&instance, &plan.instance);
                runtime_count += count;
                all_plan_sites += count;
                for interface in &class.required_interfaces {
                    all_plan_sites += assert_scope(&[interface], &interface_member_plan(interface));
                }
            }
            for interface in meta::parse_interfaces(WINMD, &namespace) {
                if !interface.is_delegate() {
                    all_plan_sites +=
                        assert_scope(&[&interface], &interface_member_plan(&interface));
                }
            }
        }

        assert_eq!(runtime_count, 766);
        assert_eq!(all_plan_sites, 897);
    }
}
