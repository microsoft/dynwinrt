# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Standalone WinRT implementations, called through real native vtables."""

from contextlib import contextmanager
from contextvars import ContextVar
import gc
from pathlib import Path
import subprocess
import sys
import textwrap
import threading
from uuid import uuid4
import warnings
import weakref

import pytest

from dynwinrt import (
    DynWinRTArray,
    DynWinRTDelegateMethod,
    DynWinRTImplementation,
    DynWinRTImplementationDescriptor,
    DynWinRTImplementationMethod,
    DynWinRTInterfacePlan,
    DynWinRTMethodSig,
    DynWinRTStruct,
    DynWinRTType,
    DynWinRTValue,
    DynWinRtDelegate,
    RoApartment,
    WinGUID,
)


STRINGABLE_IID = WinGUID.parse("96369f54-8eb6-48f0-abce-c1b211e627c3")
CLOSABLE_IID = WinGUID.parse("30d5a829-7fa4-4026-83bb-d75bae4ea99e")
BACKGROUND_TASK_IID = WinGUID.parse("7d13d534-fd12-43ce-8c22-ea1ff13c06df")
BACKGROUND_INSTANCE_IID = WinGUID.parse("865bda7a-21d8-4573-8f32-928a1b0641f6")
CANCELED_HANDLER_IID = WinGUID.parse("a6c4bac0-51f8-4c57-ac3f-156dd1680c4f")
PYTHON_EXCEPTION = -1594998779  # 0xA0EE4005
CLOSED = -2147483629  # RO_E_CLOSED
INVALID_ARGUMENT = -2147024809
WRONG_THREAD = -2147417842  # RPC_E_WRONG_THREAD


@pytest.fixture(scope="module", autouse=True)
def apartment():
    with RoApartment(1):
        yield


def _interface(name, methods, *, iid=None, required_iids=()):
    iid = iid or WinGUID.parse(str(uuid4()))
    typ = DynWinRTType.register_interface(name, iid)
    definitions = []
    for index, (method_name, signature) in enumerate(methods, 6):
        typ = typ.add_method(method_name, signature)
        definitions.append(DynWinRTImplementationMethod(method_name, index, signature))
    plan = DynWinRTInterfacePlan.create(name, typ, definitions, required_iids)
    return plan, typ


def _stringable():
    return _interface(
        "Windows.Foundation.IStringable",
        [("ToString", DynWinRTMethodSig().add_out(DynWinRTType.hstring()))],
        iid=STRINGABLE_IID,
    )


def _closable():
    return _interface(
        "Windows.Foundation.IClosable",
        [("Close", DynWinRTMethodSig())],
        iid=CLOSABLE_IID,
    )


def _view(owner, typ):
    value = owner.to_value()
    try:
        return value.cast(typ.iid())
    finally:
        value.release()


def test_stringable_closable_descriptors_and_owned_views():
    string_plan, string_type = _stringable()
    close_plan, close_type = _closable()
    calls = []

    def stringify(slot, args):
        calls.append(("string", slot, args))
        return [DynWinRTValue.from_hstring("Python \0 WinRT \U0001f600")]

    def close(slot, args):
        calls.append(("close", slot, args))
        return []

    descriptors = (
        DynWinRTImplementationDescriptor(string_plan, stringify),
        DynWinRTImplementationDescriptor(close_plan, close),
    )
    with DynWinRTImplementation.create(
        tuple(descriptor.plan for descriptor in descriptors),
        lambda interface, slot, args: descriptors[interface].dispatch(slot, args),
        "DynWinRT.Tests.PythonStandalone",
    ) as owner:
        string_view = _view(owner, string_type)
        close_view = _view(owner, close_type)
        try:
            assert string_type.method(6).get_string(string_view) == "Python \0 WinRT \U0001f600"
            assert close_type.method(6).invoke_all(close_view, []) == []
            assert string_view.identity_raw() == close_view.identity_raw()
            assert string_view.as_raw() != close_view.as_raw()
            assert calls == [("string", 6, []), ("close", 6, [])]
            assert not owner.is_closed
            assert owner.take_error() is None
        finally:
            string_view.release()
            close_view.release()
    assert owner.is_closed
    owner.dispose()


def test_background_task_receives_retained_borrowed_instance():
    registration_type = DynWinRTType.runtime_class(
        "Windows.ApplicationModel.Background.BackgroundTaskRegistration",
        DynWinRTType.interface(WinGUID.parse("10654cc2-a26e-43bf-8c12-1fb40dbfbfa0")),
    )
    deferral_type = DynWinRTType.runtime_class(
        "Windows.ApplicationModel.Background.BackgroundTaskDeferral",
        DynWinRTType.interface(WinGUID.parse("93cc156d-af27-4dd3-846e-24ee40cadd25")),
    )
    # Every IBackgroundTaskInstance slot has its SDK ABI, including unused ones.
    instance_plan, instance_type = _interface(
        "Windows.ApplicationModel.Background.IBackgroundTaskInstance",
        [
            ("get_InstanceId", DynWinRTMethodSig().add_out(DynWinRTType.guid_type())),
            ("get_Task", DynWinRTMethodSig().add_out(registration_type)),
            ("get_Progress", DynWinRTMethodSig().add_out(DynWinRTType.u32_type())),
            ("put_Progress", DynWinRTMethodSig().add_in(DynWinRTType.u32_type())),
            ("get_TriggerDetails", DynWinRTMethodSig().add_out(DynWinRTType.object())),
            ("add_Canceled", DynWinRTMethodSig()
             .add_in(DynWinRTType.delegate(CANCELED_HANDLER_IID))
             .add_out(DynWinRTType.i64_type())),
            ("remove_Canceled", DynWinRTMethodSig().add_in(DynWinRTType.i64_type())),
            ("get_SuspendedCount", DynWinRTMethodSig().add_out(DynWinRTType.u32_type())),
            ("GetDeferral", DynWinRTMethodSig().add_out(deferral_type)),
        ],
        iid=BACKGROUND_INSTANCE_IID,
    )
    progress = [0]
    instance_id = WinGUID.parse(str(uuid4()))

    def instance_dispatch(interface, slot, args):
        assert interface == 0
        if slot == 6:
            return [DynWinRTValue.from_guid(instance_id)]
        if slot == 8:
            return [DynWinRTValue.from_u32(progress[0])]
        if slot == 9:
            progress[0] = args[0].to_u32()
            return []
        raise NotImplementedError("fixture does not register tasks or acquire OS deferrals")

    task_plan, task_type = _interface(
        "Windows.ApplicationModel.Background.IBackgroundTask",
        [("Run", DynWinRTMethodSig().add_in(instance_type))],
        iid=BACKGROUND_TASK_IID,
    )
    retained = []

    def run(_interface, slot, args):
        assert slot == 6 and len(args) == 1
        assert instance_type.method(6).invoke(args[0], []).to_guid().to_string() == instance_id.to_string()
        instance_type.method(9).invoke_all(args[0], [DynWinRTValue.from_u32(73)])
        retained.append(args[0])
        return []

    with DynWinRTImplementation.create([instance_plan], instance_dispatch) as instance_owner:
        with DynWinRTImplementation.create([task_plan], run) as task_owner:
            instance = _view(instance_owner, instance_type)
            task = _view(task_owner, task_type)
            try:
                assert task_type.method(6).invoke_all(task, [instance]) == []
                instance.release()
                instance_owner.release()
                assert progress == [73]
                assert instance_type.method(8).invoke(retained[0], []).to_u32() == 73
            finally:
                instance.release()
                task.release()
                for value in retained:
                    value.release()
                retained.clear()


