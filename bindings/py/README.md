# dynwinrt

`dynwinrt` is the native CPython runtime for generated dynwinrt projections.
Release wheels support CPython 3.11 through 3.14 on Windows x64 and ARM64.
The floor is intentionally the first CPython line for which the release
infrastructure can install and execute a native Windows ARM64 interpreter; the
project does not ship cross-compiled, untested ARM64 wheels.

Install the runtime and the standalone generator from an approved feed or a
downloaded release:

```powershell
python -m pip install dynwinrt dynwinrt-codegen
dynwinrt-codegen generate --namespace Windows.Foundation --class-name Uri `
  --lang py --output generated_uri
```

Generated Python package manifests pin `dynwinrt` to exactly the version of
`dynwinrt-codegen` that emitted them.

## Develop

Run `uv run maturin develop` to build the package

Use `uv run pytest` to run the tests

After modification `uv sync --reinstall` may be needed to reinstall the package

The wheel includes `__init__.pyi` and `py.typed` for static type checking.

## Release process

`.github\workflows\python-release.yml` builds and consumes every CPython
3.11–3.14 runtime wheel on x64 and native ARM64. It also builds standalone
`py3-none-win_amd64` and `py3-none-win_arm64` codegen wheels, then exercises the
installed command with Rust removed from `PATH`.

Pull requests run the x64 matrix. ARM64 jobs run only for manual dispatches and
`v*` release tags on the repository's existing
`[self-hosted, Windows, ARM64, winui]` runner. This avoids executing untrusted
pull-request code on a self-hosted machine. A release remains blocked rather
than producing ARM64 artifacts if that runner or one of its native CPython
versions is unavailable.

To release:

1. Create the ordinary repository release tag (for example,
   `v0.1.0-preview.20`). The workflow derives the Python package version from
   the tag and updates its build workspaces without modifying the repository.
   PEP 440 normalizes `preview.N` wheel versions to `rcN`.
2. The JavaScript release pipeline creates the GitHub release. After all ten
   Python wheels pass their consumption tests, this workflow waits for that
   release and uploads the wheels alongside the npm tarballs. It never creates
   a separate Python GitHub release.
3. Configure the protected `pypi` GitHub environment with required reviewers
   and PyPI trusted publishers for `dynwinrt` and `dynwinrt-codegen`. To
   publish, manually dispatch the workflow **from that existing tag** with
   `publish_pypi` enabled and `release_version` set. OIDC trusted publishing is
   used; no API token is stored in the repository.

Do not enable publication from a branch, pull request, or ordinary push.

Generated `IReference<T>` values are projected as `T | None`; native values,
`None`, and generated `IReference_*` wrappers are accepted as inputs.

## Async WinRT operations

Generated async methods return typed, asyncio-compatible operation objects:

```python
operation = writer.store_async()
stored_bytes = await operation
```

Their public types are `WinRTAsync[T]` and
`WinRTAsyncWithProgress[T, P]`; the concrete runtime wrappers remain private.

Regenerated bindings no longer block inside async methods. Existing code that
expects an immediate result must use `await operation` or `operation.wait()`.

`asyncio` task cancellation calls `IAsyncInfo.Cancel()` on the underlying
WinRT operation. Operations with scalar progress values also expose
`operation.progress(callback)`. Fast operations can finish before registration;
in that case no future progress exists and registration is a no-op.

For scripts without an event loop, `operation.wait()` remains available as an
explicit blocking API. It rejects started operations when called from a running
asyncio loop or an STA thread, where blocking could freeze or deadlock the
caller.

WinRT HRESULT failures raise `OSError` (or a standard `OSError` subclass) with
the signed HRESULT in `error.winerror`. The exception message preserves
restricted WinRT error information when Windows provides it.

## Python-native values

Generated collection projections implement the standard `collections.abc`
protocols: vectors behave as sequences, maps as mappings, and WinRT iterables
and iterators work with `iter()` and `next()`. Mutable vectors and maps support
normal indexing, slicing, assignment, insertion, and deletion.

Method inputs accept normal Python sequences and mappings in place of compatible
WinRT collection interfaces. Byte arrays accept `bytes` and `bytearray`; GUID,
`DateTime`, and `TimeSpan` values use `uuid.UUID`, `datetime.datetime`, and
`datetime.timedelta`.

Exceptions raised by Python event or delegate callbacks are reported through
`sys.unraisablehook`. The originating WinRT invocation receives
`0xA0EE4005` (`PYWINRT_E_UNRAISABLE_PYTHON_EXCEPTION`) instead of unconditional
success. Generated delegate parameters accept normal Python callables. WinRT
chooses the callback thread, so callbacks must not assume they run on the
registration thread or an asyncio event-loop thread. Keep each token returned by
`on_*` and pass it to the matching `off_*` when the subscription is no longer
needed. For callback-style cleanup, `subscribe_*` returns an idempotent
unsubscribe function. `once_*` subscribes for at most one callback invocation.

WinRT flags enums are projected as `enum.IntFlag`. Overloaded methods share one
Python name with runtime type/arity dispatch and `typing.overload` declarations.
Activatable runtime classes use normal constructors, for example
`Uri("https://example.com")`. Constructor overloads come only from WinMD
`ActivatableAttribute` and public `ComposableAttribute` declarations. Classes
without that metadata, including system-returned classes and protected-only
composition, raise a class-named `TypeError` on normal construction and their
stubs expose no public constructor. Native return values still use the internal
`_from_native`/`DynWinRTValue` wrapping path.

## Raw object projection

Use `project_as(value, Type)` when metadata returns `Object`/`IInspectable` but
the application knows the concrete generated type. This is common with XAML
APIs such as `XamlReader.load()` and `FrameworkElement.find_name()`:

```python
from dynwinrt import project_as
from generated.microsoft.ui.xaml.controls import Button, StackPanel
from generated.microsoft.ui.xaml.markup import XamlReader

