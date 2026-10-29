# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Exercise real codegen output through isolated packaging frontends and checkers.

Run from a tools environment with build, mypy, pyright and poetry installed.
Consumers never import from the checkout or through MYPYPATH/PYTHONPATH.
"""

import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codegen", type=Path, required=True)
    parser.add_argument("--runtime-wheel", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--uv", default="uv")
    parser.add_argument("--index-url", help="Optional test-local package mirror; no global configuration is changed")
    parser.add_argument("--bootstrap", type=Path, action="append", default=[])
    parser.add_argument("--winappsdk-version", default="2.3")
    args = parser.parse_args()
    root = args.work_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    codegen = root / "dynwinrt-codegen.exe"
    shutil.copy2(args.codegen.resolve(), codegen)
    runtime = args.runtime_wheel.resolve()
    report = {
        "python": sys.version,
        "codegen_sha256": hashlib.sha256(codegen.read_bytes()).hexdigest(),
        "runtime_sha256": hashlib.sha256(runtime.read_bytes()).hexdigest(),
        "versions": {
            name: importlib.metadata.version(name)
            for name in ("pip", "build", "setuptools", "wheel", "mypy", "pyright", "poetry")
        },
        "commands": [],
        "cases": [],
    }
    environment = dict(os.environ)
    for name in ("PYTHONPATH", "MYPYPATH", "VIRTUAL_ENV"):
        environment.pop(name, None)
    environment.update(
        PIP_DISABLE_PIP_VERSION_CHECK="1",
        UV_CACHE_DIR=str(root / "uv-cache"),
        UV_PYTHON_DOWNLOADS="never",
        POETRY_CACHE_DIR=str(root / "poetry-cache"),
        POETRY_VIRTUALENVS_IN_PROJECT="true",
        POETRY_KEYRING_ENABLED="false",
    )
    if args.index_url:
        environment.update(PIP_INDEX_URL=args.index_url, UV_DEFAULT_INDEX=args.index_url)

    def run(command, cwd=root, expected=0):
        command = [str(arg) for arg in command]
        result = subprocess.run(
            command, cwd=cwd, env=environment, capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=600,
        )
        report["commands"].append({
            "argv": command, "cwd": str(cwd), "exit": result.returncode,
            "stdout": result.stdout, "stderr": result.stderr,
        })
        if result.returncode != expected:
            raise RuntimeError(f"{command}\n{result.stdout}\n{result.stderr}")
        return result.stdout

    def generate(project, classes, bootstrap=()):
        command = [
            codegen, "generate", "--lang", "py", "--class-name", classes,
            "--output", project,
        ]
        if bootstrap:
            command += ["--winappsdk-version", args.winappsdk_version]
            for dll in bootstrap:
                command += ["--winappsdk-bootstrap-path", dll.resolve()]
        run(command)

    def venv(path):
        run([sys.executable, "-m", "venv", path])
        return path / "Scripts" / "python.exe"

    def check(python, project, editable, added=False):
        consumer = python.parent.parent.parent / ("added" if added else "consumer")
        consumer.mkdir(exist_ok=True)
        imports = (
            "from sdk_bindings.windows.foundation import Uri\n"
            "from sdk_bindings.windows.foundation.uri import Uri as PerTypeUri\n"
            "from sdk_bindings.windows__foundation__uri import Uri as FlatUri\n"
            "from sdk_bindings import Uri as RootUri\n"
        )
        valid = imports + (
            "from typing import assert_type\n"
            "def consume(uri: Uri) -> str:\n"
            "    assert_type(uri.combine_uri('child'), Uri)\n"
            "    return uri.host\n"
            "def facades(a: PerTypeUri, b: FlatUri, c: RootUri) -> None:\n"
            "    consume(a)\n    consume(b)\n    consume(c)\n"
        )
        if added:
            valid += (
                "from sdk_bindings.windows.globalization import Language\n"
                "def language(value: Language) -> str:\n"
                "    return value.language_tag\n"
            )
        (consumer / "valid.py").write_text(valid, encoding="utf-8")
        (consumer / "invalid.py").write_text(
            imports + "def reject(uri: Uri) -> None:\n    uri.combine_uri(123)\n",
            encoding="utf-8",
        )
        for name, errors in (("valid", 0), ("invalid", 1)):
            text = run([
                sys.executable, "-m", "mypy", "--strict", "--no-incremental",
                "--python-executable", python, "--cache-dir", consumer / "m",
                consumer / f"{name}.py",
            ], cwd=consumer, expected=errors)
            if errors:
                assert text.count(": error:") == 1 and "[arg-type]" in text, text
            text = run([
                sys.executable, "-m", "pyright", "--pythonpath", python,
                "--outputjson", consumer / f"{name}.py",
            ], cwd=consumer, expected=errors)
            diagnostics = json.loads(text)
            assert diagnostics["summary"]["errorCount"] == errors, text
            if errors:
                assert diagnostics["generalDiagnostics"][0]["rule"] == "reportArgumentType", text
        probe = """
