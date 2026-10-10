## Prerelease v0.1.0-preview.23

This preview improves Python apartment lifetime safety, collection reference comparisons, Object value conversion, and generated-package installation on Windows. It also strengthens Python callback, overload, and typing contracts, and fixes generated JavaScript/TypeScript interface inputs and async-with-progress Promises. These changes are relative to preview.22.

## Packages and installation

Runtime packages target Windows x64 and ARM64. JavaScript requires Node.js 18 or later; the Python runtime and generated bindings require CPython 3.11-3.14. Available APIs also depend on the installed Windows version, SDK components, package identity, and hardware.

- `@microsoft/dynwinrt` - JavaScript/TypeScript runtime.
- `@microsoft/dynwinrt-codegen` - JavaScript/TypeScript code generator.
- `dynwinrt` - Native Python runtime.
- `dynwinrt-codegen` - Standalone Python code generator.

**Version matching:** runtime and codegen packages must use the same version within each language ecosystem. Upgrade them together before regenerating bindings.

**Before using this version:** uninstall the previous runtime/codegen packages from the target environment and clean the old generated bindings and their build/install artifacts in the dedicated output location. Install the matching versions below and regenerate the complete bindings selection; do not mix old modules or stubs with the new runtime.

```powershell
npm install @microsoft/dynwinrt@0.1.0-preview.23
npm install -D @microsoft/dynwinrt-codegen@0.1.0-preview.23
python -m pip install --pre "dynwinrt==0.1.0rc23" "dynwinrt-codegen==0.1.0rc23"
```

## Highlights

### Python apartment and native-owner safety

- Release library-owned raw values, generated wrappers, and COM-bearing arrays and structs before the final library-managed apartment close, including independently owned nested and cloned values. This protects ordinary `RoApartment` exits even without `projected_lifetime_scope()`. Retained Python owners report that they are released and reject further native access; scalar-only values and containers remain usable.
- Reject a final apartment close inside a synchronous native-to-Python callback before owner cleanup or `RoUninitialize`. Named guards remain active for retry after both the callback and outer native call return.
- Preserve successful nested and manual initialization counts, including `S_FALSE` and new initializations acquired by Python finalizers during owner cleanup. A close consumes only its own initialization. Failed cleanup retains that lease for retry without discarding newly acquired leases.
- Reject explicit apartment operations on the wrong OS thread without changing state. An implicit foreign-thread guard drop retains an owner-thread recovery token and emits a native diagnostic instead of calling COM there.
- Keep unsafe pending async owners from being released by apartment cleanup. Settle or explicitly cancel blocked work before retrying close; cleanup does not silently cancel it or consume externally owned COM aliases.

Use `RoApartment.recover_pending()` on the owner thread for a guard deferred during a callback or dropped on another thread. For an unnamed guard whose owner cleanup failed, use `retry_pending_apartment_close()` or recover and explicitly close it after fixing the failure. The retry helper does not consume a callback-deferred lease.

### Python collections and Object values

- Compare WinRT references by canonical COM identity in sequence containment, `index()`, `count()`, and `remove()`, mapping equality, and value/item-view membership. Class, interface, and raw views of the same object now match; distinct objects with equal content remain distinct. Ordinary Python value equality and wrapper equality/hashing outside these collection operations are unchanged.
- Preserve mapping entry counts when Python keys alias the same native reference. Raw `Object` collections do not implicitly box or unbox for comparison; `object_value_view()` compares converted Python values normally and remaining native references by COM identity.
- Add `to_winrt_object(value, property_type=None)` for explicit boxing. Generated `Object`/`IInspectable` positions remain native by default; they do not implicitly box arbitrary Python values.
- Extend `unbox_object()` with DateTime, TimeSpan, Point/Size/Rect, their arrays, and recursively unboxed InspectableArray values. `preserve_type=True`, numeric tags, typed arrays, and `PropertyType` retain exact WinRT type information through `dynwinrt.values`.
- Add the opt-in live `dynwinrt.values.object_value_view()` for supported `IMap`/`IMapView<String or Guid, Object>` projections. Reads unbox and writes box explicitly; `.raw` retains access to the native map.

### Python projection contracts

- Project supported delegate callback arguments from their actual `Invoke` signatures, including typed `MapChanged` senders and event arguments.
- Unify method names, overloads, aliases, and collision handling across `.py` and `.pyi`. Preserve existing explicit ABI aliases and successful native dispatch targets, and use QueryInterface for compatible interface-typed overload inputs rather than requiring a particular wrapper class.
- Type `.pyi` outputs as non-null by default while preserving supported nullable positions, including `IReference<T>`, `Try*` results, Object/delegate values, documented Windows SDK null results, and reference collection elements. Correct exact XML DOM declarations for absent roots, DTDs, node navigation, owner documents, and attribute lookup/replacement results. Runtime null conversion and the broader inline `.py` output annotations are unchanged.
- Reject native null before mutating stock `JsonArray`/`JsonObject`, including bulk writes and generic interface views. Rejected null input leaves the collection unchanged; custom generic collections retain their valid native-null behavior.
- QueryInterface-check direct generated interface construction before retaining a pointer. Validate array element contracts before taking independent native references, and reject Async receivers in low-level `call_0()`/`call_1()` before vtable dispatch.
- Improve Python runtime errors and expose named `RO_INIT_SINGLETHREADED` and `RO_INIT_MULTITHREADED` constants.
- Correct click-handler annotations in both Python Tic-Tac-Toe samples so they work with precise delegate typing, without changing game behavior or lifetime cleanup.

