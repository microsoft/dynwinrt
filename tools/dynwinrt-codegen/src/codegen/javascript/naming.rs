// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! JavaScript naming and identifier helpers.

pub(crate) fn to_camel_case(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    // Sanitize WinRT-only method names that begin with '.' (e.g. `.ctor`).
    // These are constructor slots on delegate / activation-factory interfaces
    // and are re-exposed via other JS APIs. If they leak through here as-is,
    // they emit invalid identifiers like `.ctor()` in the generated class.
    let s = s.strip_prefix('.').unwrap_or(s);
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
    matches!(
        s,
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
    if s.is_empty() {
        return String::new();
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap().to_uppercase().to_string();
    format!("{}{}", first, chars.collect::<String>())
}

/// Heuristic mapping from RHS expression to TS type for `export const X = ...;`.
/// Covers all patterns our generator emits.
pub(crate) fn infer_const_type(name: &str, rhs: &str) -> String {
    if rhs.starts_with("WinGuid.parse(") {
        return "WinGuid".into();
    }
    if rhs.starts_with("DynWinRtType.") {
        if rhs.contains(".iid()") {
            return "WinGuid".into();
        }
        return "DynWinRtType".into();
    }
    if rhs.starts_with('[') {
        return "DynWinRtType[]".into();
    }
    if rhs.starts_with("DynWinRtMethodSig") {
        return "DynWinRtMethodSig".into();
    }
    if name.ends_with("_PARAM_TYPES") {
        return "DynWinRtType[]".into();
    }
    if name.starts_with("IID_") {
        return "WinGuid".into();
    }
    if name.ends_with("_Type") {
        return "DynWinRtType".into();
    }
    "any".into()
}
