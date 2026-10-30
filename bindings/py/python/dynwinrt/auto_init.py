# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Opt in to identity-aware Windows App SDK Bootstrap initialization.

Import on the application's chosen COM thread after preparing its apartment.
Packaged processes skip Bootstrap, without checking Runtime deployment. Otherwise
the existing no-argument initializer discovers an installed generated SDK bundle.
Normal Python import caching applies; this module exposes no context or shutdown API.
"""

from . import has_package_identity as _has_package_identity

__all__: list[str] = []

if not _has_package_identity():
    from . import init_winappsdk as _init_winappsdk

    _init_winappsdk()