SCALARS = {
    "bool": ("bool_type", "from_bool", [True, False], "to_bool"),
    "i8": ("i8_type", "from_i8", [-128, 127], "to_int"),
    "u8": ("u8_type", "from_u8", [0, 255], "to_int"),
    "i16": ("i16_type", "from_i16", [-32768, 32767], "to_int"),
    "u16": ("u16_type", "from_u16", [0, 65535], "to_int"),
    "char16": ("char16", "from_u16", [0xD834, 65], "to_int"),
    "i32": ("i32_type", "from_i32", [-(2**31), 2**31 - 1], "to_int"),
    "u32": ("u32_type", "from_u32", [0, 2**32 - 1], "to_int"),
    "i64": ("i64_type", "from_i64", [-(2**63), 2**63 - 1], "to_i64"),
    "u64": ("u64_type", "from_u64", [0, 2**64 - 1], "to_u64"),
    "f32": ("f32_type", "from_f32", [1.25, -2.5], "to_f64"),
    "f64": ("f64_type", "from_f64", [1.25e100, -2.5e-100], "to_f64"),
    "hresult": ("hresult", "from_hresult", [-2147467259, 0], "to_number"),
    "hstring": ("hstring", "from_hstring", ["hello\0world", "\U0001f600"], "to_string"),
}
SHAPES = (*SCALARS, "guid", "enum", "struct", "object", "interface",
          "runtime_class", "delegate", "generic", "async")


@contextmanager
def _values(shape):
    owners = []
    try:
        if shape in SCALARS:
            type_name, constructor, values, reader = SCALARS[shape]
            typ = getattr(DynWinRTType, type_name)()
            values = [getattr(DynWinRTValue, constructor)(value) for value in values]
            normalize = lambda value: getattr(value, reader)()
        elif shape == "guid":
            typ = DynWinRTType.guid_type()
            values = [DynWinRTValue.from_guid(WinGUID.parse(str(uuid4()))) for _ in range(2)]
            normalize = lambda value: value.to_guid().to_string()
        elif shape == "enum":
            typ = DynWinRTType.enum_type(f"Tests.Enum{uuid4().hex}", ["Low", "High"], [-1, 42])
            values = [DynWinRTValue.enum_value(typ, value) for value in (-1, 42)]
            normalize = lambda value: value.to_number()
        elif shape == "struct":
            inner_type = DynWinRTType.struct_type(
                f"Tests.Inner{uuid4().hex}", [DynWinRTType.f32_type(), DynWinRTType.i64_type()]
            )
            typ = DynWinRTType.struct_type(
                f"Tests.Packet{uuid4().hex}",
                [DynWinRTType.hstring(), DynWinRTType.guid_type(), inner_type],
            )
            values = []
            for index in range(2):
                inner = DynWinRTStruct.create(inner_type)
                inner.set_f32(0, 1.25 + index)
                inner.set_i64(1, -(2**60) + index)
                packet = DynWinRTStruct.create(typ)
                packet.set_hstring(0, f"owned\0string {index}")
                packet.set_guid(1, WinGUID.parse(str(uuid4())))
                packet.set_struct(2, inner)
                values.append(packet.to_value())

            def normalize(value):
                packet = value.as_struct()
                inner = packet.get_struct(2)
                return (packet.get_hstring(0), packet.get_guid(1).to_string(),
                        inner.get_f32(0), inner.get_i64(1))
        elif shape in ("object", "interface", "runtime_class"):
            plan, interface_type = _stringable()
            owner = DynWinRTImplementation.create(
                [plan], lambda *_args: [DynWinRTValue.from_hstring("retained")]
            )
            owners.append(owner)
            typ = {
                "object": DynWinRTType.object(),
                "interface": interface_type,
                "runtime_class": DynWinRTType.runtime_class("Tests.Stringable", interface_type),
            }[shape]
            values = [owner.to_value(), DynWinRTValue.null_value()]
            normalize = lambda value: None if value.is_null() else value.identity_raw()
        elif shape == "delegate":
            typ = DynWinRTType.delegate(CANCELED_HANDLER_IID)
            delegate = DynWinRtDelegate.create(
                CANCELED_HANDLER_IID,
                [DynWinRTType.interface(BACKGROUND_INSTANCE_IID), DynWinRTType.i32_type()],
                lambda *_args: None,
            )
            values = [delegate.to_value(), DynWinRTValue.null_value()]
            normalize = lambda value: None if value.is_null() else value.identity_raw()
        elif shape == "generic":
            typ = DynWinRTType.parameterized(
                WinGUID.parse("913337e9-11a1-4345-a3a2-4e7f956e222d"),
                [DynWinRTType.i32_type()],
            )
            values = [
                DynWinRTValue.create_vector([DynWinRTValue.from_i32(12)], DynWinRTType.i32_type()),
                DynWinRTValue.null_value(),
            ]
            normalize = lambda value: None if value.is_null() else value.identity_raw()
        elif shape == "async":
            file_type = DynWinRTType.runtime_class(
                "Windows.Storage.StorageFile",
                DynWinRTType.interface(WinGUID.parse("fa3f6186-4214-428c-a64c-14c9ac7315ea")),
            )
            typ = DynWinRTType.i_async_operation(file_type)
            statics_iid = WinGUID.parse("5984c710-daf2-43c8-8bb4-a4d3eacfd03f")
            statics = DynWinRTType.register_interface("IStorageFileStatics", statics_iid)
            statics = statics.add_method(
                "GetFileFromPathAsync",
                DynWinRTMethodSig().add_in(DynWinRTType.hstring()).add_out(typ),
            )
            factory = DynWinRTValue.activation_factory("Windows.Storage.StorageFile")
            factory_view = factory.cast(statics_iid)
            try:
                operation = statics.method(6).invoke(
                    factory_view, [DynWinRTValue.from_hstring(str(Path(__file__).resolve()))]
                )
            finally:
                factory_view.release()
                factory.release()
            operation.wait().release()
            values = [operation, DynWinRTValue.null_value()]

            def normalize(value):
                if value.is_null():
                    return None
                result = value.wait()
                try:
                    return not result.is_null()
                finally:
                    result.release()
        else:
            raise AssertionError(shape)
        yield typ, values, normalize
    finally:
        for owner in owners:
            owner.dispose()


