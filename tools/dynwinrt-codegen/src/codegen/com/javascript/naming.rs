// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

pub(in crate::codegen::com) fn camel_case(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    lower_leading_acronym(name)
}

/// Lowercases a leading run of uppercase ASCII letters the way common
/// camelCase acronym conventions expect: a whole-string acronym (e.g. `URL`)
/// lowercases entirely; a single leading capital (e.g. `Set...`) just
/// lowercases that one letter; and a multi-letter acronym followed by a new
/// word (e.g. `MDIWindow`, `IOHandle`) lowercases all but the run's last
/// letter, since that last letter starts the next word (`mdiWindow`,
/// `ioHandle`).
fn lower_leading_acronym(name: &str) -> String {
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
    // Use the same acronym-run-aware lowering as method names (`camel_case`)
    // so a Hungarian-prefixed acronym remainder like `hwndMDI` -> `MDI` casts
    // down to `mdi`, not a naive first-letter-only `mDI`.
    let out = lower_leading_acronym(stripped);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_param_name_lowercases_a_whole_leading_acronym_after_stripping_hungarian() {
        // Regression test for ITaskbarList3::SetTabActive(HWND hwndTab, HWND
        // hwndMDI, DWORD dwReserved): the Hungarian-stripped remainder `MDI`
        // is a whole acronym, so it must lowercase entirely (`mdi`), not just
        // its first letter (`mDI`).
        assert_eq!(js_param_name("hwndMDI", 0), "mdi");
        // A single leading capital still lowercases just that letter.
        assert_eq!(js_param_name("hwndTab", 0), "tab");
        // An acronym followed by a new word keeps the word capitalized.
        assert_eq!(js_param_name("IOHandle", 0), "ioHandle");
    }
}
