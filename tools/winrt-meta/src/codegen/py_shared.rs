// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Shared Python codegen helpers used by both `python` (implementation) and
//! `python_stub` (.pyi stub) emitters.

use crate::meta::MethodMeta;

/// Reorder methods so that property getters always come before their matching setters.
/// Python requires `@property` to appear before `@prop.setter`.
pub(crate) fn reorder_getters_before_setters(methods: &[MethodMeta]) -> Vec<&MethodMeta> {
    let mut getters: Vec<&MethodMeta> = Vec::new();
    let mut setters: Vec<&MethodMeta> = Vec::new();
    let mut others: Vec<&MethodMeta> = Vec::new();
    for m in methods {
        if m.is_property_getter {
            getters.push(m);
        } else if m.is_property_setter {
            setters.push(m);
        } else {
            others.push(m);
        }
    }
    let mut result = Vec::with_capacity(methods.len());
    for g in &getters {
        result.push(*g);
        let getter_prop = g.name.strip_prefix("get_").unwrap_or(&g.name);
        if let Some(pos) = setters.iter().position(|s| {
            s.name.strip_prefix("put_").unwrap_or(&s.name) == getter_prop
        }) {
            result.push(setters.remove(pos));
        }
    }
    result.extend(setters);
    result.extend(others);
    result
}
