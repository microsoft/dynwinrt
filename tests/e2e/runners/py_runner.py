# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""
E2E test runner for Python generated bindings.

Reads e2e_specs.json, imports generated Python modules,
and executes checks against real WinRT APIs.

Usage:
    python tests/e2e/runners/py_runner.py --specs tests/e2e/e2e_specs.json --generated tests/e2e/e2e_generated/python_bindings --output results.json
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


_WINRT_UINT_SUFFIXES = {'int8', 'int16', 'int32', 'int64'}


def collapse_winrt_uint_tokens(name: str) -> str:
    tokens = name.split('_')
    collapsed = []
    index = 0
    while index < len(tokens):
        if (
            tokens[index] == 'u'
            and index + 1 < len(tokens)
            and tokens[index + 1] in _WINRT_UINT_SUFFIXES
        ):
            collapsed.append(f'u{tokens[index + 1]}')
            index += 2
        else:
            collapsed.append(tokens[index])
            index += 1
    return '_'.join(collapsed)


def to_snake_case(name: str) -> str:
    """Convert PascalCase/camelCase to snake_case."""
    value = re.sub(r'(.)([A-Z][a-z]+)', r'\1_\2', name)
    value = re.sub(r'([a-z0-9])([A-Z])', r'\1_\2', value).lstrip('_').lower()
    value = re.sub(r'_+', '_', value)
    return collapse_winrt_uint_tokens(value)


def to_camel_case(name: str) -> str:
    """Convert snake_case to camelCase."""
    parts = name.split('_')
    return parts[0] + ''.join(p.capitalize() for p in parts[1:])


def namespace_module_name(package_name: str, namespace: str) -> str:
    segments = '.'.join(to_snake_case(segment) for segment in namespace.split('.'))
    return f"{package_name}.{segments}"


def implementation_module_name(package_name: str, namespace: str, type_name: str) -> str:
    namespace_part = '__'.join(
        to_snake_case(segment) for segment in namespace.split('.')
    )
    return f"{package_name}.{namespace_part}__{to_snake_case(type_name)}"


def generated_type(package_name: str, type_name: str):
    package = importlib.import_module(package_name)
    return getattr(package, type_name)


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
    import dynwinrt as dw
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
        mod = importlib.import_module(namespace_module_name(pkg_name, ns))
        cls = getattr(mod, cls_name)

        # Instantiate
        inst_kind = spec['instantiate']['kind']
        obj = None

        if inst_kind == 'activate':
            import dynwinrt as dw
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
            check_result = await run_check(check, cls, obj, generated_dir, pkg_name, ns)
            result['checks'].append(check_result)
            if not check_result['pass']:
                result['pass'] = False

    except Exception as e:
        result['pass'] = False
        result['error'] = str(e)

    return result


