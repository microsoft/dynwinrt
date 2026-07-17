# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""
E2E test runner for Python generated bindings.

Reads e2e_specs.json, imports generated Python modules,
and executes checks against real WinRT APIs.

Usage:
    python tests/runners/py_runner.py --specs tests/e2e_specs.json --generated tests/e2e_generated/py --output results.json
"""

import argparse
import importlib
import json
import re
import sys
import os


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


def run_spec(spec: dict, generated_dir: str, pkg_name: str) -> dict:
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
        # kind == 'none': no instantiation

        # Run checks
        for check in spec.get('checks', []):
            check_result = run_check(check, cls, obj, generated_dir, pkg_name)
            result['checks'].append(check_result)
            if not check_result['pass']:
                result['pass'] = False

    except Exception as e:
        result['pass'] = False
        result['error'] = str(e)

    return result


def run_check(check: dict, cls, obj, generated_dir: str, pkg_name: str) -> dict:
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
            except Exception:
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
            write_val = check.get('write_value', 42)
            stream = cls.create() if hasattr(cls, 'create') else cls.create_default()

            writer_mod = importlib.import_module(f"{pkg_name}.data_writer")
            reader_mod = importlib.import_module(f"{pkg_name}.data_reader")
            writer_cls = getattr(writer_mod, 'DataWriter')
            reader_cls = getattr(reader_mod, 'DataReader')

            writer = writer_cls.create_data_writer(stream.get_output_stream_at(0))
            writer.write_int32(write_val)
            stored = writer.store_async()

            stream.seek(0)
            reader = reader_cls.create_data_reader(stream.get_input_stream_at(0))
            loaded = reader.load_async(4)
            read_val = reader.read_int32()

            if stored < 4 or loaded < 4 or read_val != write_val:
                cr['error'] = (
                    f'async roundtrip failed: stored={stored}, loaded={loaded}, '
                    f'wrote {write_val}, read {read_val}'
                )
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

    for spec in specs:
        r = run_spec(spec, args.generated, gen_pkg)
        results.append(r)
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
