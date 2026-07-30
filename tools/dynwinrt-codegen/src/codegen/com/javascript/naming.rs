// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

pub(in crate::codegen::com) fn camel_case(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = name.chars().collect();
    let mut run = 0usize;
    while run < chars.len() && chars[run].is_ascii_uppercase() {
        run += 1;
    }
    let mut result = String::with_capacity(name.len());
    if run == 0 {
        return name.to_string();
    }
    if run == chars.len() {
        for c in &chars {
            result.push(c.to_ascii_lowercase());
        }
        return result;
    }
    if run == 1 {
        result.push(chars[0].to_ascii_lowercase());
        for c in &chars[1..] {
            result.push(*c);
        }
        return result;
    }
    for c in &chars[..run - 1] {
        result.push(c.to_ascii_lowercase());
    }
    for c in &chars[run - 1..] {
        result.push(*c);
    }
    result
}

pub(super) fn js_param_name(raw: &str, index: usize) -> String {
    let base = if raw.is_empty() {
        format!("arg{}", index)
    } else {
        raw.to_string()
    };
    let stripped = strip_hungarian(&base);
    let mut out = String::with_capacity(stripped.len());
    let mut chars = stripped.chars();
    if let Some(first) = chars.next() {
        out.push(first.to_ascii_lowercase());
    }
    for c in chars {
        out.push(c);
    }
    match out.as_str() {
        "class" | "return" | "function" | "default" | "this" | "new" | "delete" | "let"
        | "const" | "var" | "if" | "else" | "for" | "while" | "do" | "switch" | "case"
        | "break" | "continue" | "true" | "false" | "null" | "undefined" | "in" | "of"
        | "typeof" | "instanceof" | "throw" | "try" | "catch" | "finally" | "yield" | "async"
        | "await" | "with" | "void" | "public" | "private" | "protected" | "package" | "static"
        | "import" | "export" | "extends" | "super" | "arguments" => {
            format!("{}_", out)
        }
        _ => out,
    }
}

pub(super) fn strip_hungarian(s: &str) -> &str {
    let prefixes = [
        "lpwsz", "pwsz", "lpsz", "psz", "lpsz", "pwstr", "pcwstr", "hwnd", "dw", "sz", "cb", "cx",
        "cy", "cw", "ch", "cn", "cc", "lp", "np", "ph", "pd", "pf", "pv", "ppv", "pp", "wsz",
    ];
    for p in prefixes {
        if let Some(rest) = s.strip_prefix(p) {
            if rest
                .chars()
                .next()
                .map(|c| c.is_ascii_uppercase())
                .unwrap_or(false)
            {
                return rest;
            }
        }
    }
    s
}