@pytest.mark.parametrize("shape", SHAPES)
def test_native_value_passthrough(shape):
    with _values(shape) as (typ, values, normalize):
        plan, interface_type = _interface(
            f"Tests.Echo{uuid4().hex}",
            [("Echo", DynWinRTMethodSig().add_in(typ).add_out(typ))],
        )
        received = []

        def dispatch(interface, slot, args):
            assert (interface, slot) == (0, 6)
            received.append(normalize(args[0]))
            return [args[0]]

        with DynWinRTImplementation.create([plan], dispatch) as owner:
            view = _view(owner, interface_type)
            try:
                expected = [normalize(value) for value in values]
                actual = [
                    normalize(interface_type.method(6).invoke(view, [value]))
                    for value in values
                ]
                assert actual == expected
                assert received == expected
            finally:
                view.release()


@pytest.mark.parametrize("shape", SHAPES)
@pytest.mark.parametrize("direction", ["pass", "receive", "fill"])
def test_native_arrays(shape, direction):
    with _values(shape) as (typ, values, normalize):
        array_type = DynWinRTType.array_type(typ)
        signature = DynWinRTMethodSig()
        if direction == "pass":
            signature = signature.add_in(array_type).add_out(array_type)
        elif direction == "receive":
            signature = signature.add_out(array_type)
        else:
            signature = signature.add_out_fill(array_type)
        plan, interface_type = _interface(
            f"Tests.Arrays{uuid4().hex}", [("Transform", signature)]
        )
        expected = [normalize(value) for value in values]

        def dispatch(_interface, _slot, args):
            if direction == "pass":
                assert [normalize(value) for value in args[0].as_array().to_values()] == expected
                return [args[0]]
            if direction == "fill":
                assert args[0].to_u32() == len(values)
            else:
                assert args == []
            return [DynWinRTArray.from_values(values, typ).to_value()]

        with DynWinRTImplementation.create([plan], dispatch) as owner:
            view = _view(owner, interface_type)
            try:
                if direction == "pass":
                    args = [DynWinRTArray.from_values(values, typ).to_value()]
                elif direction == "fill":
                    args = [DynWinRTArray.from_values(values, typ).to_value()]
                else:
                    args = []
                result = interface_type.method(6).invoke(view, args)
                assert [normalize(value) for value in result.as_array().to_values()] == expected
            finally:
                view.release()


@pytest.mark.parametrize("direction", ["pass", "receive", "fill"])
def test_empty_arrays_and_output_order(direction):
    element_type = DynWinRTType.hstring()
    array_type = DynWinRTType.array_type(element_type)
    signature = DynWinRTMethodSig().add_in(DynWinRTType.i32_type())
    if direction == "pass":
        signature = signature.add_in(array_type).add_out(array_type)
    elif direction == "receive":
        signature = signature.add_out(array_type)
    else:
        signature = signature.add_out_fill(array_type)
    signature = signature.add_in(DynWinRTType.hstring()).add_out(DynWinRTType.guid_type())
    signature = signature.add_out(DynWinRTType.i64_type())
    plan, typ = _interface(f"Tests.OutputOrder{uuid4().hex}", [("Multiple", signature)])
    guid = WinGUID.parse(str(uuid4()))

    def dispatch(_interface, _slot, args):
        assert args[0].to_int() == 17
        assert args[-1].to_string() == "last input"
        if direction == "pass":
            assert len(args[1].as_array()) == 0
        elif direction == "fill":
            assert args[1].to_u32() == 0
        else:
            assert len(args) == 2
        return [DynWinRTArray.from_values([], element_type).to_value(),
                DynWinRTValue.from_guid(guid), DynWinRTValue.from_i64(2**60)]

    with DynWinRTImplementation.create([plan], dispatch) as owner:
        view = _view(owner, typ)
        try:
            args = [DynWinRTValue.from_i32(17)]
            if direction == "pass":
                args.append(DynWinRTArray.from_values([], element_type).to_value())
            elif direction == "fill":
                args.append(DynWinRTArray.from_values([], element_type).to_value())
            args.append(DynWinRTValue.from_hstring("last input"))
            outputs = typ.method(6).invoke_all(view, args)
            assert len(outputs) == 3
            assert len(outputs[0].as_array()) == 0
            assert outputs[1].to_guid().to_string() == guid.to_string()
            assert outputs[2].to_i64() == 2**60
        finally:
            view.release()