import hashlib, importlib, importlib.metadata, json, pathlib, sys
from dynwinrt import RoApartment
from sdk_bindings.windows.foundation import Uri
from sdk_bindings.windows.foundation.uri import Uri as PerTypeUri
from sdk_bindings.windows__foundation__uri import Uri as FlatUri
from sdk_bindings import Uri as RootUri
assert Uri is PerTypeUri is FlatUri is RootUri
with RoApartment():
    assert Uri('https://example.com').host == 'example.com'
project = pathlib.Path(sys.argv[1])
package = pathlib.Path(importlib.import_module('sdk_bindings').__file__).parent
assert (package / 'py.typed').is_file()
entries = [e for e in importlib.metadata.entry_points(group='dynwinrt.winappsdk')
           if e.value == 'sdk_bindings._winappsdk:get_config']
manifest = project / '.dynwinrt-winappsdk.json'
assert len(entries) == int(manifest.is_file())
if manifest.is_file():
    root, config = entries[0].load()()
    assert pathlib.Path(root) == package
    expected = json.loads(manifest.read_text())
    assert config == expected['configuration']
    for name, digest in expected['files'].items():
        relative = pathlib.Path(name)
        if relative.is_relative_to(pathlib.Path('src/sdk_bindings')):
            data = (package / relative.relative_to('src/sdk_bindings')).read_bytes()
            assert hashlib.sha256(data).hexdigest() == digest, name
if sys.argv[2] == 'editable':
    pths = list(pathlib.Path(sys.prefix, 'Lib', 'site-packages').glob('*sdk_bindings*.pth'))
    assert len(pths) == 1, pths
    assert pths[0].read_text().strip() == str(project / 'src')
if sys.argv[3] == 'added':
    from sdk_bindings.windows.globalization import Language
    with RoApartment():
        assert Language('en-US').language_tag == 'en-US'