raw_panel = XamlReader.load(XAML)
if raw_panel is None:
    raise RuntimeError("XamlReader returned no value")
panel = project_as(raw_panel, StackPanel)

raw_button = panel.find_name("Submit")
if raw_button is None:
    raise RuntimeError("Submit was not found")
button = project_as(raw_button, Button)
```

`project_as()` borrows its input: the raw value or source wrapper remains
valid. For an interface target it queries that interface's IID before creating
the wrapper. The returned wrapper owns the QueryInterface result, participates
in the active `projected_lifetime_scope()`, and preserves the projection
identity cache. Incompatible types raise the ordinary WinRT `OSError`.

Use `wrapper.as_interface(InterfaceClass)` when converting an existing
runtime-class wrapper to another generated interface. Do not call the internal
`_from_native()` method from application code.

## COM apartments and cleanup

Use `RoApartment` to initialize COM for a thread and balance every successful
initialization:

```python
with RoApartment(0):  # RO_INIT_SINGLETHREADED
    use_winrt()
```

Use `RoApartment(1)` for `RO_INIT_MULTITHREADED`. Nested contexts using the same
model are supported. Requesting a conflicting model raises `OSError` with
`RPC_E_CHANGED_MODE`. The low-level `ro_initialize()` API remains available, but
each successful call, including `S_FALSE`, must be paired with one
`ro_uninitialize()` call on the same thread.

Generated runtime classes that implement `IClosable` support `with` and an
idempotent `close()` method. Prefer deterministic cleanup instead of relying on
Python garbage collection.

## Experimental WinUI bootstrap

When `Microsoft.UI.Xaml.Application` is generated with the WinUI metadata
provider and controls resources, codegen also emits
`Application.create_with_metadata_provider(...)` and `Application.create(...)`.
The latter installs `XamlControlsResources` and configures unpackaged resource
resolution when the required metadata is available.

Public composable constructors keep their existing exact-class behavior. A
Python subclass of such a class is constructed with a non-null aggregated outer,
so ordinary inherited WinUI properties and methods retain one COM identity.
Metadata-supported native overrides are exposed through local overridable
interfaces. WinUI `IFrameworkElementOverrides.MeasureOverride`,
`ArrangeOverride`, and `OnApplyTemplate` are supported; size callbacks receive
and return `(width, height)` tuples. Callbacks run synchronously on the creating
WinUI apartment under the `ContextVar` context captured during construction.
Exceptions are reported through `sys.unraisablehook` and returned to WinUI as a
failing HRESULT. Unsupported override ABI shapes raise `TypeError` during
subclass construction.

The WinUI 2.3 metadata contract used by the live test is
`IFrameworkElementOverrides`
(`ffc6fd98-f38c-5904-9ce4-97a3427cf4ba`): inherited by `StackPanel`, with
`MeasureOverride(Size) -> Size` at slot 6, `ArrangeOverride(Size) -> Size` at
slot 7, `OnApplyTemplate() -> void` at slot 8, and
`GoToElementStateCore(HSTRING, Boolean) -> Boolean` at slot 9. `Size` is two
ABI `f32` fields. Parameters are borrowed for the synchronous call and the
result is caller-owned out storage. StackPanel is metadata-agile, while Python
override callbacks remain intentionally bound to their creating UI apartment.

Publicly composable controls can also register a Python subclass for XAML
markup activation:

```python
class PythonPanel(StackPanel):
    def measure_override(self, available_size):
        return available_size