### Python package generation and Windows paths

- Fix generated-package source and wheel installation failures under legacy Windows path limits by selectively shortening over-budget module paths with stable type-identity hashes and compacting setuptools build and wheel staging. Public type names and namespace exports remain unchanged.
- Record the complete generator version in `.dynwinrt-generator.json`. Reject incompatible, unstamped, or corrupt existing generated output before changing files, including during `--dry-run`; same-version incremental generation remains supported.
- Warn when a final generated `.py`/`.pyi` path reaches the legacy 260-UTF-16-unit boundary. The warning identifies the longest final path and how much shorter the output root must be; it does not enable Windows long-path support or change names according to the output root.

### JavaScript/TypeScript and shared code generation

- Reuse the existing projected Promise from generated async-with-progress `.toPromise()`. Repeated calls no longer register native completion twice; direct await and `.toPromise()` share the converted result or rejection. Pre-aborted signals also reuse their existing rejected Promise.
- Retain explicit interface-method aliases as real JavaScript class methods with matching declarations. For example, `StorageFile` can satisfy `IStorageFile` inputs in strict TypeScript while keeping `copyAsync(...)` and adding the interface's `copyOverloadDefaultOptions(...)` alias. Existing overload dispatchers and `.as(...)` views remain supported.
- Classify explicitly selected non-generic WinRT delegates correctly in both JavaScript and Python. Delegate-only, combined class/delegate, namespace, and incremental selections reuse the existing automatic-dependency projection; malformed or open-generic delegate roots fail explicitly.

## Upgrade notes

Upgrade runtime and codegen together, then fully regenerate and rebuild/reinstall affected bindings. Generated Python manifests pin the matching runtime version; do not mix old generated packages or stubs with a different runtime.

- **Python lifecycle:** initialize and balance each worker thread's own apartment. `projected_lifetime_scope()` remains an optional earlier-cleanup tool; do not rely on garbage collection to close apartments or on a close to revoke external COM references.
- **Python regeneration and imports:** use a fresh output directory, or manually clean the dedicated generated-output directory and regenerate the entire selection. Do not remove or edit only the producer stamp. Direct imports of shortened long module names must migrate; prefer stable namespace imports such as `from generated.windows.foundation import Uri`.
- **Python typing:** regenerate `.pyi` files together with their runtime modules. Cache potentially absent results and guard them before reading members. The Windows SDK nullable-result facts are not a universal nullability guarantee for custom or Windows App SDK metadata.
- **Python JSON:** use `JsonValue.create_null_value()` for JSON semantic null. It is a non-null native `IJsonValue`; Python `None` is not a substitute in stock JSON collections.
- **Python Object conversion:** a plain `int` boxes as Int32 only. Use explicit tags or `PropertyType` for other widths, enums, empty/mixed arrays, and InspectableArray. Re-boxing preserves type and value, not box identity.
- **JavaScript/TypeScript:** regenerate wrappers to obtain interface aliases and the progress Promise fix; upgrading the native runtime alone does not repair previously generated JavaScript.
- **Rust source compatibility:** exhaustive matches on public `PropertyValueData` and `PropertyValueUnboxResult` must handle the new payload variants and `PropertyValueUnboxResult::Unsupported(PropertyType)`. Keep `Null` and `NotPropertyValue` distinct and handle unsupported boxes explicitly.

## Known boundaries

Classic COM, flat Win32, and WinUI hosting remain experimental. This preview does not expand their support guarantees or remove the documented architecture-specific WinRT collection-producer limits. Native ARM64 live WinUI coverage remains incomplete; ordinary runtime-wheel checks do not establish full UI support.

Embedded hosts retaining native aliases to Python-backed callbacks must stop new calls, settle in-flight callbacks, and call `shutdown_python_callbacks()` while Python is alive, before `Py_FinalizeEx`. Hosts skipping that protocol have only best-effort protection against callbacks during early interpreter finalization; no universal deadlock-free guarantee is claimed.

Python module shortening and compact staging reduce path risk but do not guarantee arbitrary checkout, package-name, build-temporary-directory, or venv depths. Use short paths and leave headroom for installation and bytecode caches; a missing warning is not a guarantee that every downstream tool can open the output.

For details, see the [Python runtime guide](https://github.com/microsoft/dynwinrt/blob/main/bindings/py/README.md), [Python codegen path guidance](https://github.com/microsoft/dynwinrt/blob/main/tools/dynwinrt-codegen/python/README.md#windows-path-length), and the [project README](https://github.com/microsoft/dynwinrt#readme).
