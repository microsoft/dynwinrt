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
    assert "DynWinRTImplementationHandle" in dynwinrt.__all__
    assert "class DynWinRTImplementationHandle(Generic[_Projected_co])" in stub
    assert dynwinrt.DynWinRTImplementationHandle[int]
    signature = inspect.signature(dynwinrt.DynWinRTInterfacePlan.create)
    assert signature.parameters["required_iids"].default == ()
    assert "def from_hresult(" in stub


def test_wheel_stubs_raw_native_scope_tracking():
    stub = (Path(dynwinrt.__file__).parent / "__init__.pyi").read_text(encoding="utf-8")
    scope = stub.split("class ProjectedLifetimeScope:", 1)[1].split(
        "\ndef projected_lifetime_scope()", 1
    )[0]
    assert 'def track_native(self, value: "DynWinRTValue") -> "DynWinRTValue": ...' in scope
    assert tuple(inspect.signature(dynwinrt.ProjectedLifetimeScope.track_native).parameters) == (
        "self",
        "value",
    )
