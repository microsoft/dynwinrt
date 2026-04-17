// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Naming / identifier helpers (case conversion, reserved word handling).

pub(crate) fn to_camel_case(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap().to_lowercase().to_string();
    let result = format!("{}{}", first, chars.collect::<String>());
    // Avoid JS reserved words / strict-mode restricted identifiers
    if is_js_reserved(&result) {
        format!("{}_", result)
    } else {
        result
    }
}

fn is_js_reserved(s: &str) -> bool {
    matches!(s,
        // Keywords & strict-mode restricted identifiers
        "arguments" | "eval" | "break" | "case" | "catch" | "class" | "const"
        | "continue" | "debugger" | "default" | "delete" | "do" | "else"
        | "enum" | "export" | "extends" | "false" | "finally" | "for"
        | "function" | "if" | "import" | "in" | "instanceof" | "let"
        | "new" | "null" | "return" | "super" | "switch" | "this"
        | "throw" | "true" | "try" | "typeof" | "undefined" | "var"
        | "void" | "while" | "with" | "yield"
        // Strict-mode future reserved words
        | "implements" | "interface" | "package" | "private" | "protected"
        | "public" | "static"
    )
}

pub(crate) fn capitalize(s: &str) -> String {
    if s.is_empty() { return String::new(); }
    let mut chars = s.chars();
    let first = chars.next().unwrap().to_uppercase().to_string();
    format!("{}{}", first, chars.collect::<String>())
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
            // Insert underscore before an uppercase letter when:
            // - It's not the first character, AND
            // - The previous character is lowercase, OR
            // - The next character exists and is lowercase (handles "IID" -> "iid" but "IIDComponent" -> "iid_component")
            if i > 0 {
                let prev_lower_or_digit = chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit();
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
    let r = result.trim_start_matches('_').to_string();
    if is_py_reserved(&r) {
        format!("{}_", r)
    } else {
        r
    }
}

pub(crate) fn is_py_reserved(s: &str) -> bool {
    matches!(s,
        "False" | "True" | "None" | "and" | "as" | "assert" | "async" | "await"
        | "break" | "class" | "continue" | "def" | "del" | "elif" | "else"
        | "except" | "finally" | "for" | "from" | "global" | "if" | "import"
        | "in" | "is" | "lambda" | "nonlocal" | "not" | "or" | "pass"
        | "raise" | "return" | "try" | "while" | "with" | "yield"
    )
}

/// Convert a PascalCase name to a snake_case Python filename (without extension).
pub fn to_snake_case_filename(name: &str) -> String {
    to_snake_case(name)
}
