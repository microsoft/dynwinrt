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
//! Interface implementation handlers keep one name per ABI slot and do not use
//! this module.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::codegen::winrt::shared::imports::get_in_params;
use crate::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamMeta};

use super::naming::to_snake_case;
use super::signature::py_dispatch_type_sort_key;

const ICLOSABLE_IID: &str = "30d5a829-7fa4-4026-83bb-d75bae4ea99e";

/// One overload of a planned method group.
pub(crate) struct Candidate<'a> {
    pub(crate) interface: &'a InterfaceMeta,
    pub(crate) method: &'a MethodMeta,
    /// Attribute implementing this overload: the group name when it is the only
    /// candidate, otherwise its private dispatch name.
    pub(crate) attribute: String,
}

/// Methods projected as one Python method, in dispatch order.
pub(crate) struct MethodGroup<'a> {
    pub(crate) name: String,
    pub(crate) candidates: Vec<Candidate<'a>>,
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

    /// Previously emitted names kept as aliases, sorted by name.
    pub(crate) fn aliases(&self) -> &[Alias<'a>] {
        &self.aliases
    }
}

/// Member plans for a runtime class; its static and instance members share
/// one Python class namespace.
pub(crate) struct ClassMemberPlan<'a> {
    pub(crate) statics: ScopePlan<'a>,
    pub(crate) instance: ScopePlan<'a>,
}

impl<'a> ClassMemberPlan<'a> {
    pub(crate) fn new(class: &'a ClassMeta) -> Self {
        let statics = class
            .factory_interfaces
            .iter()
            .chain(class.static_interfaces.iter())
            .collect();
        let instance = class_instance_interfaces(class).collect();
        let mut scopes = plan_scopes(&[statics, instance]).into_iter();
        Self {
            statics: scopes.next().expect("static scope"),
            instance: scopes.next().expect("instance scope"),
        }
    }
}

/// Member plan for an interface wrapper class.
pub(crate) fn interface_member_plan(interface: &InterfaceMeta) -> ScopePlan<'_> {
    plan_scopes(&[vec![interface]])
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

/// Snake-case ABI name of a method.
fn abi_name(method: &MethodMeta) -> String {
    to_snake_case(&method.name)
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

/// Plan scopes that share one Python class namespace.
fn plan_scopes<'a>(scopes: &[Vec<&'a InterfaceMeta>]) -> Vec<ScopePlan<'a>> {
    scopes
        .iter()
        .map(|interfaces| {
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
            let names = methods
                .iter()
                .map(|(_, method)| abi_name(method))
                .collect::<HashSet<_>>();
            let keys = methods
                .iter()
                .map(|(_, method)| suffix_group_key(&abi_name(method), &names))
                .collect::<Vec<_>>();
            build_scope(&methods, &keys)
        })
        .collect()
}