def test_properties_and_event_subscription_signatures():
    token_type = DynWinRTType.struct_type(
        "Windows.Foundation.EventRegistrationToken", [DynWinRTType.i64_type()]
    )
    token = DynWinRTStruct.create(token_type)
    token.set_i64(0, 101)
    plan, typ = _interface(
        f"Tests.PropertiesAndEvents{uuid4().hex}",
        [
            ("get_Count", DynWinRTMethodSig().add_out(DynWinRTType.i32_type())),
            ("put_Count", DynWinRTMethodSig().add_in(DynWinRTType.i32_type())),
            ("add_Canceled", DynWinRTMethodSig()
             .add_in(DynWinRTType.delegate(CANCELED_HANDLER_IID)).add_out(token_type)),
            ("remove_Canceled", DynWinRTMethodSig().add_in(token_type)),
        ],
    )
    count = [0]
    listeners = []

    def dispatch(_interface, slot, args):
        if slot == 6:
            return [DynWinRTValue.from_i32(count[0])]
        if slot == 7:
            count[0] = args[0].to_int()
            return []
        if slot == 8:
            listeners.append(args[0])
            return [token.to_value()]
        assert slot == 9 and args[0].as_struct().get_i64(0) == 101
        listeners.pop().release()
        return []

    delegate = DynWinRtDelegate.create(
        CANCELED_HANDLER_IID,
        [DynWinRTType.interface(BACKGROUND_INSTANCE_IID), DynWinRTType.i32_type()],
        lambda *_args: None,
    )
    with DynWinRTImplementation.create([plan], dispatch) as owner:
        view = _view(owner, typ)
        try:
            typ.method(7).invoke_all(view, [DynWinRTValue.from_i32(42)])
            assert typ.method(6).get_i32(view) == 42
            result = typ.method(8).invoke(view, [delegate.to_value()])
            assert result.as_struct().get_i64(0) == 101
            assert len(listeners) == 1
            typ.method(9).invoke_all(view, [result])
            assert listeners == []
        finally:
            view.release()


def _canceled_delegate_types():
    return [
        DynWinRTType.interface(BACKGROUND_INSTANCE_IID),
        DynWinRTType.enum_type(
            "Windows.ApplicationModel.Background.BackgroundTaskCancellationReason"
        ),
    ]


def _canceled_delegate_signature():
    signature = DynWinRTMethodSig()
    for typ in _canceled_delegate_types():
        signature = signature.add_in(typ)
    return signature


@pytest.fixture(params=["convenience", "cached"])
def invoke_canceled_delegate(request):
    signature = _canceled_delegate_signature()
    if request.param == "cached":
        return DynWinRTDelegateMethod.create(CANCELED_HANDLER_IID, signature).invoke
    return lambda value, args: value.invoke_delegate(CANCELED_HANDLER_IID, signature, args)


def test_implementation_handler_invokes_and_retains_incoming_delegate(invoke_canceled_delegate):
    plan, typ = _interface(
        f"Tests.DelegateInvocation{uuid4().hex}",
        [("Subscribe", DynWinRTMethodSig()
          .add_in(DynWinRTType.delegate(CANCELED_HANDLER_IID)))],
    )
    observed = []
    retained = []

    def canceled(sender, reason):
        assert sender.is_null()
        observed.append(reason.to_number())

    def dispatch(_interface, _slot, args):
        retained.append(args[0])
        return invoke_canceled_delegate(
            args[0],
            [DynWinRTValue.null_value(), DynWinRTValue.from_i32(3)],
        )

    delegate_value = DynWinRtDelegate.create(
        CANCELED_HANDLER_IID, _canceled_delegate_types(), canceled
    ).to_value()
    with DynWinRTImplementation.create([plan], dispatch) as owner:
        view = _view(owner, typ)
        try:
            assert typ.method(6).invoke_all(view, [delegate_value]) == []
            delegate_value.release()
            assert invoke_canceled_delegate(
                retained[0],
                [DynWinRTValue.null_value(), DynWinRTValue.from_i32(4)],
            ) == []
            assert observed == [3, 4]
        finally:
            delegate_value.release()
            view.release()
            for value in retained:
                value.release()
            retained.clear()


def test_delegate_invocation_pins_value_across_reentrant_release(invoke_canceled_delegate):
    holder = {}
    called = []

    def canceled(_sender, reason):
        holder["value"].release()
        called.append(reason.to_number())

    holder["value"] = DynWinRtDelegate.create(
        CANCELED_HANDLER_IID, _canceled_delegate_types(), canceled
    ).to_value()
    assert invoke_canceled_delegate(
        holder["value"],
        [DynWinRTValue.null_value(), DynWinRTValue.from_i32(5)],
    ) == []
    assert called == [5]
    assert holder["value"].is_null()


def test_delegate_invocation_errors_propagate(monkeypatch, invoke_canceled_delegate):
    errors = []
    monkeypatch.setattr(sys, "unraisablehook", errors.append)

    def canceled(_sender, _reason):
        raise ValueError("native delegate callback failed")

    value = DynWinRtDelegate.create(
        CANCELED_HANDLER_IID, _canceled_delegate_types(), canceled
    ).to_value()
    try:
        with pytest.raises(OSError) as caught:
            invoke_canceled_delegate(
                value,
                [DynWinRTValue.null_value(), DynWinRTValue.from_i32(6)],
            )
        assert caught.value.winerror == PYTHON_EXCEPTION
        assert len(errors) == 1
        assert isinstance(errors[0].exc_value, ValueError)
        for iid in (
            "00000000-0000-0000-0000-000000000000",
            "00000000-0000-0000-c000-000000000046",
            "af86e2e0-b12d-4c6a-9c5a-d7aa65101e90",
        ):
            with pytest.raises(TypeError, match="delegate IID"):
                value.invoke_delegate(WinGUID.parse(iid), _canceled_delegate_signature(), [])
            with pytest.raises(TypeError, match="delegate IID"):
                DynWinRTDelegateMethod.create(WinGUID.parse(iid), _canceled_delegate_signature())
    finally:
        value.release()
        errors.clear()


