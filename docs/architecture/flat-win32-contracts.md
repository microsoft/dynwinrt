# Contract-driven flat Win32

Flat Win32 bindings dynamically invoke DLL exports described by Windows
metadata. They use a Win32-local ABI model and explicit semantic contracts,
separate from WinRT activation and Classic COM vtables.

## Metadata and supported capabilities

Built-in contracts are validated against
`Microsoft.Windows.SDK.Win32Metadata 71.0.14-preview` with SHA-256
`B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D`.

The measured inventory contains 18,321 eligible exports and 8,936 complete
projections: 8,934 synchronous functions and two asynchronous helpers. There
are 182 supported containers and 10,680 public exports including aliases,
enums and helper functions. These are generation counts, not a claim that
every API has been exercised against a live system.

| Area | Supported behavior |
| --- | --- |
| Native ABI | All admitted scalars, enums, GUID/scalar pointers, native structures/unions, pointer and by-value aggregate shapes; system/cdecl calls and target availability. |
| Return contracts | Direct, void, status/HRESULT and LastError behavior, including expected failure results. |
| Strings and buffers | UTF-16/ANSI storage, double-NUL lists, reserved inputs, counted buffers, capacity/actual-size relationships and caller-owned output semantics. |
| Resources | Exact HKEY, HANDLE, HLOCAL, HGLOBAL, HMODULE, SC_HANDLE, CoTaskMem and credential cleanup; synchronized leases and consuming calls. |
| Native builders | Existing SECURITY_ATTRIBUTES, STARTUPINFOA/W, PROCESS_INFORMATION and POD helpers, including retained pointer fields and owned outputs. |
| COM inputs | Exact-IID, borrowed managed interface inputs without changing the WinRT model. |
| Asynchronous I/O | IOCP-backed ReadFile/WriteFile Promises, cancellation, EOF, retained storage, detachment checks and bounded pending work. |
| Subsystems | Winsock, GDI+, Media Foundation and explicit unsafe MAPI utility context behavior. |
| Tooling | Safe/unsafe entrypoints, declarations, CLI/census, manifests, examples and original stock-Windows scenarios. |

The examples under `samples/js/win32` demonstrate system information,
Registry queries and asynchronous file I/O with the namespace layout.

## Contract boundary

```text
Windows.Win32.winmd facts
    + independent Win32 JSON semantic evidence
    -> validated Win32 model
    -> typed language projection
    -> generic renderer and immutable native call plans
```

`tools/dynwinrt-codegen/contracts/win32/` owns Win32-specific manual evidence.
COM retains its own interface/IID/vtable contracts. Both follow the
[contract evidence principles](classic-com-contract-evidence-registry.md);
their selectors and ABI models are not interchangeable.

Metadata-complete shapes use generic projection. Manual facts such as missing
count relationships, ownership, cleanup, special layouts and lifecycle
requirements belong in typed, closed contracts with exact selectors,
fingerprints, metadata provenance and authoritative evidence. Contracts must
not contain JavaScript, declaration fragments, arbitrary cleanup functions or
unconstrained native adapter names. Unsupported facts remain explicit errors.

Refactoring must not weaken an existing safety boundary to increase
coverage. If a behavior needs a correctness fix, implement and
document that fix and exercise its nearest failure case rather than silently
dropping the API or relabeling it as supported.

## Runtime and package boundary

Win32 stays outside the public WinRT model and the `@microsoft/dynwinrt` root.
Generated Win32 code uses `/win32`, with manual raw ABI capabilities isolated
under `/win32/unsafe`. Existing WinRT and COM entrypoints remain unchanged.
Sharing behavior-neutral private FFI storage/execution remains allowed.

Native invocation follows completed plans; it does not infer pointer meanings
from JavaScript objects. Handle values, dereferenced storage and owned resources
remain distinct. Resource consumption and lease acquisition are synchronized.
Buffer bounds come from validated native backing storage, not spoofable JS
length properties. Unknown ownership or successful out-of-bounds lengths must
not produce success-shaped fallback values.

Output relationships and validity are part of the native call contract, not
renderer-specific Registry rules. Bounded input predicates are evaluated before
the call; native return conditions are evaluated immediately after the call.
The resulting output disposition is applied before decoding, ownership adoption
or cleanup. Undefined outputs use `Value::Unavailable`; language projection maps
that outcome to `null`, rather than reading a native value and hiding it afterward.

`RegOpenKeyA/W` with a null/empty subkey returns an alias of its input.
Managed aliases share the existing owner, close state and leases; borrowed input
handles stay borrowed. `RegOpenKeyExA/W` applies that relationship only to the
documented native predefined-key case. Its predicate compares the full,
pointer-sized signed handle value: on 64-bit Windows, a zero-extended
`0x80000002` can produce a new owned handle, whereas the native predefined value
is sign-extended. These representations must not be merged by masking away
the upper bits. Aliases are authorized by exact contracts and
checked against the specified input, never discovered by globally merging handle
numbers. For `RegQueryValueExA/W` and `RegGetValueA/W`, performance-data queries
remain supported: successful results retain their size, while
`HKEY_PERFORMANCE_DATA` combined with `ERROR_MORE_DATA` exposes `dataSize: null`
without decoding the undefined count. Callers grow a separate capacity and retry.
Normal-key size queries retain their prior behavior.

Native Win32 carriers use type tags rather than mutable JavaScript prototypes.
Manual aggregate descriptors and MAPI utility initialization are available on
the explicit unsafe subpath; generated validated helpers call those primitives
internally. The full `DynWin32*` capability set is retained across the Win32
entrypoints. x86 plan invocation remains explicitly unsupported;
x64 is exercised live and ARM64 is compile-validated.

