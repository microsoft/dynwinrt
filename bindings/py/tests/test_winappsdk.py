# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import copy
import hashlib
from pathlib import Path
from types import SimpleNamespace

import pytest

import dynwinrt
from dynwinrt import _winappsdk

ENVIRONMENT = "WINAPPSDK_BOOTSTRAP_DLL_PATH"
DLL = "Microsoft.WindowsAppRuntime.Bootstrap.dll"


def bundle(root, major=2, minor=5, architectures=("x64", "arm64")):
    config = {"schema_version": 1, "major": major, "minor": minor, "dlls": {}}
    for architecture in architectures:
        path = root / "_native" / "winappsdk" / architecture / DLL
        path.parent.mkdir(parents=True, exist_ok=True)
        data = f"test resource {architecture}".encode()
        path.write_bytes(data)
        config["dlls"][architecture] = hashlib.sha256(data).hexdigest()
    return str(root), config


def providers(monkeypatch, *values):
    entries = [
        SimpleNamespace(
            name=f"package_{index}",
            value=f"package_{index}._winappsdk:get_config",
            dist=SimpleNamespace(
                metadata={"Name": f"package-{index}"}, version="0.1.0"
            ),
            load=lambda value=value: lambda: value,
        )
        for index, value in enumerate(values)
    ]

    def discover(*, group):
        assert group == "dynwinrt.winappsdk"
        return entries

    monkeypatch.setattr(_winappsdk, "entry_points", discover)


@pytest.fixture
def native_calls(monkeypatch):
    calls = []
    result = object()

    def initialize(*args, **kwargs):
        calls.append((args, kwargs))
        return result

    monkeypatch.setattr(_winappsdk._native, "init_winappsdk", initialize)
    monkeypatch.setattr(_winappsdk.sysconfig, "get_platform", lambda: "win-amd64")
    monkeypatch.delenv(ENVIRONMENT, raising=False)
    return calls, result


def test_missing_provider_is_actionable(monkeypatch, native_calls):
    providers(monkeypatch)
    with pytest.raises(RuntimeError, match="No generated Windows App SDK configuration"):
        dynwinrt.init_winappsdk()
    assert native_calls[0] == []


@pytest.mark.parametrize("args,kwargs", [((2,), {}), ((), {"minor": 5})])
def test_partial_version_never_falls_back_to_discovery(monkeypatch, native_calls, args, kwargs):
    monkeypatch.setattr(_winappsdk, "entry_points", lambda **_: pytest.fail("must not discover"))
    with pytest.raises(TypeError, match="both major and minor"):
        dynwinrt.init_winappsdk(*args, **kwargs)
    assert native_calls[0] == []


def test_legacy_explicit_versions_bypass_configuration_and_preserve_env(monkeypatch, native_calls):
    monkeypatch.setenv(ENVIRONMENT, "old-relative-dll-path.dll")
    monkeypatch.setattr(_winappsdk, "entry_points", lambda **_: pytest.fail("must not discover"))
    calls, result = native_calls
    assert dynwinrt.init_winappsdk(2, 5) is result
    assert dynwinrt.init_winappsdk(major=2, minor=5) is result
    assert calls == [((2, 5), {}), ((2, 5), {})]
    assert _winappsdk.os.environ[ENVIRONMENT] == "old-relative-dll-path.dll"


@pytest.mark.parametrize("platform,architecture", [("win-amd64", "x64"), ("win-arm64", "arm64")])
def test_generated_resources_use_interpreter_architecture_and_not_cwd(
    monkeypatch, tmp_path, native_calls, platform, architecture
):
    root = tmp_path / "installed"
    providers(monkeypatch, bundle(root))
    monkeypatch.setattr(_winappsdk.sysconfig, "get_platform", lambda: platform)
    elsewhere = tmp_path / "different_cwd"
    elsewhere.mkdir()
    monkeypatch.chdir(elsewhere)
    calls, result = native_calls
    assert dynwinrt.init_winappsdk() is result
    assert calls == [
        ((2, 5), {"bootstrap_dll_path": str(root / "_native" / "winappsdk" / architecture / DLL)})
    ]
    assert ENVIRONMENT not in _winappsdk.os.environ


def test_consistent_providers_merge_and_conflicting_providers_list_sources(
    monkeypatch, tmp_path, native_calls
):
    first = bundle(tmp_path / "first")
    second = bundle(tmp_path / "second")
    providers(monkeypatch, first, second)
    dynwinrt.init_winappsdk()
    assert len(native_calls[0]) == 1
    for changed in [
        {**second[1], "minor": 6},
        {**second[1], "dlls": {**second[1]["dlls"], "x64": "0" * 64}},
    ]:
        providers(monkeypatch, first, (second[0], changed))
        with pytest.raises(RuntimeError, match="Conflicting") as error:
            dynwinrt.init_winappsdk()
        assert "package-0" in str(error.value) and "package-1" in str(error.value)
        assert len(native_calls[0]) == 1


def test_explicit_path_and_environment_override_generated_resource(
    monkeypatch, tmp_path, native_calls
):
    root, config = bundle(tmp_path / "package")
    providers(monkeypatch, (root, config))
    override = tmp_path / "override.dll"
    override.touch()
    explicit = tmp_path / "explicit.dll"
    explicit.touch()
    monkeypatch.setenv(ENVIRONMENT, str(override))
    for path in Path(root).rglob("*.dll"):
        path.unlink()
    dynwinrt.init_winappsdk()
    dynwinrt.init_winappsdk(bootstrap_dll_path=explicit)
    dynwinrt.init_winappsdk(2, 5, bootstrap_dll_path=explicit)
    assert [call[1]["bootstrap_dll_path"] for call in native_calls[0]] == [
        str(override), str(explicit), str(explicit)
    ]
    assert _winappsdk.os.environ[ENVIRONMENT] == str(override)


