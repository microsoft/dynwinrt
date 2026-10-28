# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Configuration discovery for the explicit Windows App SDK initializer."""

from dataclasses import dataclass
import hashlib
from importlib import import_module as _import_module
from importlib.metadata import entry_points
import os
from pathlib import Path
import re
import sysconfig

from dynwinrt import WinAppSDKContext

_native = _import_module("dynwinrt.dynwinrt")

_ENVIRONMENT = "WINAPPSDK_BOOTSTRAP_DLL_PATH"
_GROUP = "dynwinrt.winappsdk"
_DLL = "Microsoft.WindowsAppRuntime.Bootstrap.dll"
_MISSING = object()


@dataclass(frozen=True)
class _Configuration:
    source: str
    root: Path
    major: int
    minor: int
    digests: tuple[tuple[str, str], ...]

    @property
    def identity(self) -> tuple[int, int, tuple[tuple[str, str], ...]]:
        return self.major, self.minor, self.digests


def _configuration(source: str, value: object) -> _Configuration:
    def invalid(reason: str) -> ValueError:
        return ValueError(f"Invalid Windows App SDK configuration from {source}: {reason}")

    if not isinstance(value, tuple) or len(value) != 2:
        raise invalid("provider must return (package_directory, configuration)")
    root, config = value
    if not isinstance(root, str) or not Path(root).is_absolute():
        raise invalid("package_directory must be an absolute path")
    if not isinstance(config, dict) or set(config) != {"schema_version", "major", "minor", "dlls"}:
        raise invalid("unexpected configuration fields")
    if type(config["schema_version"]) is not int or config["schema_version"] != 1:
        raise invalid("unsupported schema_version")
    for name in ("major", "minor"):
        number = config[name]
        if type(number) is not int or not 0 <= number <= 65535:
            raise invalid(f"{name} must be an integer between 0 and 65535")
    dlls = config["dlls"]
    if not isinstance(dlls, dict) or not dlls or not set(dlls) <= {"x64", "arm64"}:
        raise invalid("x64 and/or arm64 DLL digests are required")
    digests = []
    for architecture in sorted(dlls):
        digest = dlls[architecture]
        if not isinstance(digest, str) or not re.fullmatch("[0-9a-f]{64}", digest):
            raise invalid(f"invalid {architecture} SHA-256 digest")
        digests.append((architecture, digest))
    return _Configuration(
        source, Path(root), config["major"], config["minor"], tuple(digests)
    )


def _discover() -> _Configuration:
    configurations = []
    for provider in sorted(entry_points(group=_GROUP), key=lambda item: (item.name, item.value)):
        distribution = provider.dist
        source = (
            f"{distribution.metadata['Name']}=={distribution.version} ({provider.value})"
            if distribution is not None
            else f"{provider.name} ({provider.value})"
        )
        get_config = provider.load()
        if not callable(get_config):
            raise TypeError(f"Windows App SDK configuration provider {source} is not callable")
        configurations.append(_configuration(source, get_config()))
    if not configurations:
        raise RuntimeError(
            "No generated Windows App SDK configuration is installed. Install the generated "
            "bindings package (ordinary or editable), or pass major, minor and "
            "bootstrap_dll_path explicitly to init_winappsdk()."
        )
    first = configurations[0]
    if any(item.identity != first.identity for item in configurations[1:]):
        sources = "; ".join(
            f"{item.source}: {item.major}.{item.minor}" for item in configurations
        )
        raise RuntimeError(
            f"Conflicting generated Windows App SDK configurations: {sources}. "
            "Use one consistent SDK bundle or pass explicit initialization arguments."
        )
    return first


def _explicit_path(value: str | os.PathLike[str]) -> str:
    path = os.fspath(value)
    if not isinstance(path, str) or not path.strip() or "\0" in path:
        raise ValueError("Windows App SDK bootstrap_dll_path must be a nonempty path without NUL")
    resolved = Path(path).resolve()
    if not resolved.is_file():
        raise FileNotFoundError(f"Windows App SDK bootstrap DLL not found: {resolved}")
    return str(resolved)


def _resource_path(config: _Configuration) -> str:
    platform = sysconfig.get_platform().lower().replace("_", "-")
    architecture = {"win-amd64": "x64", "win-arm64": "arm64"}.get(platform)
    if architecture is None:
        raise RuntimeError(
            f"Unsupported Python platform {platform!r} for Windows App SDK; "
            "use Windows x64 or ARM64 Python."
        )
    expected = dict(config.digests).get(architecture)
    if expected is None:
        raise RuntimeError(
            f"Windows App SDK configuration from {config.source} has no {architecture} DLL. "
            "Install or generate bindings for the running interpreter architecture."
        )
    path = config.root / "_native" / "winappsdk" / architecture / _DLL
    try:
        data = path.read_bytes()
    except FileNotFoundError as error:
        raise FileNotFoundError(
            f"Generated Windows App SDK bootstrap DLL is missing: {path} ({config.source}). "
            "Reinstall the complete generated bindings package."
        ) from error
    digest = hashlib.sha256(data).hexdigest()
    if digest != expected:
        raise ValueError(
            f"Windows App SDK bootstrap DLL digest mismatch: {path} ({config.source}). "
            "Reinstall the complete generated bindings package."
        )
    return str(path.resolve())


def init_winappsdk(
    major=_MISSING,
    minor=_MISSING,
    *,
    bootstrap_dll_path: str | os.PathLike[str] | None = None,
) -> WinAppSDKContext:
    """Initialize explicitly, or discover a consistent installed generated SDK bundle.

    This does not initialize COM, install the Runtime, or unload it on return.
    Existing two-version-argument calls keep their native environment semantics.
    """
    if major is not _MISSING or minor is not _MISSING:
        if major is _MISSING or minor is _MISSING:
            raise TypeError("init_winappsdk() requires both major and minor, or neither")
        if bootstrap_dll_path is None:
            return _native.init_winappsdk(major, minor)
        return _native.init_winappsdk(
            major, minor, bootstrap_dll_path=_explicit_path(bootstrap_dll_path)
        )

    config = _discover()
    override = (
        bootstrap_dll_path
        if bootstrap_dll_path is not None
        else os.environ.get(_ENVIRONMENT)
    )
    path = _explicit_path(override) if override is not None else _resource_path(config)
    return _native.init_winappsdk(config.major, config.minor, bootstrap_dll_path=path)
