# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""WinUI ownership regressions; live cases require explicitly prepared inputs."""

import os
from pathlib import Path
import subprocess
import sys
import textwrap

import pytest


def run_child(code, *, cwd=None, env=None):
    environment = os.environ.copy()
    environment.update(
        PYTHONUNBUFFERED="1",
        PYTHONFAULTHANDLER="1",
        PYTHONDONTWRITEBYTECODE="1",
    )
    if env:
        environment.update(env)
    result = subprocess.run(
        [sys.executable, "-B", "-c", textwrap.dedent(code)],
        cwd=cwd,
        env=environment,
        capture_output=True,
        text=True,
        timeout=90,
    )
    assert result.returncode == 0, (
        f"native exit 0x{result.returncode & 0xFFFFFFFF:08X}\n"
        f"{result.stdout}\n{result.stderr}"
    )
    return result.stdout


def test_plain_winrt_keeps_optional_modules_unloaded():
    run_child("""
        import ctypes
        import dynwinrt as dw
        from types import SimpleNamespace
        from dynwinrt.dynwinrt import _dynwinrt_track_projected

        kernel = ctypes.WinDLL("kernel32")
        kernel.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
        kernel.GetModuleHandleW.restype = ctypes.c_void_p
        def check():
            for name in ("Microsoft.UI.Xaml.dll", "Microsoft.UI.Xaml.Controls.dll", "CoreMessaging.dll"):
                assert not kernel.GetModuleHandleW(name), name
        check()
        for _ in range(2):
            with dw.RoApartment(0), dw.projected_lifetime_scope():
                factory = dw.DynWinRTValue.activation_factory("Windows.Foundation.Uri")
                _dynwinrt_track_projected(SimpleNamespace(_obj=factory))
                with dw.RoApartment(0):
                    assert not factory.is_null()
            assert factory.is_null()
        check()
        assert not hasattr(dw, "WinUIHost")
    """)


@pytest.mark.parametrize("failure", ["fail", "null", "forward"])
def test_winui_fallback_failure_retry_and_deduplicated_ownership(failure):
    fixture_dir = os.environ.get("DYNWINRT_WINUI_FIXTURE_DIR")
    if not fixture_dir:
        pytest.skip("requires the SDK-ABI WinUI activation DLL fixture")
    assert (Path(fixture_dir) / "Microsoft.UI.Xaml.dll").is_file()
    run_child("""
        import ctypes
        import os
        import threading
        import dynwinrt as dw
        from dynwinrt.dynwinrt import _test_winui_module_paths, _test_winui_owned_references

        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.SetDllDirectoryW.argtypes = [ctypes.c_wchar_p]
        kernel.SetDllDirectoryW.restype = ctypes.c_int
        kernel.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
        kernel.GetModuleHandleW.restype = ctypes.c_void_p
        kernel.GetProcAddress.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
        kernel.GetProcAddress.restype = ctypes.c_void_p
        assert kernel.SetDllDirectoryW(os.getcwd())
        name = "Microsoft.UI.Xaml.DynWinRTFixture"
        assert _test_winui_module_paths() == []
        assert _test_winui_owned_references() == 0
        with dw.RoApartment(1):
            try:
                dw.DynWinRTValue.activation_factory(name)
            except OSError as error:
                expected = {
                    "fail": -2147024891,
                    "null": -2147467261,
                    "forward": -2147418113,
                }
                assert error.winerror == expected[os.environ["DYNWINRT_WINUI_FIXTURE_MODE"]]
            else:
                raise AssertionError("invalid factory was accepted")
            assert _test_winui_module_paths() == []
            assert _test_winui_owned_references() == 0
            assert not kernel.GetModuleHandleW("Microsoft.UI.Xaml.dll")
            os.environ.pop("DYNWINRT_WINUI_FIXTURE_MODE")
            failures = []
            barrier = threading.Barrier(8)
            def activate():
                try:
                    with dw.RoApartment(1):
                        barrier.wait(10)
                        for _ in range(10):
                            factory = dw.DynWinRTValue.activation_factory(name)
                            factory.release()
                            factory.release()
                except BaseException as error:
                    failures.append(error)
            workers = [threading.Thread(target=activate) for _ in range(8)]
            for worker in workers:
                worker.start()
            for worker in workers:
                worker.join(20)
                assert not worker.is_alive()
            assert not failures, failures
            assert len(_test_winui_module_paths()) == 1
            assert _test_winui_owned_references() == 1
            handle = kernel.GetModuleHandleW("Microsoft.UI.Xaml.dll")
            assert handle
            address = kernel.GetProcAddress(handle, b"FixtureObjectCount")
            assert address
            count = ctypes.WINFUNCTYPE(ctypes.c_ulong)(address)
            assert count() == 0
        assert kernel.GetModuleHandleW("Microsoft.UI.Xaml.dll")
        assert len(_test_winui_module_paths()) == 1
        assert _test_winui_owned_references() == 1
        assert kernel.SetDllDirectoryW(None)
    """, cwd=fixture_dir, env={"DYNWINRT_WINUI_FIXTURE_MODE": failure})


@pytest.mark.parametrize(
    "mode", ["plain", "managed", "external", "tokio", "default-async", "body-error", "provider-error"]
)
def test_real_winui_teardown(mode):
    sample_root = os.environ.get("DYNWINRT_WINUI_SAMPLE_ROOT")
    if not sample_root:
        pytest.skip("requires generated Hello World and its restored WinUI runtime")
    runner = Path(__file__).resolve().parents[3] / "tests" / "e2e" / "runners" / "py_winui_lifetime.py"
    command = [
        sys.executable, "-B", str(runner), "--sample-root", sample_root, "--mode", mode,
    ]
    if mode == "tokio" or os.environ.get("DYNWINRT_WINUI_REQUIRE_OWNER_HOOK") == "1":
        command.append("--require-owner-hook")
    environment = os.environ.copy()
    environment.update(PYTHONUNBUFFERED="1", PYTHONFAULTHANDLER="1", PYTHONDONTWRITEBYTECODE="1")
    result = subprocess.run(command, env=environment, capture_output=True, text=True, timeout=90)
    assert result.returncode == 0, (
        f"native exit 0x{result.returncode & 0xFFFFFFFF:08X}\n{result.stdout}\n{result.stderr}"
    )
    assert '"event": "winui-lifetime-ok"' in result.stdout
