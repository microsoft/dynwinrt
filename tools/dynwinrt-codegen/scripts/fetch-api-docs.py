#!/usr/bin/env python3
"""
Convert MicrosoftDocs/winapps-winrt-api markdown files to C# XML doc format.
Output: one .xml file per namespace, compatible with dynwinrt-codegen's xml_doc.rs.

Usage:
    python fetch-api-docs.py <repo-dir> <output-dir> [--namespaces ns1,ns2,...]
"""

import os
import re
import sys
from collections import defaultdict

# Default namespaces to process
DEFAULT_NAMESPACES = [
    "microsoft.windows.ai",
    "microsoft.windows.ai.text",
    "microsoft.windows.ai.foundation",
    "microsoft.windows.ai.generative",
    "microsoft.windows.ai.machinelearning",
    "microsoft.windows.ai.contentmoderation",
    "microsoft.graphics.imaging",
    "microsoft.windows.vision",
    "microsoft.windows.workloads",
]


def parse_frontmatter(text: str):
    """Extract api-id and api-type from YAML front matter."""
    m = re.match(r"^---\s*\n(.*?)\n---", text, re.DOTALL)
    if not m:
        return None, None
    fm = m.group(1)
    api_id = None
    api_type = None
    for line in fm.split("\n"):
        line = line.strip()
        if line.startswith("-api-id:"):
            api_id = line.split(":", 1)[1].strip()
        elif line.startswith("-api-type:"):
            api_type = line.split(":", 1)[1].strip()
    return api_id, api_type


def extract_section(text: str, header: str) -> str:
    """Extract content under a ## -header section, up to next ## section."""
    pattern = rf"^## -{re.escape(header)}\s*\n(.*?)(?=^## -|\Z)"
    m = re.search(pattern, text, re.MULTILINE | re.DOTALL)
    if not m:
        return ""
    return m.group(1).strip()


def extract_params(text: str) -> dict:
    """Extract ### -param name / description pairs from -parameters section."""
    params = {}
    section = extract_section(text, "parameters")
    if not section:
        return params
    parts = re.split(r"^### -param\s+", section, flags=re.MULTILINE)
    for part in parts[1:]:
        lines = part.strip().split("\n", 1)
        if lines:
            param_name = lines[0].strip()
            param_doc = lines[1].strip() if len(lines) > 1 else ""
            param_doc = clean_text(param_doc)
            if param_name and param_doc:
                params[param_name] = param_doc
    return params


def extract_enum_fields(text: str) -> dict:
    """Extract ### -field Name: Value / description pairs."""
    fields = {}
    section = extract_section(text, "enum-fields")
    if not section:
        return fields
    parts = re.split(r"^### -field\s+", section, flags=re.MULTILINE)
    for part in parts[1:]:
        lines = part.strip().split("\n", 1)
        if lines:
            field_header = lines[0].strip()
            field_name = field_header.split(":")[0].strip()
            field_doc = lines[1].strip() if len(lines) > 1 else ""
            field_doc = clean_text(field_doc)
            if field_name:
                fields[field_name] = field_doc
    return fields


def clean_text(text: str) -> str:
    """Clean markdown text for XML output."""
    # Remove > [!NOTE] blocks
    text = re.sub(r">\s*\[!NOTE\].*?(?=\n[^>]|\Z)", "", text, flags=re.DOTALL)
    # Remove markdown links [text](url) -> text
    text = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", text)
    # Remove bold/italic
    text = re.sub(r"\*\*(.+?)\*\*", r"\1", text)
    text = re.sub(r"\*(.+?)\*", r"\1", text)
    # Remove code backticks
    text = re.sub(r"`([^`]+)`", r"\1", text)
    # Collapse whitespace
    text = re.sub(r"\n\s*\n", "\n", text)
    return text.strip()


def xml_escape(text: str) -> str:
    """Escape text for XML content."""
    text = text.replace("&", "&amp;")
    text = text.replace("<", "&lt;")
    text = text.replace(">", "&gt;")
    text = text.replace('"', "&quot;")
    return text


