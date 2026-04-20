// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Parse .NET-style XML documentation files (sibling to `.winmd`).
//! The result is a [`DocTable`] keyed by fully-qualified member names
//! (`T:NS.Class`, `M:NS.Class.Method(sig)`, `P:NS.Class.Prop`, etc.).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Doc text extracted for a single member of a WinMD XML doc file.
#[derive(Debug, Clone, Default)]
pub struct MemberDoc {
    pub summary: Option<String>,
    pub remarks: Option<String>,
    pub returns: Option<String>,
    pub deprecated: Option<String>,
    /// Per-parameter doc, keyed by raw param name.
    pub param_docs: HashMap<String, String>,
}

/// In-memory store of all XML docs parsed from sibling `*.xml` files.
#[derive(Debug, Default)]
pub struct DocTable {
    /// All entries, keyed by the full member name verbatim from XML
    /// (e.g. `T:NS.Class`, `M:NS.Class.Method(sig)`).
    members: HashMap<String, MemberDoc>,
}

impl DocTable {
    /// Scan each winmd path for a sibling `.xml` file and parse all members.
    /// Missing or unparseable files are silently skipped.
    pub fn load_from_winmd_paths(winmd_paths: &[String]) -> Self {
        let mut table = DocTable::default();
        for wp in winmd_paths {
            let wp_path = Path::new(wp);
            let xml_path = wp_path.with_extension("xml");
            if !xml_path.exists() {
                continue;
            }
            let text = match fs::read_to_string(&xml_path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            table.ingest_xml(&text);
        }
        table
    }

    /// Load built-in API docs shipped with the codegen binary.
    /// Uses "insert if absent" semantics so sibling .xml docs take priority.
    pub fn load_builtin_docs(&mut self) {
        static BUILTIN_DOCS: &[(&str, &str)] = &[
            ("Microsoft.Graphics.Imaging", include_str!("../api-docs/Microsoft.Graphics.Imaging.xml")),
            ("Microsoft.Windows.AI", include_str!("../api-docs/Microsoft.Windows.AI.xml")),
            ("Microsoft.Windows.AI.Contentmoderation", include_str!("../api-docs/Microsoft.Windows.AI.Contentmoderation.xml")),
            ("Microsoft.Windows.AI.ContentSafety", include_str!("../api-docs/Microsoft.Windows.AI.ContentSafety.xml")),
            ("Microsoft.Windows.AI.Foundation", include_str!("../api-docs/Microsoft.Windows.AI.Foundation.xml")),
            ("Microsoft.Windows.AI.Generative", include_str!("../api-docs/Microsoft.Windows.AI.Generative.xml")),
            ("Microsoft.Windows.AI.Imaging", include_str!("../api-docs/Microsoft.Windows.AI.Imaging.xml")),
            ("Microsoft.Windows.AI.Machinelearning", include_str!("../api-docs/Microsoft.Windows.AI.Machinelearning.xml")),
            ("Microsoft.Windows.AI.Text", include_str!("../api-docs/Microsoft.Windows.AI.Text.xml")),
            ("Microsoft.Windows.Vision", include_str!("../api-docs/Microsoft.Windows.Vision.xml")),
            ("Microsoft.Windows.Workloads", include_str!("../api-docs/Microsoft.Windows.Workloads.xml")),
        ];
        for (_ns, xml_text) in BUILTIN_DOCS {
            self.ingest_xml_if_absent(xml_text);
        }
    }

    /// Parse XML doc content and merge into the table, but only insert entries
    /// that do not already exist. This allows sibling .xml to take priority.
    pub fn ingest_xml_if_absent(&mut self, xml_text: &str) {
        let doc = match roxmltree::Document::parse(xml_text) {
            Ok(d) => d,
            Err(_) => return,
        };
        for members_node in doc.descendants().filter(|n| n.has_tag_name("members")) {
            for m in members_node.children().filter(|n| n.is_element() && n.has_tag_name("member")) {
                let name = match m.attribute("name") {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if self.members.contains_key(&name) {
                    continue; // sibling .xml already has this entry
                }
                let mut md = MemberDoc::default();
                for child in m.children().filter(|n| n.is_element()) {
                    match child.tag_name().name() {
                        "summary" => md.summary = Some(normalize(child)),
                        "remarks" => md.remarks = Some(normalize(child)),
                        "returns" => md.returns = Some(normalize(child)),
                        "deprecated" => md.deprecated = Some(normalize(child)),
                        "param" => {
                            if let Some(pname) = child.attribute("name") {
                                md.param_docs.insert(pname.to_string(), normalize(child));
                            }
                        }
                        _ => {}
                    }
                }
                self.members.insert(name, md);
            }
        }
    }

    /// Parse XML doc content and merge into the table. Public for tests.
    pub fn ingest_xml(&mut self, xml_text: &str) {
        let doc = match roxmltree::Document::parse(xml_text) {
            Ok(d) => d,
            Err(_) => return,
        };
        for members_node in doc.descendants().filter(|n| n.has_tag_name("members")) {
            for m in members_node.children().filter(|n| n.is_element() && n.has_tag_name("member")) {
                let name = match m.attribute("name") {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let mut md = MemberDoc::default();
                for child in m.children().filter(|n| n.is_element()) {
                    match child.tag_name().name() {
                        "summary" => md.summary = Some(normalize(child)),
                        "remarks" => md.remarks = Some(normalize(child)),
                        "returns" => md.returns = Some(normalize(child)),
                        "deprecated" => md.deprecated = Some(normalize(child)),
                        "param" => {
                            if let Some(pname) = child.attribute("name") {
                                md.param_docs.insert(pname.to_string(), normalize(child));
                            }
                        }
                        _ => {}
                    }
                }
                self.members.insert(name, md);
            }
        }
    }

    pub fn lookup_type(&self, full_name: &str) -> Option<&MemberDoc> {
        self.members.get(&format!("T:{}", full_name))
    }

    pub fn lookup_method(&self, full_name: &str, sig_key: &str) -> Option<&MemberDoc> {
        // sig_key is `()` or `(T1,T2)`; XML uses `()` omitted sometimes.
        if sig_key == "()" {
            self.members
                .get(&format!("M:{}", full_name))
                .or_else(|| self.members.get(&format!("M:{}()", full_name)))
        } else {
            self.members.get(&format!("M:{}{}", full_name, sig_key))
        }
        // Fallback: prefix match for methods whose signatures differ due to
        // generic/parameterized type encoding (e.g. IVectorView`1 vs IVectorView{T}).
        // Only used when exact match fails and there is exactly one candidate.
        .or_else(|| {
            let prefix = format!("M:{}(", full_name);
            let candidates: Vec<_> = self.members.iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .collect();
            if candidates.len() == 1 {
                Some(candidates[0].1)
            } else {
                // Also try without signature at all
                self.members.get(&format!("M:{}", full_name))
            }
        })
    }

    pub fn lookup_property(&self, full_name: &str) -> Option<&MemberDoc> {
        self.members.get(&format!("P:{}", full_name))
    }

    pub fn lookup_field(&self, full_name: &str) -> Option<&MemberDoc> {
        self.members.get(&format!("F:{}", full_name))
    }

    pub fn lookup_event(&self, full_name: &str) -> Option<&MemberDoc> {
        self.members.get(&format!("E:{}", full_name))
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Populate `doc`, `deprecated`, `returns_doc`, and `param_docs` on all
    /// classes, interfaces, and their methods from XML entries. No-op if the
    /// table is empty. Safe to call on types parsed from Windows.winmd which
    /// has no sibling .xml file (all doc fields remain None).
    /// Apply docs to an interface using an explicit owner name (typically the runtime class name),
    /// because WinRT XML doc members are keyed by class, not by interface.
    pub fn apply_to_interface_as(&self, iface: &mut crate::meta::InterfaceMeta, owner_ns: &str, owner_name: &str) {
        if self.is_empty() {
            return;
        }
        // Interface-level type doc still uses the interface's own name if present.
        let iface_full = format!("{}.{}", iface.namespace, iface.name);
        if let Some(doc) = self.lookup_type(&iface_full) {
            iface.doc = doc.summary.clone();
            iface.deprecated = doc.deprecated.clone();
        }
        for m in iface.methods.iter_mut() {
            // Prefer the class-scoped key; fall back to interface-scoped.
            self.apply_to_method(owner_ns, owner_name, m);
            if m.doc.is_none() {
                self.apply_to_method(&iface.namespace, &iface.name, m);
            }
        }
    }

    pub fn apply_to_class(&self, class: &mut crate::meta::ClassMeta) {
        if self.is_empty() {
            return;
        }
        let full = format!("{}.{}", class.namespace, class.name);
        if let Some(doc) = self.lookup_type(&full) {
            class.doc = doc.summary.clone();
            class.deprecated = doc.deprecated.clone();
        }
        if let Some(ref mut di) = class.default_interface {
            self.apply_to_interface_as(di, &class.namespace, &class.name);
        }
        for i in class.required_interfaces.iter_mut() { self.apply_to_interface_as(i, &class.namespace, &class.name); }
        for i in class.factory_interfaces.iter_mut() { self.apply_to_interface_as(i, &class.namespace, &class.name); }
        for i in class.static_interfaces.iter_mut() { self.apply_to_interface_as(i, &class.namespace, &class.name); }
    }

    pub fn apply_to_interface(&self, iface: &mut crate::meta::InterfaceMeta) {
        if self.is_empty() {
            return;
        }
        let full = format!("{}.{}", iface.namespace, iface.name);
        if let Some(doc) = self.lookup_type(&full) {
            iface.doc = doc.summary.clone();
            iface.deprecated = doc.deprecated.clone();
        }
        // Try methods with interface name first
        for m in iface.methods.iter_mut() {
            self.apply_to_method(&iface.namespace, &iface.name, m);
        }
        // Fallback: try class name derived from interface name (IFoo2 -> Foo)
        // WinRT XML docs key methods by the runtime class name, not the interface.
        let class_name = interface_to_class_name(&iface.name);
        if let Some(ref cn) = class_name {
            if iface.doc.is_none() {
                let class_full = format!("{}.{}", iface.namespace, cn);
                if let Some(doc) = self.lookup_type(&class_full) {
                    iface.doc = doc.summary.clone();
                    iface.deprecated = doc.deprecated.clone();
                }
            }
            for m in iface.methods.iter_mut() {
                if m.doc.is_none() {
                    self.apply_to_method(&iface.namespace, cn, m);
                }
            }
        }
    }

    fn apply_to_method(&self, ns: &str, iface_name: &str, m: &mut crate::meta::MethodMeta) {
        let full = format!("{}.{}.{}", ns, iface_name, m.raw_name);
        // Property getters/setters -> P: lookup too (preferred if present)
        if m.is_property_getter || m.is_property_setter {
            let prop_name = m.raw_name
                .strip_prefix("get_")
                .or_else(|| m.raw_name.strip_prefix("put_"))
                .or_else(|| m.raw_name.strip_prefix("set_"))
                .unwrap_or(&m.raw_name);
            let prop_full = format!("{}.{}.{}", ns, iface_name, prop_name);
            if let Some(doc) = self.lookup_property(&prop_full) {
                m.doc = doc.summary.clone();
                m.deprecated = doc.deprecated.clone();
                m.returns_doc = doc.returns.clone();
                for (k, v) in doc.param_docs.iter() {
                    m.param_docs.insert(k.clone(), v.clone());
                }
                return;
            }
        }
        if m.is_event_add || m.is_event_remove {
            let evt_name = m.raw_name
                .strip_prefix("add_")
                .or_else(|| m.raw_name.strip_prefix("remove_"))
                .unwrap_or(&m.raw_name);
            let evt_full = format!("{}.{}.{}", ns, iface_name, evt_name);
            if let Some(doc) = self.lookup_event(&evt_full) {
                m.doc = doc.summary.clone();
                m.deprecated = doc.deprecated.clone();
                return;
            }
        }
        if let Some(doc) = self.lookup_method(&full, &m.raw_signature_key) {
            m.doc = doc.summary.clone();
            m.deprecated = doc.deprecated.clone();
            m.returns_doc = doc.returns.clone();
            for (k, v) in doc.param_docs.iter() {
                m.param_docs.insert(k.clone(), v.clone());
            }
        }
    }

    /// Apply docs to an enum `TypeMeta::Enum`. No-op for other variants.
    pub fn apply_to_enum(&self, e: &mut crate::types::TypeMeta) {
        if self.is_empty() {
            return;
        }
        use crate::types::TypeMeta;
        if let TypeMeta::Enum { namespace, name, members, doc, deprecated, .. } = e {
            let full = format!("{}.{}", namespace, name);
            if let Some(tdoc) = self.lookup_type(&full) {
                *doc = tdoc.summary.clone();
                *deprecated = tdoc.deprecated.clone();
            }
            for mem in members.iter_mut() {
                let mfull = format!("{}.{}", full, mem.name);
                if let Some(mdoc) = self.lookup_field(&mfull) {
                    mem.doc = mdoc.summary.clone();
                }
            }
        }
    }
}

/// Normalize an XML doc element into plain text with inline markup.
/// - `<para>` → blank line between paragraphs
/// - `<c>text</c>` → `` `text` ``
/// - `<see cref="T:NS.X"/>` → `X` (last segment after any kind prefix and namespace)
/// - `<paramref name="x"/>` → `` `x` ``
/// - `<typeparamref name="T"/>` → `` `T` ``
/// - Other elements: inner text preserved verbatim
fn normalize(node: roxmltree::Node) -> String {
    // Collect paragraph-by-paragraph. Each top-level <para> becomes its own
    // paragraph; other inline content accumulates into the "current" paragraph.
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();

    for child in node.children() {
        if child.is_text() {
            if let Some(t) = child.text() {
                current.push_str(t);
            }
        } else if child.is_element() {
            let tag = child.tag_name().name();
            if tag == "para" {
                if !current.trim().is_empty() {
                    paragraphs.push(collapse(&current));
                }
                current.clear();
                let p = normalize_inline(child);
                if !p.trim().is_empty() {
                    paragraphs.push(collapse(&p));
                }
            } else {
                current.push_str(&normalize_inline(child));
            }
        }
    }
    if !current.trim().is_empty() {
        paragraphs.push(collapse(&current));
    }
    paragraphs.join("\n\n")
}

/// Normalize an element and its children as a single-line flow (no paragraph breaks).
fn normalize_inline(node: roxmltree::Node) -> String {
    let tag = node.tag_name().name();
    match tag {
        "c" | "code" => {
            let inner = node_text(node);
            format!("`{}`", inner.trim())
        }
        "see" | "seealso" => {
            if let Some(cref) = node.attribute("cref") {
                format!("`{}`", strip_cref(cref))
            } else if let Some(href) = node.attribute("href") {
                href.to_string()
            } else {
                node_text(node)
            }
        }
        "paramref" | "typeparamref" => {
            if let Some(name) = node.attribute("name") {
                format!("`{}`", name)
            } else {
                String::new()
            }
        }
        _ => {
            // Unknown tag: preserve inner text + recurse children
            let mut s = String::new();
            for child in node.children() {
                if child.is_text() {
                    if let Some(t) = child.text() {
                        s.push_str(t);
                    }
                } else if child.is_element() {
                    s.push_str(&normalize_inline(child));
                }
            }
            s
        }
    }
}

fn node_text(node: roxmltree::Node) -> String {
    let mut s = String::new();
    for child in node.children() {
        if child.is_text() {
            if let Some(t) = child.text() {
                s.push_str(t);
            }
        } else if child.is_element() {
            s.push_str(&normalize_inline(child));
        }
    }
    s
}

fn strip_cref(cref: &str) -> String {
    // Remove leading kind prefix: T:, M:, P:, F:, E:, N:, !:
    let without_kind = cref
        .strip_prefix("T:")
        .or_else(|| cref.strip_prefix("M:"))
        .or_else(|| cref.strip_prefix("P:"))
        .or_else(|| cref.strip_prefix("F:"))
        .or_else(|| cref.strip_prefix("E:"))
        .or_else(|| cref.strip_prefix("N:"))
        .or_else(|| cref.strip_prefix("!:"))
        .unwrap_or(cref);
    // Strip any signature tail `(...)`
    let without_sig = match without_kind.find('(') {
        Some(idx) => &without_kind[..idx],
        None => without_kind,
    };
    // Last path segment
    match without_sig.rfind('.') {
        Some(idx) => without_sig[idx + 1..].to_string(),
        None => without_sig.to_string(),
    }
}

fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_ws = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out.trim().to_string()
}

/// Derive a likely runtime class name from a WinRT interface name.
/// E.g. `ILanguageModel2` → `LanguageModel`, `ITextSummarizer4` → `TextSummarizer`,
/// `IClosable` → `Closable`. Returns `None` if the name doesn't start with `I`.
fn interface_to_class_name(iface_name: &str) -> Option<String> {
    let stripped = iface_name.strip_prefix('I')?;
    // The next char must be uppercase (to avoid false positives like "Image")
    if stripped.is_empty() || !stripped.chars().next().unwrap().is_uppercase() {
        return None;
    }
    // Strip trailing digits (version suffix: IFoo2 -> Foo)
    let name = stripped.trim_end_matches(|c: char| c.is_ascii_digit());
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(xml: &str) -> DocTable {
        let mut t = DocTable::default();
        t.ingest_xml(xml);
        t
    }

    #[test]
    fn plain_summary() {
        let t = mk(r#"<doc><members>
            <member name="T:A.B"><summary>Hello world.</summary></member>
        </members></doc>"#);
        assert_eq!(t.lookup_type("A.B").unwrap().summary.as_deref(), Some("Hello world."));
    }

    #[test]
    fn see_cref_last_segment() {
        let t = mk(r#"<doc><members>
            <member name="T:A.B"><summary>Use <see cref="T:A.B.C"/> for this.</summary></member>
        </members></doc>"#);
        let s = t.lookup_type("A.B").unwrap().summary.clone().unwrap();
        assert!(s.contains("`C`"), "got: {}", s);
    }

    #[test]
    fn inline_c_tag() {
        let t = mk(r#"<doc><members>
            <member name="T:A.B"><summary>Use <c>foo</c> here.</summary></member>
        </members></doc>"#);
        let s = t.lookup_type("A.B").unwrap().summary.clone().unwrap();
        assert!(s.contains("`foo`"), "got: {}", s);
    }

    #[test]
    fn param_doc_captured() {
        let t = mk(r#"<doc><members>
            <member name="M:A.B.M(System.Int32)">
                <summary>Does things.</summary>
                <param name="x">The input.</param>
            </member>
        </members></doc>"#);
        let m = t.lookup_method("A.B.M", "(System.Int32)").unwrap();
        assert_eq!(m.param_docs.get("x").map(|s| s.as_str()), Some("The input."));
    }

    #[test]
    fn multi_paragraph_summary() {
        let t = mk(r#"<doc><members>
            <member name="T:A.B"><summary><para>First.</para><para>Second.</para></summary></member>
        </members></doc>"#);
        let s = t.lookup_type("A.B").unwrap().summary.clone().unwrap();
        assert_eq!(s, "First.\n\nSecond.");
    }

    #[test]
    fn overload_disambiguation() {
        let t = mk(r#"<doc><members>
            <member name="M:A.B.M(System.Int32)"><summary>Int version.</summary></member>
            <member name="M:A.B.M(System.String)"><summary>String version.</summary></member>
        </members></doc>"#);
        let i = t.lookup_method("A.B.M", "(System.Int32)").unwrap();
        let s = t.lookup_method("A.B.M", "(System.String)").unwrap();
        assert_eq!(i.summary.as_deref(), Some("Int version."));
        assert_eq!(s.summary.as_deref(), Some("String version."));
    }

    #[test]
    fn missing_xml_is_empty() {
        let t = DocTable::default();
        assert!(t.is_empty());
        assert!(t.lookup_type("A.B").is_none());
    }

    #[test]
    fn load_from_missing_path() {
        let t = DocTable::load_from_winmd_paths(&["C:/nonexistent/file.winmd".into()]);
        assert!(t.is_empty());
    }
}
