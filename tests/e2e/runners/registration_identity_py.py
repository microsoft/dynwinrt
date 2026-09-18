# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import importlib
import json
from pathlib import Path
import sys

import dynwinrt
from dynwinrt import RoApartment, projected_lifetime_scope


def check(name):
    package = importlib.import_module(f"{name}_py")

    class Handlers:
        added = 0
        removed = 0
        retained = None

        def add_property_changed(self, callback):
            assert callable(callback)
            self.retained = callback
            self.added += 1
            return package.EventRegistrationToken(value=1)

        def remove_property_changed(self, token):
            assert token.value == 1
            self.retained._obj.release()
            self.retained = None
            self.removed += 1

    handlers = Handlers()
    with projected_lifetime_scope():
        owner = package.INotifyPropertyChanged.implement(handlers)
        try:
            unsubscribe = owner.value.subscribe_property_changed(lambda *_: None)
            unsubscribe()
            assert handlers.added == handlers.removed == 1
            assert owner.take_error() is None
            return {"name": name, "added": handlers.added, "removed": handlers.removed}
        finally:
            if handlers.retained is not None:
                handlers.retained._obj.release()
            owner.dispose()


if __name__ == "__main__":
    sys.path.insert(0, str(Path(sys.argv[1]).resolve()))
    with RoApartment(1):
        results = [check(name) for name in sys.argv[2].split(",")]
    print(json.dumps({
        "runtime": dynwinrt.__file__,
        "native": importlib.import_module("dynwinrt.dynwinrt").__file__,
        "results": results,
    }))
