# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import argparse
import importlib
import os
import subprocess
import sys
import sysconfig
import tempfile
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


def check_uri_typing(package: str, install: Path, typed: bool) -> None:
    environment = dict(os.environ)
    environment.pop("MYPYPATH", None)
    if install not in {
        Path(sysconfig.get_path(name)).resolve() for name in ("purelib", "platlib")
    }:
        environment["MYPYPATH"] = str(install)
    with tempfile.TemporaryDirectory(prefix="dtype-") as temporary:
        directory = Path(temporary)
        module = "windows.foundation" if typed else "windows__foundation__uri"
        imports = f"from {package}.{module} import Uri\n"
        result_type = "Uri" if typed else "Uri | None"
        valid = (
            imports
            + f"def consume(uri: Uri) -> {result_type}:\n"
            + "    host: str = uri.host\n"
            + "    return uri.combine_uri('sub/page')\n"
        )
        if typed:
            valid += (
                f"from {package}.windows.foundation.uri import Uri as PerTypeUri\n"
                f"from {package}.windows__foundation__uri import Uri as FlatUri\n"
                "uri: Uri = PerTypeUri('https://example.com')\n"
                "other: Uri = FlatUri('https://example.com')\n"
            )
        invalid = (
            imports
            + f"def reject(uri: Uri) -> {result_type}:\n"
            + "    return uri.combine_uri(123)\n"
        )
        for name, source in [("valid", valid), ("invalid", invalid)]:
            consumer = directory / f"{name}.py"
            consumer.write_text(source, encoding="utf-8")
            command = [
                sys.executable,
                "-m",
                "mypy",
                "--strict",
                "--no-incremental",
                "--no-pretty",
                "--show-error-codes",
                "--cache-dir",
                str(directory / "m"),
                str(consumer),
            ]
            if not typed:
                command.extend(["--follow-untyped-imports", "--follow-imports=silent"])
            result = subprocess.run(
                command,
                capture_output=True,
                text=True,
                env=environment,
            )
            errors = [line for line in result.stdout.splitlines() if ": error:" in line]
            expected = 0 if name == "valid" else 1
            if result.returncode != expected or len(errors) != expected:
                raise RuntimeError(result.stdout + result.stderr)
            if name == "invalid" and not errors[0].endswith("[arg-type]"):
                raise RuntimeError(result.stdout + result.stderr)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--install", type=Path, required=True)
    parser.add_argument("--package", required=True)
    parser.add_argument("--check-uri", action="store_true")
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
    if args.check_uri:
        from dynwinrt import RoApartment

        namespace = importlib.import_module(f"{args.package}.windows.foundation")
        per_type = importlib.import_module(f"{args.package}.windows.foundation.uri")
        flat = importlib.import_module(f"{args.package}.windows__foundation__uri")
        assert namespace.Uri is per_type.Uri is flat.Uri
        with RoApartment():
            uri = namespace.Uri("https://example.com/compact-paths")
            assert uri.host == "example.com"
        assert uri._obj.is_released()
        check_uri_typing(
            args.package, args.install.resolve(), (installed / "py.typed").exists()
        )
    print(f"verified {len(expected)} files and imported {len(modules)} modules")


if __name__ == "__main__":
    main()
