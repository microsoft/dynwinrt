// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Docstring rendering for generated Python bindings.

use crate::codegen::winrt::shared::docs::DocText;

/// Escape `"""` so a Python triple-quoted string cannot terminate early.
fn escape_pydoc(s: &str) -> String {
    s.replace("\"\"\"", "\\\"\\\"\\\"")
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
        assert_eq!(format_pydoc(&DocText::default(), "    "), "");
    }

    #[test]
    fn escapes_triple_quote() {
        let doc = DocText {
            summary: Some("see \"\"\" end"),
            ..Default::default()
        };
        assert!(format_pydoc(&doc, "    ").contains("\\\"\\\"\\\""));
    }
}
