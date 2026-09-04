// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Runtime class constructor projection.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::codegen::winrt::extensions::winui;
use crate::meta::{ConstructorKind, ConstructorMeta, InterfaceMeta, MethodMeta, ParamMeta};
use crate::types::TypeMeta;

use super::*;
use crate::codegen::winrt::javascript::signature::ref_marker;

struct ConstructorCandidate {
    params: Vec<ProjectedParam>,
    param_types: Vec<TypeMeta>,
    call_expr: String,
}

pub(super) fn default_activation_method_name(class: &ClassMeta) -> &'static str {
    let has_create_factory = class.factory_interfaces.iter().any(|iface| {
        iface.methods.iter().any(|method| {
            let name = to_camel_case(&method.name);
            name == "create" || name.starts_with("create")
        })
    });
    if has_create_factory {
        "createDefault"
    } else {
        "create"
    }
}

pub(super) fn project_constructor(
    context: &JavaScriptProjectionContext,
    class: &ClassMeta,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> ProjectedConstructor {
    if winui::is_application(class) {
        return inaccessible_constructor(class);
    }

    let mut candidates = Vec::new();
    for constructor in &class.constructors {
        match constructor.kind {
            ConstructorKind::DefaultActivation => {
                candidates.push(ConstructorCandidate {
                    params: Vec::new(),
                    param_types: Vec::new(),
                    call_expr: format!(
                        "{}.{}()",
                        class.name,
                        default_activation_method_name(class)
                    ),
                });
            }
            ConstructorKind::FactoryActivation => {
                let Some(factory) = find_factory(class, constructor) else {
                    continue;
                };
                for method in &factory.methods {
                    if !method_constructs_class(method, class) {
                        continue;
                    }
                    let in_params = get_in_params(method);
                    candidates.push(factory_candidate(
                        context,
                        class,
                        method,
                        &in_params,
                        &in_params,
                        None,
                        known_types,
                        delegate_names,
                        delegate_sigs,
                        delegate_param_wraps,
                    ));
                }
            }
            ConstructorKind::PublicComposition => {
                let Some(factory) = find_factory(class, constructor) else {
                    continue;
                };
                for method in &factory.methods {
                    if !method_constructs_class(method, class) {
                        continue;
                    }
                    let in_params = get_in_params(method);
                    let Some((outer_index, public_params)) =
                        split_composable_params(method, &in_params)
                    else {
                        continue;
                    };
                    candidates.push(factory_candidate(
                        context,
                        class,
                        method,
                        &public_params,
                        &in_params,
                        Some(outer_index),
                        known_types,
                        delegate_names,
                        delegate_sigs,
                        delegate_param_wraps,
                    ));
                }
            }
            ConstructorKind::ProtectedComposition => {}
        }
    }

    let candidates = deduplicate_candidates(candidates);
    let supported = supported_candidate_indices(&candidates);
    if supported.is_empty() {
        return inaccessible_constructor(class);
    }

    let overloads = supported
        .iter()
        .map(|&index| candidates[index].params.clone())
        .collect();
    let body_lines = constructor_dispatch_body(class, &candidates, &supported);
    ProjectedConstructor {
        overloads,
        body_lines,
    }
}

fn inaccessible_constructor(class: &ClassMeta) -> ProjectedConstructor {
    ProjectedConstructor {
        overloads: Vec::new(),
        body_lines: vec![format!(
            "throw new TypeError('{} cannot be constructed directly.');",
            class.name
        )],
    }
}

fn find_factory<'a>(
    class: &'a ClassMeta,
    constructor: &ConstructorMeta,
) -> Option<&'a InterfaceMeta> {
    let reference = constructor.factory_interface.as_ref()?;
    class.factory_interfaces.iter().find(|interface| {
        interface.namespace == reference.namespace && interface.name == reference.name
    })
}

fn method_constructs_class(method: &MethodMeta, class: &ClassMeta) -> bool {
    matches!(
        method.return_type.as_ref(),
        Some(TypeMeta::RuntimeClass {
            namespace,
            name,
            ..
        }) if namespace == &class.namespace && name == &class.name
    )
}

