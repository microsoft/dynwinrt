## Prerelease v0.1.0-preview.23

This preview improves Python apartment lifetime safety, collection comparisons and write contracts, Object value conversion, and generated-package installation on Windows. It also strengthens Python callback, overload, and typing contracts, and fixes generated JavaScript/TypeScript interface inputs and async-with-progress Promises. These changes are relative to preview.22.

## Packages and installation

- `@microsoft/dynwinrt` - JavaScript/TypeScript runtime.
- `@microsoft/dynwinrt-codegen` - JavaScript/TypeScript code generator.
- `dynwinrt` - Native Python runtime.
- `dynwinrt-codegen` - Standalone Python code generator.

- **Runtime platforms:** Windows x64 and ARM64.
- **JavaScript version:** Node.js 18 or later.
- **Python version:** CPython 3.11-3.14 for the runtime and generated bindings.
- **API availability:** Depends on the Windows version, installed SDK components, package identity, and hardware.
- **Package versions:** Use the same runtime and codegen version for each language.
- **Upgrade cleanup:** Clean old generated bindings before using the new version.

```powershell
npm install @microsoft/dynwinrt@0.1.0-preview.23
npm install -D @microsoft/dynwinrt-codegen@0.1.0-preview.23
python -m pip install --pre "dynwinrt==0.1.0rc23" "dynwinrt-codegen==0.1.0rc23"
```

## Highlights

### Python apartment and native-owner safety

- Release library-owned COM references before the final managed apartment close. `projected_lifetime_scope()` is optional.
- Block final apartment teardown inside synchronous native callbacks. Preserve guards for recovery.
- Keep nested initialization counts balanced, including initialization during cleanup. Preserve failed closes for retry.
- Reject wrong-thread apartment closes without changing state. Preserve dropped guards for owner-thread recovery.
- Refuse cleanup of unsafe pending async owners. Cleanup does not silently cancel work.

Recover deferred guards with `RoApartment.recover_pending()` on the owner thread. Use `retry_pending_apartment_close()` for failed unnamed closes.

### Python collections and Object values

- Match class, interface, and raw references by COM identity in collection comparisons. Equality and hashing outside collections stay unchanged.
- Preserve mapping entry counts and ordinary Python value equality. Raw `Object` collections do not implicitly box or unbox.
- Align bulk-write input types with item assignment. Preserve native member names and dispatch.
- Return the stored read projection from `setdefault()`. Do not consume the caller's input.
- Add explicit Object boxing with `to_winrt_object()`. Generated Object positions remain native by default.
- Extend `unbox_object()` to DateTime, TimeSpan, geometry, and arrays. Retain exact WinRT types when requested.
- Add opt-in Object map views with `object_value_view()`. Reads unbox and writes box. `.raw` stays native.

### Python projection contracts

- Project typed delegate arguments from native signatures. Include typed `MapChanged` callbacks.
- Keep `.py` and `.pyi` overloads and aliases consistent. Accept compatible interface inputs through QueryInterface.
- Default `.pyi` outputs to non-null. Preserve supported nullable results, including XML DOM results.
- Reject native null in stock JSON writes before mutation. Keep valid nulls in custom generic collections.
- Validate interface construction and arrays before retaining native references. Reject Async receivers in `call_0()` and `call_1()`.
- Add named apartment constants and clearer runtime errors.
- Fix strict click-handler typing in both Python Tic-Tac-Toe samples.

### Python package generation and Windows paths

- Improve Windows installs with shorter module and staging paths. Keep public type names and namespace exports unchanged.
- Record the producer version in `.dynwinrt-generator.json`. Reject incompatible output before changing files. Keep same-version incremental generation.
- Warn when final generated paths reach the legacy Windows limit.

### JavaScript/TypeScript and shared code generation

- Reuse the projected Promise for repeated async-with-progress `.toPromise()` calls. Avoid duplicate native completion registration.
- Generate real interface-method aliases with matching TypeScript declarations. Keep existing overload dispatch unchanged.
- Handle explicitly selected non-generic WinRT delegates in JavaScript and Python. Reject malformed or open-generic delegate roots.

## Upgrade notes

Upgrade runtime and codegen together, then fully regenerate and rebuild/reinstall affected bindings. Generated Python manifests pin the matching runtime version; do not mix old generated packages or stubs with a different runtime.

- **Python lifecycle:** initialize and balance each worker thread's own apartment. `projected_lifetime_scope()` remains an optional earlier-cleanup tool; do not rely on garbage collection to close apartments or on a close to revoke external COM references.
- **Python regeneration and imports:** use a fresh output directory, or manually clean the dedicated generated-output directory and regenerate the entire selection. Do not remove or edit only the producer stamp. Direct imports of shortened long module names must migrate; prefer stable namespace imports such as `from generated.windows.foundation import Uri`.
- **Python typing:** regenerate `.pyi` files together with their runtime modules. Cache potentially absent results and guard them before reading members. The Windows SDK nullable-result facts are not a universal nullability guarantee for custom or Windows App SDK metadata.
- **Python collection defaults:** `setdefault()` follows the map's read projection rather than returning an unconverted input wrapper. Raw `Object`-valued maps return `DynWinRTValue | None`; project or unbox the result explicitly, or use `object_value_view()` on supported maps.
- **Python JSON:** use `JsonValue.create_null_value()` for JSON semantic null. It is a non-null native `IJsonValue`; Python `None` is not a substitute in stock JSON collections.
- **Python Object conversion:** a plain `int` boxes as Int32 only. Use explicit tags or `PropertyType` for other widths, enums, empty/mixed arrays, and InspectableArray. Re-boxing preserves type and value, not box identity.
- **JavaScript/TypeScript:** regenerate wrappers to obtain interface aliases and the progress Promise fix; upgrading the native runtime alone does not repair previously generated JavaScript.
- **Rust source compatibility:** exhaustive matches on public `PropertyValueData` and `PropertyValueUnboxResult` must handle the new payload variants and `PropertyValueUnboxResult::Unsupported(PropertyType)`. Keep `Null` and `NotPropertyValue` distinct and handle unsupported boxes explicitly.

## Known boundaries

Classic COM, flat Win32, and WinUI hosting remain experimental. This preview does not expand their support guarantees or remove the documented architecture-specific WinRT collection-producer limits. Native ARM64 live WinUI coverage remains incomplete; ordinary runtime-wheel checks do not establish full UI support.

Embedded hosts retaining native aliases to Python-backed callbacks must stop new calls, settle in-flight callbacks, and call `shutdown_python_callbacks()` while Python is alive, before `Py_FinalizeEx`. Hosts skipping that protocol have only best-effort protection against callbacks during early interpreter finalization; no universal deadlock-free guarantee is claimed.

Python module shortening and compact staging reduce path risk but do not guarantee arbitrary checkout, package-name, build-temporary-directory, or venv depths. Use short paths and leave headroom for installation and bytecode caches; a missing warning is not a guarantee that every downstream tool can open the output.

For details, see the [Python runtime guide](https://github.com/microsoft/dynwinrt/blob/main/bindings/py/README.md), [Python codegen path guidance](https://github.com/microsoft/dynwinrt/blob/main/tools/dynwinrt-codegen/python/README.md#windows-path-length), and the [project README](https://github.com/microsoft/dynwinrt#readme).
