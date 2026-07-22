# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""
E2E test runner for Python generated bindings.

Reads e2e_specs.json, imports generated Python modules,
and executes checks against real WinRT APIs.

Usage:
    python tests/runners/py_runner.py --specs tests/e2e_specs.json --generated tests/e2e_generated/python_bindings --output results.json
"""

import argparse
import asyncio
import importlib
import inspect
import json
import re
import sys
import os
import threading


def to_snake_case(name: str) -> str:
    """Convert PascalCase/camelCase to snake_case."""
    s = re.sub(r'([A-Z])', r'_\1', name).lstrip('_').lower()
    return s


def to_camel_case(name: str) -> str:
    """Convert snake_case to camelCase."""
    parts = name.split('_')
    return parts[0] + ''.join(p.capitalize() for p in parts[1:])


def literal_arg(val):
    """Convert a JSON value to a Python value."""
    if isinstance(val, str):
        return val
    if isinstance(val, bool):
        return val
    if isinstance(val, (int, float)):
        return val
    return val


def projected_values_equal(left, right):
    if type(left) is not type(right):
        return False
    for attribute in ("name", "value", "path"):
        if hasattr(left, attribute) and hasattr(right, attribute):
            return getattr(left, attribute) == getattr(right, attribute)
    if isinstance(left, (str, bytes, int, float, bool, tuple)):
        return left == right
    return True


def wrap_arg(val):
    """Wrap a Python value into a DynWinRTValue for method args."""
    import dynwinrt_py as dw
    if isinstance(val, str):
        return dw.DynWinRTValue.from_hstring(val)
    if isinstance(val, bool):
        return dw.DynWinRTValue.from_bool(val)
    if isinstance(val, int):
        return dw.DynWinRTValue.from_i32(val)
    if isinstance(val, float):
        return dw.DynWinRTValue.from_f64(val)
    # If it has _obj, it's already a wrapper class instance
    if hasattr(val, '_obj'):
        return val._obj
    return val


async def run_spec(spec: dict, generated_dir: str, pkg_name: str) -> dict:
    """Run a single test spec. Returns a result dict."""
    ns = spec['namespace']
    cls_name = spec['class']
    spec_id = spec.get('id', f"{ns}.{cls_name}")
    result = {
        'id': spec_id,
        'namespace': ns,
        'class': cls_name,
        'language': 'py',
        'checks': [],
        'pass': True,
        'error': None,
    }

    try:
        # Import the generated module as part of the package
        mod_name = to_snake_case(cls_name)
        mod = importlib.import_module(f"{pkg_name}.{mod_name}")
        cls = getattr(mod, cls_name)

        # Instantiate
        inst_kind = spec['instantiate']['kind']
        obj = None

        if inst_kind == 'activate':
            import dynwinrt_py as dw
            raw = dw.DynWinRTValue.activation_factory(f'{ns}.{cls_name}').activate()
            obj = cls(raw)
        elif inst_kind == 'static_factory':
            method_name = to_snake_case(spec['instantiate']['method'])
            args = [literal_arg(a) for a in spec['instantiate'].get('args', [])]
            factory = getattr(cls, method_name)
            obj = factory(*args)
        elif inst_kind == 'constructor':
            args = [literal_arg(a) for a in spec['instantiate'].get('args', [])]
            obj = cls(*args)
        # kind == 'none': no instantiation

        # Run checks
        for check in spec.get('checks', []):
            if 'py' not in check.get('langs', ['py', 'ts']):
                continue
            check_result = await run_check(check, cls, obj, generated_dir, pkg_name)
            result['checks'].append(check_result)
            if not check_result['pass']:
                result['pass'] = False

    except Exception as e:
        result['pass'] = False
        result['error'] = str(e)

    return result


async def run_check(check: dict, cls, obj, generated_dir: str, pkg_name: str) -> dict:
    """Run a single check. Returns { kind, member, pass, error }."""
    kind = check['kind']
    member = to_snake_case(check['member']) if 'member' in check else ''
    cr = {'kind': kind, 'member': member, 'pass': False, 'error': None}

    try:
        if kind == 'property_equals':
            actual = getattr(obj, member)
            expected = check['expected']
            if actual != expected:
                cr['error'] = f'expected {expected!r}, got {actual!r}'
            else:
                cr['pass'] = True

        elif kind == 'property_exists':
            _ = getattr(obj, member)
            cr['pass'] = True

        elif kind == 'method_equals':
            method = getattr(obj, member)
            args = []
            if 'args' in check:
                args = [literal_arg(a) for a in check['args']]
            elif 'args_factory' in check:
                af = check['args_factory']
                af_mod = importlib.import_module(f"{pkg_name}.{to_snake_case(af['class'])}")
                af_cls = getattr(af_mod, af['class'])
                af_method = getattr(af_cls, to_snake_case(af['method']))
                af_args = [literal_arg(a) for a in af.get('args', [])]
                args = [af_method(*af_args)]
            actual = method(*args)
            expected = check['expected']
            if actual != expected:
                cr['error'] = f'expected {expected!r}, got {actual!r}'
            else:
                cr['pass'] = True

        elif kind == 'method_result_contains':
            method = getattr(obj, member)
            args = [literal_arg(a) for a in check.get('args', [])]
            result_obj = method(*args)
            # Try to get a string representation
            if hasattr(result_obj, 'absolute_uri'):
                actual = result_obj.absolute_uri
            elif hasattr(result_obj, 'to_string'):
                actual = result_obj.to_string()
            else:
                actual = str(result_obj)
            if check['contains'] not in actual:
                cr['error'] = f'"{check["contains"]}" not in "{actual}"'
            else:
                cr['pass'] = True

        elif kind == 'method_then_property_equals':
            target = obj
            if check.get('interface_class'):
                iface_mod_name = check.get(
                    'interface_module',
                    to_snake_case(check['interface_class']),
                )
                iface_mod = importlib.import_module(f"{pkg_name}.{iface_mod_name}")
                iface_cls = getattr(iface_mod, check['interface_class'])
                target = obj.as_interface(iface_cls)

            method = getattr(target, member)
            method(*[literal_arg(a) for a in check.get('args', [])])

            actual = obj
            for segment in check.get('property_path', []):
                actual = getattr(actual, to_snake_case(segment))
            expected = check['expected']
            if actual != expected:
                cr['error'] = f'expected {expected!r}, got {actual!r}'
            else:
                cr['pass'] = True

        elif kind == 'static_equals':
            method = getattr(cls, member)
            args = [literal_arg(a) for a in check.get('args', [])]
            actual = method(*args)
            expected = check['expected']
            if actual != expected:
                cr['error'] = f'expected {expected!r}, got {actual!r}'
            else:
                cr['pass'] = True

        elif kind == 'static_not_null':
            method = getattr(cls, member)
            args = [literal_arg(a) for a in check.get('args', [])]
            actual = method(*args)
            if actual is None:
                cr['error'] = 'returned None'
            else:
                cr['pass'] = True

        elif kind == 'property_in_range':
            actual = getattr(obj, member)
            expected_type = check.get('expected_type')
            if expected_type and type(actual).__name__ != expected_type:
                cr['error'] = (
                    f'expected type {expected_type}, '
                    f'got {type(actual).__name__}'
                )
                return cr
            min_val = check.get('min', float('-inf'))
            max_val = check.get('max', float('inf'))
            # Handle enum: extract .value if it's an IntEnum
            val = actual.value if hasattr(actual, 'value') else actual
            if not (min_val <= val <= max_val):
                cr['error'] = f'value {val} not in [{min_val}, {max_val}]'
            else:
                cr['pass'] = True

        elif kind == 'interface_cast':
            iface_mod_name = to_snake_case(check.get('interface_module', check['interface_class']))
            iface_mod = importlib.import_module(f"{pkg_name}.{iface_mod_name}")
            iface_cls = getattr(iface_mod, check['interface_class'])
            casted = obj.as_interface(iface_cls)
            method_name = to_snake_case(check['method'])
            result_val = getattr(casted, method_name)
            if callable(result_val):
                result_val = result_val()
            actual = str(result_val)
            if check.get('contains') and check['contains'] not in actual:
                cr['error'] = f'"{check["contains"]}" not in "{actual}"'
            elif check.get('expected') and actual != check['expected']:
                cr['error'] = f'expected {check["expected"]!r}, got {actual!r}'
            else:
                cr['pass'] = True

        elif kind == 'property_set_equals':
            set_value = check['set_value']
            setattr(obj, member, set_value)
            actual = getattr(obj, member)
            expected = check['expected']
            if actual != expected:
                cr['error'] = f'expected {expected!r}, got {actual!r}'
            else:
                cr['pass'] = True

        elif kind == 'vector_view_access':
            vec = getattr(obj, member)
            min_size = check.get('min_size', 1)
            size = vec.size
            if size < min_size:
                cr['error'] = f'vector size {size} < {min_size}'
            else:
                first = vec.get_at(0)
                if first is None:
                    cr['error'] = 'get_at(0) returned None'
                else:
                    cr['pass'] = True

        elif kind == 'vector_index_of':
            vec = getattr(obj, member)
            search_value = check.get('search_value')
            if search_value is None:
                search_value = vec.get_at(0)
            index, found = vec.index_of(search_value)
            actual = index if found else -1
            expected = check['expected_index']
            if actual != expected:
                cr['error'] = f'index_of returned {actual}, expected {expected}'
            else:
                cr['pass'] = True

        elif kind == 'vector_get_many':
            import dynwinrt_py as dw

            vec = getattr(obj, member)
            capacity = min(check.get('capacity', 4), vec.size)
            buffer = dw.DynWinRTArray.from_string_values([''] * capacity)
            at_end = check.get('at_end', False)
            items = vec.get_many(vec.size if at_end else 0, buffer)
            if at_end and len(items) != 0:
                cr['error'] = f'get_many at Size returned {len(items)} items'
            elif not at_end and len(items) == 0:
                cr['error'] = 'get_many returned no items'
            elif not at_end and items[0] != vec.get_at(0):
                cr['error'] = f'first item {items[0]!r} does not match get_at(0)'
            else:
                cr['pass'] = True

        elif kind == 'ireference_roundtrip':
            setattr(obj, member, check['value'])
            actual = getattr(obj, member)
            if actual != check['value']:
                cr['error'] = (
                    f'native IReference roundtrip returned {actual!r}, '
                    f'expected {check["value"]!r}'
                )
                return cr

            setattr(obj, member, None)
            if getattr(obj, member) is not None:
                cr['error'] = 'setting nullable value to None did not clear it'
                return cr

            property_value_mod = importlib.import_module(f"{pkg_name}.property_value")
            property_value_cls = getattr(property_value_mod, 'PropertyValue')
            factory = getattr(property_value_cls, check['factory'])
            boxed = factory(check['compatibility_value'])

            reference_mod = importlib.import_module(
                f"{pkg_name}.{check['reference_module']}"
            )
            reference_cls = getattr(reference_mod, check['reference_class'])
            reference = reference_cls.from_value(getattr(boxed, '_obj', boxed))

            setattr(obj, member, reference)
            actual = getattr(obj, member)
            if actual != check['compatibility_value']:
                cr['error'] = (
                    f'wrapper IReference roundtrip returned {actual!r}, '
                    f'expected {check["compatibility_value"]!r}'
                )
            else:
                cr['pass'] = True

        elif kind == 'struct_roundtrip':
            struct_mod = importlib.import_module(f"{pkg_name}.{check['struct_module']}")
            struct_cls = getattr(struct_mod, check['struct_class'])

            # Create struct instance with kwargs
            struct_obj = struct_cls(**{to_snake_case(k): v for k, v in check['struct_args'].items()})

            # Pass struct directly to static method (generated code handles pack internally)
            static_method = getattr(cls, to_snake_case(check['member']))
            result = static_method(struct_obj)
            if result is None:
                cr['error'] = 'static method returned None'
            else:
                # Verify struct fields
                if check.get('expected_fields'):
                    for field, expected in check['expected_fields'].items():
                        actual = getattr(struct_obj, to_snake_case(field))
                        if isinstance(expected, float):
                            if abs(actual - expected) > 0.001:
                                cr['error'] = f'field {field}: expected {expected}, got {actual}'
                                break
                        elif actual != expected:
                            cr['error'] = f'field {field}: expected {expected}, got {actual}'
                            break
                    else:
                        cr['pass'] = True
                else:
                    cr['pass'] = True

        elif kind == 'array_roundtrip':
            import dynwinrt_py as dw
            elem_type = check['element_type']
            values = check['values']

            # Create array using DynWinRTArray factory
            if elem_type == 'i32':
                arr = dw.DynWinRTArray.from_i32_values(values)
            elif elem_type == 'string':
                arr = dw.DynWinRTArray.from_string_values(values)
            elif elem_type == 'f64':
                arr = dw.DynWinRTArray.from_f64_values(values)
            elif elem_type == 'u8':
                arr = dw.DynWinRTArray.from_u8_values(values)
            elif elem_type == 'i64':
                arr = dw.DynWinRTArray.from_i64_values(values)
            elif elem_type == 'f32':
                arr = dw.DynWinRTArray.from_f32_values(values)
            else:
                cr['error'] = f'unsupported array element_type: {elem_type}'
                return cr

            # Pass to static method
            static_method = getattr(cls, to_snake_case(check['member']))
            result = static_method(arr)
            if result is None:
                cr['error'] = 'static method returned None for array'
            else:
                cr['pass'] = True

        elif kind == 'event_callback':
            source_method = getattr(obj, member)
            source = source_method()
            event_name = to_snake_case(check['event_name'])
            trigger = to_snake_case(check['trigger'])

            fired = [False]
            on_method = f'on_{event_name}'
            getattr(source, on_method)(lambda *args: fired.__setitem__(0, True))

            # Try direct method, fall back to IClosable cast
            if hasattr(source, trigger):
                getattr(source, trigger)()
            else:
                iface_mod = importlib.import_module(f"{pkg_name}.i_closable")
                IClosable = getattr(iface_mod, 'IClosable')
                IClosable.from_value(source._obj).close()

            if not fired[0]:
                cr['error'] = f'event {check["event_name"]} was not fired after {trigger}()'
            else:
                cr['pass'] = True

        elif kind == 'static_string_length':
            method = getattr(cls, to_snake_case(check['member']))
            args = [literal_arg(a) for a in check.get('args', [])]
            actual = method(*args)
            actual_str = str(actual)
            min_len = check.get('min_length', 0)
            if len(actual_str) < min_len:
                cr['error'] = f'string length {len(actual_str)} < {min_len}'
            else:
                cr['pass'] = True

        elif kind == 'static_expect_error':
            method = getattr(cls, to_snake_case(check['member']))
            args = [literal_arg(a) for a in check.get('args', [])]
            try:
                method(*args)
                cr['error'] = 'expected error but call succeeded'
            except OSError as error:
                expected_winerror = check.get('expected_winerror')
                if expected_winerror is not None and error.winerror != expected_winerror:
                    cr['error'] = (
                        f'expected winerror {expected_winerror}, '
                        f'got {error.winerror}'
                    )
                elif not isinstance(error.winerror, int):
                    cr['error'] = f'expected integer winerror, got {error.winerror!r}'
                else:
                    cr['pass'] = True
            except Exception as error:
                cr['error'] = f'expected OSError, got {type(error).__name__}: {error}'

        elif kind == 'sequence_protocol':
            sequence = obj if member == 'self' else getattr(obj, member)
            expected_size = check['expected_size']
            values = list(sequence)
            if len(sequence) != expected_size or len(values) != expected_size:
                cr['error'] = (
                    f'expected sequence size {expected_size}, '
                    f'got len={len(sequence)}, iter={len(values)}'
                )
            elif expected_size and not projected_values_equal(sequence[-1], values[-1]):
                cr['error'] = 'negative indexing returned a different value'
            elif len(sequence[:]) != len(values) or any(
                not projected_values_equal(left, right)
                for left, right in zip(sequence[:], values)
            ):
                cr['error'] = 'full slice did not match iteration'
            else:
                cr['pass'] = True

        elif kind == 'mapping_protocol':
            mapping = getattr(obj, member)
            expected_size = check['expected_size']
            snapshot = dict(mapping)
            if len(mapping) != expected_size or len(snapshot) != expected_size:
                cr['error'] = (
                    f'expected mapping size {expected_size}, '
                    f'got len={len(mapping)}, items={len(snapshot)}'
                )
            elif set(iter(mapping)) != set(snapshot):
                cr['error'] = 'mapping iteration did not yield its keys'
            else:
                key = next(iter(snapshot))
                if mapping[key] != snapshot[key]:
                    cr['error'] = 'mapping lookup disagreed with items()'
                else:
                    if 'set_key' in check:
                        mapping[check['set_key']] = check['set_value']
                        if mapping[check['set_key']] != check['set_value']:
                            cr['error'] = 'mapping assignment did not round-trip'
                            return cr
                        del mapping[check['set_key']]
                        if check['set_key'] in mapping:
                            cr['error'] = 'mapping deletion did not remove the key'
                            return cr
                    cr['pass'] = True

        elif kind == 'mutable_sequence_protocol':
            sequence = getattr(obj, member)
            values = check['set_value']
            sequence.extend(values)
            if list(sequence) != values:
                cr['error'] = f'extend mismatch: {list(sequence)!r}'
            else:
                sequence[0] = values[-1]
                sequence.insert(-100, values[0])
                del sequence[2:]
                expected = [values[0], values[-1]]
                if list(sequence) != expected:
                    cr['error'] = (
                        f'mutable sequence operations expected {expected!r}, '
                        f'got {list(sequence)!r}'
                    )
                else:
                    cr['pass'] = True

        elif kind == 'datetime_roundtrip':
            from datetime import datetime, timezone

            value = datetime(2024, 1, 2, 3, 4, 5, 678901, tzinfo=timezone.utc)
            getattr(obj, f'set_{member}')(value)
            actual = getattr(obj, f'get_{member}')()
            if actual != value:
                cr['error'] = f'expected {value!r}, got {actual!r}'
            else:
                cr['pass'] = True

        elif kind == 'static_uuid_roundtrip':
            from uuid import UUID

            actual = getattr(cls, member)()
            if not isinstance(actual, UUID):
                cr['error'] = f'expected UUID, got {type(actual).__name__}'
            else:
                cr['pass'] = True

        elif kind == 'static_uuid_input':
            from uuid import uuid4

            actual = getattr(cls, member)(uuid4())
            if actual is None:
                cr['error'] = 'UUID input returned None'
            else:
                cr['pass'] = True

        elif kind == 'static_bytes_input':
            actual = getattr(cls, member)(bytes([0, 1, 127, 255]))
            if actual is None:
                cr['error'] = 'bytes input returned None'
            else:
                cr['pass'] = True

        elif kind == 'static_sequence_input':
            actual = getattr(cls, member)([1, 2, 3])
            if actual is None:
                cr['error'] = 'sequence input returned None'
            else:
                cr['pass'] = True

        elif kind == 'closable_context':
            resource = cls(*[literal_arg(value) for value in check.get('args', [])])
            with resource as entered:
                if entered is not resource:
                    cr['error'] = '__enter__ returned a different object'
                    return cr
            resource.close()
            try:
                resource.__enter__()
                cr['error'] = 'closed object allowed context re-entry'
            except RuntimeError:
                cr['pass'] = True

        elif kind == 'cross_class_chain':
            saved = {}
            chain_ok = True
            for step in check['steps']:
                step_cls_name = step['class']
                step_mod = importlib.import_module(f"{pkg_name}.{to_snake_case(step_cls_name)}")
                step_cls = getattr(step_mod, step_cls_name)
                step_method = getattr(step_cls, to_snake_case(step['method']))

                # Build args: literal or refs to saved values
                step_args = []
                for a in step.get('args', []):
                    step_args.append(literal_arg(a))
                for ref in step.get('args_refs', []):
                    step_args.append(saved[ref])

                result = step_method(*step_args)

                if 'save_as' in step:
                    saved[step['save_as']] = result
                if 'expected' in step:
                    actual = str(result) if not isinstance(result, (int, float, bool)) else result
                    if actual != step['expected']:
                        cr['error'] = f'{step["method"]}: expected {step["expected"]!r}, got {actual!r}'
                        chain_ok = False
                        break
            if chain_ok:
                cr['pass'] = True

        elif kind == 'async_memory_roundtrip':
            import dynwinrt_py as dw
            from dynwinrt_py.dynwinrt_py import _DynWinRTAsync

            write_val = check.get('write_value', 42)
            stream = cls.create() if hasattr(cls, 'create') else cls.create_default()

            writer_mod = importlib.import_module(f"{pkg_name}.data_writer")
            reader_mod = importlib.import_module(f"{pkg_name}.data_reader")
            writer_cls = getattr(writer_mod, 'DataWriter')
            reader_cls = getattr(reader_mod, 'DataReader')

            writer = writer_cls.create_data_writer(stream.get_output_stream_at(0))
            writer.write_int32(write_val)
            store_op = writer.store_async()
            if not inspect.isawaitable(store_op):
                cr['error'] = 'store_async() did not return an awaitable'
                return cr
            stored = await store_op
            stored_again = await store_op

            stream.seek(0)
            reader = reader_cls.create_data_reader(stream.get_input_stream_at(0))
            loaded = await reader.load_async(4)
            read_val = reader.read_int32()

            def blocking_store():
                import dynwinrt_py as dw

                dw.ro_initialize(1)
                try:
                    blocking_stream = (
                        cls.create() if hasattr(cls, 'create') else cls.create_default()
                    )
                    blocking_writer = writer_cls.create_data_writer(
                        blocking_stream.get_output_stream_at(0)
                    )
                    blocking_writer.write_int32(write_val)
                    return blocking_writer.store_async().wait()
                finally:
                    dw.ro_uninitialize()

            blocked = await asyncio.get_running_loop().run_in_executor(
                None, blocking_store
            )

            conversion_threads = []
            loop_thread = threading.get_ident()
            conversion_stream = (
                cls.create() if hasattr(cls, 'create') else cls.create_default()
            )
            conversion_writer = writer_cls.create_data_writer(
                conversion_stream.get_output_stream_at(0)
            )
            conversion_writer.write_int32(write_val)
            raw_store = writer_mod._IDataWriter.method_by_name('StoreAsync').invoke(
                conversion_writer._obj, []
            )

            def convert_store_result(value):
                conversion_threads.append(threading.get_ident())
                return value.to_number()

            converted = await _DynWinRTAsync(raw_store, convert_store_result)

            buffer_mod = importlib.import_module(f"{pkg_name}.buffer")
            buffer_cls = getattr(buffer_mod, 'Buffer')
            progress_buffer = buffer_cls.create(1024 * 1024)
            progress_buffer.length = progress_buffer.capacity
            progress = []
            write_op = stream.get_output_stream_at(stream.size).write_async(progress_buffer)
            write_op.progress(progress.append)
            written = await write_op
            await asyncio.sleep(0)

            if (
                stored < 4
                or stored_again != stored
                or loaded < 4
                or blocked < 4
                or read_val != write_val
                or converted < 4
                or conversion_threads != [loop_thread]
                or written != progress_buffer.length
                or not all(isinstance(value, int) for value in progress)
            ):
                cr['error'] = (
                    f'async roundtrip failed: stored={stored}, stored_again={stored_again}, '
                    f'loaded={loaded}, blocked={blocked}, converted={converted}, '
                    f'conversion_threads={conversion_threads}, loop_thread={loop_thread}, '
                    f'written={written}, '
                    f'progress={progress!r}, wrote {write_val}, read {read_val}'
                )
            else:
                cr['pass'] = True

        elif kind == 'async_cancellation':
            import dynwinrt_py as dw

            info_iid = dw.WinGUID.parse('00000036-0000-0000-c000-000000000046')
            info_type = (
                dw.DynWinRTType.register_interface('IAsyncInfoE2E', info_iid)
                .add_method(
                    'get_Id',
                    dw.DynWinRTMethodSig().add_out(dw.DynWinRTType.u32_type()),
                )
                .add_method(
                    'get_Status',
                    dw.DynWinRTMethodSig().add_out(dw.DynWinRTType.i32_type()),
                )
            )
            status_method = info_type.method(7)
            started = threading.Event()
            release = threading.Event()
            cancel_seen = threading.Event()
            worker_errors = []

            def work(action):
                started.set()
                try:
                    action = action.cast(info_iid)
                    while not release.wait(0.01):
                        if status_method.invoke(action, []).to_number() == 2:
                            cancel_seen.set()
                            break
                except BaseException as error:
                    worker_errors.append(error)

            operation = cls.run_async(work)
            loop = asyncio.get_running_loop()

            if not await loop.run_in_executor(None, started.wait, 2.0):
                release.set()
                cr['error'] = 'ThreadPool work item did not start'
                return cr

            try:
                operation.wait()
                release.set()
                cr['error'] = 'wait() was not rejected on a running asyncio loop'
                return cr
            except RuntimeError as error:
                if 'asyncio event loop' not in str(error):
                    release.set()
                    cr['error'] = f'unexpected asyncio wait() error: {error}'
                    return cr

            sta_errors = []

            def block_on_sta():
                dw.ro_initialize(0)
                try:
                    operation.wait()
                except RuntimeError as error:
                    sta_errors.append(str(error))
                finally:
                    dw.ro_uninitialize()

            sta_thread = threading.Thread(target=block_on_sta)
            sta_thread.start()
            await loop.run_in_executor(None, sta_thread.join, 2.0)
            if sta_thread.is_alive():
                release.set()
                cr['error'] = 'STA wait() did not return'
                return cr
            if not sta_errors or 'STA thread' not in sta_errors[0]:
                release.set()
                cr['error'] = f'wait() was not rejected on STA: {sta_errors!r}'
                return cr

            task = asyncio.ensure_future(operation)
            await asyncio.sleep(0)
            task.cancel()
            try:
                await task
                cr['error'] = 'cancelled asyncio task completed successfully'
                return cr
            except asyncio.CancelledError:
                pass

            observed = await loop.run_in_executor(None, cancel_seen.wait, 2.0)
            release.set()
            if worker_errors:
                cr['error'] = f'work item failed: {worker_errors[0]}'
            elif not observed:
                cr['error'] = 'IAsyncInfo.Cancel was not observed by the work item'
            else:
                cr['pass'] = True

        else:
            cr['error'] = f'unknown check kind: {kind}'

    except Exception as e:
        cr['error'] = str(e)

    return cr


def main():
    parser = argparse.ArgumentParser(description='E2E Python test runner')
    parser.add_argument('--specs', required=True, help='Path to e2e_specs.json')
    parser.add_argument('--generated', required=True, help='Path to generated Python package dir')
    parser.add_argument('--output', default=None, help='Path to write results.json')
    args = parser.parse_args()

    # Add parent of generated dir to sys.path so package imports work
    # Generated files use relative imports (from .xxx import), so the dir must be a package
    gen_parent = os.path.dirname(os.path.abspath(args.generated))
    gen_pkg = os.path.basename(os.path.abspath(args.generated))
    sys.path.insert(0, gen_parent)

    # Init WinRT
    import dynwinrt_py as dw
    dw.ro_initialize(1)

    # Load specs
    with open(args.specs) as f:
        data = json.load(f)

    specs = [s for s in data['specs'] if 'py' in s.get('langs', ['py', 'ts']) and not s.get('skip_reason')]

    results = []
    passed = 0
    failed = 0

    async def run_all_specs():
        all_results = []
        for spec in specs:
            all_results.append(await run_spec(spec, args.generated, gen_pkg))
        return all_results

    results = asyncio.run(run_all_specs())

    for r in results:
        if r['pass']:
            passed += 1
            print(f"  PASS {r['id']}")
        else:
            failed += 1
            err = r['error'] or '; '.join(c['error'] for c in r['checks'] if not c['pass'])
            print(f"  FAIL {r['id']}: {err}")

    print(f"\n  Python: {passed} passed, {failed} failed")

    # Write results
    output = {
        'language': 'py',
        'total': len(results),
        'passed': passed,
        'failed': failed,
        'results': results,
    }

    if args.output:
        with open(args.output, 'w') as f:
            json.dump(output, f, indent=2)

    sys.exit(1 if failed > 0 else 0)


if __name__ == '__main__':
    main()
