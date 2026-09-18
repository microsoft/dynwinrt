# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Shipped management source and its real native lifecycle paths."""

import importlib.util
import inspect
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import textwrap
from typing import get_origin
from uuid import uuid4

import pytest

import dynwinrt
import dynwinrt.dynwinrt as native
from dynwinrt import (
    DynWinRTImplementation,
    DynWinRTImplementationDescriptor,
    DynWinRTImplementationHandle,
    DynWinRTImplementationMethod,
    DynWinRTInterfacePlan,
    DynWinRTMethodSig,
    DynWinRTType,
    RoApartment,
    WinGUID,
)


ROOT = Path(__file__).resolve().parents[3]
PACKAGE = Path(dynwinrt.__file__).resolve().parent
SOURCE = PACKAGE / "_implementation.py"
CLOSED = -2147483629


@pytest.fixture
def closable():
    iid = WinGUID.parse("30d5a829-7fa4-4026-83bb-d75bae4ea99e")
    signature = DynWinRTMethodSig()
    typ = DynWinRTType.register_interface("Windows.Foundation.IClosable", iid)
    typ = typ.add_method("Close", signature)
    plan = DynWinRTInterfacePlan.create(
        "Windows.Foundation.IClosable",
        typ,
        [DynWinRTImplementationMethod("Close", 6, signature)],
    )

    class Closable:
        _dynwinrt_interface_type = True

        def __init__(self, value):
            self._obj = value

        @staticmethod
        def implementation(handler):
            def dispatch(slot, args):
                assert slot == 6 and args == []
                return handler()

            return DynWinRTImplementationDescriptor(plan, dispatch)

        @classmethod
        def from_implementation(cls, owner):
            value = owner.to_value()
            try:
                return cls(value.cast(iid))
            finally:
                value.release()

        def close(self):
            assert typ.method(6).invoke_all(self._obj, []) == []

    with RoApartment(1):
        yield Closable, plan


@pytest.fixture
def source_artifacts():
    directory = ROOT / "bindings" / "py" / "target" / ("source-test-" + uuid4().hex)
    directory.mkdir(parents=True)
    try:
        yield directory
    finally:
        shutil.rmtree(directory)


def test_management_code_has_retrievable_source_and_unchanged_identity():
    assert SOURCE.is_file()
    assert SOURCE.read_text(encoding="utf-8") == (
        ROOT / "bindings" / "py" / "python" / "dynwinrt" / "_implementation.py"
    ).read_text(encoding="utf-8")
    assert SOURCE == Path(native.__spec__.origin).with_name("_implementation.py")
    assert DynWinRTImplementationHandle.__module__ == "dynwinrt.dynwinrt"
    assert DynWinRTImplementationDescriptor.__module__ == "dynwinrt.dynwinrt"
    assert native.DynWinRTImplementationHandle is DynWinRTImplementationHandle
    assert get_origin(DynWinRTImplementationHandle[int]) is DynWinRTImplementationHandle
    functions = [
        value.fget if isinstance(value, property)
        else value.__func__ if isinstance(value, staticmethod)
        else value
        for cls in (DynWinRTImplementationHandle, DynWinRTImplementationDescriptor)
        for value in vars(cls).values()
        if inspect.isfunction(value) or isinstance(value, (property, staticmethod))
    ]
    functions.extend((
        native._dynwinrt_checked_implementation_callback,
        native._dynwinrt_require_sync_implementation_callable,
        native._dynwinrt_validate_implementation_result,
    ))
    for function in functions:
        assert Path(function.__code__.co_filename) == SOURCE
        assert function.__globals__ is vars(native)
        assert f"def {function.__name__}(" in inspect.getsource(function)
    spec = importlib.util.find_spec("dynwinrt._implementation")
    assert Path(spec.origin) == SOURCE
    assert spec.loader.get_source(spec.name) == SOURCE.read_text(encoding="utf-8")
    assert (PACKAGE / "__init__.pyi").read_text(encoding="utf-8") == (
        ROOT / "bindings" / "py" / "dynwinrt.pyi"
    ).read_text(encoding="utf-8")


def test_lazy_primary_view_release_and_retained_native_alias(closable):
    interface, _ = closable
    calls = []
    handle = DynWinRTImplementationHandle._create(
        interface, lambda: calls.append("close") or [], (), ()
    )
    assert handle._value is None
    with handle:
        view = handle.value
        assert handle.value is view
        raw = handle.to_value()
        alias = interface.from_implementation(handle)
        raw.release()
        try:
            view.close()
            handle.release()
            handle.release()
            assert view._obj.is_null()
            assert not handle.is_closed
            alias.close()
            for action in (lambda: handle.value, handle.to_value, handle.__enter__):
                with pytest.raises(RuntimeError, match="released"):
                    action()
            handle.disconnect()
            assert handle.is_closed
            with pytest.raises(OSError) as caught:
                alias.close()
            assert caught.value.winerror == CLOSED
            assert "disconnected" in handle.take_error()
            assert handle.take_error() is None
        finally:
            alias._obj.release()
    assert calls == ["close", "close"]
    handle.dispose()