def test_failed_delegate_query_releases_temporary_native_references(invoke_canceled_delegate):
    owner, owner_ref, handler_ref, typ = _owner_cycle()
    value = _view(owner, typ)
    with pytest.raises(OSError) as caught:
        invoke_canceled_delegate(value, [])
    assert caught.value.winerror == -2147467262  # E_NOINTERFACE
    assert typ.method(6).invoke(value, []).to_string() == "cycle"
    value.release()
    del caught, value, owner
    gc.collect()
    assert owner_ref() is None and handler_ref() is None


def test_cached_delegate_method_reuses_metadata_without_retaining_targets():
    signature = _canceled_delegate_signature()
    method = DynWinRTDelegateMethod.create(CANCELED_HANDLER_IID, signature)
    del signature
    observed = []

    class Handler:
        def __init__(self, name):
            self.name = name

        def canceled(self, sender, reason):
            assert sender.is_null()
            observed.append((self.name, reason.to_number()))

    for name in ("first", "second"):
        handler = Handler(name)
        reference = weakref.ref(handler)
        value = DynWinRtDelegate.create(
            CANCELED_HANDLER_IID, _canceled_delegate_types(), handler.canceled
        ).to_value()
        del handler
        try:
            assert method.invoke(
                value, [DynWinRTValue.null_value(), DynWinRTValue.from_i32(7)]
            ) == []
            with pytest.raises(OSError):
                method.invoke(value, [])
        finally:
            value.release()
        assert reference() is None
    assert observed == [("first", 7), ("second", 7)]
    with pytest.raises(RuntimeError):
        method.invoke(DynWinRTValue.null_value(), [])


def test_cached_delegate_method_shares_prepared_cif_across_python_threads():
    method = DynWinRTDelegateMethod.create(
        CANCELED_HANDLER_IID, _canceled_delegate_signature()
    )
    barrier = threading.Barrier(2)
    observed = []
    errors = []

    def run(code):
        try:
            def canceled(sender, reason):
                assert sender.is_null()
                barrier.wait(timeout=10)
                observed.append(reason.to_number())
                assert reason.to_number() == code

            value = DynWinRtDelegate.create(
                CANCELED_HANDLER_IID, _canceled_delegate_types(), canceled
            ).to_value()
            try:
                assert method.invoke(
                    value, [DynWinRTValue.null_value(), DynWinRTValue.from_i32(code)]
                ) == []
            finally:
                value.release()
        except BaseException as error:
            errors.append(f"{type(error).__name__}: {error}")

    threads = [threading.Thread(target=run, args=(code,), daemon=True) for code in (17, 29)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=15)
        assert not thread.is_alive()
    assert errors == []
    assert sorted(observed) == [17, 29]


def test_python_exceptions_reach_native_caller_and_unraisable_hook(monkeypatch):
    plan, typ = _stringable()
    errors = []
    monkeypatch.setattr(sys, "unraisablehook", errors.append)

    def fail(*_args):
        raise ValueError("intentional callback failure")

    with DynWinRTImplementation.create([plan], fail) as owner:
        view = _view(owner, typ)
        try:
            with pytest.raises(OSError) as caught:
                typ.method(6).invoke(view, [])
            assert caught.value.winerror == PYTHON_EXCEPTION
            assert len(errors) == 1
            assert isinstance(errors[0].exc_value, ValueError)
            assert "intentional callback failure" in owner.take_error()
            assert owner.take_error() is None
        finally:
            view.release()
            errors.clear()


@pytest.mark.parametrize("result", [None, (), ["not a value"], 12])
def test_invalid_python_output_containers_fail(monkeypatch, result):
    plan, typ = _stringable()
    errors = []
    monkeypatch.setattr(sys, "unraisablehook", errors.append)
    with DynWinRTImplementation.create([plan], lambda *_args: result) as owner:
        view = _view(owner, typ)
        try:
            with pytest.raises(OSError) as caught:
                typ.method(6).invoke(view, [])
            assert caught.value.winerror == PYTHON_EXCEPTION
            assert isinstance(errors[0].exc_value, TypeError)
            assert "list of DynWinRTValue" in owner.take_error()
        finally:
            view.release()
            errors.clear()


@pytest.mark.parametrize("result", [[], [DynWinRTValue.from_i32(1)],
                                  [DynWinRTValue.from_hstring("one"),
                                   DynWinRTValue.from_hstring("two")]])
def test_native_output_shape_mismatch_fails(result):
    plan, typ = _stringable()
    with DynWinRTImplementation.create([plan], lambda *_args: result) as owner:
        view = _view(owner, typ)
        try:
            with pytest.raises(OSError) as caught:
                typ.method(6).invoke(view, [])
            assert caught.value.winerror == INVALID_ARGUMENT
            assert owner.take_error()
        finally:
            view.release()


def test_fill_array_capacity_must_match_exactly():
    array_type = DynWinRTType.array_type(DynWinRTType.i32_type())
    plan, typ = _interface(
        f"Tests.BadFill{uuid4().hex}", [("Fill", DynWinRTMethodSig().add_out_fill(array_type))]
    )
    with DynWinRTImplementation.create(
        [plan], lambda *_args: [DynWinRTArray.from_i32_values([1]).to_value()]
    ) as owner:
        view = _view(owner, typ)
        try:
            with pytest.raises(OSError) as caught:
                typ.method(6).invoke(view, [DynWinRTArray.from_i32_values([0, 0]).to_value()])
            assert caught.value.winerror == INVALID_ARGUMENT
            assert owner.take_error()
        finally:
            view.release()