/// Build a scope plan from methods (in scope order) and their group keys.
fn build_scope<'a>(
    methods: &[(&'a InterfaceMeta, &'a MethodMeta)],
    keys: &[String],
) -> ScopePlan<'a> {
    let mut grouped: Vec<(String, Vec<(&'a InterfaceMeta, &'a MethodMeta)>)> = Vec::new();
    for (&(interface, method), key) in methods.iter().zip(keys) {
        match grouped.iter_mut().find(|(name, _)| name == key) {
            Some((_, members)) => members.push((interface, method)),
            None => grouped.push((key.clone(), vec![(interface, method)])),
        }
    }
    let mut groups = Vec::with_capacity(grouped.len());
    let mut group_of = HashMap::new();
    for (index, (name, mut members)) in grouped.into_iter().enumerate() {
        members.sort_by(|(_, left), (_, right)| cmp_python_dispatch_methods(left, right));
        let attributes = if members.len() == 1 {
            vec![name.clone()]
        } else {
            private_overload_names(&name, members.iter().map(|(_, method)| *method))
        };
        let candidates = members
            .into_iter()
            .zip(attributes)
            .map(|((interface, method), attribute)| {
                group_of.insert(method as *const MethodMeta, index);
                Candidate {
                    interface,
                    method,
                    attribute,
                }
            })
            .collect();
        groups.push(MethodGroup { name, candidates });
    }

    let canonical = keys.iter().collect::<HashSet<_>>();
    let mut targets = BTreeMap::new();
    for (&(_, method), key) in methods.iter().zip(keys) {
        let name = abi_name(method);
        if &name != key && !canonical.contains(&name) {
            targets.entry(name).or_insert_with(|| key.clone());
        }
    }
    let aliases = targets
        .into_iter()
        .map(|(name, target)| Alias {
            signatures: methods
                .iter()
                .filter(|(_, method)| abi_name(method) == name)
                .map(|(_, method)| *method)
                .collect(),
            name,
            target,
        })
        .collect();
    ScopePlan {
        groups,
        group_of,
        aliases,
    }
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
        MethodMeta {
            name: name.into(),
            raw_name: name.into(),
            vtable_index,
            params: vec![ParamMeta {
                name: "value".into(),
                typ,
                direction: ParamDirection::In,
            }],
            ..Default::default()
        }
    }

    fn interface(methods: Vec<MethodMeta>) -> InterfaceMeta {
        InterfaceMeta {
            name: "IWidget".into(),
            namespace: "Contoso".into(),
            methods,
            ..Default::default()
        }
    }

    fn group_names(plan: &ScopePlan<'_>, interface: &InterfaceMeta) -> Vec<String> {
        plan.members(interface.methods.iter().map(|method| (interface, method)))
            .into_iter()
            .map(|member| match member {
                PlannedMember::Group(group) => group.name.clone(),
                PlannedMember::Accessor(_, method) => method.name.clone(),
            })
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
        let widget = interface(vec![
            method("CreateFileAsync", 6, TypeMeta::String),
            method("CreateFileAsyncOverloadDefaultOptions", 7, TypeMeta::String),
            method("RunEventLoopWithOptions", 8, TypeMeta::String),
        ]);
        let plan = interface_member_plan(&widget);

        assert_eq!(
            group_names(&plan, &widget),
            ["create_file_async", "run_event_loop_with_options"]
        );
        let aliases = plan
            .aliases()
            .iter()
            .map(|alias| (alias.name.as_str(), alias.target.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            aliases,
            [(
                "create_file_async_overload_default_options",
                "create_file_async"
            )]
        );
        assert_eq!(
            plan.attribute(&widget.methods[1]),
            Some("_create_file_async_7")
        );
        assert_eq!(
            plan.attribute(&widget.methods[2]),
            Some("run_event_loop_with_options")
        );
    }

    #[test]
    fn plan_orders_candidates_for_dispatch_and_names_private_overloads() {
        let first = interface(vec![method("Register", 6, TypeMeta::String)]);
        let second = interface(vec![method("Register", 6, TypeMeta::I32)]);
        let widget = interface(vec![
            method("Read2", 7, TypeMeta::F64),
            method("Read", 6, TypeMeta::I8),
        ]);
        let mut plans = plan_scopes(&[vec![&first, &second], vec![&widget]]).into_iter();
        let registered = plans.next().unwrap();
        let read = plans.next().unwrap();

        let PlannedMember::Group(group) =
            &registered.members([(&first, &first.methods[0]), (&second, &second.methods[0])])[0]
        else {
            panic!("expected a method group");
        };
        let attributes = group
            .candidates
            .iter()
            .map(|candidate| candidate.attribute.as_str())
            .collect::<Vec<_>>();
        assert_eq!(attributes, ["_register_6_0", "_register_6_1"]);
        assert_eq!(group.candidates[0].method.params[0].typ, TypeMeta::String);

        let PlannedMember::Group(group) = &read.members([(&widget, &widget.methods[0])])[0] else {
            panic!("expected a method group");
        };
        assert_eq!(group.name, "read");
        let order = group
            .candidates
            .iter()
            .map(|candidate| candidate.method.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(order, ["Read", "Read2"]);
    }
}