def test_release_without_projecting_and_dispose_on_context_exception(closable):
    interface, _ = closable
    first = DynWinRTImplementationHandle._create(interface, lambda: [], (), ())
    first.release()
    assert first._value is None
    first.dispose()
    with pytest.raises(LookupError, match="consumer"):
        with DynWinRTImplementationHandle._create(interface, lambda: [], (), ()) as second:
            view = second.value
            raise LookupError("consumer")
    assert second.is_closed and view._obj.is_null()


@pytest.mark.parametrize("operation", ["release", "disconnect", "dispose"])
def test_disposal_during_primary_projection_releases_the_new_view(closable, operation):
    interface, _ = closable
    owner = DynWinRTImplementation.create([interface.implementation(lambda: []).plan], lambda *_: [])
    views = []
    handle = None

    def project(value):
        view = interface.from_implementation(value)
        views.append(view)
        getattr(handle, operation)()
        return view

    handle = DynWinRTImplementationHandle(owner, project)
    with pytest.raises(RuntimeError, match="during primary view creation"):
        _ = handle.value
    assert owner.is_closed and not handle._projecting
    assert len(views) == 1 and views[0]._obj.is_null()
    handle.dispose()


@pytest.mark.parametrize("reentrant", [False, True])
def test_projection_failure_closes_native_owner(closable, reentrant):
    _, plan = closable
    owner = DynWinRTImplementation.create([plan], lambda *_: [])
    handle = None

    def project(_owner):
        if reentrant:
            return handle.value
        raise RuntimeError("projection failed")

    handle = DynWinRTImplementationHandle(owner, project)
    with pytest.raises(RuntimeError, match="reentrant" if reentrant else "projection failed"):
        _ = handle.value
    assert owner.is_closed and not handle._projecting
    with pytest.raises(RuntimeError, match="closed"):
        _ = handle.value


def test_owner_release_still_runs_if_projected_release_raises(closable):
    interface, plan = closable
    owner = DynWinRTImplementation.create([plan], lambda *_: [])

    class Projection:
        def __init__(self, owner):
            self.native_view = interface.from_implementation(owner)

        def release(self):
            self.native_view._obj.release()
            raise RuntimeError("projection cleanup failed")

    handle = DynWinRTImplementationHandle(owner, Projection)
    projection = handle.value
    with pytest.raises(RuntimeError, match="cleanup failed"):
        handle.release()
    assert projection.native_view._obj.is_null()
    with pytest.raises(OSError):
        owner.to_value()
    handle.dispose()


@pytest.mark.parametrize("entry", [None, (), (object(),), (object(), object())])
def test_invalid_interface_pairs_fail_before_native_creation(closable, entry):
    interface, _ = closable
    with pytest.raises(TypeError, match="interface entry"):
        DynWinRTImplementationHandle._create(interface, lambda: [], (), (entry,))


def test_invalid_management_inputs_and_descriptors(closable):
    interface, plan = closable
    with pytest.raises(TypeError, match="owner and typed projector"):
        DynWinRTImplementationHandle(object(), lambda _: None)
    with DynWinRTImplementation.create([plan], lambda *_: []) as owner:
        with pytest.raises(TypeError, match="owner and typed projector"):
            DynWinRTImplementationHandle(owner, None)
    with pytest.raises(TypeError, match="Invalid WinRT implementation descriptor"):
        DynWinRTImplementationHandle._create(interface, lambda: [], (object(),), ())
    with pytest.raises(TypeError, match="DynWinRTInterfacePlan"):
        DynWinRTImplementationDescriptor(object(), lambda *_: [])
    with pytest.raises(TypeError, match="callable"):
        DynWinRTImplementationDescriptor(plan, None)


def test_additional_interface_pairs_dispatch_through_native_slots(closable):
    interface, _ = closable
    iid = WinGUID.parse("96369f54-8eb6-48f0-abce-c1b211e627c3")
    signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
    typ = DynWinRTType.register_interface("Windows.Foundation.IStringable", iid)
    typ = typ.add_method("ToString", signature)
    plan = DynWinRTInterfacePlan.create(
        "Windows.Foundation.IStringable", typ,
        [DynWinRTImplementationMethod("ToString", 6, signature)],
    )

    class Stringable:
        _dynwinrt_interface_type = True
        from_implementation = staticmethod(lambda owner: owner.to_value())

        @staticmethod
        def implementation(handler):
            return DynWinRTImplementationDescriptor(plan, lambda *_: [handler()])

    with DynWinRTImplementationHandle._create(
        interface, lambda: [], (),
        ((Stringable, lambda: dynwinrt.DynWinRTValue.from_hstring("source-visible")),),
    ) as handle:
        handle.value.close()
        raw = handle.to_value()
        view = raw.cast(iid)
        raw.release()
        try:
            assert typ.method(6).get_string(view) == "source-visible"
        finally:
            view.release()


