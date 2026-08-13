# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from pathlib import Path

import dynwinrt


def test_wheel_contains_typing_metadata():
    package_dir = Path(dynwinrt.__file__).parent

    assert (package_dir / "__init__.pyi").is_file()
    assert (package_dir / "py.typed").is_file()