def test_plan_validation_and_immutable_definitions():
    signature = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
    method = DynWinRTImplementationMethod("ToString", 6, signature)
    methods = [method]
    plan = DynWinRTInterfacePlan.create("IStringable", DynWinRTType.interface(STRINGABLE_IID), methods)
    methods.clear()
    assert plan.name == "IStringable" and len(plan.methods) == 1
    assert method.name == "ToString" and method.vtable_index == 6
    assert isinstance(method.signature, DynWinRTMethodSig)
    with pytest.raises(AttributeError):
        method.vtable_index = 9
    with pytest.raises(AttributeError):
        plan.name = "changed"
    for slot in (3, 7):
        with pytest.raises(OSError):
            DynWinRTInterfacePlan.create(
                "BadSlots", plan.interface_type,
                [DynWinRTImplementationMethod("Bad", slot, signature)],
            )
    with pytest.raises(OSError):
        DynWinRTInterfacePlan.create("BadRoot", DynWinRTType.object(), [method])
    with pytest.raises(OSError):
        DynWinRTInterfacePlan.create(
            "Generic", DynWinRTType.parameterized(
                WinGUID.parse("913337e9-11a1-4345-a3a2-4e7f956e222d"),
                [DynWinRTType.i32_type()],
            ), [method],
        )
    with pytest.raises(OSError):
        DynWinRTInterfacePlan.create(
            "NestedArray", DynWinRTType.interface(WinGUID.parse(str(uuid4()))),
            [DynWinRTImplementationMethod("Bad", 6, DynWinRTMethodSig().add_in(
                DynWinRTType.array_type(DynWinRTType.array_type(DynWinRTType.i32_type()))
            ))],
        )
    with pytest.raises(OSError):
        DynWinRTImplementation.create([], lambda *_args: [])
    with pytest.raises(OSError):
        DynWinRTImplementation.create([plan, plan], lambda *_args: [])


def test_required_interfaces_are_validated_as_a_complete_set():
    close_plan, close_type = _closable()
    string_plan, _ = _stringable()
    required = DynWinRTInterfacePlan.create(
        string_plan.name, string_plan.interface_type, string_plan.methods, (CLOSABLE_IID,)
    )
    with pytest.raises(OSError, match="requires"):
        DynWinRTImplementation.create([required], lambda *_args: [])
    with DynWinRTImplementation.create(
        [required, close_plan],
        lambda interface, _slot, _args: [] if interface else [DynWinRTValue.from_hstring("valid")],
    ) as owner:
        view = _view(owner, close_type)
        try:
            assert close_type.method(6).invoke_all(view, []) == []
        finally:
            view.release()


def test_async_callables_are_rejected_before_publication():
    plan, _ = _stringable()

    async def coroutine(*_args):
        return []

    async def async_generator(*_args):
        yield []

    class AsyncCallable:
        async def __call__(self, *_args):
            return []

    for callback in (coroutine, async_generator, AsyncCallable()):
        with pytest.raises(TypeError, match="synchronous"):
            DynWinRTImplementation.create([plan], callback)
        with pytest.raises(TypeError, match="synchronous"):
            DynWinRTImplementationDescriptor(plan, callback)
    with pytest.raises(TypeError, match="callable"):
        DynWinRTImplementation.create([plan], None)


@pytest.mark.parametrize("kind", ["coroutine", "awaitable"])
def test_async_results_are_rejected_without_await_or_warning(monkeypatch, kind):
    plan, typ = _stringable()
    errors = []
    monkeypatch.setattr(sys, "unraisablehook", errors.append)

    async def coroutine():
        raise AssertionError("coroutine must not run")

    class Awaitable:
        def __await__(self):
            raise AssertionError("awaitable must not be scheduled")

    def callback(*_args):
        return coroutine() if kind == "coroutine" else Awaitable()

    with warnings.catch_warnings(record=True) as recorded:
        warnings.simplefilter("always", RuntimeWarning)
        with DynWinRTImplementation.create([plan], callback) as owner:
            view = _view(owner, typ)
            try:
                with pytest.raises(OSError) as caught:
                    typ.method(6).invoke(view, [])
                assert caught.value.winerror == PYTHON_EXCEPTION
                assert isinstance(errors[0].exc_value, TypeError)
                assert "synchronously" in owner.take_error()
            finally:
                view.release()
                errors.clear()
        gc.collect()
    assert not any("was never awaited" in str(item.message) for item in recorded)


def test_contextvars_and_same_thread_detached_reentrancy():
    plan, typ = _stringable()
    context = ContextVar("python_winrt_reverse_context", default="outside")
    token = context.set("construction")
    calls = []
    depth = [0]
    view = None

    def callback(*_args):
        calls.append(context.get())
        context.set("handler mutation")
        if depth[0] == 0:
            depth[0] += 1
            assert typ.method(6).invoke(view, []).to_string() == "nested"
            depth[0] -= 1
        return [DynWinRTValue.from_hstring("nested")]

    try:
        with DynWinRTImplementation.create([plan], callback) as owner:
            context.set("call site")
            view = _view(owner, typ)
            try:
                assert typ.method(6).invoke_detached(view, []).to_string() == "nested"
                assert calls == ["construction", "construction"]
                assert context.get() == "call site"
            finally:
                view.release()
    finally:
        context.reset(token)


@pytest.mark.parametrize("operation", ["release", "disconnect", "dispose"])
def test_reentrant_lifetime_operations(operation):
    plan, typ = _stringable()
    owner = None

    def callback(*_args):
        getattr(owner, operation)()
        return [DynWinRTValue.from_hstring("completed in-flight call")]

    owner = DynWinRTImplementation.create([plan], callback)
    view = _view(owner, typ)
    try:
        assert typ.method(6).invoke(view, []).to_string() == "completed in-flight call"
        if operation == "release":
            assert not owner.is_closed
            assert typ.method(6).invoke(view, []).to_string() == "completed in-flight call"
        else:
            assert owner.is_closed
            with pytest.raises(OSError) as caught:
                typ.method(6).invoke(view, [])
            assert caught.value.winerror == CLOSED
        owner.dispose()
        owner.dispose()
    finally:
        view.release()


