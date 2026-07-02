// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Format normalized XML doc text into JSDoc (for TypeScript) or Google-style
//! Python docstrings. Produces EMPTY output when no doc fields are present,
//! preserving byte-identity for metadata without sibling .xml files.

use std::collections::HashMap;

/// Input for JSDoc / pydoc formatting: a normalized summary plus optional
/// parameter / return / deprecation text.
#[derive(Default)]
pub struct DocText<'a> {
    pub summary: Option<&'a str>,
    pub deprecated: Option<&'a str>,
    pub returns: Option<&'a str>,
    /// Pairs of (param_display_name, doc_text). Order preserved.
    pub params: Vec<(&'a str, &'a str)>,
}

impl<'a> DocText<'a> {
    pub fn is_empty(&self) -> bool {
        self.summary.is_none()
            && self.deprecated.is_none()
            && self.returns.is_none()
            && self.params.is_empty()
    }
}

/// Lookup helper: resolve a raw param name to its doc, ignoring case.
/// Used by call sites that need to translate codegen display names back to
/// raw names.
pub fn find_param_doc<'a>(
    param_docs: &'a HashMap<String, String>,
    raw_name: &str,
) -> Option<&'a str> {
    if let Some(v) = param_docs.get(raw_name) {
        return Some(v.as_str());
    }
    for (k, v) in param_docs.iter() {
        if k.eq_ignore_ascii_case(raw_name) {
            return Some(v.as_str());
        }
    }
    None
}

/// Escape `*/` sequences so a JSDoc block comment cannot terminate early.
fn escape_jsdoc(s: &str) -> String {
    s.replace("*/", "*\\/")
}

/// Escape `"""` so a Python triple-quoted string cannot terminate early.
fn escape_pydoc(s: &str) -> String {
    s.replace("\"\"\"", "\\\"\\\"\\\"")
}

/// Emit a JSDoc block comment prefixed by `indent` (e.g. "  ").
/// Returns an empty string when there is nothing to document.
pub fn format_jsdoc(doc: &DocText, indent: &str) -> String {
    if doc.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(indent);
    out.push_str("/**\n");

    // Summary
    if let Some(s) = doc.summary {
        write_block(&mut out, &escape_jsdoc(s), indent);
    }

    // @deprecated
    if let Some(d) = doc.deprecated {
        if doc.summary.is_some() {
            out.push_str(indent);
            out.push_str(" *\n");
        }
        out.push_str(indent);
        out.push_str(" * @deprecated ");
        out.push_str(&escape_jsdoc(d).replace('\n', " "));
        out.push('\n');
    }

    // @param
    for (name, text) in doc.params.iter() {
        out.push_str(indent);
        out.push_str(" * @param ");
        out.push_str(name);
        out.push(' ');
        out.push_str(&escape_jsdoc(text).replace('\n', " "));
        out.push('\n');
    }

    // @returns
    if let Some(r) = doc.returns {
        out.push_str(indent);
        out.push_str(" * @returns ");
        out.push_str(&escape_jsdoc(r).replace('\n', " "));
        out.push('\n');
    }

    out.push_str(indent);
    out.push_str(" */\n");
    out
}

fn write_block(out: &mut String, text: &str, indent: &str) {
    for line in text.split('\n') {
        out.push_str(indent);
        if line.is_empty() {
            out.push_str(" *\n");
        } else {
            out.push_str(" * ");
            out.push_str(line);
            out.push('\n');
        }
    }
}

/// Emit a Google-style Python docstring (triple-quoted) at the given indent.
/// Returns an empty string when there is nothing to document.
pub fn format_pydoc(doc: &DocText, indent: &str) -> String {
    if doc.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(indent);
    out.push_str("\"\"\"");

    if let Some(s) = doc.summary {
        let escaped = escape_pydoc(s);
        let mut lines = escaped.split('\n');
        if let Some(first) = lines.next() {
            out.push_str(first);
        }
        for line in lines {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(indent);
                out.push_str(line);
            }
        }
    }

    if !doc.params.is_empty() {
        out.push_str("\n\n");
        out.push_str(indent);
        out.push_str("Args:\n");
        for (name, text) in doc.params.iter() {
            out.push_str(indent);
            out.push_str("    ");
            out.push_str(name);
            out.push_str(": ");
            out.push_str(&escape_pydoc(text).replace('\n', " "));
            out.push('\n');
        }
        // remove trailing newline to keep sections tight; we'll add one back if needed
        if out.ends_with('\n') {
            out.pop();
        }
    }

    if let Some(r) = doc.returns {
        out.push_str("\n\n");
        out.push_str(indent);
        out.push_str("Returns:\n");
        out.push_str(indent);
        out.push_str("    ");
        out.push_str(&escape_pydoc(r).replace('\n', " "));
    }

    if let Some(d) = doc.deprecated {
        out.push_str("\n\n");
        out.push_str(indent);
        out.push_str(".. deprecated::\n");
        out.push_str(indent);
        out.push_str("    ");
        out.push_str(&escape_pydoc(d).replace('\n', " "));
    }

    out.push_str("\n");
    out.push_str(indent);
    out.push_str("\"\"\"\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_doc_returns_empty() {
        let d = DocText::default();
        assert_eq!(format_jsdoc(&d, "  "), "");
        assert_eq!(format_pydoc(&d, "    "), "");
    }

    #[test]
    fn jsdoc_summary_only() {
        let d = DocText {
            summary: Some("Hi."),
            ..Default::default()
        };
        let s = format_jsdoc(&d, "  ");
        assert_eq!(s, "  /**\n   * Hi.\n   */\n");
    }

    #[test]
    fn jsdoc_escapes_terminator() {
        let d = DocText {
            summary: Some("use a*/b"),
            ..Default::default()
        };
        let s = format_jsdoc(&d, "");
        assert!(s.contains("a*\\/b"), "got: {}", s);
        assert!(!s.contains("a*/b"));
    }

    #[test]
    fn pydoc_escapes_triple_quote() {
        let d = DocText {
            summary: Some("see \"\"\" end"),
            ..Default::default()
        };
        let s = format_pydoc(&d, "    ");
        assert!(s.contains("\\\"\\\"\\\""), "got: {}", s);
    }

    #[test]
    fn jsdoc_with_params_and_returns() {
        let d = DocText {
            summary: Some("Does it."),
            params: vec![("x", "the x"), ("y", "the y")],
            returns: Some("a value"),
            deprecated: Some("use bar()"),
            ..Default::default()
        };
        let s = format_jsdoc(&d, "");
        assert!(s.contains("@param x the x"));
        assert!(s.contains("@param y the y"));
        assert!(s.contains("@returns a value"));
        assert!(s.contains("@deprecated use bar()"));
    }
}