def parse_md_file(filepath: str) -> dict | None:
    """Parse a single markdown file into a doc entry."""
    with open(filepath, "r", encoding="utf-8") as f:
        text = f.read()

    api_id, api_type = parse_frontmatter(text)
    if not api_id:
        return None

    entry = {"api_id": api_id, "api_type": api_type}

    description = extract_section(text, "description")
    if description:
        entry["summary"] = clean_text(description)

    returns = extract_section(text, "returns")
    if returns:
        entry["returns"] = clean_text(returns)

    if api_type == "winrt enum":
        fields = extract_enum_fields(text)
        if fields:
            entry["fields"] = fields
    else:
        params = extract_params(text)
        if params:
            entry["params"] = params

    return entry


def build_xml(entries: list) -> str:
    """Build C# XML doc format string from parsed entries."""
    lines = ['<?xml version="1.0" encoding="utf-8"?>', "<doc>", "  <members>"]

    for entry in sorted(entries, key=lambda e: e["api_id"]):
        api_id = entry["api_id"]
        summary = entry.get("summary", "")
        returns = entry.get("returns", "")
        params = entry.get("params", {})
        fields = entry.get("fields", {})

        # Skip entries with no useful content
        if not summary and not returns and not params and not fields:
            continue

        lines.append(f'    <member name="{xml_escape(api_id)}">')
        if summary:
            lines.append(f"      <summary>{xml_escape(summary)}</summary>")
        for pname, pdoc in params.items():
            lines.append(
                f'      <param name="{xml_escape(pname)}">{xml_escape(pdoc)}</param>'
            )
        if returns:
            lines.append(f"      <returns>{xml_escape(returns)}</returns>")
        lines.append("    </member>")

        # For enums, emit F: entries for each field
        if fields:
            type_name = api_id[2:] if api_id.startswith("T:") else api_id
            for fname, fdoc in fields.items():
                field_id = f"F:{type_name}.{fname}"
                lines.append(f'    <member name="{xml_escape(field_id)}">')
                if fdoc:
                    lines.append(f"      <summary>{xml_escape(fdoc)}</summary>")
                lines.append("    </member>")

    lines.append("  </members>")
    lines.append("</doc>")
    return "\n".join(lines) + "\n"


def process_namespace(repo_dir: str, namespace: str) -> list:
    """Process all markdown files in a namespace directory."""
    ns_dir = os.path.join(repo_dir, namespace)
    if not os.path.isdir(ns_dir):
        print(f"  Skipping {namespace}: directory not found")
        return []

    entries = []
    for fname in sorted(os.listdir(ns_dir)):
        if not fname.endswith(".md"):
            continue
        filepath = os.path.join(ns_dir, fname)
        entry = parse_md_file(filepath)
        if entry:
            entries.append(entry)

    return entries


def namespace_to_xmlname(ns_dir_name: str) -> str:
    """Convert directory name to PascalCase XML filename."""
    parts = ns_dir_name.split(".")
    result = []
    acronyms = {"ai", "ui", "ml"}
    for p in parts:
        if p.lower() in acronyms:
            result.append(p.upper())
        else:
            result.append(p.capitalize())
    return ".".join(result) + ".xml"


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <repo-dir> <output-dir> [--namespaces ns1,ns2,...]")
        sys.exit(1)

    repo_dir = sys.argv[1]
    output_dir = sys.argv[2]

    namespaces = DEFAULT_NAMESPACES
    for i, arg in enumerate(sys.argv[3:], 3):
        if arg == "--namespaces" and i + 1 < len(sys.argv):
            namespaces = sys.argv[i + 1].split(",")

    os.makedirs(output_dir, exist_ok=True)

    total_members = 0
    for ns in namespaces:
        entries = process_namespace(repo_dir, ns)
        if not entries:
            continue

        xml_content = build_xml(entries)
        xml_filename = namespace_to_xmlname(ns)
        xml_path = os.path.join(output_dir, xml_filename)

        with open(xml_path, "w", encoding="utf-8") as f:
            f.write(xml_content)

        print(f"  {xml_filename}: {len(entries)} members")
        total_members += len(entries)

    print(f"\nDone. {total_members} members across {len(namespaces)} namespaces.")


if __name__ == "__main__":
    main()
