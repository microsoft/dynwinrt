// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::meta::MethodMeta;
use std::collections::HashSet;

use super::naming::to_snake_case;

pub(crate) fn grouped_methods<'a>(
    methods: impl IntoIterator<Item = &'a MethodMeta>,
) -> Vec<Vec<&'a MethodMeta>> {
    let methods = methods.into_iter().collect::<Vec<_>>();
    let names = method_names(methods.iter().copied());
    let mut groups: Vec<(String, Vec<&MethodMeta>)> = Vec::new();
    for method in methods {
        let key = method_group_key(method, &names);
        if let Some((_, group)) = groups.iter_mut().find(|(name, _)| name == &key) {
            group.push(method);
        } else {
            groups.push((key, vec![method]));
        }
    }
    groups.into_iter().map(|(_, methods)| methods).collect()
}

pub(crate) fn method_names<'a>(
    methods: impl IntoIterator<Item = &'a MethodMeta>,
) -> HashSet<String> {
    methods
        .into_iter()
        .filter(|method| {
            !method.is_property_getter
                && !method.is_property_setter
                && !method.is_event_add
                && !method.is_event_remove
        })
        .map(|method| to_snake_case(&method.name))
        .collect()
}

pub(crate) fn method_group_key(method: &MethodMeta, names: &HashSet<String>) -> String {
    if method.is_property_getter
        || method.is_property_setter
        || method.is_event_add
        || method.is_event_remove
    {
        format!("{}#{}", method.name, method.vtable_index)
    } else {
        let name = to_snake_case(&method.name);
        let base = name.trim_end_matches(|character: char| character.is_ascii_digit());
        if base.len() < name.len() && !base.is_empty() && names.contains(base) {
            base.to_string()
        } else {
            name
        }
    }
}
