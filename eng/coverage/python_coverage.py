# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Combine subprocess coverage and report every measured Python product family."""

import argparse
import json
import os
from pathlib import Path

from coverage import Coverage, CoverageData
from coverage.exceptions import NoDataError


GENERATED_LAYERS = {
    "generated-winrt": "tests/e2e/e2e_generated/python_bindings/*",
    "generated-implementations": (
        "tests/e2e/e2e_generated/implementations/python_bindings/*"
    ),
}
RUNTIME_INCLUDES = ["*/site-packages/dynwinrt/*", "*/bindings/py/src/*.py"]


def _without_device_prefix(path: str) -> str:
    path = path.replace("\\", "/")
    if path[:8].lower() == "//?/unc/":
        return "//" + path[8:]
    if path.startswith("//?/"):
        return path[4:]
    return path


def normalize_path(path: str, root: Path) -> str:
    """Use one repo-relative identity for normal and extended Windows paths."""
    path = _without_device_prefix(path)
    prefix = _without_device_prefix(str(root)).rstrip("/") + "/"
    if path.lower().startswith(prefix.lower()):
        path = path[len(prefix):]
    return os.path.normpath(path)


def filesystem_path(path: Path) -> str:
    absolute = os.path.abspath(path)
    if os.name != "nt" or absolute.startswith("\\\\?\\"):
        return absolute
    if absolute.startswith("\\\\"):
        return "\\\\?\\UNC\\" + absolute[2:]
    return "\\\\?\\" + absolute


def normalize_data(data: CoverageData, root: Path) -> None:
    def source_path(path: str) -> str:
        path = normalize_path(path, root)
        absolute = os.path.normpath(os.path.join(root, path))
        # Removing the extended prefix from a genuinely long source makes it
        # unreadable on Windows hosts where normal paths still have MAX_PATH.
        if os.name == "nt" and len(absolute) >= 260:
            return filesystem_path(Path(absolute))
        return path

    normalized = CoverageData(no_disk=True)
    try:
        normalized.update(data, map_path=source_path)
        serialized = normalized.dumps()
    finally:
        # update() can attach the original SQLite database to this connection.
        # Release it before replacing the original data on Windows.
        normalized.close(force=True)
    data.erase()
    data.loads(serialized)


def _write_layer(coverage: Coverage, directory: Path, includes: list[str]) -> dict:
    directory = Path(filesystem_path(directory))
    directory.mkdir(parents=True, exist_ok=True)
    coverage.json_report(outfile=str(directory / "coverage.json"), include=includes)
    coverage.html_report(directory=str(directory / "html"), include=includes)
    coverage.xml_report(outfile=str(directory / "coverage.xml"), include=includes)
    coverage.lcov_report(outfile=str(directory / "lcov.info"), include=includes)
    return json.loads((directory / "coverage.json").read_text(encoding="utf-8"))


def write_reports(
    root: Path,
    directory: Path,
    config: Path,
    *,
    require_generated: bool = False,
) -> None:
    root, directory, config = root.resolve(), directory.resolve(), config.resolve()
    original_directory = Path.cwd()
    try:
        os.chdir(root)
        coverage = Coverage(config_file=str(config), data_file=str(directory / ".coverage"))
        coverage.combine(data_paths=[str(directory)], strict=True, keep=True)
        normalize_data(coverage.get_data(), root)
        coverage.save()

        # Keep relative_files enabled for diagnostics too. Without it the same
        # recorded relative filenames can be reported as entirely unexecuted.
        coverage.json_report(
            outfile=str(directory / "all-data.json"), include=["*"], omit=[]
        )
        includes = coverage.get_option("report:include")
        try:
            aggregate = _write_layer(coverage, directory, includes)
        except NoDataError:
            if require_generated:
                raise
            print("No Python product files were measured; skipping product reports.")
            return

        files = {
            normalize_path(path, root).replace("\\", "/"): entry
            for path, entry in aggregate["files"].items()
        }
        missing_layers = []
        generated_files = set()
        for name, pattern in GENERATED_LAYERS.items():
            prefix = pattern.removesuffix("*")
            selected = {path: entry for path, entry in files.items() if path.startswith(prefix)}
            generated_files.update(selected)
            if selected:
                _write_layer(coverage, directory / name, ["*/" + pattern])
            if require_generated and not any(
                entry["summary"]["covered_lines"] for entry in selected.values()
            ):
                missing_layers.append(name)

        if files.keys() - generated_files:
            _write_layer(coverage, directory / "runtime", RUNTIME_INCLUDES)
        coverage.report(include=includes)
        if missing_layers:
            raise RuntimeError(
                "Python coverage did not execute required source families: "
                + ", ".join(missing_layers)
            )
    finally:
        os.chdir(original_directory)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--require-generated", action="store_true")
    args = parser.parse_args()
    write_reports(
        args.root, args.output, args.config, require_generated=args.require_generated
    )


if __name__ == "__main__":
    main()
