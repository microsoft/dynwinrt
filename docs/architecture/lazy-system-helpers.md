# Lazy GDI and USER32 helpers

The native bindings resolve the following existing helpers on demand rather
than importing them through the PE ordinary or delay import tables. This is
a **direct-import boundary**, not complete physical isolation from UI DLLs.
The binaries remain monolithic, and the host or other Windows dependencies
can still load USER32 and GDI. No startup-speed improvement is claimed.

| DLL | Helpers | First resolution |
| --- | --- | --- |
| GDI32 | `DeleteObject` | Before invoking a completed call plan that can produce an owned GDI output, or at an explicit raw cleanup request |
| USER32 | `DestroyIcon` | At an explicit raw icon cleanup request |
| USER32 | `CreateWindowExW` | At the existing `DynCom.createTestHwnd()` request on `/com/unsafe` |
| USER32 | `AreDpiAwarenessContextsEqual`, `GetDpiAwarenessContextForProcess`, `SetProcessDpiAwarenessContext`, `SetThreadDpiAwarenessContext` | When creating a XAML application and applying its existing per-monitor-v2 policy |

The metadata-independent implementation is in
`crates/dynwinrt/src/system_helpers.rs`. Its small, doc-hidden Rust surface is
shared with the JS binding; it is not a new supported projection API or a
general-purpose DLL loader. The WinRT, Classic COM, and flat Win32 semantic
models, generated code, and public JS/Python signatures are unchanged.

## Resolution and lifetime

Each helper has its SDK/windows-rs `extern "system"` signature, including
typed `HGDIOBJ`, `HICON`, `HWND`, DPI contexts, and native BOOL results.
The four DPI exports are published as one complete table. GDI deletion, icon
destruction, and window creation have independent caches, so using one does
not require unrelated exports.

Loads use `LoadLibraryExW` with `LOAD_LIBRARY_SEARCH_SYSTEM32`. Loading and
`GetProcAddress` execute outside the cache publication lock. Only successful
resolutions are cached. A winning loader reference is retained for the
process lifetime before publication; failure and concurrent-loser references
are released. Repeated calls reuse the typed function pointer without another
export lookup. Failure is retryable and reports the DLL/export and original
Windows error code.

Successful cold resolution restores the incoming thread LastError **before**
calling the target API. It does not restore anything after that call: SDK-style
BOOL/null failures read the target's LastError immediately. This avoids
exposing successful loader bookkeeping as a native API failure.

## Ownership admission

Building a type, registering an interface, importing a projection, and
initializing a native module do not resolve these helpers.

The shared native executor records whether the completed parameter or direct
return cleanup plan can own a GDI handle. At actual invocation, it resolves
the deleter before the dispatch marker and native operation. This covers
fast calls, libffi calls, the COM-value path, and direct returns. Failure
prevents native execution and therefore cannot strand a newly produced owned
output.

Partial outputs on failed HRESULTs, native result-extraction failures,
COM post-call validation failures, and JS owners/finalizers use the already
prepared, permanently backed GDI deleter. They never attempt a fallible first
load during cleanup. Original HRESULTs remain failures after cleanup.
The existing best-effort treatment of a native destructor's failure is not
changed; explicit JS release still retains ownership/provenance when native
deletion fails so the caller can retry.

Raw GDI/icon cleanup has an existing fallible contract. It resolves and calls
the matching deleter before consuming the external pointer's provenance.
Load, export, and native cleanup failures leave that pointer available for
retry. `HICON` continues to use `DestroyIcon`, not `DeleteObject`.
No apartment, callback, shutdown, or DPI policy is changed.

## Production import gate

`bindings/js/scripts/pe-imports.mjs` supplies the shared PE32/PE32+ reader and
policy. The new production policy is intentionally **stronger than a
seven-symbol check**: it rejects every GDI32/USER32 descriptor, including
empty and ordinal-only descriptors, plus the scoped named exports through
other DLL names. It checks both ordinary and delay imports. OLEAUT32,
OLE32, and COMBASE are not prohibited.

`npm run test:imports` and production artifact verification check every
`.node` architecture without loading foreign-architecture binaries.
The dependency-free CLI also checks an existing Python extension:

```powershell
node .\bindings\js\scripts\check-ui-helper-imports.mjs <production.pyd>
```

The Build workflow checks the installed, verified, no-test-hooks CI Python
wheel (its existing profile is **dev**, not release). The Python release
workflow separately checks the installed **release** wheel in each x64/ARM64
runtime-consumer lane, with explicit Node 24 setup. Both use the same reader
and policy, without rebuilding or loading the `.pyd` for inspection.
Existing artifact identity, upload, and publication rules are unchanged.

Native test programs and explicitly marked test-hooks addons can import
GDI/USER32 fixture-creation APIs. They are not evidence about shipping binary
imports. Production checks do not grant those fixtures an exception.

## Retained validation

[Machine-readable evidence](../status/generated/lazy-system-helpers-validation.json)
records before/after production PE tables and hashes, calibrated cold-process
module observations, and regression-log digests. The baseline is
`34eb6246627c671113a501c00edc8e3317fc8b3d`.

| Observation | Before | After |
| --- | --- | --- |
| ARM64 release Node addon | GDI32 `DeleteObject`; USER32 six scoped exports | No GDI32/USER32 ordinary or delay descriptors |
| ARM64 release CPython 3.13 extension | GDI32 `DeleteObject`; USER32 four DPI exports | No GDI32/USER32 ordinary or delay descriptors |
| Cross-built x64 release addon payload | Not measured locally at baseline | No GDI32/USER32 ordinary or delay descriptors |
| Fresh ARM64 Node: host / root import / primitive / Uri | 35 / 37 / 37 / 39 modules | 35 / 37 / 37 / 39 modules |
| Fresh ARM64 Python: host / root import / primitive / Uri | 15 / 28 / 28 / 30 modules | 15 / 28 / 28 / 30 modules |

MSVC `dumpbin /imports` independently confirms the changed production PE
tables. In this machine's cold processes, **module counts did not decrease**.
Node already maps the UI DLLs; Python still maps them through remaining
system dependencies. Neither absence of new Node modules nor removed direct
PE entries proves transitive isolation.

The observer runs in a separate ARM64 PowerShell process and enumerates the
child-reported PID with `K32EnumProcessModulesEx` /
`K32GetModuleFileNameExW`. Each phase has a stdin/stdout handshake, an
`ntdll.dll` positive control, a plausible module-count check, and bounded
natural exit. No `ctypes`, process-report call, or observer is injected into
the measured child. Registration and hidden-window opt-in probes are separate
from non-UI import/primitive/Uri probes.

Focused coverage includes retryable missing DLL/export resolution, concurrent
cache publication/reference balance, LastError preservation, real bitmap and
icon cleanup, native failure and partial outputs, failed extraction and
post-call validation, explicit release retry, and hidden-window/DPI use.
The native scoped tests ran on ARM64 and on x64/i686 under
**Windows-on-ARM emulation**, not physical x64/i686 hardware.
The x64 addon was cross-built and inspected, not executed in an x64 Node host.
Local Python runtime measurements cover ARM64 CPython 3.13 only.

Existing CoreMessaging and optional Win32 import gates, WinRT snapshots,
generated non-UI Uri calls in both languages, generated stock WIC COM calls,
and a flat Win32 `GetTickCount` package smoke test remain covered.
This work does not delay OLEAUT32, replace existing optional-subsystem loaders,
split binaries, establish support for older Windows versions, or prove
complete absence of COM/UI DLLs in a process.