@pytest.mark.parametrize("invocation", ["invoke", "invoke_detached"])
@pytest.mark.parametrize("void_result", [False, True], ids=["hstring-output", "void-output"])
def test_reentrant_dispose_can_release_the_calling_receiver(invocation, void_result):
    plan, typ = _closable() if void_result else _stringable()
    thread_id = threading.get_ident()
    calls = []

    def callback(*_args):
        assert threading.get_ident() == thread_id
        # A generated management handle releases both its primary view and its
        # native owner on dispose. The call must not retain a Python borrow of
        # that view; only an independent, call-local native pin may survive.
        owner.dispose()
        receiver.release()
        calls.append("completed")
        return [] if void_result else [DynWinRTValue.from_hstring("completed after receiver release")]

    owner = DynWinRTImplementation.create([plan], callback)
    receiver = _view(owner, typ)
    retained = _view(owner, typ)
    invoke = getattr(typ.method(6), invocation)
    try:
        result = invoke(receiver, [])
        if void_result:
            assert result.to_int() == 0
        else:
            assert result.to_string() == "completed after receiver release"
        assert calls == ["completed"]
        assert receiver.is_null() and owner.is_closed
        assert owner.take_error() is None
        with pytest.raises(OSError) as caught:
            invoke(retained, [])
        assert caught.value.winerror == CLOSED
        assert calls == ["completed"]
    finally:
        owner.dispose()
        receiver.release()
        retained.release()


def test_disposal_breaks_handler_cycle_and_allows_finalizer_reentrancy():
    plan, typ = _stringable()
    finalized = []

    class Handler:
        def dispatch(self, *_args):
            return [DynWinRTValue.from_hstring("live")]

        def __del__(self):
            owner = self.owner()
            if owner is not None:
                owner.dispose()
            finalized.append("handler")

    handler = Handler()
    owner = DynWinRTImplementation.create([plan], handler.dispatch)
    handler.owner = weakref.ref(owner)
    reference = weakref.ref(handler)
    view = _view(owner, typ)
    del handler
    owner.dispose()
    assert reference() is None
    assert finalized == ["handler"]
    with pytest.raises(OSError) as caught:
        typ.method(6).invoke(view, [])
    assert caught.value.winerror == CLOSED
    view.release()


def test_native_reference_retains_callback_after_owner_wrapper_disappears():
    plan, typ = _stringable()

    class Handler:
        def dispatch(self, *_args):
            return [DynWinRTValue.from_hstring("native retained")]

    handler = Handler()
    owner = DynWinRTImplementation.create([plan], handler.dispatch)
    view = _view(owner, typ)
    handler_ref, owner_ref = weakref.ref(handler), weakref.ref(owner)
    del handler, owner
    gc.collect()
    assert owner_ref() is None and handler_ref() is not None
    assert typ.method(6).invoke(view, []).to_string() == "native retained"
    view.release()
    gc.collect()
    assert handler_ref() is None


def test_final_native_release_drops_callback_even_if_released_owner_is_alive():
    plan, typ = _stringable()

    class Handler:
        def dispatch(self, *_args):
            return [DynWinRTValue.from_hstring("live")]

    handler = Handler()
    owner = DynWinRTImplementation.create([plan], handler.dispatch)
    view = _view(owner, typ)
    reference = weakref.ref(handler)
    del handler
    owner.release()
    owner.release()
    assert not owner.is_closed and reference() is not None
    with pytest.raises(OSError) as caught:
        owner.to_value()
    assert caught.value.winerror == CLOSED
    view.release()
    assert owner.is_closed and reference() is None


def _run_python_thread(callback):
    errors = []

    def run():
        try:
            callback()
        except BaseException as error:
            errors.append(f"{type(error).__name__}: {error}")

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    thread.join(timeout=10)
    assert not thread.is_alive(), "worker did not finish native owner cleanup"
    assert errors == []


@pytest.mark.parametrize("cycle", [False, True], ids=["last-owner-drop", "owner-cycle-gc"])
def test_foreign_python_thread_releases_last_owner_and_handler_captures(cycle):
    plan, _typ = _stringable()
    entered = []

    class Handler:
        def dispatch(self, *_args):
            entered.append(threading.get_ident())
            raise AssertionError("cleanup must not enter a callback")

    gc_enabled = gc.isenabled()
    gc.disable()
    try:
        handler = Handler()
        owner = DynWinRTImplementation.create([plan], handler.dispatch)
        if cycle:
            handler.owner = owner
        owner_ref, handler_ref = weakref.ref(owner), weakref.ref(handler)
        handoff = [owner]
        del owner, handler

        def drop_on_worker():
            handoff.clear()
            gc.collect()
            # Flush any decrefs deferred by PyO3's native destruction path.
            DynWinRTType.i32_type()
            gc.collect()
            assert owner_ref() is None and handler_ref() is None

        _run_python_thread(drop_on_worker)
        assert owner_ref() is None and handler_ref() is None
        assert entered == []
    finally:
        if gc_enabled:
            gc.enable()


def test_foreign_python_thread_to_value_reports_core_wrong_thread_error():
    plan, _typ = _stringable()
    entered = []

    def callback(*_args):
        entered.append(True)
        raise AssertionError("wrong-thread access must not enter a callback")

    with DynWinRTImplementation.create([plan], callback) as owner:
        errors = []

        def access_on_worker():
            assert not owner.is_closed
            try:
                owner.to_value()
            except OSError as error:
                errors.append(error.winerror)
            else:
                raise AssertionError("foreign thread obtained a native view")

        _run_python_thread(access_on_worker)
        assert errors == [WRONG_THREAD]
        assert entered == []
        value = owner.to_value()
        value.release()
        assert not owner.is_closed


def test_foreign_python_gc_preserves_externally_retained_native_root():
    gc_enabled = gc.isenabled()
    gc.disable()
    try:
        owner, owner_ref, handler_ref, typ = _owner_cycle()
        view = _view(owner, typ)
        del owner
        _run_python_thread(gc.collect)
        assert owner_ref() is not None and handler_ref() is not None
        assert not owner_ref().is_closed
        view.release()

        def collect_on_worker():
            gc.collect()
            DynWinRTType.i32_type()
            gc.collect()

        _run_python_thread(collect_on_worker)
        assert owner_ref() is None and handler_ref() is None
    finally:
        if gc_enabled:
            gc.enable()