print(package)
"""
        run([python, "-I", "-B", "-c", probe, project,
             "editable" if editable else "wheel", "added" if added else "initial"], cwd=consumer)

    def inspect_wheel(wheel, project):
        with zipfile.ZipFile(wheel) as archive:
            names = archive.namelist()
            installed = {}
            for name in names:
                parts = name.split("/")
                if len(parts) > 2 and parts[0].endswith(".data") and parts[1] in ("purelib", "platlib"):
                    parts = parts[2:]
                target = "/".join(parts)
                assert not target.startswith(("src/", "_dynwinrt_build/")), name
                assert target not in installed, target
                installed[target] = name
            assert "sdk_bindings/py.typed" in installed
            assert "sdk_bindings/windows/foundation/uri.pyi" in installed
            expected = project / ".dynwinrt-winappsdk.json"
            if expected.is_file():
                bundle = json.loads(expected.read_text())
                platforms = sorted(
                    "win_amd64" if arch == "x64" else "win_arm64"
                    for arch in bundle["configuration"]["dlls"]
                )
                assert wheel.name.endswith(f"-py3-none-{'.'.join(platforms)}.whl"), wheel
                for name, digest in bundle["files"].items():
                    if name.startswith("src/sdk_bindings/"):
                        assert hashlib.sha256(archive.read(installed[name[4:]])).hexdigest() == digest, name
            else:
                assert wheel.name.endswith("-py3-none-any.whl"), wheel

    try:
        report["versions"]["uv"] = run([args.uv, "--version"]).strip()
        variants = [("none", [])]
        if args.bootstrap:
            variants.append(("single", args.bootstrap[:1]))
            if len(args.bootstrap) > 1:
                variants.append(("dual", args.bootstrap))
        for variant, bootstrap in variants:
            directory = root / variant
            directory.mkdir()
            project = directory / "sdk_bindings"
            generate(project, "Windows.Foundation.Uri", bootstrap)
            report["versions"]["codegen"] = json.loads(
                (project / ".dynwinrt-generator.json").read_text()
            )["generator_version"]
            wheel_dir = directory / "wheels"
            run([sys.executable, "-m", "build", "--wheel", "--outdir", wheel_dir, project])
            direct = next(wheel_dir.glob("*.whl"))
            inspect_wheel(direct, project)
            run([sys.executable, "-m", "build", "--sdist", "--outdir", wheel_dir, project])
            sdist = next(wheel_dir.glob("*.tar.gz"))
            rebuilt = directory / "rebuilt"
            run([sys.executable, "-m", "pip", "wheel", "--no-deps", "--wheel-dir", rebuilt, sdist])
            rebuilt_wheel = next(rebuilt.glob("*.whl"))
            inspect_wheel(rebuilt_wheel, project)
            editable_consumers = []
            for backend, wheel_version in (("68.0.0", "0.43.0"), ("84.2.1", "0.48.0")):
                case = directory / f"setuptools-{backend}"
                case.mkdir()
                python = venv(case / "venv")
                run([python, "-m", "pip", "install", f"setuptools=={backend}",
                     f"wheel=={wheel_version}", runtime])
                run([python, "-m", "pip", "wheel", "--no-build-isolation", "--no-deps",
                     "--wheel-dir", case / "wheels", project])
                inspect_wheel(next((case / "wheels").glob("*.whl")), project)
                run([python, "-m", "pip", "install", "--no-build-isolation", "--editable", project])
                check(python, project, True)
                editable_consumers.append(python)
                report["cases"].append({"bundle": variant, "backend": backend, "wheel": wheel_version, "status": "passed"})
            for frontend in ("pip-wheel", "pip-sdist", "pip-editable", "uv-normal", "uv-editable",
                             "uv-sync", "poetry-normal", "poetry-editable"):
                case = directory / frontend
                case.mkdir()
                editable = frontend in ("pip-editable", "uv-editable", "uv-sync", "poetry-editable")
                if frontend == "uv-sync":
                    (case / "pyproject.toml").write_text(
                        '[project]\nname = "consumer"\nversion = "0.0.0"\n'
                        'requires-python = ">=3.11,<3.15"\ndependencies = ["sdk-bindings", "dynwinrt"]\n'
                        '[tool.uv.sources]\n'
                        f'sdk-bindings = {{ path = {json.dumps(project.as_posix())}, editable = true }}\n'
                        f'dynwinrt = {{ path = {json.dumps(runtime.as_posix())} }}\n',
                        encoding="utf-8",
                    )
                    run([args.uv, "sync", "--python", sys._base_executable], cwd=case)
                    python = case / ".venv" / "Scripts" / "python.exe"
                elif frontend.startswith("poetry"):
                    (case / "pyproject.toml").write_text(
                        '[tool.poetry]\nname = "consumer"\nversion = "0.0.0"\npackage-mode = false\n'
                        '[tool.poetry.dependencies]\npython = ">=3.11,<3.15"\n'
                        f'sdk-bindings = {{ path = {json.dumps(project.as_posix())}, develop = {str(editable).lower()} }}\n'
                        f'dynwinrt = {{ path = {json.dumps(runtime.as_posix())} }}\n'
                        + (f'[[tool.poetry.source]]\nname = "test-mirror"\nurl = {json.dumps(args.index_url)}\npriority = "primary"\n'
                           if args.index_url else ""),
                        encoding="utf-8",
                    )
                    run([sys.executable, "-m", "poetry", "env", "use", sys._base_executable], cwd=case)
                    run([sys.executable, "-m", "poetry", "install", "--no-interaction"], cwd=case)
                    python = case / ".venv" / "Scripts" / "python.exe"
                else:
                    python = venv(case / "venv")
                    source = direct if frontend == "pip-wheel" else rebuilt_wheel if frontend == "pip-sdist" else project
                    installer = [args.uv, "pip", "install", "--python", python] if frontend.startswith("uv") else [python, "-m", "pip", "install"]
                    run(installer + [runtime] + (["--editable"] if editable else []) + [source])
                check(python, project, editable)
                report["cases"].append({"bundle": variant, "frontend": frontend, "status": "passed"})
                if editable:
                    editable_consumers.append(python)
            generate(project, "Windows.Globalization.Language")
            for python in editable_consumers:
                check(python, project, True, added=True)
            report["cases"].append({"bundle": variant, "incremental_without_reinstall": "passed"})
            print(f"{variant}: packaging, frontends, typing and incremental regeneration passed", flush=True)
    finally:
        (root / "results.json").write_text(json.dumps(report, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
