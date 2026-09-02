# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from __future__ import annotations

import argparse
import email
import re
import sys
import zipfile
from pathlib import Path

from packaging.specifiers import SpecifierSet
from packaging.utils import canonicalize_name, parse_wheel_filename
from packaging.version import Version


ROOT = Path(__file__).resolve().parents[3]
RUNTIME_REQUIRES = ">=3.11,<3.15"
CODEGEN_REQUIRES = ">=3.8,<3.15"
RUNTIME_MINORS = tuple(f"3.{minor}" for minor in range(11, 15))
PLATFORM_TAGS = ("win_amd64", "win_arm64")
README_SPEC = {"file": "README.md", "content-type": "text/markdown"}
README_HEADINGS = {
    "dynwinrt": "# dynwinrt",
    "dynwinrt-codegen": "# dynwinrt-codegen",
}


def load_toml(path: Path) -> dict:
    try:
        import tomllib
    except ModuleNotFoundError:
        import tomli as tomllib
    with path.open("rb") as stream:
        return tomllib.load(stream)


def cargo_package(path: Path) -> dict:
    return load_toml(path)["package"]


def verify_source(tag: str | None, release_version: str | None) -> None:
    runtime_manifest = load_toml(ROOT / "bindings" / "py" / "Cargo.toml")
    runtime_cargo = runtime_manifest["package"]
    codegen_cargo = cargo_package(ROOT / "tools" / "dynwinrt-codegen" / "Cargo.toml")
    runtime_project = load_toml(ROOT / "bindings" / "py" / "pyproject.toml")["project"]
    codegen_project = load_toml(
        ROOT / "tools" / "dynwinrt-codegen" / "pyproject.toml"
    )["project"]

    assert runtime_cargo["name"] == "dynwinrt-py"
    assert runtime_project["name"] == "dynwinrt"
    assert codegen_cargo["name"] == codegen_project["name"] == "dynwinrt-codegen"
    assert runtime_cargo["version"] == codegen_cargo["version"], (
        "Runtime and codegen Cargo versions must match because generated packages "
        "pin dynwinrt to the codegen version"
    )
    assert SpecifierSet(runtime_project["requires-python"]) == SpecifierSet(
        RUNTIME_REQUIRES
    )
    assert SpecifierSet(codegen_project["requires-python"]) == SpecifierSet(
        CODEGEN_REQUIRES
    )
    assert runtime_project["readme"] == README_SPEC
    assert codegen_project["readme"] == README_SPEC

    generated_manifest = (
        ROOT / "tools" / "dynwinrt-codegen" / "src" / "codegen" / "package.rs"
    ).read_text(encoding="utf-8")
    assert f'requires-python = \\"{RUNTIME_REQUIRES}\\"' in generated_manifest
    assert 'dependencies = [\\"dynwinrt=={runtime_version}\\"]' in generated_manifest

    pyo3 = runtime_manifest["dependencies"]["pyo3"]
    assert Version(str(pyo3)) >= Version("0.29.0")

    raw_version = runtime_cargo["version"]
    version = Version(raw_version)
    if release_version:
        assert release_version == raw_version, (
            f"Requested version {release_version!r} does not exactly match Cargo "
            f"version {raw_version!r}"
        )
    if tag:
        match = re.fullmatch(r"v(.+)", tag)
        assert match, f"Release tags must match v<version>, got {tag!r}"
        assert match.group(1) == raw_version, (
            f"Tag {tag!r} does not exactly match Cargo version {raw_version!r}"
        )

    print(
        f"source metadata OK: version={version}, "
        f"runtime CPython={','.join(RUNTIME_MINORS)}, "
        f"runtime requires-python={RUNTIME_REQUIRES}, "
        f"codegen requires-python={CODEGEN_REQUIRES}"
    )


def read_metadata(archive: zipfile.ZipFile) -> email.message.Message:
    names = [name for name in archive.namelist() if name.endswith(".dist-info/METADATA")]
    assert len(names) == 1, f"Expected one METADATA file, found {names}"
    return email.message_from_bytes(archive.read(names[0]))


