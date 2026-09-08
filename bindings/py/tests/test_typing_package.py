# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from pathlib import Path
import inspect

import dynwinrt


def test_wheel_contains_typing_metadata():
    package_dir = Path(dynwinrt.__file__).parent

    assert (package_dir / "__init__.pyi").is_file()
    assert (package_dir / "py.typed").is_file()


def test_wheel_exports_typed_implementation_surface():
    stub = (Path(dynwinrt.__file__).parent / "__init__.pyi").read_text(encoding="utf-8")
    for name in (
        "DynWinRTImplementationMethod",
        "DynWinRTInterfacePlan",
        "DynWinRTImplementationDescriptor",
        "DynWinRTImplementation",
        "DynWinRTDelegateMethod",
    ):
        assert name in dynwinrt.__all__
        assert hasattr(dynwinrt, name)
        assert f"class {name}:" in stub
    signature = inspect.signature(dynwinrt.DynWinRTInterfacePlan.create)
    assert signature.parameters["required_iids"].default == ()
    assert "def from_hresult(" in stub