def _owner_cycle():
    plan, typ = _stringable()

    class Handler:
        def dispatch(self, *_args):
            return [DynWinRTValue.from_hstring("cycle")]

    handler = Handler()
    owner = DynWinRTImplementation.create([plan], handler.dispatch)
    handler.owner = owner
    return owner, weakref.ref(owner), weakref.ref(handler), typ


def test_gc_collects_owner_only_python_native_cycle_once():
    owner, owner_ref, handler_ref, _typ = _owner_cycle()
    assert gc.is_tracked(owner)
    del owner
    gc.collect()
    assert owner_ref() is None
    assert handler_ref() is None


def test_gc_does_not_trace_external_native_root_as_python_only_edge():
    owner, owner_ref, handler_ref, typ = _owner_cycle()
    view = _view(owner, typ)
    del owner
    gc.collect()
    assert owner_ref() is not None and handler_ref() is not None
    assert typ.method(6).invoke(view, []).to_string() == "cycle"
    view.release()
    gc.collect()
    assert owner_ref() is None and handler_ref() is None


def test_retained_native_self_cycle_requires_explicit_dispose():
    owner, owner_ref, handler_ref, typ = _owner_cycle()
    handler_ref().native_alias = _view(owner, typ)
    del owner
    gc.collect()
    assert owner_ref() is not None and handler_ref() is not None
    owner_ref().dispose()
    gc.collect()
    assert owner_ref() is None and handler_ref() is None


def test_context_manager_preserves_user_exception_and_disconnects_aliases():
    plan, typ = _stringable()
    owner = DynWinRTImplementation.create(
        [plan], lambda *_args: [DynWinRTValue.from_hstring("live")]
    )
    view = _view(owner, typ)
    try:
        with pytest.raises(ValueError, match="body failure"):
            with owner:
                raise ValueError("body failure")
        assert owner.is_closed
        with pytest.raises(OSError) as caught:
            typ.method(6).invoke(view, [])
        assert caught.value.winerror == CLOSED
        with pytest.raises(OSError):
            owner.to_value()
        with pytest.raises(OSError):
            owner.__enter__()
    finally:
        view.release()


@pytest.mark.parametrize("during_callback", [False, True])
def test_interpreter_shutdown_closes_retained_native_callbacks(during_callback):
    script = r'''
        import atexit
        import sys
        state = {}

        def late_native_call():
            owner, view, method = state["native"]
            assert owner.is_closed
            try:
                method.invoke(view, [])
            except OSError as error:
                assert error.winerror == -2147483629, error
            else:
                raise AssertionError("callback entered after shutdown")
            try:
                method.invoke(state["native_only"], [])
            except OSError as error:
                assert error.winerror == -2147483629, error
            else:
                raise AssertionError("ownerless callback entered after shutdown")
            assert state["calls"] == 1
            try:
                state["factory"]()
            except RuntimeError as error:
                assert "shutting down" in str(error)
            else:
                raise AssertionError("created implementation during shutdown")
            owner.dispose()
            view.release()
            state["native_only"].release()
            print("late native callback safely rejected", flush=True)

        atexit.register(late_native_call)
        from dynwinrt import (DynWinRTImplementation, DynWinRTImplementationMethod,
                              DynWinRTInterfacePlan, DynWinRTType, DynWinRTValue,
                              DynWinRTMethodSig, WinGUID, ro_initialize)
        ro_initialize(1)
        iid = WinGUID.parse("96369f54-8eb6-48f0-abce-c1b211e627c3")
        sig = DynWinRTMethodSig().add_out(DynWinRTType.hstring())
        typ = DynWinRTType.register_interface("IStringable", iid).add_method("ToString", sig)
        plan = DynWinRTInterfacePlan.create(
            "IStringable", typ, [DynWinRTImplementationMethod("ToString", 6, sig)]
        )
        state["calls"] = 0
        def callback(*args):
            state["calls"] += 1
            if sys.argv[1] == "1":
                from dynwinrt.dynwinrt import _dynwinrt_implementation_runtime
                _dynwinrt_implementation_runtime.shutdown()
            return [DynWinRTValue.from_hstring("before shutdown")]
        state["factory"] = lambda: DynWinRTImplementation.create([plan], callback)
        owner = state["factory"]()
        view = owner.to_value().cast(iid)
        method = typ.method(6)
        state["native"] = owner, view, method
        second_owner = state["factory"]()
        state["native_only"] = second_owner.to_value().cast(iid)
        second_owner.release()
        del second_owner
        assert method.invoke(view, []).to_string() == "before shutdown"
    '''
    result = subprocess.run(
        [sys.executable, "-c", textwrap.dedent(script), str(int(during_callback))],
        capture_output=True, text=True, timeout=30, check=False,
    )
    assert result.returncode == 0, result.stderr
    assert "late native callback safely rejected" in result.stdout
    assert not result.stderr


def test_subinterpreters_are_rejected_without_native_callback_publication():
    script = r'''
        import dynwinrt
        try:
            import _interpreters as interpreters
        except ImportError:
            import _xxsubinterpreters as interpreters
        interpreter = interpreters.create()
        try:
            try:
                result = interpreters.run_string(interpreter, "import dynwinrt")
            except BaseException as error:
                message = str(error)
            else:
                message = getattr(result, "formatted", str(result))
            assert "subinterpreter" in message.lower(), message
        finally:
            interpreters.destroy(interpreter)
        print("subinterpreter import rejected")
    '''
    result = subprocess.run(
        [sys.executable, "-c", textwrap.dedent(script)],
        capture_output=True, text=True, timeout=30, check=False,
    )
    assert result.returncode == 0, result.stderr
    assert "subinterpreter import rejected" in result.stdout
