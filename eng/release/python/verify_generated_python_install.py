# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import argparse
import importlib
import sys
from pathlib import Path


def package_files(root: Path) -> set[Path]:
    return {
        path.relative_to(root)
        for extension in ("*.py", "*.pyi")
        for path in root.rglob(extension)
    }


def generated_source_files(root: Path) -> set[Path]:
    inventory = root / ".dynwinrt-generated-files"
    files = {
        Path(line)
        for line in inventory.read_text(encoding="utf-8").splitlines()
        if Path(line).suffix in {".py", ".pyi"}
    }
    invalid = [path for path in files if path.is_absolute() or ".." in path.parts]
    if invalid:
        raise RuntimeError(f"Generated inventory contains invalid paths: {invalid[:10]}")
    missing = [path for path in files if not (root / path).is_file()]
    if missing:
        raise RuntimeError(f"Generated inventory references missing files: {missing[:10]}")
    return files


def module_name(package: str, relative: Path) -> str:
    parts = (
        relative.parts[:-1]
        if relative.name == "__init__.py"
        else relative.with_suffix("").parts
    )
    return ".".join((package, *parts))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--install", type=Path, required=True)
    parser.add_argument("--package", required=True)
    args = parser.parse_args()

    source = args.source.resolve()
    installed = (args.install.resolve() / args.package)
    expected = generated_source_files(source)
    actual = package_files(installed)
    missing = sorted(expected - actual)
    if missing:
        raise RuntimeError(
            f"Installed package is missing {len(missing)} generated files: {missing[:10]}"
        )
    unexpected = sorted(actual - expected)
    if unexpected:
        raise RuntimeError(
            f"Installed package has {len(unexpected)} unexpected generated files: "
            f"{unexpected[:10]}"
        )

    sys.path.insert(0, str(args.install.resolve()))
    modules = {
        module_name(args.package, path)
        for path in expected
        if path.suffix == ".py"
    }
    for name in sorted(modules):
        importlib.import_module(name)
    print(f"verified {len(expected)} files and imported {len(modules)} modules")


if __name__ == "__main__":
    main()
