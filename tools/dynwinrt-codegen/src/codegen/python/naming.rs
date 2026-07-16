// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python naming and identifier helpers.

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
    to_snake_case(name)
}
