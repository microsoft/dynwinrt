# Contract-driven flat Win32

Flat DLL exports are a separate frontend, not COM vtable methods or WinRT
activation. This first migration slice deliberately supports only:

| Namespace | Exports |
| --- | --- |
| `Windows.Win32.System.SystemInformation` | `GetTickCount`, `GetTickCount64` |
| `Windows.Win32.System.Registry` | `RegOpenKeyExW`, `RegQueryValueExW`, `RegCloseKey` |

Other exports receive explicit omission diagnostics. Selecting an entirely
unsupported container fails generation instead of producing an empty binding.
This is not a claim of complete support for either namespace.

## Contract boundary

```text
Windows.Win32.winmd + contracts/win32
    -> validated Win32 semantic contracts
    -> typed JavaScript projection
    -> namespace modules
    -> immutable dynamic native call plans
```

`tools/dynwinrt-codegen/contracts/win32/` is the Win32-specific source of
method contracts that metadata cannot establish by itself. COM retains its
independent interface/IID/vtable contract registry. Both domains follow the
[contract evidence principles](classic-com-contract-evidence-registry.md);
their ABI models and selectors are not interchangeable.

Win32 selectors identify the namespace, DLL, entry point, complete native
signature and architecture. Built-in entries require authoritative evidence
and an exact match to the pinned Win32 metadata. Schema and manifest validation
reject unknown fields, unsupported contract kinds, duplicates and drift.
The registry is embedded at build time, not loaded from application-provided
JSON at runtime.

Contracts describe native facts: ownership, cleanup, directions, reserved
inputs, count/capacity relationships and status behavior. They do not contain
JavaScript, TypeScript declarations, arbitrary native addresses, renderer
fragments or unconstrained cleanup names. The renderer consumes typed
projection decisions, not API-name switches or registry entries.

The first slice exercises reusable scalar/status returns, owned and borrowed
handles, consuming cleanup, UTF-16 input, and a nullable counted byte buffer.
Registry size probes and `ERROR_MORE_DATA` are normal status results, not
successful populated output data. Consuming operations require the managed
resource rather than a numeric alias, and resource state changes are
synchronized with native invocation.

`Win32Handle.hkey(bigint)` constructs an explicitly borrowed handle.
`regOpenKeyExW` returns `{ status, phkResult }`; a new key is a
`Win32Resource`, while a refreshed predefined key remains a borrowed
`Win32Handle`. `regCloseKey` accepts only a resource and is idempotent after a
successful close.

`regQueryValueExW` returns `{ status, lpType, lpData, lpcbData }`. The input
buffer supplies capacity, not mutable output storage: returned `lpData` is
privately staged, owned bytes and the input remains unchanged. A null input
probes the required size. Status 234 preserves `lpcbData` but returns null
data/type; other failures return null outputs. `HKEY_PERFORMANCE_DATA` is
excluded because its documented failure-size contract differs. A successful
native result with a count larger than capacity is rejected, not truncated.

## Package and output boundary

The runtime root `@microsoft/dynwinrt` remains WinRT-only. Safe flat Win32
helpers live under `@microsoft/dynwinrt/win32`; manual native call-plan access
requires `@microsoft/dynwinrt/win32/unsafe`. Generated safe code may use that
low-level plan facility only after metadata and contract validation.

Generated files use the existing lowercase/kebab namespace directory mapping:

```text
generated/
  package.json
  win32/
    .dynwinrt-win32-manifest.json
    index.js
    index.mjs
    index.d.ts
    windows/win32/system/registry/
      Apis.js
      Apis.d.ts
      index.js
      index.mjs
      index.d.ts
```

The output manifest records namespace exports and hashes of generated
implementation/declaration modules. Incremental generation rejects drift and
retains other namespaces. The CLI uses the same whole-output transaction as
WinRT and COM, so a failed batch does not publish earlier partial results.
`--dry-run` validates without writing files. Python projection is not included
in this slice and fails explicitly.
Namespaces that also contain Classic COM interfaces retain the explicit
`--class-name` selection requirement; select their `Apis` container for flat
exports instead of changing the existing COM namespace behavior.

Win32-only packages have no package-root entrypoint. Mixed generation preserves
the WinRT root exports and adds only explicit `win32` subpaths. Namespace
imports use, for example,
`@winapp/bindings/win32/windows/win32/system/registry`.
The root `win32` barrel exports qualified namespace objects, not unqualified
functions that can collide across namespaces.

## Deferred migration

The previous broad Win32 implementation is reference material, not a coverage
baseline for this slice. The following capabilities remain separate follow-up
work and must not be inferred from the five admitted exports:

- native aggregate layouts, flexible arrays and discriminated pointer graphs;
- additional allocators, borrowed outputs and contextual resource cleanup;
- IOCP operations, cancellation and retained asynchronous storage;
- subsystem startup, parent/child sessions and managed callbacks;
- network, security, graphics, messaging and JSRT-specific state machines;
- Windows Search consumer COM support, tracked independently of flat Win32.

Each migration should add a validated contract or a reusable semantic
mechanism, not a new family of renderer exceptions. Unknown ownership,
unbounded pointers and callback lifetimes remain fail-closed.
