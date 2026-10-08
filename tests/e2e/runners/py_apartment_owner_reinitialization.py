# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Real delegate finalizers must preserve initialization leases during owner drain."""

import argparse
import ctypes
import gc
import importlib
import os
import sys
import threading
import traceback
import weakref


SCENARIOS = (
    "close", "exit", "drop", "manual", "retry",
    "close_failure", "manual_failure", "retry_failure",
    "manual_nested_pair", "retry_nested_pair",
    "recover", "recover_failure",
    "foreign_retry", "foreign_retry_failure", "foreign_retry_nested_pair",
    "foreign_recover", "foreign_recover_failure",
)


def run_case(generated, mode, scenario):
    import dynwinrt as dw
    import dynwinrt.dynwinrt as native

    if generated is None:
        # Windows.Foundation.DeferralCompletedHandler has an empty Invoke signature.
        iid = dw.WinGUID.parse("ed32a372-f3c8-4faa-9cfb-470148da3888")
        parameter_types = []
        uri_type = None
    else:
        sys.path.insert(0, os.path.dirname(os.path.abspath(generated)))
        package = os.path.basename(os.path.abspath(generated))
        metadata = importlib.import_module(
            f"{package}.windows__foundation__deferral_completed_handler"
        )
        iid = metadata.IID_DeferralCompletedHandler
        parameter_types = metadata.DeferralCompletedHandler_PARAM_TYPES
        uri_type = importlib.import_module(f"{package}.windows.foundation").Uri
    assert parameter_types == []

    combase = ctypes.WinDLL("combase.dll")
    combase.RoInitialize.argtypes = (ctypes.c_uint32,)
    combase.RoInitialize.restype = ctypes.c_long
    combase.RoUninitialize.argtypes = ()
    combase.RoUninitialize.restype = None
    manual = scenario.startswith("manual")
    foreign = scenario.startswith("foreign_")
    retry = scenario.startswith("retry") or scenario.startswith("foreign_retry")
    recover = "recover" in scenario
    pending = retry or recover
    nested_pair = scenario.endswith("nested_pair")
    fail_after = scenario.endswith("failure")
    finalized = []
    destructor_errors = []
    unraisable = []
    observed = []
    calls = []
    original_hook = sys.unraisablehook
    sys.unraisablehook = lambda event: unraisable.append(str(event.exc_value))

    def unavailable(action, messages):
        try:
            action()
        except RuntimeError as error:
            assert any(message in str(error) for message in messages), error
        else:
            raise AssertionError("an in-progress or completed lease was consumed twice")

    def check_pending_unavailable():
        unavailable(
            dw.retry_pending_apartment_close,
            ("no failed RoApartment close", "already in progress"),
        )
        unavailable(
            dw.RoApartment.recover_pending,
            ("no dropped RoApartment", "already in progress"),
        )

    class Captured:
        def __del__(self):
            try:
                frame = sys._getframe()
                in_drain = False
                while frame is not None:
                    in_drain |= frame.f_code.co_name == "_dynwinrt_drain_apartment_owners"
                    frame = frame.f_back
                assert in_drain, "captured object finalized outside the owner drain"
                assert native._managed_apartment_depth() == 1
                if nested_pair:
                    if manual:
                        unavailable(dw.ro_uninitialize, ("requires a successful",))
                    else:
                        check_pending_unavailable()
                try:
                    dw.ro_initialize(1 - mode)
                except OSError as error:
                    assert error.winerror == -2147417850, error  # RPC_E_CHANGED_MODE
                else:
                    raise AssertionError("a conflicting initialization succeeded")
                assert native._managed_apartment_depth() == 1
                same_model = combase.RoInitialize(mode)
                assert same_model == 1, same_model  # S_FALSE also requires a paired close.
                combase.RoUninitialize()
                dw.ro_initialize(mode)
                assert native._managed_apartment_depth() == 2
                if nested_pair:
                    if retry:
                        check_pending_unavailable()
                    dw.ro_uninitialize()
                    assert native._managed_apartment_depth() == 1
                    if manual:
                        unavailable(dw.ro_uninitialize, ("requires a successful",))
                finalized.append((in_drain, same_model))
            except BaseException:
                destructor_errors.append(traceback.format_exc())

    class FailingOwner:
        def __init__(self):
            self.checks = 0
            self.releases = 0

        def _check_apartment_release(self):
            self.checks += 1
            if pending and self.checks == 1:
                raise RuntimeError("initial owner drain failed")

        def release(self):
            self.releases += 1
            if fail_after and self.releases == 1:
                assert finalized, "failure occurred before the reinitializing finalizer"
                raise RuntimeError("owner drain failed after reinitialization")

    def make_delegate():
        captured = Captured()
        observed.append(weakref.ref(captured))

        def callback():
            assert captured is observed[0]()
            calls.append("native")

        return dw.DynWinRtDelegate.create(iid, parameter_types, callback)

    def reference_count(value):
        pointer = value.as_raw()
        vtable = ctypes.c_void_p.from_address(pointer).value
        width = ctypes.sizeof(ctypes.c_void_p)
        call = ctypes.WINFUNCTYPE(ctypes.c_uint32, ctypes.c_void_p)
        add_ref = call(ctypes.c_void_p.from_address(vtable + width).value)
        release = call(ctypes.c_void_p.from_address(vtable + 2 * width).value)
        added = add_ref(pointer)
        remaining = release(pointer)
        assert added == remaining + 1
        return remaining

    apartment = None
    if manual:
        dw.ro_initialize(mode)
    else:
        apartment = dw.RoApartment(mode)
        apartment.__enter__()
    failure = native._dynwinrt_track_native(FailingOwner()) if pending or fail_after else None
    delegate = make_delegate()
    alias = delegate.to_value().cast(iid)
    assert alias.invoke_delegate(iid, dw.DynWinRTMethodSig(), []) == []
    assert calls == ["native"] and not unraisable
    references = reference_count(alias)
    delegate.release()
    remaining = reference_count(alias)
    assert references == remaining + 1 and remaining == 1
    gc.collect()
    assert observed[0]() is not None and not finalized and not destructor_errors

    if pending:
        try:
            apartment.close()
        except RuntimeError as error:
            assert str(error) == "initial owner drain failed", error
        else:
            raise AssertionError("owner-drain preflight failure was hidden")
        assert "active=true" in repr(apartment) and not alias.is_released()
        assert native._managed_apartment_depth() == 1 and not finalized
        if foreign:
            holder = [apartment]
            apartment = None
            worker = threading.Thread(target=holder.clear)
            worker.start()
            worker.join(5)
            assert not worker.is_alive() and not holder, "foreign guard Drop did not complete"
        else:
            apartment = None
        gc.collect()
        assert failure.releases == 0, "guard Drop retried an already failed close"
        if recover:
            apartment = dw.RoApartment.recover_pending()
            assert f"apartment_type={mode}, active=true" in repr(apartment)
            check_pending_unavailable()

    def close():
        nonlocal apartment
        if manual:
            dw.ro_uninitialize()
        elif retry:
            dw.retry_pending_apartment_close()
        elif scenario == "exit":
            assert apartment.__exit__(None, None, None) is False
        elif scenario == "drop":
            apartment = None
            gc.collect()
        else:
            apartment.close()

    if fail_after:
        try:
            close()
        except RuntimeError as error:
            assert str(error) == "owner drain failed after reinitialization", error
        else:
            raise AssertionError("failure after reinitialization was hidden")
        assert native._managed_apartment_depth() == 2
        assert failure.releases == 1 and finalized == [(True, 1)]
        if apartment is not None:
            assert "active=true" in repr(apartment)
    close()
    gc.collect()
    assert not destructor_errors and not unraisable, (destructor_errors, unraisable)
    assert finalized == [(True, 1)] and observed[0]() is None
    assert delegate.is_released() and alias.is_released()
    if apartment is not None:
        assert "active=false" in repr(apartment)
        apartment.close()
    if nested_pair:
        assert native._managed_apartment_depth() == 0
    else:
        assert native._managed_apartment_depth() == 1
        if uri_type is None:
            live = dw.DynWinRTValue.activation_factory("Windows.Foundation.Uri")
            value = live
        else:
            live = uri_type("https://example.com/reinitialized")
            assert live.host == "example.com"
            value = live._obj
        scalar = dw.DynWinRTArray.from_i32_values([17])
        dw.ro_uninitialize()
        assert value.is_released() and scalar.to_i32_list() == [17]
        if failure is not None:
            assert failure.releases == (2 if fail_after else 1)
    assert native._managed_apartment_depth() == 0
    unavailable(dw.ro_uninitialize, ("requires a successful",))
    check_pending_unavailable()
    opposite = combase.RoInitialize(1 - mode)
    assert opposite == 0, f"native initialization leaked: {opposite:#x}"
    combase.RoUninitialize()
    assert not destructor_errors and not unraisable, (destructor_errors, unraisable)
    sys.unraisablehook = original_hook
    print(
        f"PASS owner-reinitialization {scenario} apartment={mode} "
        f"native-refs={references}->{remaining}->finalized S_FALSE=1 opposite=S_OK",
        flush=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--generated")
    parser.add_argument("--mode", type=int, choices=(0, 1), required=True)
    parser.add_argument("--scenario", choices=SCENARIOS, required=True)
    args = parser.parse_args()
    run_case(args.generated, args.mode, args.scenario)
