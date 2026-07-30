// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! JSDoc rendering for generated JavaScript declarations.

use crate::codegen::winrt::shared::docs::DocText;

/// Escape `*/` sequences so a JSDoc block comment cannot terminate early.
fn escape_jsdoc(s: &str) -> String {
    s.replace("*/", "*\\/")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_doc_returns_empty() {
        assert_eq!(format_jsdoc(&DocText::default(), "  "), "");
    }

    #[test]
    fn summary_only() {
        let doc = DocText {
            summary: Some("Hi."),
            ..Default::default()
        };
        assert_eq!(format_jsdoc(&doc, "  "), "  /**\n   * Hi.\n   */\n");
    }

    #[test]
    fn escapes_terminator() {
        let doc = DocText {
            summary: Some("use a*/b"),
            ..Default::default()
        };
        let output = format_jsdoc(&doc, "");
        assert!(output.contains("a*\\/b"));
        assert!(!output.contains("a*/b"));
    }

    #[test]
    fn includes_params_returns_and_deprecation() {
        let doc = DocText {
            summary: Some("Does it."),
            params: vec![("x", "the x"), ("y", "the y")],
            returns: Some("a value"),
            deprecated: Some("use bar()"),
        };
        let output = format_jsdoc(&doc, "");
        assert!(output.contains("@param x the x"));
        assert!(output.contains("@param y the y"));
        assert!(output.contains("@returns a value"));
        assert!(output.contains("@deprecated use bar()"));
    }
}
