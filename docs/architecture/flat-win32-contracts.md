# Contract-driven flat Win32: full replacement

This migration must preserve the complete functionality of PR #102.
Architecture and namespace organization may change; supported capabilities
must not disappear. The earlier five-export foundation was not a completed
replacement. The full port now passes the frozen parity gate and original
stock-Windows scenarios described below. PR #102 remains open while the
replacement goes through integration review.

## Immutable baseline

The reference is commit `13e47588fc54b14cf11364af329aa882ddefaddc`,
the committed head of #102, using
`Microsoft.Windows.SDK.Win32Metadata 71.0.14-preview` with SHA-256
`B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D`.

Its measured inventory contains 18,321 eligible exports and 8,936 complete
projections: 8,934 synchronous functions and two asynchronous helpers. There
are 182 supported containers and 10,680 public exports including aliases,
enums and helper functions. These are generation counts, not a claim that
every API has been exercised against a live system.

`tools/dynwinrt-codegen/tests/fixtures/win32-pr102.json` was generated from the
detached reference implementation, not from this migration. It records
native/public identities, subsystem requirements and semantic fingerprints.
Do not regenerate it from current code to make a regression disappear.
`win32_parity_test` requires no missing reference functions, aliases or helpers,
and detects changes to ABI plans, inputs/results, enums and native builders.
A higher total function count cannot compensate for a missing reference API.

The reference export identities, public names and helpers, default ABI/input/
result plans, enum values, native builders and subsystem requirements all
match. The three original samples and Win32 runners also execute through the
new namespace layout. WinRT JavaScript/Python and Classic COM live scenarios
remain covered independently; no claim is made that all 8,936 functions were
executed against real devices or optional Windows components.

## Required capabilities

| Area | Required reference behavior |
| --- | --- |
| Native ABI | All admitted scalars, enums, GUID/scalar pointers, native structures/unions, pointer and by-value aggregate shapes; system/cdecl calls and target availability. |
| Return contracts | Direct, void, status/HRESULT and LastError behavior, including expected failure results. |
| Strings and buffers | UTF-16/ANSI storage, double-NUL lists, reserved inputs, counted buffers, capacity/actual-size relationships and caller-owned output semantics. |
| Resources | Exact HKEY, HANDLE, HLOCAL, HGLOBAL, HMODULE, SC_HANDLE, CoTaskMem and credential cleanup; synchronized leases and consuming calls. |
| Native builders | Existing SECURITY_ATTRIBUTES, STARTUPINFOA/W, PROCESS_INFORMATION and POD helpers, including retained pointer fields and owned outputs. |
| COM inputs | Exact-IID, borrowed managed interface inputs without changing the WinRT model. |
| Asynchronous I/O | IOCP-backed ReadFile/WriteFile Promises, cancellation, EOF, retained storage, detachment checks and bounded pending work. |
| Subsystems | Existing Winsock, GDI+, Media Foundation and explicit unsafe MAPI utility context behavior. |
| Tooling | Safe/unsafe entrypoints, declarations, CLI/census, manifests, examples and original stock-Windows scenarios. |

The original `registry.mjs`, `returns.mjs` and `subsystems.mjs` scenarios are
retained, with imports adapted to the current namespace layout. The examples
under `samples/js/win32` remain part of the migration, not optional replacements
for missing runtime features.

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

Refactoring must not weaken an existing safety boundary to satisfy the
inventory. If a baseline behavior needs a correctness fix, implement and
document that fix and exercise its nearest failure case rather than silently
dropping the API or relabeling it as migrated.

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

Two Registry edge cases have explicit typed policies rather than capability
removal. Opening a null/empty subkey of a predefined HKEY returns a borrowed
handle, not a second owner. Performance-data queries remain supported:
successful results retain their size, while `HKEY_PERFORMANCE_DATA` combined
with `ERROR_MORE_DATA` exposes `dataSize: null` without reading an undefined
count. Callers can grow capacity and retry. Normal-key size queries retain
their prior behavior.

Native Win32 carriers use type tags rather than mutable JavaScript prototypes.
Manual aggregate descriptors and MAPI utility initialization are available on
the explicit unsafe subpath; generated validated helpers call those primitives
internally. The full `DynWin32*` capability set is retained across the Win32
entrypoints. x86 plan invocation remains explicitly unsupported as in #102;
x64 is exercised live and ARM64 is compile-validated.

The IOCP engine retains native state through terminal completion, including
cancellation. The reference limits are 1,024 pending operations, 64 MiB per
operation and 256 MiB of pending private buffers. A shared worker set is used,
not one blocked OS/libuv worker per operation. Subsystem close cannot race
dependent calls or retained operations.

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
generation remains explicitly unsupported, as in the reference.

## Separate archived work

The much larger previously uncommitted worktree is separately preserved.
Its additional JSRT, network/security, pointer-graph and other adapters are
not confused with the committed #102 baseline. They can be considered after
the complete #102 capability set has been migrated; they cannot justify
leaving reference functionality missing.