fn split_composable_params<'a>(
    method: &MethodMeta,
    in_params: &[&'a ParamMeta],
) -> Option<(usize, Vec<&'a ParamMeta>)> {
    // MIDL composable factories append baseInterface and innerInterface to the public parameters.
    let outer = *in_params.last()?;
    let outer_name = outer.name.to_ascii_lowercase();
    let is_outer_name = outer_name == "outer"
        || outer_name == "base"
        || outer_name == "baseinterface"
        || outer_name == "outerinterface";
    let has_inner_output = method.params.iter().any(|param| {
        param.direction == ParamDirection::Out
            && matches!(param.typ, TypeMeta::Object)
            && param.name.to_ascii_lowercase().contains("inner")
    });
    if !is_outer_name || !matches!(outer.typ, TypeMeta::Object) || !has_inner_output {
        return None;
    }

    Some((
        in_params.len() - 1,
        in_params[..in_params.len() - 1].to_vec(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn factory_candidate(
    context: &JavaScriptProjectionContext,
    class: &ClassMeta,
    method: &MethodMeta,
    public_params: &[&ParamMeta],
    in_params: &[&ParamMeta],
    outer_index: Option<usize>,
    known_types: &HashSet<String>,
    delegate_names: &HashSet<String>,
    delegate_sigs: &HashMap<String, String>,
    delegate_param_wraps: &HashMap<String, Vec<String>>,
) -> ConstructorCandidate {
    ConstructorCandidate {
        params: project_params(
            context,
            public_params,
            known_types,
            delegate_names,
            delegate_sigs,
            delegate_param_wraps,
        ),
        param_types: public_params
            .iter()
            .map(|param| param.typ.clone())
            .collect(),
        call_expr: factory_call_expr(class, method, in_params, outer_index),
    }
}

fn factory_call_expr(
    class: &ClassMeta,
    method: &MethodMeta,
    in_params: &[&ParamMeta],
    outer_index: Option<usize>,
) -> String {
    let mut public_index = 0;
    let args = in_params
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if outer_index == Some(index) {
                "null".to_string()
            } else {
                let arg = format!("args[{}]", public_index);
                public_index += 1;
                arg
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}.{}({})", class.name, to_camel_case(&method.name), args)
}

fn deduplicate_candidates(candidates: Vec<ConstructorCandidate>) -> Vec<ConstructorCandidate> {
    let mut signatures = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            signatures.insert(
                candidate
                    .params
                    .iter()
                    .map(|param| param.ts_type.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn supported_candidate_indices(candidates: &[ConstructorCandidate]) -> Vec<usize> {
    let mut by_arity: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        by_arity
            .entry(candidate.params.len())
            .or_default()
            .push(index);
    }

    let mut supported = Vec::new();
    for indices in by_arity.values() {
        if indices.len() == 1 {
            supported.push(indices[0]);
            continue;
        }

        let mut predicate_counts = HashMap::new();
        for &index in indices {
            let predicate = candidate_type_predicate(&candidates[index]);
            *predicate_counts.entry(predicate).or_insert(0usize) += 1;
        }
        for &index in indices {
            let predicate = candidate_type_predicate(&candidates[index]);
            if !predicate.is_empty() && predicate_counts[&predicate] == 1 {
                supported.push(index);
            }
        }
    }
    supported
}

fn constructor_dispatch_body(
    class: &ClassMeta,
    candidates: &[ConstructorCandidate],
    supported: &[usize],
) -> Vec<String> {
    let mut indices = supported.to_vec();
    indices.sort_by_key(|&index| {
        let candidate = &candidates[index];
        (
            candidate.params.len(),
            std::cmp::Reverse(predicate_specificity(candidate)),
        )
    });

    let mut arity_counts = HashMap::new();
    for candidate in candidates {
        *arity_counts.entry(candidate.params.len()).or_insert(0usize) += 1;
    }

    let mut body = Vec::new();
    for index in indices {
        let candidate = &candidates[index];
        let mut conditions = vec![format!("args.length === {}", candidate.params.len())];
        if arity_counts[&candidate.params.len()] > 1 {
            conditions.extend(candidate_type_conditions(candidate));
        }
        body.push(format!("if ({}) {{", conditions.join(" && ")));
        body.push(format!("    this._obj = {}._obj;", candidate.call_expr));
        body.push("    return;".into());
        body.push("}".into());
    }
    body.push(format!(
        "throw new TypeError('No matching constructor for {}.');",
        class.name
    ));
    body
}

fn candidate_type_predicate(candidate: &ConstructorCandidate) -> String {
    candidate_type_conditions(candidate).join(" && ")
}

fn candidate_type_conditions(candidate: &ConstructorCandidate) -> Vec<String> {
    candidate
        .param_types
        .iter()
        .enumerate()
        .filter_map(|(index, typ)| js_type_condition(index, typ))
        .collect()
}

fn predicate_specificity(candidate: &ConstructorCandidate) -> usize {
    candidate
        .param_types
        .iter()
        .map(|typ| match typ {
            TypeMeta::RuntimeClass { .. } | TypeMeta::Array(_) => 3,
            TypeMeta::String
            | TypeMeta::Bool
            | TypeMeta::I8
            | TypeMeta::U8
            | TypeMeta::I16
            | TypeMeta::U16
            | TypeMeta::I32
            | TypeMeta::U32
            | TypeMeta::F32
            | TypeMeta::F64
            | TypeMeta::Char16
            | TypeMeta::I64
            | TypeMeta::U64
            | TypeMeta::Enum { .. }
            | TypeMeta::Delegate { .. } => 2,
            _ => 1,
        })
        .sum()
}

fn js_type_condition(index: usize, typ: &TypeMeta) -> Option<String> {
    let arg = format!("args[{}]", index);
    match typ {
        TypeMeta::String => Some(format!("typeof {} === 'string'", arg)),
        TypeMeta::Bool => Some(format!("typeof {} === 'boolean'", arg)),
        TypeMeta::I64 | TypeMeta::U64 => Some(format!("typeof {} === 'bigint'", arg)),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::F32
        | TypeMeta::F64
        | TypeMeta::Char16
        | TypeMeta::Enum { .. } => Some(format!("typeof {} === 'number'", arg)),
        TypeMeta::RuntimeClass { name, .. } => {
            Some(format!("{} instanceof {}", arg, ref_marker(name)))
        }
        TypeMeta::Array(_) => Some(format!("Array.isArray({})", arg)),
        TypeMeta::Delegate { .. } => Some(format!("typeof {} === 'function'", arg)),
        TypeMeta::Object
        | TypeMeta::Interface { .. }
        | TypeMeta::Guid
        | TypeMeta::Struct { .. }
        | TypeMeta::Parameterized { .. } => Some(format!(
            "({0} === null || (typeof {0} === 'object' && !Array.isArray({0})))",
            arg
        )),
        TypeMeta::AsyncAction
        | TypeMeta::AsyncActionWithProgress(_)
        | TypeMeta::AsyncOperation(_)
        | TypeMeta::AsyncOperationWithProgress(_, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(ts_type: &str, typ: TypeMeta) -> ConstructorCandidate {
        ConstructorCandidate {
            params: vec![ProjectedParam {
                name: "value".into(),
                ts_type: ts_type.into(),
                optional: false,
                delegate_wrap: None,
            }],
            param_types: vec![typ],
            call_expr: String::new(),
        }
    }

    #[test]
    fn same_arity_primitive_overloads_are_distinguished() {
        let candidates = vec![
            candidate("string", TypeMeta::String),
            candidate("boolean", TypeMeta::Bool),
        ];
        assert_eq!(supported_candidate_indices(&candidates), [0, 1]);
    }

    #[test]
    fn indistinguishable_overloads_are_not_exposed() {
        let candidates = vec![
            candidate("FirstEnum", TypeMeta::I32),
            candidate("SecondEnum", TypeMeta::U32),
        ];
        assert!(supported_candidate_indices(&candidates).is_empty());
    }

    #[test]
    fn runtime_class_dispatch_uses_lazy_reference_marker() {
        assert_eq!(
            js_type_condition(
                0,
                &TypeMeta::RuntimeClass {
                    namespace: "Contoso".into(),
                    name: "Widget".into(),
                    default_interface: None,
                }
            ),
            Some("args[0] instanceof __DWRT_REF__Widget__".into())
        );
    }
}