@pytest.mark.parametrize("loading", ["installed", "relocated-installation", "checkout", "direct-extension"])
def test_real_lifecycle_is_counted_by_the_product_report(source_artifacts, loading):
    output = source_artifacts / "reports"
    output.mkdir()
    consumer = source_artifacts / "consumer"
    consumer.mkdir()
    runtime = PACKAGE.parent if loading == "installed" else source_artifacts / "runtime"
    source = (
        ROOT / "bindings" / "py" / "python" / "dynwinrt"
        if loading == "checkout" else PACKAGE
    )
    if loading != "installed":
        shutil.copytree(
            source, runtime / "dynwinrt",
            ignore=shutil.ignore_patterns("*.pyd", ".gitignore"),
        )
        shutil.copy2(native.__file__, runtime / "dynwinrt" / Path(native.__file__).name)
    script = textwrap.dedent("""
        import sys
        from pathlib import Path
        sys.path.insert(0, sys.argv[1])
        from coverage import Coverage
        cov = Coverage(config_file=sys.argv[2], data_file=sys.argv[3], data_suffix=True)
        cov.start()
        if sys.argv[4] == "direct-extension":
            from importlib.util import module_from_spec, spec_from_file_location
            extension = next((Path(sys.argv[1]) / "dynwinrt").glob("*.pyd"))
            spec = spec_from_file_location("dynwinrt.dynwinrt", extension)
            module = module_from_spec(spec)
            sys.modules[spec.name] = module
            spec.loader.exec_module(module)
        from dynwinrt import (
            DynWinRTImplementationDescriptor, DynWinRTImplementationHandle,
            DynWinRTImplementationMethod, DynWinRTInterfacePlan, DynWinRTMethodSig,
            DynWinRTType, RoApartment, WinGUID,
        )
        iid = WinGUID.parse("30d5a829-7fa4-4026-83bb-d75bae4ea99e")
        signature = DynWinRTMethodSig()
        typ = DynWinRTType.register_interface("Windows.Foundation.IClosable", iid)
        typ = typ.add_method("Close", signature)
        plan = DynWinRTInterfacePlan.create(
            "Windows.Foundation.IClosable", typ,
            [DynWinRTImplementationMethod("Close", 6, signature)],
        )
        calls = []
        class Closable:
            @staticmethod
            def implementation(handler):
                return DynWinRTImplementationDescriptor(plan, lambda *_: handler())
            @classmethod
            def from_implementation(cls, owner):
                value = owner.to_value()
                try:
                    result = cls()
                    result._obj = value.cast(iid)
                    return result
                finally:
                    value.release()
        with RoApartment(1):
            with DynWinRTImplementationHandle._create(
                Closable, lambda: calls.append("native close") or [], (), ()
            ) as handle:
                primary = handle.value
                assert primary is handle.value
                assert typ.method(6).invoke_all(primary._obj, []) == []
                handle.release()
                handle.dispose()
                assert primary._obj.is_null()
        assert calls == ["native close"]
        source = Path(DynWinRTImplementationHandle.value.fget.__code__.co_filename)
        assert source == Path(sys.argv[1]) / "dynwinrt" / "_implementation.py"
        assert "class DynWinRTImplementationHandle" in source.read_text(encoding="utf-8")
        cov.stop()
        cov.save()
    """)
    config = ROOT / "eng" / "coverage" / "python-coveragerc"
    environment = dict(os.environ, COVERAGE_PROCESS_START="")
    result = subprocess.run(
        [sys.executable, "-I", "-c", script, str(runtime), str(config),
         str(output / ".coverage"), loading],
        cwd=consumer, env=environment, capture_output=True, text=True, timeout=60,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    result = subprocess.run(
        [sys.executable, str(ROOT / "eng" / "coverage" / "python_coverage.py"),
         "--root", str(ROOT), "--output", str(output), "--config", str(config)],
        cwd=consumer, env=environment, capture_output=True, text=True, timeout=60,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    report = json.loads((output / "runtime" / "coverage.json").read_text(encoding="utf-8"))
    path = next(path for path in report["files"] if path.endswith("_implementation.py"))
    measured = report["files"][path]
    assert 0 < measured["summary"]["covered_lines"] < measured["summary"]["num_statements"]
    for name in (
        "DynWinRTImplementationHandle._create",
        "DynWinRTImplementationHandle.value",
        "DynWinRTImplementationHandle.release",
        "DynWinRTImplementationHandle.dispose",
        "DynWinRTImplementationDescriptor.__init__",
        "DynWinRTImplementationDescriptor.dispatch",
        "_dynwinrt_checked_implementation_callback.invoke",
    ):
        counts = measured["functions"][name]["summary"]
        assert counts["covered_lines"] > 0, name
        assert counts["num_statements"] > 0, name
    aggregate = json.loads((output / "coverage.json").read_text(encoding="utf-8"))
    assert aggregate["files"][path]["summary"] == measured["summary"]
