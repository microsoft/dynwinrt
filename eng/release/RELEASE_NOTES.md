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

Upgrade runtime and codegen together. Regenerate bindings and rebuild/reinstall generated packages. Do not mix old bindings or stubs with the new runtime.

- **Python lifecycle:** Balance each worker thread's apartment explicitly. `projected_lifetime_scope()` is optional. External COM references remain caller-owned.
- **Python regeneration:** Regenerate fully in a fresh or cleaned generated-output directory. Do not delete or edit only the producer stamp. Use namespace imports for shortened modules.
- **Python typing:** Regenerate `.py` and `.pyi` together. Guard nullable results before member access. SDK nullability facts are not universal.
- **Python collection defaults:** `setdefault()` returns the map's read projection. Raw `Object` maps return `DynWinRTValue | None`. Project or unbox results explicitly.
- **Python JSON:** Use `JsonValue.create_null_value()` for JSON null. Do not pass `None` to stock JSON collections.
- **Python Object conversion:** Plain `int` values box as Int32. Use tags or `PropertyType` for other types. Re-boxing does not preserve box identity.
- **JavaScript/TypeScript:** Regenerate wrappers for interface aliases and the progress Promise fix. A runtime-only upgrade does not update generated code.
- **Rust source compatibility:** Update exhaustive matches on `PropertyValueData` and `PropertyValueUnboxResult`. Handle new variants and `Unsupported(PropertyType)`. Keep `Null` and `NotPropertyValue` distinct.

## Known boundaries

- **Experimental APIs:** Classic COM, flat Win32, and WinUI hosting remain experimental.
- **Collection producers:** Architecture-specific support limits remain unchanged.
- **ARM64 WinUI:** Native live coverage remains incomplete.
- **Embedded hosts:** Stop new calls and settle in-flight callbacks. Call `shutdown_python_callbacks()` before `Py_FinalizeEx`. Skipping this protocol leaves only best-effort protection.
- **Windows paths:** Short names do not remove all path limits. Keep output and venv paths short. A missing warning does not guarantee compatibility.

For details, see the [Python runtime guide](https://github.com/microsoft/dynwinrt/blob/main/bindings/py/README.md), [Python codegen path guidance](https://github.com/microsoft/dynwinrt/blob/main/tools/dynwinrt-codegen/python/README.md#windows-path-length), and the [project README](https://github.com/microsoft/dynwinrt#readme).
