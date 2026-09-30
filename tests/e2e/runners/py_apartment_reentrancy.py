# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Isolated, real PropertySet MapChanged/apartment lifetime regressions."""

import argparse
import os
import sys
import threading


def blocked_close(action):
    try:
        action()
    except RuntimeError as error:
        message = str(error)
        assert 'synchronous native callback' in message, message
        assert 'retry after the callback' in message, message
    else:
        raise AssertionError('final apartment uninitialization succeeded in a native callback')


def run_case(generated, mode, scenario):
    sys.path.insert(0, os.path.dirname(os.path.abspath(generated)))
    import dynwinrt as dw
    from python_bindings import PropertySet

    def switch_models():
        with dw.RoApartment(1 - mode) as opposite:
            assert 'active=true' in repr(opposite)

    def dispose(mapping, token):
        mapping.off_map_changed(token)
        dw.release_projected(mapping)

    if scenario in ('close', 'exit'):
        apartment = dw.RoApartment(mode)
        apartment.__enter__()
        events = []
        retained = []

        def on_changed(sender, args):
            events.append(args.key)
            if not retained:
                retained.extend((sender, args))
            assert sender[args.key] is None
            if scenario == 'close':
                blocked_close(apartment.close)
            else:
                blocked_close(lambda: apartment.__exit__(None, None, None))
            assert 'active=true' in repr(apartment)
            assert args.key in sender

        mapping = PropertySet()
        token = mapping.on_map_changed(on_changed)
        mapping.insert('first', None)
        mapping.insert('second', None)
        assert events == ['first', 'second'], events
        sender, args = retained
        assert args.key == 'first' and sender['first'] is None
        dispose(mapping, token)
        dw.release_projected(args)
        dw.release_projected(sender)
        if scenario == 'close':
            apartment.close()
            apartment.close()
        else:
            assert apartment.__exit__(None, None, None) is False
        assert 'active=false' in repr(apartment)
        switch_models()

    elif scenario == 'nested':
        outer = dw.RoApartment(mode)
        inner = dw.RoApartment(mode)
        outer.__enter__()
        inner.__enter__()
        events = []

        def on_changed(sender, args):
            events.append(args.key)
            inner.close()
            assert 'active=false' in repr(inner)
            blocked_close(outer.close)
            assert 'active=true' in repr(outer)
            assert sender[args.key] is None

        mapping = PropertySet()
        token = mapping.on_map_changed(on_changed)
        mapping.insert('nested', None)
        assert events == ['nested'], events
        dispose(mapping, token)
        outer.close()
        switch_models()

    elif scenario == 'manual':
        dw.ro_initialize(mode)
        dw.ro_initialize(mode)
        events = []

        def on_changed(sender, args):
            events.append(args.key)
            dw.ro_uninitialize()
            blocked_close(dw.ro_uninitialize)
            assert sender[args.key] is None

        mapping = PropertySet()
        token = mapping.on_map_changed(on_changed)
        mapping.insert('manual', None)
        assert events == ['manual'], events
        dispose(mapping, token)
        dw.ro_uninitialize()
        switch_models()

    elif scenario == 'pending':
        outer = dw.RoApartment(mode)
        outer.__enter__()
        events = []

        def on_changed(sender, args):
            events.append(args.key)

            def unnamed_context():
                with dw.RoApartment(mode):
                    outer.close()

            blocked_close(unnamed_context)
            assert 'active=false' in repr(outer)
            try:
                dw.RoApartment.recover_pending()
            except RuntimeError as error:
                assert 'after' in str(error), error
            else:
                raise AssertionError('pending apartment recovered inside its native callback')
            assert sender[args.key] is None

        mapping = PropertySet()
        token = mapping.on_map_changed(on_changed)
        mapping.insert('pending', None)
        assert events == ['pending'], events

        wrong_thread = []

        def recover_on_wrong_thread():
            try:
                dw.RoApartment.recover_pending()
            except RuntimeError as error:
                wrong_thread.append(str(error))

        thread = threading.Thread(target=recover_on_wrong_thread)
        thread.start()
        thread.join(timeout=5)
        assert not thread.is_alive() and len(wrong_thread) == 1, wrong_thread
        assert 'on this thread' in wrong_thread[0], wrong_thread

        recovered = dw.RoApartment.recover_pending()
        assert 'active=true' in repr(recovered)
        try:
            dw.RoApartment.recover_pending()
        except RuntimeError as error:
            assert 'no dropped RoApartment' in str(error), error
        else:
            raise AssertionError('recovered the same pending apartment twice')
        dispose(mapping, token)
        recovered.close()
        switch_models()

    elif scenario == 'drop':
        outer = dw.RoApartment(mode)
        outer.__enter__()
        inner = dw.RoApartment(mode)
        inner.__enter__()
        unraisable = []
        original_hook = sys.unraisablehook
        sys.unraisablehook = unraisable.append

        def on_changed(sender, args):
            outer.close()
            inner_holder.pop()
            assert sender[args.key] is None

        inner_holder = [inner]
        del inner
        mapping = PropertySet()
        token = mapping.on_map_changed(on_changed)
        try:
            mapping.insert('dropped', None)
        finally:
            sys.unraisablehook = original_hook
        assert len(unraisable) == 1, unraisable
        assert 'recover_pending()' in str(unraisable[0].exc_value)
        unraisable.clear()
        recovered = dw.RoApartment.recover_pending()
        dispose(mapping, token)
        recovered.close()
        switch_models()

    elif scenario == 'pending_error':
        outer = dw.RoApartment(mode)
        outer.__enter__()
        unraisable = []
        original_hook = sys.unraisablehook
        sys.unraisablehook = unraisable.append

        def on_changed(_sender, _args):
            with dw.RoApartment(mode):
                outer.close()

        mapping = PropertySet()
        token = mapping.on_map_changed(on_changed)
        try:
            try:
                mapping.insert('pending-error', None)
            except OSError:
                pass
        finally:
            sys.unraisablehook = original_hook
        assert len(unraisable) == 1, unraisable
        assert isinstance(unraisable[0].exc_value, RuntimeError)
        assert 'synchronous native callback' in str(unraisable[0].exc_value)
        unraisable.clear()
        assert 'active=false' in repr(outer)
        recovered = dw.RoApartment.recover_pending()
        dispose(mapping, token)
        recovered.close()
        switch_models()

    elif scenario == 'exception':
        apartment = dw.RoApartment(mode)
        apartment.__enter__()
        unraisable = []
        original_hook = sys.unraisablehook
        sys.unraisablehook = unraisable.append

        def on_changed(_sender, _args):
            blocked_close(apartment.close)
            raise ValueError('handler failed after blocked close')

        mapping = PropertySet()
        token = mapping.on_map_changed(on_changed)
        try:
            try:
                mapping.insert('failed', None)
            except OSError:
                pass
        finally:
            sys.unraisablehook = original_hook
        assert len(unraisable) == 1, unraisable
        assert isinstance(unraisable[0].exc_value, ValueError)
        assert 'handler failed after blocked close' in str(unraisable[0].exc_value)
        mapping.off_map_changed(token)
        unraisable.clear()
        events = []
        token = mapping.on_map_changed(lambda _sender, args: events.append(args.key))
        mapping.insert('after', None)
        assert events == ['after'], events
        dispose(mapping, token)
        apartment.close()
        switch_models()

    else:
        raise AssertionError(f'unknown apartment scenario: {scenario}')

    print(f'PASS {scenario} apartment={mode}', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--generated', required=True)
    parser.add_argument('--mode', type=int, choices=[0, 1], required=True)
    parser.add_argument(
        '--scenario',
        choices=[
            'close', 'exit', 'nested', 'manual', 'pending', 'drop',
            'pending_error', 'exception',
        ],
        required=True,
    )
    args = parser.parse_args()
    run_case(args.generated, args.mode, args.scenario)