MAPI utility symbols are resolved lazily from the system DLL. Their x86
exports carry stdcall suffixes (`ScInitMapiUtil@4`, `DeinitMapiUtil@0`);
binding them as unconditional undecorated imports would prevent unrelated
WinRT/COM consumers from loading the addon. Missing utility exports produce
an explicit error when that subsystem is requested. The system DLL is a
dispatch stub, not a MAPI provider: utility initialization requires an installed,
configured provider matching the process architecture. Without one, the stub
can display a native initialization message before returning `E_FAIL`, as
documented in [MAPI stub registry settings](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/windowsmapi/mapi32-dll-stub-registry-settings).
The unsafe entrypoint preserves that native behavior; it does not install a
provider, change mail-client registration, or substitute a successful no-op.

The IOCP engine retains native state through terminal completion, including
cancellation. The limits are 1,024 pending operations, 64 MiB per
operation and 256 MiB of pending private buffers. A shared worker set is used,
not one blocked OS/libuv worker per operation. Subsystem close cannot race
dependent calls or retained operations.

File completion notification changes have a typed resource state effect.
Native mode changes are serialized with managed calls and rejected while an
asynchronous lease exists, including a prepared operation not yet submitted.
Successful changes only add mode bits; failures do not update state. Before
each IOCP submission the runtime queries the actual native notification modes,
so a preconfigured handle is not assumed to use defaults. Synchronous success
with `FILE_SKIP_COMPLETION_PORT_ON_SUCCESS` completes locally through the same
retirement path; ordinary synchronous success and `ERROR_IO_PENDING` still wait
for their IOCP packet. Buffers and leases are retired exactly once in either path.
Raw/unsafe handle escape does not authorize concurrent foreign mutation or close
of a managed handle.

## Namespace output

Generated modules use main's lowercase/kebab namespace directory mapping, for
example `win32/windows/win32/system/registry/Apis.js`. The Win32 output manifest
records namespace exports and generated file hashes. The CLI uses the shared
atomic output transaction, retaining other generated namespaces on incremental
runs and rejecting stale or conflicting output.

Win32-only packages do not acquire a WinRT package-root entrypoint. Mixed
generation preserves existing root exports and adds explicit Win32 subpaths.
Namespaces containing COM interfaces retain explicit `--class-name`
selection; use their `Apis` container for flat exports. Python flat-Win32
generation remains explicitly unsupported.

Flags declarations permit normal bitwise combinations while ordinary enums
retain their member types. ABI-width validation remains separate from the
TypeScript surface. CI uploads the complete `dist` runtime tree, matching the
npm distribution boundary, rather than maintaining a second filename list.
It then downloads that artifact and checks every public package entrypoint
through CommonJS, ESM and native dispatch.

## Tests

Behavior is protected by ordinary tests, not a frozen copy of a previous
PR's implementation or hashes of Rust `Debug` output:

| Tests | Assertions |
| --- | --- |
| Codegen Win32 unit tests | Exact contract selectors and drift rejection, typed ABI and ownership plans, count/size relationships, native layouts, builders, return conventions and generated behavior. |
| Core Win32 unit tests | Real FFI scalar/aggregate calls, output ordering, success/failure and cleanup, handle leases and consuming calls. |
| JS Win32 tests | Native carrier identity, argument/descriptor validation, encoded strings and buffers, resource lifetimes, IOCP cancellation/capacity and subsystem state. |
| Win32 CLI tests | Namespace/enum files, relative runtime imports, CJS/ESM resolution, missing or malformed output, atomic failure/rollback and retry, incremental regeneration and coexistence with WinRT/COM. |
| Generated-module tests | Load every currently admitted JS/enum module as CJS and ESM without native dispatch, and compile its declarations with TypeScript. Expected behavior comes from actual exports and type rules, not stored implementation hashes. |
| Win32 E2E runners | Actual Registry, aggregate, resource, IOCP, subsystem and generated declaration behavior. |

The existing CI jobs run these tests together with WinRT and Classic COM
regressions. The census minimum is an additional coverage signal, not proof
of correctness or a substitute for explicit behavioral assertions.
`npm test` includes Win32 consumer typechecking; the existing CI jobs run the
Rust binding tests and Node suite in addition to generated E2E scenarios.

Real-metadata tests read `DYNWINRT_WIN32_WINMD`. CI also sets
`DYNWINRT_REQUIRE_WIN32_METADATA=1` so missing metadata is a failure, not an
unnoticed skip. Live scenarios require stock Windows; optional-device and
ARM64 execution coverage must be reported separately from compilation.
Native subsystem lifecycle tests run in isolated, time-bounded processes with
initialization/cleanup stage output, so a platform startup failure cannot
silently block unrelated tests behind a process-global lock.

Winsock, GDI+ and Media Foundation lifecycle tests call the native OS APIs.
MAPI lifecycle tests use a test-only function table because stock CI images
do not supply an Extended MAPI provider. These mandatory tests exercise the
same context, counting and call-guard implementation, asserting reserved flags,
first-lease initialization, last-lease cleanup, retries after initialization
failure, missing exports, aliases, idempotent close, `Drop` and concurrent close.
Architecture-specific system export resolution is also mandatory and does
not invoke the provider.

The real MAPI lifecycle test remains available separately on a machine with a
configured provider (for example, matching-bitness Outlook). It is ignored by
default, fails on initialization errors/timeouts, and never treats an unavailable
provider as success:

```powershell
cargo test -p jswinrt_rs --lib win32_subsystem::tests::mapi_utility_contexts_use_installed_provider -- --exact --ignored --nocapture
```
