// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::codegen::winrt::shared::imports::get_in_params;
use crate::meta::{MethodMeta, ParamMeta};
use std::cmp::Ordering;
use std::collections::HashSet;

use super::naming::to_snake_case;
use super::signature::py_dispatch_type_sort_key;

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
}