async def run_check(
    check: dict,
    cls,
    obj,
    generated_dir: str,
    pkg_name: str,
    namespace: str,
) -> dict:
    """Run a single check. Returns { kind, member, pass, error }."""
    import dynwinrt as dw

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
                af_cls = generated_type(pkg_name, af['class'])
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

        elif kind == 'constructor_raises_type_error':
            args = [literal_arg(a) for a in check.get('args', [])]
            try:
                cls(*args)
                cr['error'] = 'constructor unexpectedly succeeded'
            except TypeError as exc:
                expected = check.get('contains', cls.__name__)
                if expected not in str(exc):
                    cr['error'] = (
                        f'expected TypeError containing {expected!r}, got {str(exc)!r}'
                    )
                else:
                    cr['pass'] = True

        elif kind == 'method_then_property_equals':
            target = obj
            if check.get('interface_class'):
                iface_cls = generated_type(pkg_name, check['interface_class'])
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
            iface_cls = generated_type(pkg_name, check['interface_class'])
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

        elif kind == 'narrow_integer_overflow':
            cases = (
                ('create_uint8', (256,)),
                ('create_int16', (32768,)),
                ('create_uint16', (65536,)),
                ('create_char16', ('\U0001f600',)),
                ('create_uint16_array', ([65536],)),
                ('create_char16_array', (['\U0001f600'],)),
            )
            for method_name, args in cases:
                try:
                    getattr(cls, method_name)(*args)
                except OverflowError:
                    continue
                except Exception as error:
                    cr['error'] = (
                        f'{method_name} raised {type(error).__name__}, '
                        'expected OverflowError'
                    )
                    return cr
                cr['error'] = f'{method_name} accepted an out-of-range value'
                return cr
            cr['pass'] = True

        elif kind == 'nullable_object_array_roundtrip':
            uri_cls = generated_type(pkg_name, 'Uri')
            uri = uri_cls.create_uri('https://example.com/null-array')
            boxed = getattr(cls, member)(
                [dw.DynWinRTValue.null_value(), uri._obj]
            )
            if boxed is None:
                cr['error'] = 'CreateInspectableArray returned None'
                return cr
            values = boxed.call_0(
                38,
                dw.DynWinRTType.array_type(dw.DynWinRTType.object()),
            ).as_array().to_values()
            if len(values) != 2:
                cr['error'] = f'expected 2 inspectable values, got {len(values)}'
            elif not values[0].is_null():
                cr['error'] = 'null inspectable array element was not preserved'
            elif values[1].identity_raw() != uri._obj.identity_raw():
                cr['error'] = 'inspectable array element lost COM identity'
            else:
                cr['pass'] = True

        elif kind == 'projection_identity':
            import weakref

            iface_cls = generated_type(pkg_name, check['interface_class'])
            same_class = cls(obj._obj)
            iface_one = obj.as_interface(iface_cls)
            iface_two = iface_cls.from_value(obj._obj)

            try:
                class_ref = weakref.ref(obj)
                iface_ref = weakref.ref(iface_one)
            except TypeError as error:
                cr['error'] = f'projected wrappers must support weak references: {error}'
                return cr

            if same_class is not obj:
                cr['error'] = 'runtime-class projection did not preserve wrapper identity'
            elif iface_one is not iface_two:
                cr['error'] = 'interface projection did not preserve wrapper identity'
            elif iface_one is obj:
                cr['error'] = 'distinct projected wrapper types shared one cache entry'
            elif class_ref() is not obj or iface_ref() is not iface_one:
                cr['error'] = 'projected wrappers did not remain weak-referenceable'
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
            import dynwinrt as dw

            vec = getattr(obj, member)
            capacity = min(check.get('capacity', 4), vec.size)
            buffer = dw.DynWinRTArray.from_string_values([''] * capacity)
            at_end = check.get('at_end', False)
            items = vec.get_many(vec.size if at_end else 0, buffer)
            if capacity == 0:
                if len(items) != 0:
                    cr['error'] = 'zero-capacity get_many returned items'
                else:
                    cr['pass'] = True
            elif at_end and len(items) != 0:
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

            property_value_cls = generated_type(pkg_name, 'PropertyValue')
            factory = getattr(property_value_cls, to_snake_case(check['factory']))
            boxed = factory(check['compatibility_value'])

            reference_cls = generated_type(pkg_name, check['reference_class'])
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
            struct_module = check.get(
                'py_struct_module',
                to_snake_case(check['struct_class']),
            )
            struct_mod = importlib.import_module(
                f"{namespace_module_name(pkg_name, namespace)}.{struct_module}"
            )
            struct_cls = getattr(struct_mod, check['struct_class'])
            pack = getattr(struct_mod, check['pack_fn'])
            unpack = getattr(struct_mod, check['unpack_fn'])

            # Create struct instance with kwargs
            struct_obj = struct_cls(**{to_snake_case(k): v for k, v in check['struct_args'].items()})
            roundtrip = unpack(pack(struct_obj).to_value())
            if roundtrip != struct_obj:
                cr['error'] = (
                    f'struct helper roundtrip returned {roundtrip!r}, '
                    f'expected {struct_obj!r}'
                )
                return cr
            if roundtrip.__eq__(object()) is not NotImplemented:
                cr['error'] = 'struct equality accepted a different type'
                return cr
            if check['struct_class'] not in repr(roundtrip):
                cr['error'] = 'struct repr omitted the projected type name'
                return cr

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
            import dynwinrt as dw
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
                IClosable = generated_type(pkg_name, 'IClosable')
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
                        if mapping.get(check['set_key']) != check['set_value']:
                            cr['error'] = 'mapping get() disagreed with lookup'
                            return cr
                        missing = '__dynwinrt_missing__'
                        if mapping.get(missing, 'fallback') != 'fallback':
                            cr['error'] = 'mapping get() ignored its default'
                            return cr
                        try:
                            _ = mapping[missing]
                            cr['error'] = 'missing mapping lookup did not raise KeyError'
                            return cr
                        except KeyError:
                            pass
                        del mapping[check['set_key']]
                        if check['set_key'] in mapping:
                            cr['error'] = 'mapping deletion did not remove the key'
                            return cr
                        try:
                            del mapping[check['set_key']]
                            cr['error'] = 'missing mapping deletion did not raise KeyError'
                            return cr
                        except KeyError:
                            pass
                        view = mapping.get_view()
                        if view is None or dict(view) != dict(mapping):
                            cr['error'] = 'mapping view did not match the mutable map'
                            return cr
                        try:
                            view[check['set_key']] = check['set_value']
                            cr['error'] = 'read-only map view accepted assignment'
                            return cr
                        except TypeError:
                            pass
                    cr['pass'] = True

        elif kind == 'constructor_overload_dispatch':
            single = cls(uri='https://example.com/single')
            relative = cls(
                base_uri='https://example.com/root/',
                relative_uri='child',
            )
            mixed = cls('https://example.com/root/', relative_uri='mixed')
            if (
                single.host != 'example.com'
                or not relative.absolute_uri.endswith('/root/child')
                or not mixed.absolute_uri.endswith('/root/mixed')
            ):
                cr['error'] = 'keyword or mixed constructor overload returned wrong URI'
                return cr

            invalid_calls = [
                ((), {}),
                ((42,), {}),
                (('https://example.com', 'child', 'extra'), {}),
                (('https://example.com',), {'uri': 'https://duplicate.example'}),
                ((), {'unknown': 'https://example.com'}),
            ]
            for args, kwargs in invalid_calls:
                try:
                    cls(*args, **kwargs)
                    cr['error'] = (
                        f'invalid constructor unexpectedly accepted args={args!r}, '
                        f'kwargs={kwargs!r}'
                    )
                    return cr
                except TypeError:
                    pass
            cr['pass'] = True

        elif kind == 'value_set_mapping':
            property_value_cls = generated_type(pkg_name, 'PropertyValue')
            first = property_value_cls.create_string('first')
            second = property_value_cls.create_int32(2)
            first_raw = getattr(first, '_obj', first)
            second_raw = getattr(second, '_obj', second)

            if len(obj) != 0 or obj.size != 0:
                cr['error'] = 'new ValueSet was not empty'
                return cr
            if obj.insert('first', first_raw):
                cr['error'] = 'first ValueSet insert reported replacement'
                return cr
            obj['second'] = second
            if len(obj) != 2 or not obj.has_key('first') or 'second' not in obj:
                cr['error'] = 'ValueSet insertion or membership failed'
                return cr
            first_lookup = obj.lookup('first')
            second_lookup = obj['second']
            if first_lookup.is_null() or second_lookup.is_null():
                cr['error'] = 'ValueSet lookup returned null'
                return cr
            if (
                first_lookup.identity_raw() != first_raw.identity_raw()
                or second_lookup.identity_raw() != second_raw.identity_raw()
            ):
                cr['error'] = 'ValueSet payloads did not round-trip'
                return cr

            view = obj.get_view()
            if view is None or len(view) != 2 or set(view) != {'first', 'second'}:
                cr['error'] = 'ValueSet view did not preserve entries'
                return cr

            try:
                _ = obj['missing']
                cr['error'] = 'missing ValueSet lookup did not raise KeyError'
                return cr
            except KeyError:
                pass
            try:
                del obj['missing']
                cr['error'] = 'missing ValueSet deletion did not raise KeyError'
                return cr
            except KeyError:
                pass

            del obj['first']
            if obj.has_key('first') or len(obj) != 1:
                cr['error'] = 'ValueSet deletion failed'
                return cr
            obj.clear()
            if len(obj) != 0 or list(obj):
                cr['error'] = 'ValueSet clear failed'
                return cr
            cr['pass'] = True

        elif kind == 'value_set_event_lifecycle':
            import dynwinrt as dw

            counts = {'on': 0, 'subscribe': 0, 'once': 0}

            def increment(name):
                def callback(*_args):
                    counts[name] += 1
                return callback

            token = obj.on_map_changed(increment('on'))
            unsubscribe = obj.subscribe_map_changed(increment('subscribe'))
            obj.once_map_changed(increment('once'))
            property_value_cls = generated_type(pkg_name, 'PropertyValue')

            obj['first-event'] = property_value_cls.create_int32(1)
            obj['second-event'] = property_value_cls.create_int32(2)
            if counts != {'on': 2, 'subscribe': 2, 'once': 1}:
                cr['error'] = f'event lifecycle counts were wrong: {counts!r}'
                return cr

            obj.off_map_changed(token)
            unsubscribe()
            unsubscribe()
            obj['after-unsubscribe'] = property_value_cls.create_int32(3)
            if counts != {'on': 2, 'subscribe': 2, 'once': 1}:
                cr['error'] = f'event unsubscribe was ineffective: {counts!r}'
                return cr

            errors = []
            previous_hook = sys.unraisablehook
            sys.unraisablehook = errors.append

            def fail(*_args):
                raise RuntimeError('ValueSet callback failed')

            failing_token = obj.on_map_changed(fail)
            try:
                try:
                    obj['callback-error'] = property_value_cls.create_int32(4)
                except OSError:
                    pass
            finally:
                sys.unraisablehook = previous_hook
                obj.off_map_changed(failing_token)

            if (
                len(errors) != 1
                or 'ValueSet callback failed' not in str(errors[0].exc_value)
            ):
                cr['error'] = (
                    'event callback failure was not reported through '
                    f'sys.unraisablehook: {errors!r}'
                )
            else:
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
                    sequence[:] = values
                    sequence[1:2] = ['.bmp', '.webp']
                    if list(sequence) != [
                        values[0],
                        '.bmp',
                        '.webp',
                        values[2],
                    ]:
                        cr['error'] = (
                            f'slice assignment failed: {list(sequence)!r}'
                        )
                        return cr
                    for operation in (
                        lambda: sequence[len(sequence)],
                        lambda: sequence.__setitem__(len(sequence), '.bad'),
                        lambda: sequence.__delitem__(len(sequence)),
                    ):
                        try:
                            operation()
                            cr['error'] = (
                                'out-of-range sequence operation succeeded'
                            )
                            return cr
                        except IndexError:
                            pass
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
                if hasattr(resource, 'create_reference'):
                    from dynwinrt import release_projected

                    release_projected(resource)
                    try:
                        resource.create_reference()
                        cr['error'] = 'released object allowed a WinRT method call'
                        return cr
                    except (OSError, RuntimeError):
                        pass
                cr['pass'] = True

        elif kind == 'cross_class_chain':
            saved = {}
            chain_ok = True
            for step in check['steps']:
                step_cls_name = step['class']
                step_cls = generated_type(pkg_name, step_cls_name)
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
            import dynwinrt as dw
            from dynwinrt.dynwinrt import _DynWinRTAsync

            write_val = check.get('write_value', 42)
            stream = cls.create() if hasattr(cls, 'create') else cls.create_default()

            writer_cls = generated_type(pkg_name, 'DataWriter')
            reader_cls = generated_type(pkg_name, 'DataReader')
            writer_mod = importlib.import_module(
                implementation_module_name(
                    pkg_name, 'Windows.Storage.Streams', 'DataWriter'
                )
            )

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
                import dynwinrt as dw

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

            failing_stream = (
                cls.create() if hasattr(cls, 'create') else cls.create_default()
            )
            failing_writer = writer_cls.create_data_writer(
                failing_stream.get_output_stream_at(0)
            )
            failing_writer.write_int32(write_val)
            raw_failing_store = writer_mod._IDataWriter.method_by_name(
                'StoreAsync'
            ).invoke(failing_writer._obj, [])

            def reject_store_result(_value):
                raise ValueError('async result conversion failed')

            try:
                await _DynWinRTAsync(raw_failing_store, reject_store_result)
                cr['error'] = 'async result converter failure was not propagated'
                return cr
            except ValueError as error:
                if str(error) != 'async result conversion failed':
                    cr['error'] = f'unexpected async conversion error: {error}'
                    return cr

            release_stream = (
                cls.create() if hasattr(cls, 'create') else cls.create_default()
            )
            release_writer = writer_cls.create_data_writer(
                release_stream.get_output_stream_at(0)
            )
            release_writer.write_int32(write_val)
            released_operation = release_writer.store_async()
            await released_operation
            released_operation.release()
            released_operation.release()
            for action in (
                lambda: released_operation.wait(),
                lambda: released_operation.cancel(),
            ):
                try:
                    action()
                    cr['error'] = 'released async operation accepted an operation'
                    return cr
                except RuntimeError as error:
                    if 'has been released' not in str(error):
                        cr['error'] = (
                            f'unexpected released operation error: {error}'
                        )
                        return cr
            try:
                await released_operation
                cr['error'] = 'released async operation remained awaitable'
                return cr
            except RuntimeError as error:
                if 'has been released' not in str(error):
                    cr['error'] = f'unexpected released await error: {error}'
                    return cr

            buffer_cls = generated_type(pkg_name, 'Buffer')
            progress_buffer = buffer_cls.create(1024 * 1024)
            progress_buffer.length = progress_buffer.capacity
            progress = []
            write_op = stream.get_output_stream_at(stream.size).write_async(progress_buffer)
            write_op.progress(progress.append)
            written = await write_op
            await asyncio.sleep(0)
            progress_errors = []

            def progress_without_loop():
                try:
                    write_op.progress(lambda _value: None)
                except RuntimeError as error:
                    progress_errors.append(str(error))

            await asyncio.get_running_loop().run_in_executor(
                None, progress_without_loop
            )
            if (
                progress_errors
                != ['progress() requires a running asyncio event loop']
            ):
                cr['error'] = (
                    'progress() without an event loop returned an unexpected '
                    f'error: {progress_errors!r}'
                )
                return cr

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

        elif kind == 'data_stream_scalar_roundtrip':
            from datetime import datetime, timedelta, timezone
            from uuid import UUID

            stream = cls.create() if hasattr(cls, 'create') else cls.create_default()
            writer_cls = generated_type(pkg_name, 'DataWriter')
            reader_cls = generated_type(pkg_name, 'DataReader')
            writer = writer_cls.create_data_writer(stream.get_output_stream_at(0))

            guid = UUID('12345678-1234-5678-9abc-def012345678')
            timestamp = datetime(2024, 1, 2, 3, 4, 5, 6000, tzinfo=timezone.utc)
            duration = timedelta(days=1, seconds=2, microseconds=3000)
            text = 'dynwinrt'

            writer.write_byte(0xAB)
            writer.write_bytes(b'\x01\x02\x03')
            writer.write_boolean(True)
            writer.write_guid(guid)
            writer.write_int16(-1234)
            writer.write_int32(-12345678)
            writer.write_int64(-1234567890123)
            writer.write_uint16(54321)
            writer.write_uint32(3_000_000_000)
            writer.write_uint64(9_000_000_000_000_000_000)
            writer.write_single(1.25)
            writer.write_double(2.5)
            writer.write_date_time(timestamp)
            writer.write_time_span(duration)
            text_units = writer.measure_string(text)
            writer.write_string(text)
            pending = writer.unstored_buffer_length
            stored = await writer.store_async()
            flushed = await writer.flush_async()

            stream.seek(0)
            reader = reader_cls.create_data_reader(stream.get_input_stream_at(0))
            loaded = await reader.load_async(stored)
            values = {
                'byte': reader.read_byte(),
                'bytes': reader.read_bytes(bytearray(3)),
                'bool': reader.read_boolean(),
                'guid': reader.read_guid(),
                'i16': reader.read_int16(),
                'i32': reader.read_int32(),
                'i64': reader.read_int64(),
                'u16': reader.read_uint16(),
                'u32': reader.read_uint32(),
                'u64': reader.read_uint64(),
                'f32': reader.read_single(),
                'f64': reader.read_double(),
                'datetime': reader.read_date_time(),
                'duration': reader.read_time_span(),
                'text': reader.read_string(text_units),
            }
            remaining = reader.unconsumed_buffer_length
            writer_stream = writer.detach_stream()
            reader_stream = reader.detach_stream()
            writer.close()
            reader.close()

            expected = {
                'byte': 0xAB,
                'bytes': b'\x01\x02\x03',
                'bool': True,
                'guid': guid,
                'i16': -1234,
                'i32': -12345678,
                'i64': -1234567890123,
                'u16': 54321,
                'u32': 3_000_000_000,
                'u64': 9_000_000_000_000_000_000,
                'datetime': timestamp,
                'duration': duration,
                'text': text,
            }
            mismatches = {
                key: (expected[key], values[key])
                for key in expected
                if expected[key] != values[key]
            }
            if abs(values['f32'] - 1.25) > 0.0001:
                mismatches['f32'] = (1.25, values['f32'])
            if abs(values['f64'] - 2.5) > 1e-10:
                mismatches['f64'] = (2.5, values['f64'])
            if (
                pending <= 0
                or stored != pending
                or loaded != stored
                or not flushed
                or remaining != 0
                or writer_stream is None
                or reader_stream is None
                or mismatches
            ):
                cr['error'] = (
                    f'data stream roundtrip failed: pending={pending}, '
                    f'stored={stored}, loaded={loaded}, flushed={flushed}, '
                    f'remaining={remaining}, mismatches={mismatches!r}'
                )
            else:
                cr['pass'] = True

        elif kind == 'calendar_comprehensive':
            obj.year = 2024
            obj.month = 1
            obj.day = 2
            obj.hour = 3
            obj.minute = 4
            obj.second = 5
            obj.nanosecond = 0
            numeric_properties = [
                'first_era',
                'last_era',
                'number_of_eras',
                'era',
                'first_year_in_this_era',
                'last_year_in_this_era',
                'number_of_years_in_this_era',
                'first_month_in_this_year',
                'last_month_in_this_year',
                'number_of_months_in_this_year',
                'first_day_in_this_month',
                'last_day_in_this_month',
                'number_of_days_in_this_month',
                'first_period_in_this_day',
                'last_period_in_this_day',
                'number_of_periods_in_this_day',
                'first_hour_in_this_period',
                'last_hour_in_this_period',
                'number_of_hours_in_this_period',
                'first_minute_in_this_hour',
                'last_minute_in_this_hour',
                'number_of_minutes_in_this_hour',
                'first_second_in_this_minute',
                'last_second_in_this_minute',
                'number_of_seconds_in_this_minute',
                'nanosecond',
            ]
            if not all(isinstance(getattr(obj, name), int) for name in numeric_properties):
                cr['error'] = 'Calendar numeric metadata returned a non-integer'
                return cr
            if not isinstance(obj.is_daylight_saving_time, bool):
                cr['error'] = 'Calendar daylight-saving property was not bool'
                return cr

            original = obj.get_date_time()
            clone = obj.clone()
            if clone is None or obj.compare(clone) != 0 or obj.compare_date_time(original) != 0:
                cr['error'] = 'Calendar clone or comparison failed'
                return cr

            for method, amount in [
                ('add_years', 1),
                ('add_months', 1),
                ('add_weeks', 1),
                ('add_days', 1),
                ('add_hours', 1),
                ('add_minutes', 1),
                ('add_seconds', 1),
                ('add_nanoseconds', 10_000),
            ]:
                before = obj.get_date_time()
                getattr(obj, method)(amount)
                if obj.get_date_time() == before:
                    cr['error'] = f'Calendar {method} did not change the value'
                    return cr
                getattr(obj, method)(-amount)
                if obj.get_date_time() != before:
                    cr['error'] = f'Calendar {method} did not round-trip'
                    return cr
            if obj.compare_date_time(original) != 0:
                cr['error'] = 'Calendar arithmetic changed the original value'
                return cr

            other = obj.clone()
            obj.add_days(1)
            obj.copy_to(other)
            if obj.compare(other) != 0:
                cr['error'] = 'Calendar copy_to did not copy the current value'
                return cr
            obj.set_date_time(original)

            calendar_system = obj.get_calendar_system()
            clock = obj.get_clock()
            time_zone = obj.get_time_zone()
            numeral_system = obj.numeral_system
            obj.change_calendar_system(calendar_system)
            obj.change_clock(clock)
            obj.change_time_zone(time_zone)
            obj.numeral_system = numeral_system

            string_calls = [
                ('era_as_full_string', ()),
                ('era_as_string', (3,)),
                ('year_as_string', ()),
                ('year_as_truncated_string', (2,)),
                ('year_as_padded_string', (4,)),
                ('month_as_full_string', ()),
                ('month_as_string', (3,)),
                ('month_as_full_solo_string', ()),
                ('month_as_solo_string', (3,)),
                ('month_as_numeric_string', ()),
                ('month_as_padded_numeric_string', (2,)),
                ('day_as_string', ()),
                ('day_as_padded_string', (2,)),
                ('day_of_week_as_full_string', ()),
                ('day_of_week_as_string', (3,)),
                ('day_of_week_as_full_solo_string', ()),
                ('day_of_week_as_solo_string', (3,)),
                ('period_as_full_string', ()),
                ('period_as_string', (2,)),
                ('hour_as_string', ()),
                ('hour_as_padded_string', (2,)),
                ('minute_as_string', ()),
                ('minute_as_padded_string', (2,)),
                ('second_as_string', ()),
                ('second_as_padded_string', (2,)),
                ('nanosecond_as_string', ()),
                ('nanosecond_as_padded_string', (3,)),
                ('time_zone_as_full_string', ()),
                ('time_zone_as_string', (3,)),
            ]
            formatted = [
                getattr(obj, method)(*args)
                for method, args in string_calls
            ]
            if not all(isinstance(value, str) for value in formatted):
                cr['error'] = 'Calendar formatting returned a non-string'
                return cr

            minimum = obj.clone()
            maximum = obj.clone()
            minimum.set_to_min()
            maximum.set_to_max()
            if minimum.compare(maximum) >= 0:
                cr['error'] = 'Calendar min/max ordering was invalid'
                return cr
            obj.set_to_now()
            if not isinstance(obj.resolved_language, str):
                cr['error'] = 'Calendar resolved_language was not a string'
            else:
                cr['pass'] = True

        elif kind == 'storage_query_temp_folder':
            from pathlib import Path
            from tempfile import TemporaryDirectory

            with TemporaryDirectory(prefix='dynwinrt-query-') as temp_dir:
                root = Path(temp_dir)
                (root / 'alpha.txt').write_text('alpha', encoding='utf-8')
                (root / 'beta.txt').write_text('beta', encoding='utf-8')

                folder = await cls.get_folder_from_path_async(str(root))
                if folder is None or not Path(folder.path).samefile(root):
                    cr['error'] = (
                        'StorageFolder path lookup failed: '
                        f'expected={str(root)!r}, actual={getattr(folder, "path", None)!r}'
                    )
                    return cr

                direct_files = await (
                    folder.get_files_async_overload_default_options_start_and_count()
                )
                query = folder.create_file_query_overload_default()
                if query is None:
                    cr['error'] = 'StorageFolder.create_file_query returned null'
                    return cr
                count = await query.get_item_count_async()
                query_files = await query.get_files_async_default_start_and_count()
                options = query.get_current_query_options()
                query_folder = query.folder
                missing = await folder.try_get_item_async('missing.file')
                alpha = await folder.get_file_async('alpha.txt')

                if options is not None:
                    query.apply_new_query_options(options)

                direct_names = sorted(file.name for file in direct_files or [])
                query_names = sorted(file.name for file in query_files or [])
                if (
                    direct_names != ['alpha.txt', 'beta.txt']
                    or query_names != direct_names
                    or count != 2
                    or query_folder is None
                    or not query_folder.is_equal(folder)
                    or missing is not None
                    or alpha is None
                    or alpha.name != 'alpha.txt'
                    or not alpha.is_of_type(
                        generated_type(pkg_name, 'StorageItemTypes').File
                    )
                ):
                    cr['error'] = (
                        f'Storage query failed: direct={direct_names!r}, '
                        f'query={query_names!r}, count={count}, '
                        f'missing={missing!r}, alpha={alpha!r}'
                    )
                else:
                    cr['pass'] = True

        elif kind == 'async_cancellation':
            import dynwinrt as dw

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
            try:
                operation.wait()
                task.cancel()
                release.set()
                cr['error'] = 'wait() was not rejected after awaiting started'
                return cr
            except RuntimeError as error:
                if 'after awaiting has started' not in str(error):
                    task.cancel()
                    release.set()
                    cr['error'] = f'unexpected started wait() error: {error}'
                    return cr
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

        elif kind == 'device_information_async_collection':
            devices = await getattr(cls, member)()
            if devices is None or not isinstance(devices.size, int):
                cr['error'] = (
                    'DeviceInformation.find_all_async() did not return '
                    'a collection with an integer size'
                )
            else:
                cr['pass'] = True

        elif kind == 'bitmap_encoder_async_create':
            stream_cls = generated_type(pkg_name, 'InMemoryRandomAccessStream')
            stream = (
                stream_cls.create()
                if hasattr(stream_cls, 'create')
                else stream_cls.create_default()
            )
            encoder_id = (
                cls.get_jpeg_encoder_id()
                if hasattr(cls, 'get_jpeg_encoder_id')
                else cls.jpeg_encoder_id
            )
            encoder = await getattr(cls, member)(encoder_id, stream)
            if encoder is None:
                cr['error'] = 'BitmapEncoder.create_async() returned null'
            else:
                cr['pass'] = True

        elif kind == 'nested_struct_runtime':
            from typing import get_type_hints

            module = importlib.import_module(
                implementation_module_name(
                    pkg_name, namespace, 'Direct3DSurfaceDescription'
                )
            )
            descriptor_type = getattr(module, 'Direct3DSurfaceDescription')
            nested_type = getattr(module, 'Direct3DMultisampleDescription')
            pixel_format = generated_type(pkg_name, 'DirectXPixelFormat')
            pack = getattr(module, 'pack_direct3_d_surface_description')
            unpack = getattr(module, 'unpack_direct3_d_surface_description')
            constructor_hints = get_type_hints(
                descriptor_type.__init__,
                globalns={
                    **vars(module),
                    'DirectXPixelFormat': pixel_format,
                },
            )
            if (
                constructor_hints.get('multisample_description')
                != nested_type | None
            ):
                cr['error'] = (
                    'nested struct constructor annotation did not resolve: '
                    f'{constructor_hints!r}'
                )
                return cr

            first = descriptor_type()
            second = descriptor_type()
            if (
                not isinstance(first.multisample_description, nested_type)
                or first.multisample_description is second.multisample_description
                or not isinstance(first.format, pixel_format)
            ):
                cr['error'] = 'nested struct defaults were not Python-native'
                return cr

            first.multisample_description.count = 4
            first.multisample_description.quality = 7
            first.format = pixel_format.R32G32B32A32Typeless
            roundtrip = unpack(pack(first).to_value())
            if (
                roundtrip.multisample_description.count != 4
                or roundtrip.multisample_description.quality != 7
                or roundtrip.format != pixel_format.R32G32B32A32Typeless
            ):
                cr['error'] = 'nested struct or enum did not round-trip'
            else:
                cr['pass'] = True

        elif kind == 'generated_helper_matrix':
            from pathlib import Path

            import dynwinrt as dw

            # This spec runs last. Earlier E2E specs validate real generated
            # call sites; this matrix covers the shared runtime once and each
            # module-local helper shape that remains after runtime extraction.
            property_type = generated_type(pkg_name, 'PropertyType')
            property_type_module = implementation_module_name(
                pkg_name, 'Windows.Foundation', 'PropertyType'
            )
            valid_enum = next(iter(property_type))
            invalid_enum = 2_000_000_000
            uri_type = generated_type(pkg_name, 'Uri')
            uri = uri_type(
                'https://example.com:8443/'
                'path/file.txt?name=value#fragment'
            )
            uri_module = implementation_module_name(
                pkg_name, 'Windows.Foundation', 'Uri'
            )
            delegate_name = (
                'TypedEventHandler_IMemoryBufferReference_Object'
            )
            delegate_module = importlib.import_module(
                implementation_module_name(
                    pkg_name,
                    'Windows.Foundation',
                    delegate_name,
                )
            )
            delegate_iid = getattr(
                delegate_module,
                f'IID_{delegate_name}',
            )
            delegate_params = getattr(
                delegate_module,
                f'{delegate_name}_PARAM_TYPES',
            )
            reference_type = generated_type(pkg_name, 'IReference_UInt32')
            value_type = dw.DynWinRTType.u32_type()
            runtime_module = importlib.import_module(f'{pkg_name}._runtime')
            module_paths = [
                path
                for path in sorted(Path(generated_dir).glob('*.py'))
                if path.name not in ('__init__.py', '_runtime.py')
            ]
            reference_modules = []
            shared_definitions = (
                'def _dynwinrt_enum(',
                'def _dynwinrt_delegate(',
                'def _dynwinrt_wrap_values(',
            )
            for module_path in module_paths:
                source = module_path.read_text(encoding='utf-8')
                if any(definition in source for definition in shared_definitions):
                    cr['error'] = (
                        f'{module_path.name}: shared runtime helper was duplicated'
                    )
                    return cr
                if 'def _dynwinrt_box_reference(' in source:
                    reference_modules.append(module_path)

            counters = {
                'modules': len(module_paths),
                'enum': 0,
                'delegate': 0,
                'wrap_values': 0,
                'ireference': 0,
                'struct_helpers': 0,
                'projection': 0,
            }

            uri_impl = importlib.import_module(uri_module)
            projected_uri = dw.project_as(uri._obj, uri_type)
            can_cast = runtime_module._dynwinrt_can_cast
            uri_iid = getattr(uri_impl, 'IID_IUriRuntimeClass')
            unsupported_iid = dw.WinGUID.parse(
                '11111111-1111-1111-1111-111111111111'
            )
            if (
                not can_cast(uri, uri_iid)
                or can_cast(object(), uri_iid)
                or can_cast(uri, unsupported_iid)
            ):
                cr['error'] = 'RuntimeClass overload QI guard failed'
                return cr
            query = uri.query_parsed
            if query is None:
                cr['error'] = 'Uri query projection returned None'
                return cr
            decoder_impl = importlib.import_module(
                implementation_module_name(
                    pkg_name,
                    'Windows.Foundation',
                    'WwwFormUrlDecoder',
                )
            )
            entry = query[0]
            entry_index = query.index_of(entry)
            entries = query.get_many(0, [entry])
            iterator = query.first()
            iterated_entry = next(iterator) if iterator is not None else None
            view = query.as_interface(
                getattr(
                    decoder_impl,
                    'IVectorView_IWwwFormUrlDecoderEntry',
                )
            )
            view_entry = view.get_at(0)
            view_index = view.index_of(entry)
            view_entries = view.get_many(0, [entry])
            iterable = query.as_interface(
                getattr(
                    decoder_impl,
                    'IIterable_IWwwFormUrlDecoderEntry',
                )
            )
            iterable_iterator = iterable.first()
            iterable_entry = (
                next(iterable_iterator)
                if iterable_iterator is not None
                else None
            )
            combined = uri.combine_uri('child.txt')
            canonical_type = getattr(
                uri_impl,
                'IUriRuntimeClassWithAbsoluteCanonicalUri',
            )
            try:
                dw.project_as(uri._obj, canonical_type)
                cr['error'] = 'project_as accepted an embedded interface'
                return cr
            except TypeError:
                pass
            canonical = canonical_type.from_value(uri._obj)
            stringable = uri.as_interface(getattr(uri_impl, 'IStringable'))
            uri_strings = (
                uri.absolute_uri,
                uri.display_uri,
                uri.domain,
                uri.extension,
                uri.fragment,
                uri.host,
                uri.password,
                uri.path,
                uri.query,
                uri.raw_uri,
                uri.scheme_name,
                uri.user_name,
                uri.absolute_canonical_uri,
                uri.display_iri,
                uri.to_string(),
                str(uri),
                repr(uri),
                canonical.absolute_canonical_uri,
                canonical.display_iri,
                stringable.to_string(),
            )
            if (
                projected_uri is not uri
                or len(query) != 1
                or query.get_first_value_by_name('name') != 'value'
                or entry.name != 'name'
                or entry.value != 'value'
                or entry_index != (0, True)
                or len(entries) != 1
                or iterated_entry is None
                or iterated_entry.value != 'value'
                or view.size != 1
                or view_entry is None
                or view_entry.name != 'name'
                or view_index != (0, True)
                or len(view_entries) != 1
                or iterable_entry is None
                or iterable_entry.value != 'value'
                or combined is None
                or not combined.path.endswith('/path/child.txt')
                or not uri.equals(uri)
                or uri.port != 8443
                or uri.suspicious
                or not all(isinstance(value, str) for value in uri_strings)
            ):
                cr['error'] = 'generated projection helper matrix failed'
                return cr
            counters['projection'] = 1

            enum_helper = runtime_module._dynwinrt_enum
            converted = enum_helper(
                property_type_module.removeprefix(f'{pkg_name}.'),
                'PropertyType',
                int(valid_enum),
            )
            unknown = enum_helper(
                property_type_module.removeprefix(f'{pkg_name}.'),
                'PropertyType',
                invalid_enum,
            )
            if not isinstance(converted, property_type):
                cr['error'] = 'shared runtime did not project a valid enum'
                return cr
            if type(unknown) is not int or unknown != invalid_enum:
                cr['error'] = 'shared runtime did not preserve an unknown enum'
                return cr
            counters['enum'] = 1

            delegate_helper = runtime_module._dynwinrt_delegate
            raw = dw.DynWinRTValue.null_value()
            if delegate_helper(raw, delegate_iid, delegate_params) is not raw:
                cr['error'] = 'shared runtime did not preserve a raw delegate'
                return cr
            try:
                delegate_helper(17, delegate_iid, delegate_params)
                cr['error'] = 'shared runtime accepted an invalid delegate'
                return cr
            except TypeError:
                pass
            callback_value = delegate_helper(
                lambda *_args: None,
                delegate_iid,
                delegate_params,
            )
            if not isinstance(callback_value, dw.DynWinRTValue):
                cr['error'] = 'shared runtime did not wrap a callable delegate'
                return cr
            callback_value.release()
            counters['delegate'] = 1

            wrapped = runtime_module._dynwinrt_wrap_values(
                uri_module.removeprefix(f'{pkg_name}.'),
                'Uri',
                [dw.DynWinRTValue.null_value(), uri._obj],
            )
            if wrapped[0] is not None or not isinstance(wrapped[1], uri_type):
                cr['error'] = 'shared runtime value wrapping branches failed'
                return cr
            if wrapped[1] is not uri:
                cr['error'] = 'shared runtime did not reuse wrapper identity'
                return cr
            counters['wrap_values'] = 1

            for namespace, struct_name, values in (
                (
                    'Windows.Data.Text',
                    'TextSegment',
                    {'start_position': 3, 'length': 5},
                ),
                (
                    'Windows.Foundation',
                    'EventRegistrationToken',
                    {'value': 9},
                ),
            ):
                struct_module = importlib.import_module(
                    f'{namespace_module_name(pkg_name, namespace)}.'
                    f'{to_snake_case(struct_name)}'
                )
                struct_type = getattr(struct_module, struct_name)
                pack = getattr(
                    struct_module,
                    f'pack_{to_snake_case(struct_name)}',
                )
                unpack = getattr(
                    struct_module,
                    f'unpack_{to_snake_case(struct_name)}',
                )
                value = struct_type(**values)
                roundtrip = unpack(pack(value).to_value())
                if roundtrip != value or struct_name not in repr(roundtrip):
                    cr['error'] = (
                        f'{struct_name}: canonical struct helper roundtrip failed'
                    )
                    return cr
                if roundtrip.__eq__(object()) is not NotImplemented:
                    cr['error'] = (
                        f'{struct_name}: struct equality accepted another type'
                    )
                    return cr
                counters['struct_helpers'] += 1

            for module_path in reference_modules:
                generated_module = importlib.import_module(
                    f'{pkg_name}.{module_path.stem}'
                )
                box_reference = getattr(
                    generated_module, '_dynwinrt_box_reference', None
                )
                unbox_reference = getattr(
                    generated_module, '_dynwinrt_unbox_reference', None
                )
                if box_reference is not None and unbox_reference is not None:
                    raw = dw.DynWinRTValue.null_value()
                    if (
                        box_reference(
                            raw,
                            value_type,
                            dw.DynWinRTValue.from_u32,
                        )
                        is not raw
                    ):
                        cr['error'] = (
                            f'{module_path.name}: raw IReference was not preserved'
                        )
                        return cr
                    if not box_reference(
                        None,
                        value_type,
                        dw.DynWinRTValue.from_u32,
                    ).is_null():
                        cr['error'] = (
                            f'{module_path.name}: None IReference was not null'
                        )
                        return cr
                    boxed = box_reference(
                        17,
                        value_type,
                        dw.DynWinRTValue.from_u32,
                    )
                    reference = reference_type.from_value(boxed)
                    if unbox_reference(reference) != 17:
                        cr['error'] = (
                            f'{module_path.name}: IReference did not unbox'
                        )
                        return cr
                    if unbox_reference(23) != 23:
                        cr['error'] = (
                            f'{module_path.name}: native optional value changed'
                        )
                        return cr
                    counters['ireference'] += 1

            uri._obj.release()
            if (
                counters['modules'] < 100
                or counters['enum'] != 1
                or counters['delegate'] != 1
                or counters['wrap_values'] != 1
                or counters['ireference'] < 1
                or counters['struct_helpers'] != 2
                or counters['projection'] != 1
            ):
                cr['error'] = f'generated helper matrix was too small: {counters}'
            else:
                print(f'  generated helper matrix: {counters}')
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
    import dynwinrt as dw
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
