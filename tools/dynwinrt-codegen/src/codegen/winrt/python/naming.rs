// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python naming and identifier helpers.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PythonTypeIdentity {
    pub namespace: String,
    pub name: String,
}

#[derive(Default)]
struct PythonModuleLayout {
    modules: HashMap<PythonTypeIdentity, String>,
    unique_modules: HashMap<String, String>,
}

thread_local! {
    static MODULE_LAYOUT: RefCell<Option<PythonModuleLayout>> = const { RefCell::new(None) };
}

pub struct PythonModuleLayoutGuard;

impl Drop for PythonModuleLayoutGuard {
    fn drop(&mut self) {
        MODULE_LAYOUT.with(|layout| {
            *layout.borrow_mut() = None;
        });
    }
}

pub fn install_python_module_layout(
    identities: impl IntoIterator<Item = PythonTypeIdentity>,
) -> Result<PythonModuleLayoutGuard, String> {
    let identities: HashSet<_> = identities.into_iter().collect();
    let mut counts = HashMap::<String, usize>::new();
    for identity in &identities {
        *counts.entry(identity.name.clone()).or_default() += 1;
    }

    let mut modules = HashMap::new();
    let mut unique_modules = HashMap::new();
    let mut owners = HashMap::<String, PythonTypeIdentity>::new();
    for identity in identities {
        let namespace = identity
            .namespace
            .split('.')
            .filter(|segment| !segment.is_empty())
            .map(to_snake_case)
            .collect::<Vec<_>>()
            .join("__");
        let module = if namespace.is_empty() {
            to_snake_case(&identity.name)
        } else {
            format!("{namespace}__{}", to_snake_case(&identity.name))
        };
        if let Some(existing) = owners.insert(module.clone(), identity.clone()) {
            return Err(format!(
                "Python module name collision: `{}.{}` and `{}.{}` both normalize to `{module}.py`",
                existing.namespace, existing.name, identity.namespace, identity.name
            ));
        }
        if counts[&identity.name] == 1 {
            unique_modules.insert(identity.name.clone(), module.clone());
        }
        modules.insert(identity, module);
    }

    MODULE_LAYOUT.with(|layout| {
        *layout.borrow_mut() = Some(PythonModuleLayout {
            modules,
            unique_modules,
        });
    });
    Ok(PythonModuleLayoutGuard)
}

pub fn python_module_name(namespace: &str, name: &str) -> String {
    MODULE_LAYOUT.with(|layout| {
        layout
            .borrow()
            .as_ref()
            .and_then(|layout| {
                layout
                    .modules
                    .get(&PythonTypeIdentity {
                        namespace: namespace.to_string(),
                        name: name.to_string(),
                    })
                    .cloned()
            })
            .unwrap_or_else(|| to_snake_case(name))
    })
}

pub fn python_namespace_segments(namespace: &str) -> Vec<String> {
    namespace
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(to_snake_case)
        .collect()
}

pub fn python_public_module_name(name: &str) -> String {
    to_snake_case(name)
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
    let result = result.trim_start_matches('_').to_string();
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
    MODULE_LAYOUT.with(|layout| {
        layout
            .borrow()
            .as_ref()
            .and_then(|layout| layout.unique_modules.get(name).cloned())
            .unwrap_or_else(|| to_snake_case(name))
    })
}