@pytest.mark.parametrize("override", ["", "   ", "bad\0path", "missing.dll"])
def test_bad_overrides_do_not_silently_use_bundle(
    monkeypatch, tmp_path, native_calls, override
):
    providers(monkeypatch, bundle(tmp_path / "package"))
    with pytest.raises((ValueError, FileNotFoundError)):
        dynwinrt.init_winappsdk(bootstrap_dll_path=override)
    monkeypatch.setenv(ENVIRONMENT, override.replace("\0", ""))
    with pytest.raises((ValueError, FileNotFoundError)):
        dynwinrt.init_winappsdk()
    assert not native_calls[0]


def test_modified_resource_rejected_before_native_call(monkeypatch, tmp_path, native_calls):
    root = tmp_path / "package"
    providers(monkeypatch, bundle(root))
    (root / "_native" / "winappsdk" / "x64" / DLL).write_bytes(b"changed")
    with pytest.raises(ValueError, match="digest mismatch"):
        dynwinrt.init_winappsdk()
    assert not native_calls[0]


def test_missing_generated_resource_reports_reinstallation(monkeypatch, tmp_path, native_calls):
    root = tmp_path / "package"
    providers(monkeypatch, bundle(root))
    (root / "_native" / "winappsdk" / "x64" / DLL).unlink()
    with pytest.raises(FileNotFoundError, match="Reinstall the complete generated"):
        dynwinrt.init_winappsdk()
    assert not native_calls[0]


def test_unsupported_architecture_is_clear(monkeypatch, tmp_path, native_calls):
    providers(monkeypatch, bundle(tmp_path / "package", architectures=("arm64",)))
    with pytest.raises(RuntimeError, match="has no x64 DLL"):
        dynwinrt.init_winappsdk()
    monkeypatch.setattr(_winappsdk.sysconfig, "get_platform", lambda: "win32")
    with pytest.raises(RuntimeError, match="Unsupported Python platform"):
        dynwinrt.init_winappsdk()


@pytest.mark.parametrize(
    "change",
    [
        lambda c: c.update(schema_version=2),
        lambda c: c.update(schema_version=True),
        lambda c: c.update(major=-1),
        lambda c: c.update(minor=65536),
        lambda c: c.update(major=True),
        lambda c: c.update(dlls={}),
        lambda c: c.update(dlls={"x86": "0" * 64}),
        lambda c: c.update(dlls={"x64": "bad"}),
        lambda c: c.update(extra="ignored?"),
    ],
)
def test_invalid_provider_fails_closed(monkeypatch, tmp_path, native_calls, change):
    root, original = bundle(tmp_path / "package")
    config = copy.deepcopy(original)
    change(config)
    providers(monkeypatch, (root, config))
    with pytest.raises(ValueError, match="configuration from package-0"):
        dynwinrt.init_winappsdk()
    assert not native_calls[0]


def test_native_failure_propagates_and_can_retry(monkeypatch, tmp_path):
    providers(monkeypatch, bundle(tmp_path / "package"))
    monkeypatch.delenv(ENVIRONMENT, raising=False)
    failure = OSError("bootstrap failed")
    count = 0
    returned = object()

    def initialize(*args, **kwargs):
        nonlocal count
        count += 1
        if count == 1:
            raise failure
        return returned

    monkeypatch.setattr(_winappsdk._native, "init_winappsdk", initialize)
    with pytest.raises(OSError) as error:
        dynwinrt.init_winappsdk()
    assert error.value is failure
    assert dynwinrt.init_winappsdk() is returned
    assert count == 2


def test_real_native_missing_input_errors_and_return_type_contract(monkeypatch, tmp_path):
    import ctypes

    monkeypatch.delenv(ENVIRONMENT, raising=False)
    with pytest.raises(OSError, match="bootstrap DLL path is required"):
        dynwinrt.init_winappsdk(2, 5)
    with pytest.raises(OSError, match="major version component"):
        dynwinrt.init_winappsdk(65536, 0)
    not_a_dll = tmp_path / "invalid.dll"
    not_a_dll.write_bytes(b"not a native library")
    kernel = ctypes.WinDLL("kernel32")
    kernel.SetErrorMode.argtypes = [ctypes.c_uint]
    kernel.SetErrorMode.restype = ctypes.c_uint
    previous = kernel.SetErrorMode(0x0001 | 0x8000)
    try:
        with pytest.raises(OSError):
            dynwinrt.init_winappsdk(2, 5, bootstrap_dll_path=not_a_dll)
    finally:
        kernel.SetErrorMode(previous)
    assert hasattr(dynwinrt.WinAppSDKContext, "resource_pri_path")
    assert not hasattr(dynwinrt.WinAppSDKContext, "__exit__")
    stub = (Path(dynwinrt.__file__).parent / "__init__.pyi").read_text(encoding="utf-8")
    assert "def init_winappsdk(*, bootstrap_dll_path:" in stub
    assert ") -> WinAppSDKContext:" in stub
