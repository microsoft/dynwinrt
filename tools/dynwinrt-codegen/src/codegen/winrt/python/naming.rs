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
    MODULE_LAYOUT.with(|layout| {
        layout
            .borrow()
            .as_ref()
            .and_then(|layout| layout.unique_modules.get(name).cloned())
            .unwrap_or_else(|| to_snake_case(name))
    })
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
    fn module_layout_collision_detection_uses_normalized_names() {
        let err = install_python_module_layout([
            PythonTypeIdentity {
                namespace: "Example".into(),
                name: "UInt32".into(),
            },
            PythonTypeIdentity {
                namespace: "Example".into(),
                name: "Uint32".into(),
            },
        ])
        .err()
        .expect("normalized module name collision should fail");

        assert!(err.contains("Example.UInt32"), "{err}");
        assert!(err.contains("Example.Uint32"), "{err}");
        assert!(err.contains("example__uint32.py"), "{err}");
    }
}
