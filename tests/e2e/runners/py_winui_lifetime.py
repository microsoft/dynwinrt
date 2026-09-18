# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Run the unchanged Hello World with observable native lifetime boundaries."""

import argparse
import asyncio
import ctypes
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import threading


def run(sample_root, mode, require_owner_hook):
    sys.path.insert(0, str(sample_root))
    import dynwinrt as dw
    from dynwinrt import dynwinrt as native

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
    kernel.GetModuleHandleW.restype = ctypes.c_void_p
    kernel.GetModuleFileNameW.argtypes = [
        ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_uint32
    ]
    kernel.GetModuleFileNameW.restype = ctypes.c_uint32
    kernel.GetProcAddress.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    kernel.GetProcAddress.restype = ctypes.c_void_p
    owner_paths = getattr(native, "_test_winui_module_paths", None)
    if require_owner_hook and owner_paths is None:
        raise RuntimeError("This case requires a test-hooks wheel")

    def record(event, **values):
        print(json.dumps({"event": event, "thread": threading.get_native_id(), **values}), flush=True)

    def snapshot(phase):
        state = {
            "xaml_loaded": bool(kernel.GetModuleHandleW("Microsoft.UI.Xaml.dll")),
            "controls_loaded": bool(kernel.GetModuleHandleW("Microsoft.UI.Xaml.Controls.dll")),
        }
        if owner_paths is not None:
            state["owned_modules"] = owner_paths()
            state["owned_loader_references"] = native._test_winui_owned_references()
            assert state["owned_loader_references"] == len(state["owned_modules"])
        record("modules", phase=phase, **state)
        return state

    assert not snapshot("import")["xaml_loaded"]
    if owner_paths is not None:
        assert owner_paths() == []
    app_path = sample_root / "app.py"
    record(
        "inputs", mode=mode, python=sys.version, executable=sys.executable,
        app_sha256=hashlib.sha256(app_path.read_bytes()).hexdigest(),
        pyd_sha256=hashlib.sha256(Path(native.__file__).read_bytes()).hexdigest(),
    )
    spec = importlib.util.spec_from_file_location("hello_lifetime", app_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot load {app_path}")
    app = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(app)

    if mode == "provider-error":
        context = dw.init_winappsdk(2, 3)
        try:
            with dw.RoApartment(0), dw.projected_lifetime_scope() as scope:
                factory = dw.DynWinRTValue.activation_factory(
                    "Microsoft.UI.Xaml.XamlTypeInfo.XamlControlsXamlMetaDataProvider"
                )
                try:
                    provider = scope.track(factory.activate())
                    assert not provider.is_null()
                    state = snapshot("provider-before-application")
                    assert state["xaml_loaded"]
                    raise ValueError("intentional provider setup failure")
                finally:
                    factory.release()
        except ValueError as error:
            assert str(error) == "intentional provider setup failure"
            assert provider.is_null()
        finally:
            del context
        state = snapshot("provider-error-cleanup-finished")
        assert state["xaml_loaded"] and not state["controls_loaded"]
        if owner_paths is not None:
            assert len(owner_paths()) == 1
        record("winui-lifetime-ok", mode=mode)
        return

    if mode == "default-async":
        def use_default_async():
            with dw.RoApartment(1):
                async def read_file():
                    file_type = dw.DynWinRTType.runtime_class(
                        "Windows.Storage.StorageFile",
                        dw.DynWinRTType.interface(dw.WinGUID.parse(
                            "FA3F6186-4214-428C-A64C-14C9AC7315EA"
                        )),
                    )
                    iid = dw.WinGUID.parse("5984C710-DAF2-43C8-8BB4-A4D3EACFD03F")
                    statics = dw.DynWinRTType.register_interface("IStorageFileStatics", iid)
                    statics.add_method(
                        "GetFileFromPathAsync",
                        dw.DynWinRTMethodSig().add_in(dw.DynWinRTType.hstring()).add_out(
                            dw.DynWinRTType.i_async_operation(file_type)
                        ),
                    )
                    factory = dw.DynWinRTValue.activation_factory("Windows.Storage.StorageFile")
                    factory_view = factory.cast(iid)
                    try:
                        raw = statics.method(6).invoke(factory_view, [
                            dw.DynWinRTValue.from_hstring(str(app_path))
                        ])
                        operation = native._DynWinRTAsync(raw, lambda value: value)
                        raw.release()
                        try:
                            result = await operation
                            assert not result.is_null()
                            result.release()
                        finally:
                            operation.release()
                    finally:
                        factory_view.release()
                        factory.release()
                asyncio.run(read_file())
            record("default-async-completed", runtime_joined=False)
        use_default_async()

    apartment = app.RoApartment
    scope = app.projected_lifetime_scope
    start = app.Application.start
    observed = {"scope": False, "sta": False}

    class ApartmentTrace:
        def __init__(self, apartment_type):
            self.inner = apartment(apartment_type)

        def __enter__(self):
            return self.inner.__enter__()

        def __exit__(self, *exception):
            assert observed["scope"]
            record("sta-uninitialize-begin")
            result = self.inner.__exit__(*exception)
            observed["sta"] = True
            record("sta-uninitialize-end")
            snapshot("sta-finished")
            return result

    class ScopeTrace:
        def __init__(self):
            self.inner = scope()

        def __enter__(self):
            return self.inner.__enter__()

        def __exit__(self, *exception):
            count = len(self.inner._registry)
            record("projected-release-begin", count=count)
            result = self.inner.__exit__(*exception)
            assert self.inner.disposed and not self.inner._registry
            assert count > 0
            observed["scope"] = True
            record("projected-release-end", count=count, remaining=0)
            return result

    def traced_start(callback):
        for _ in range(5):
            factory = dw.DynWinRTValue.activation_factory("Microsoft.UI.Xaml.Application")
            factory.release()
        state = snapshot("before-start")
        assert state["xaml_loaded"]
        if owner_paths is not None:
            assert len(owner_paths()) == 1
        result = start(callback)
        record("application-start-returned")
        if mode == "body-error":
            raise ValueError("intentional post-start body error")
        return result

    app.RoApartment = ApartmentTrace
    app.projected_lifetime_scope = ScopeTrace
    app.Application.start = staticmethod(traced_start)

    def hello():
        try:
            app.run(True, 2, 3)
        except ValueError as error:
            if mode != "body-error" or str(error) != "intentional post-start body error":
                raise
            record("expected-body-error")
        assert observed == {"scope": True, "sta": True}

    if mode in ("managed", "external"):
        ready, finish = threading.Event(), threading.Event()
        errors = []

        def hold_mta():
            record("mta-initialized", owner=mode)
            ready.set()
            if not finish.wait(60):
                raise TimeoutError("STA did not finish")
            assert observed["sta"]
            record("mta-uninitialize-begin", owner=mode)

        def worker():
            try:
                if mode == "managed":
                    with dw.RoApartment(1):
                        hold_mta()
                else:
                    com = ctypes.WinDLL("combase")
                    com.RoInitialize.argtypes = [ctypes.c_uint32]
                    com.RoInitialize.restype = ctypes.c_long
                    com.RoUninitialize.argtypes = []
                    com.RoUninitialize.restype = None
                    hr = com.RoInitialize(1)
                    assert hr >= 0, hr
                    try:
                        hold_mta()
                    finally:
                        com.RoUninitialize()
                record("mta-uninitialize-end", owner=mode)
            except BaseException as error:
                errors.append(error)
                ready.set()

        thread = threading.Thread(target=worker)
        thread.start()
        try:
            assert ready.wait(10)
            if errors:
                raise errors[0]
            hello()
        finally:
            finish.set()
            thread.join(60)
            assert not thread.is_alive()
        if errors:
            raise errors[0]
        record("mta-joined")
    elif mode == "tokio":
        events = native._test_winui_with_mta_runtime(hello)
        started = sorted(thread for started, thread in events if started)
        stopped = sorted(thread for started, thread in events if not started)
        assert len(started) >= 3 and started == stopped, events
        record("tokio-stopped-and-joined", started=started, stopped=stopped)
    else:
        hello()

    state = snapshot("all-explicit-cleanup-finished")
    assert state["xaml_loaded"]
    if mode != "default-async":
        assert not state["controls_loaded"]
    if owner_paths is not None:
        assert len(owner_paths()) == 1
    xaml = kernel.GetModuleHandleW("Microsoft.UI.Xaml.dll")
    can_unload = kernel.GetProcAddress(xaml, b"DllCanUnloadNow")
    assert can_unload
    hr = ctypes.WINFUNCTYPE(ctypes.c_long)(can_unload)()
    assert hr in (0, 1), hr
    record("xaml-code-call-after-cleanup", hresult=hr)
    record("winui-lifetime-ok", mode=mode)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--sample-root", type=Path, required=True)
    parser.add_argument(
        "--mode",
        choices=("plain", "managed", "external", "tokio", "default-async", "body-error", "provider-error"),
        default="plain",
    )
    parser.add_argument("--require-owner-hook", action="store_true")
    args = parser.parse_args()
    run(args.sample_root.resolve(), args.mode, args.require_owner_hook)
