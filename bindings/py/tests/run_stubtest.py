# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Run recursive stubtest with the opt-in module's import precondition prepared.

Only its initial import uses a simulated packaged process. The real public
identity probe is restored before stubtest inspects every module and signature.
Actual startup behavior is covered separately by test_winappsdk.py.
"""

from importlib import import_module
import runpy
from unittest.mock import patch

import dynwinrt


if __name__ == "__main__":
    with patch.object(dynwinrt, "has_package_identity", return_value=True):
        import_module("dynwinrt.auto_init")
    runpy.run_module("mypy.stubtest", run_name="__main__")
