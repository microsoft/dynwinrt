#!/usr/bin/env python3
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.
"""Extract the Windows SDK members whose documented result can be null.

WinRT metadata carries no nullability, so dynwinrt-codegen types most outputs
as non-null and keeps `| None` for members whose documentation says that the
result can be null. This script derives that list from MicrosoftDocs/winrt-api
at a pinned commit and writes only doc comment IDs (api-ids), never
documentation text, to api-docs/windows-null-results.txt.

A method (`M:`) or property (`P:`) is listed when
- a sentence of its `## -returns` or `## -property-value` section says that
  the result can be null (for an asynchronous method: its completed result), or
- a sentence of its `## -remarks` section says that the member itself
  ("this method", "this property", "it", or the member's name) returns or is
  null.
Sentences that negate null, that describe null arguments, or that describe an
object holding a null value are ignored, and so are Boolean results, attached
properties, constructors and members of generic types. Reviewed corrections
from api-docs/windows-null-results.overrides.txt are applied last.

The documentation is read from git objects, without a working tree:

    python extract-null-results.py                 # fetch the pinned commit
    python extract-null-results.py --repo DIR      # reuse a local clone
    python extract-null-results.py --commit SHA    # move the pin
"""

from __future__ import annotations

import argparse
import fnmatch
import re
import subprocess
import sys
import tempfile
from collections import Counter
from collections.abc import Iterator
from pathlib import Path

REPOSITORY = "https://github.com/MicrosoftDocs/winrt-api"
COMMIT = "8448d5eecfbc2ed903f659f350841dcb4888bc8b"
CODEGEN = Path(__file__).resolve().parent.parent
OUTPUT = CODEGEN / "api-docs" / "windows-null-results.txt"
OVERRIDES = CODEGEN / "api-docs" / "windows-null-results.overrides.txt"

NULL = re.compile(r"\bnull(?:ptr)?\b(?![- ](?:terminat|character|char\b))", re.I)
NEGATION = re.compile(
    r"\b(?:never|not|cannot|can't|won't|doesn't|does not|isn't|is not|will not|must not|may not)"
    r"\s+(?:be\s+|is\s+|return\s+|returns\s+)?(?:a\s+)?null\b|\bnon-?null\b",
    re.I,
)
ARGUMENT = re.compile(
    r"\bnull\s+(?:was|is|were|are)\s+passed\b"
    r"|\bpass(?:es|ed|ing)?\s+(?:in\s+)?(?:a\s+)?null\b"
    r"|\bother than\s+null\b"
    r"|\b(?:argument|parameter)\s+(?:is|was|are)\s+null\b"
    r"|\bnull\s+(?:for|as)\s+(?:the\s+)?\w+\s+(?:argument|parameter)\b"
    r"|\bexception\b[^.]*\bset\s+to\s+null\b|\bset\s+to\s+null\b[^.]*\bexception\b",
    re.I,
)
HOLDER = re.compile(
    r"\b(?:with|holds?|holding|contains?|containing|supports?|has|have)\s+(?:a|an)\s+"
    r"(?:json\s+)?null\s+value\b",
    re.I,
)
BOOLEAN = re.compile(r"\A\W*(?:true|false)\b", re.I)
REMARKS_GAP = r"(?:(?!\b(?:when|if|unless|called|until|whether|and|but)\b)[^.;,]){0,40}?"
REMARKS_VERB = (
    r"\b(?:returns?|is|will\s+be|may\s+be|can\s+be|could\s+be|might\s+be|is\s+set\s+to)"
    r"\s+(?:a\s+|an\s+|the\s+)?null(?:ptr)?\b"
)


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True, capture_output=True, text=True
    ).stdout


def ensure_commit(repo: Path, commit: str) -> None:
    if not (repo / ".git").exists():
        repo.mkdir(parents=True, exist_ok=True)
        git(repo, "init", "--quiet")
    present = subprocess.run(
        ["git", "-C", str(repo), "cat-file", "-e", f"{commit}^{{commit}}"],
        capture_output=True,
    )
    if present.returncode != 0:
        git(repo, "fetch", "--quiet", "--depth", "1", REPOSITORY, commit)