registration = StackPanel.register_xaml_runtime_class(
    "MyApp.Controls.PythonPanel",
    PythonPanel,
)
panel = StackPanel(XamlReader.load(
    '<local:PythonPanel xmlns:local="using:MyApp.Controls" />'
))
# Remove every instance from the XAML tree before releasing Python owners.
panel = None
registration.unregister()
registration.release_instances()
```

Registrations are process-local, duplicate names fail, and dropping or closing
the returned registration removes new lookups. The composed Application
metadata provider checks Python registrations before
`XamlControlsXamlMetaDataProvider`. Its `IXamlType` delegates inherited content
and member metadata to the declared WinUI base and activates through the Python
constructor on the registering STA. Constructor callbacks and captured `ContextVar` context remain available for
new activation until unregister. XAML-created Python owners stay process-rooted
until `release_instances()` so live native controls cannot lose their override
targets; call it only after those controls leave the XAML tree. Unsupported native
override ABIs, generic names, collection/dictionary/markup-extension bases, and
Python-defined XAML members fail closed.

This deliberately does **not** call `RoRegisterActivationFactories`: ordinary
`RoActivateInstance("MyApp.Controls.PythonPanel")` remains unavailable.
`XamlReader` does not need that global activation path; WinUI resolves the name
through `IXamlMetadataProvider` and calls `IXamlType.ActivateInstance` directly
(WinUI source: `components/metadata/MetadataAPI.cpp` and
`ExtMetadataProvider/XamlType.cpp`). A class that must be activated outside this
Application/XAML provider boundary still requires a static WinMD plus packaged
manifest/in-process server registration. Protected composition and
system-returned classes remain non-constructible.

Generated `Application.start()` and `DispatcherQueue.run_event_loop()` calls
remain on the caller's native thread but release the Python GIL while WinUI
pumps messages. WinRT callbacks reacquire the GIL, and worker-thread asyncio
code can use `DispatcherQueue.try_enqueue()` to return to the UI thread.

Use a projected lifetime scope inside the COM apartment so generated wrappers
release their native values before `RoUninitialize`:

```python
from dynwinrt import RoApartment, projected_lifetime_scope

with RoApartment(0), projected_lifetime_scope():
    app = Application.create()
    # Create and use WinUI objects here.
```

Scopes nest in LIFO order. Wrappers that survive the scope remain Python
objects, but their native values are released and further WinRT calls fail
instead of releasing COM references after apartment teardown.

The live x64 smoke test accepts explicit WinAppSDK metadata and bootstrap
inputs:

```powershell
.\tests\e2e\python_winui_e2e.ps1 `
  -WinuiWinmd <Microsoft.UI.Xaml.winmd> `
  -RefList <winmd-reference-list.txt> `
  -BootstrapDll <Microsoft.WindowsAppRuntime.Bootstrap.dll> `
  -Major 2 -Minor 3
```