def verify_wheel(args: argparse.Namespace) -> None:
    wheel = Path(args.wheel).resolve()
    distribution, parsed_version, build, tags = parse_wheel_filename(wheel.name)
    assert not build, f"Unexpected wheel build tag: {build}"
    assert canonicalize_name(str(distribution)) == canonicalize_name(args.package)
    assert parsed_version == Version(args.version)
    actual_tags = {str(tag) for tag in tags}
    expected_tag = f"{args.python_tag}-{args.abi_tag}-{args.platform_tag}"
    assert actual_tags == {expected_tag}, f"Expected {expected_tag}, got {actual_tags}"

    with zipfile.ZipFile(wheel) as archive:
        metadata = read_metadata(archive)
        assert canonicalize_name(metadata["Name"]) == canonicalize_name(args.package)
        assert Version(metadata["Version"]) == Version(args.version)
        expected_requires = (
            RUNTIME_REQUIRES if args.package == "dynwinrt" else CODEGEN_REQUIRES
        )
        assert SpecifierSet(metadata["Requires-Python"]) == SpecifierSet(
            expected_requires
        )
        dependencies = metadata.get_all("Requires-Dist", [])
        assert dependencies == [], f"Unexpected wheel dependencies: {dependencies}"
        content_type = metadata["Description-Content-Type"]
        assert content_type == "text/markdown", (
            f"Expected Markdown long description, got {content_type!r}"
        )
        payload = metadata.get_payload(decode=True)
        assert isinstance(payload, bytes), "Wheel long description is missing"
        description = payload.decode("utf-8").strip()
        expected_heading = README_HEADINGS[args.package]
        assert description.startswith(expected_heading), (
            f"Wheel long description must start with {expected_heading!r}"
        )
        names = archive.namelist()
        if args.package == "dynwinrt":
            assert any(name.endswith(".pyd") for name in names)
            assert any(name.endswith("/py.typed") for name in names)
            assert any(name.endswith(".pyi") for name in names)
        else:
            assert any(name.endswith("/dynwinrt-codegen.exe") for name in names)

    print(f"wheel metadata OK: {wheel.name}")


def release_wheels(version: str) -> list[tuple[str, str, str, str, str]]:
    normalized_version = str(Version(version))
    wheels = []
    for platform_tag in PLATFORM_TAGS:
        for minor in RUNTIME_MINORS:
            python_tag = f"cp{minor.replace('.', '')}"
            wheels.append(
                (
                    f"dynwinrt-{normalized_version}-{python_tag}-{python_tag}-{platform_tag}.whl",
                    "dynwinrt",
                    python_tag,
                    python_tag,
                    platform_tag,
                )
            )
        wheels.append(
            (
                f"dynwinrt_codegen-{normalized_version}-py3-none-{platform_tag}.whl",
                "dynwinrt-codegen",
                "py3",
                "none",
                platform_tag,
            )
        )
    return wheels


def verify_release_set(args: argparse.Namespace) -> None:
    directory = Path(args.directory).resolve()
    wheels = list(directory.rglob("*.whl"))
    actual_names = [wheel.name for wheel in wheels]
    assert len(actual_names) == len(set(actual_names)), (
        f"Release wheel filenames must be unique: {actual_names}"
    )

    expected = release_wheels(args.version)
    expected_names = {wheel[0] for wheel in expected}
    assert set(actual_names) == expected_names, (
        f"Release wheel set mismatch; missing={sorted(expected_names - set(actual_names))}, "
        f"unexpected={sorted(set(actual_names) - expected_names)}"
    )

    wheels_by_name = {wheel.name: wheel for wheel in wheels}
    for name, package, python_tag, abi_tag, platform_tag in expected:
        verify_wheel(
            argparse.Namespace(
                wheel=wheels_by_name[name],
                package=package,
                version=args.version,
                python_tag=python_tag,
                abi_tag=abi_tag,
                platform_tag=platform_tag,
            )
        )
    print(f"complete Python release set OK: {len(wheels)} wheels")


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    source = subparsers.add_parser("source")
    source.add_argument("--tag")
    source.add_argument("--release-version")

    wheel = subparsers.add_parser("wheel")
    wheel.add_argument("--wheel", required=True)
    wheel.add_argument("--package", required=True)
    wheel.add_argument("--version", required=True)
    wheel.add_argument("--python-tag", required=True)
    wheel.add_argument("--abi-tag", required=True)
    wheel.add_argument("--platform-tag", required=True)

    release_set = subparsers.add_parser("release-set")
    release_set.add_argument("--directory", required=True)
    release_set.add_argument("--version", required=True)

    args = parser.parse_args()
    if args.command == "source":
        verify_source(args.tag, args.release_version)
    elif args.command == "wheel":
        verify_wheel(args)
    else:
        verify_release_set(args)


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, KeyError, ValueError) as error:
        print(f"release validation failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
