# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Generated bindings route every WinRT ``Object`` position through one layer.

Generates Windows SDK bindings with ``dynwinrt-codegen`` (``DYNWINRT_CODEGEN``
or the local ``target`` directory) and runs each scenario in
``object_values_scenarios.py`` in a fresh process: collections, factories,
real property stores, and event arguments typed ``Object``.
"""

import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

TESTS = Path(__file__).resolve().parent
ROOT = TESTS.parents[2]
SCENARIOS = TESTS / "object_values_scenarios.py"
PACKAGE = "object_values_sdk"
NAMES = re.findall(r"^def scenario_(\w+)", SCENARIOS.read_text(encoding="utf-8"), re.M)


def _codegen():
    configured = os.environ.get("DYNWINRT_CODEGEN")
    candidates = (
        [Path(configured)]
        if configured
        else [ROOT / "target" / profile / "dynwinrt-codegen.exe" for profile in ("debug", "release")]
    )
    return next((path for path in candidates if path.is_file()), None)


@pytest.fixture(scope="module")
def generated(tmp_path_factory):
    codegen = _codegen()
    if codegen is None:
        pytest.skip("dynwinrt-codegen is unavailable; set DYNWINRT_CODEGEN")
    sys.path.insert(0, str(TESTS))
    try:
        from object_values_scenarios import CLASSES
    finally:
        sys.path.remove(str(TESTS))
    root = tmp_path_factory.mktemp("object_values")
    result = subprocess.run(
        [str(codegen), "generate", "--class-name", CLASSES, "--lang", "py",
         "--output", str(root / PACKAGE)],
        capture_output=True, text=True, timeout=900,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    return root


@pytest.mark.parametrize("scenario", NAMES)
def test_generated_object_positions(generated, scenario):
    result = subprocess.run(
        [sys.executable, "-B", str(SCENARIOS), str(generated), PACKAGE, scenario],
        capture_output=True, text=True, timeout=300,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    if "object-values-skip:" in result.stdout:
        pytest.skip(result.stdout.split("object-values-skip:", 1)[1].strip())
    assert f"object-values-ok: {scenario}" in result.stdout, result.stdout + result.stderr


def test_every_scenario_is_collected():
    assert len(NAMES) == 8
