# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from __future__ import annotations

import argparse
import re
from pathlib import Path

from packaging.version import Version


ROOT = Path(__file__).resolve().parents[3]
PACKAGE_MANIFESTS = (
    Path("bindings/py/Cargo.toml"),
    Path("tools/dynwinrt-codegen/Cargo.toml"),
)
PACKAGE_NAMES = ("dynwinrt-py", "dynwinrt-codegen")
CARGO_SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


def replace_once(path: Path, pattern: str, replacement: str) -> None:
    with path.open("r", encoding="utf-8", newline="") as stream:
        contents = stream.read()
    updated, count = re.subn(pattern, replacement, contents, count=1)
    if count != 1:
        raise RuntimeError(f"Expected one version entry in {path}, found {count}")
    if updated != contents:
        with path.open("w", encoding="utf-8", newline="") as stream:
            stream.write(updated)


def prepare_release(root: Path, version: str) -> None:
    if not CARGO_SEMVER.fullmatch(version):
        raise ValueError(f"Release version is not valid Cargo SemVer: {version!r}")

    python_version = Version(version)
    for relative_path in PACKAGE_MANIFESTS:
        replace_once(
            root / relative_path,
            r'(?ms)(^\[package\].*?^version\s*=\s*")[^"]+(")',
            rf"\g<1>{version}\g<2>",
        )

    lock_path = root / "Cargo.lock"
    for package_name in PACKAGE_NAMES:
        replace_once(
            lock_path,
            rf'(?ms)(^\[\[package\]\]\r?\nname = "{re.escape(package_name)}"\r?\nversion = ")[^"]+(")',
            rf"\g<1>{version}\g<2>",
        )

    print(f"Prepared Python release version {version} (PEP 440: {python_version})")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    prepare_release(args.root.resolve(), args.version)


if __name__ == "__main__":
    main()