def documents(repo: Path, commit: str) -> Iterator[str]:
    listing = git(repo, "ls-tree", "-r", "-z", commit)
    blobs = []
    for entry in listing.split("\0"):
        if not entry:
            continue
        info, path = entry.split("\t", 1)
        _, kind, sha = info.split()
        if kind == "blob" and path.endswith(".md"):
            blobs.append(sha)
    with subprocess.Popen(
        ["git", "-C", str(repo), "cat-file", "--batch"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    ) as batch:
        assert batch.stdin is not None and batch.stdout is not None
        for sha in blobs:
            batch.stdin.write(sha.encode() + b"\n")
            batch.stdin.flush()
            size = int(batch.stdout.readline().split()[2])
            data = batch.stdout.read(size)
            batch.stdout.read(1)
            yield data.decode("utf-8", "replace").replace("\r\n", "\n")
        batch.stdin.close()


def front_matter(text: str) -> dict[str, str]:
    match = re.match(r"\A---[ \t]*\n(.*?)\n---", text, re.S)
    fields = {}
    for line in match.group(1).splitlines() if match else []:
        key, _, value = line.strip().partition(":")
        if key.startswith("-"):
            fields[key[1:]] = value.strip()
    return fields


def section(text: str, name: str) -> str:
    match = re.search(rf"^## -{re.escape(name)}[ \t]*\n(.*?)(?=^## -|\Z)", text, re.M | re.S)
    return plain(match.group(1)) if match else ""


def plain(markdown: str) -> str:
    text = re.sub(r"<!--.*?-->", " ", markdown, flags=re.S)
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = re.sub(r"\[!(?:NOTE|IMPORTANT|TIP|WARNING|CAUTION)\]", " ", text)
    text = re.sub(r"(?m)^\s*>\s?", " ", text)
    text = re.sub(r"[*_`]", "", text)
    return " ".join(text.split())


def sentences(text: str) -> list[str]:
    return [sentence for sentence in re.split(r"(?<=[.!?])\s+", text) if NULL.search(sentence)]


def states_null(sentence: str) -> bool:
    return not (NEGATION.search(sentence) or ARGUMENT.search(sentence) or HOLDER.search(sentence))


def result_is_nullable(result: str) -> bool:
    if not result or BOOLEAN.match(result):
        return False
    return any(states_null(sentence) for sentence in sentences(result))


def remarks_say_null(remarks: str, member: str) -> bool:
    subject = (
        r"(?:\bthis\s+(?:method|property|function|call|operation)"
        r"|\bthe\s+(?:method|property|call|operation)"
        rf"|\bit|\b{re.escape(member)})\b"
    )
    claim = re.compile(subject + REMARKS_GAP + REMARKS_VERB, re.I)
    returns = re.compile(rf"\bif\s+(?:this|it|{re.escape(member)})\s+returns\s+null\b", re.I)
    return any(
        (claim.search(sentence) or returns.search(sentence)) and states_null(sentence)
        for sentence in sentences(remarks)
    )


def member_name(api_id: str) -> str:
    return api_id[2:].split("(", 1)[0].rsplit(".", 1)[-1]


def extract(repo: Path, commit: str) -> tuple[set[str], set[str], Counter[str]]:
    documented: set[str] = set()
    nullable: set[str] = set()
    sources: Counter[str] = Counter()
    for text in documents(repo, commit):
        fields = front_matter(text)
        api_id = fields.get("api-id", "")
        api_type = fields.get("api-type", "")
        if not api_id.startswith(("M:", "P:")) or api_type in {
            "winrt attachedproperty",
            "winrt constructor",
        }:
            continue
        if "#ctor" in api_id or "`" in api_id.split("(", 1)[0]:
            continue
        documented.add(api_id)
        result = section(text, "returns" if api_id.startswith("M:") else "property-value")
        if result_is_nullable(result):
            sources["returns" if api_id.startswith("M:") else "property-value"] += 1
            nullable.add(api_id)
        elif remarks_say_null(section(text, "remarks"), member_name(api_id)):
            sources["remarks"] += 1
            nullable.add(api_id)
    return documented, nullable, sources


def apply_overrides(documented: set[str], nullable: set[str], sources: Counter[str]) -> set[str]:
    result = set(nullable)
    for number, raw in enumerate(OVERRIDES.read_text(encoding="utf-8").splitlines(), 1):
        entry = raw.split("#", 1)[0].strip()
        if not entry:
            continue
        sign, pattern = entry[0], entry[1:].strip()
        if sign not in "+-" or not pattern:
            sys.exit(f"{OVERRIDES.name}:{number}: expected '+api-id' or '-api-id'")
        matches = {api_id for api_id in documented if fnmatch.fnmatchcase(api_id, pattern)}
        if not matches:
            sys.exit(f"{OVERRIDES.name}:{number}: {pattern} matches no documented member")
        if sign == "+":
            sources["override additions"] += len(matches - result)
            result |= matches
        else:
            sources["override removals"] += len(matches & result)
            result -= matches
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--commit", default=COMMIT, help="winrt-api commit to read")
    parser.add_argument("--repo", type=Path, help="local winrt-api clone to reuse")
    arguments = parser.parse_args()

    with tempfile.TemporaryDirectory(prefix="winrt-api-") as scratch:
        repo = arguments.repo or Path(scratch)
        ensure_commit(repo, arguments.commit)
        documented, nullable, sources = extract(repo, arguments.commit)
    members = sorted(apply_overrides(documented, nullable, sources))

    header = [
        "# Windows SDK members whose documented result can be null: a method's",
        "# return value (for asynchronous methods, the completed result) or a",
        "# property's value. dynwinrt-codegen keeps `| None` on these outputs.",
        f"# Source: {REPOSITORY} at commit {arguments.commit}",
        "# Generated by scripts/extract-null-results.py; do not edit. Reviewed",
        "# corrections belong in windows-null-results.overrides.txt.",
    ]
    OUTPUT.write_text("\n".join(header + members) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(members)} members from {len(documented)} documented methods and properties")
    for source, count in sources.most_common():
        print(f"  {source}: {count}")


if __name__ == "__main__":
    main()
