# Generated High-Level Unsafe COM Companions

This document defines how dynwinrt-codegen exposes metadata-derived Classic COM
methods that cannot satisfy the complete safe projection contract but can be
executed through the semantic-unsafe or raw-ABI runtime.

## Decision

The ordinary `generate` command performs capability classification
automatically. Users do not select a `raw` projection mode and do not need to
know the classification before generation.

Codegen emits the highest supported API level under a distinct public identity:

```text
safe complete              -> IFoo
unsafe outbound companion  -> IFooUnsafe
manual-contract method     -> IFooUnsafe + required strategies
runtime blocked method     -> support report only
```

Unsafe output is never substituted for a safe symbol with the same name. Safe
generation never imports or falls back to raw APIs.

## Why this builds on Phase 1

The completed raw Phase 1 implementation supplies:

- aligned owned and bounded external memory;
- pointer slots and pointer depths through caller storage;
- natural C-POD struct/union descriptors;
- outbound by-value, pointer, Out, InOut, and direct-return ABI;
- managed/raw COM reference transitions;
- explicit standard cleanup operations; and
- x64/i686 MSVC C ABI evidence.

Generated unsafe companions hide interface registration, IID, inheritance,
absolute slots, calling convention, architecture selection, and signature
construction. They do not remove the caller's responsibility for any fact that
metadata does not contain.

## Output layout

```text
generated/
└── com/
    ├── IFoo.js
    ├── IFoo.d.ts
    ├── index.js
    └── unsafe/
        ├── Windows/Win32/System/Wmi/
        │   ├── IWbemServicesUnsafe.js
        │   └── IWbemServicesUnsafe.d.ts
        ├── index.js
        ├── index.mjs
        ├── index.d.ts
        ├── runtime.js
        ├── runtime.d.ts
        ├── package.json
        └── support.json
```

Rules:

- `generated/com/index.js` exports only safe classes and values.
- `generated/com/unsafe/index.js`, `index.mjs`, and `index.d.ts` export only
  `*Unsafe` companions.
- unsafe companions import their runtime machinery from
  `@microsoft/dynwinrt/com/unsafe/raw`.
- the npm package root and `@microsoft/dynwinrt/com` remain unchanged.
- a safe-complete interface does not need a duplicate unsafe companion unless a
  future explicit diagnostic feature requests one.
- unsafe module paths include every namespace segment. The class inside remains
  the ergonomic short `<Interface>Unsafe` name.

## Class and method naming

The unsafe boundary is carried by the module path and class suffix:

```js
import { IAudioClientUnsafe } from "./generated/com/unsafe/index.js";
```

Methods keep their natural projected names:

```js
audio.isFormatSupported(...);
```

Do not emit a class named `IAudioClient` for an unsafe projection. Method names
need an `Unsafe` suffix only if safe and unsafe methods are ever placed on the
same class; the preferred design keeps them in separate companion classes.

`Contoso.A.IFoo` and `Contoso.B.IFoo` therefore generate:

```text
com/unsafe/Contoso/A/IFooUnsafe.js
com/unsafe/Contoso/B/IFooUnsafe.js
```

If an unsafe class short name is globally unique in `support.json`, the unsafe
barrel exports it. If multiple namespaces use that short name, the barrel omits
the ambiguous export and consumers use the deep module path. Incremental
generation recomputes uniqueness from all retained support entries, so either
generation order converges to identical barrels. Removing one root restores
the remaining unique short export.

Every executable method declaration includes `@unsafe` documentation.
Non-executable methods and their exact classifier reasons remain visible in
`support.json`.

## Classification

Codegen uses the same capability classifier as `com-capability-census`.

### Safe complete

The complete inherited interface passes the existing safe semantic projection.
Generate the existing safe class and no unsafe replacement.

### Raw metadata complete

Metadata contains every fact needed to express the outbound ABI through Phase 1
raw primitives. Generate an executable `*Unsafe` companion without requiring a
user contract.

The companion may still require:

- a COM object acquired by another API;
- execution in the current valid apartment;
- caller-created native storage; or
- explicit cleanup after the call.

These requirements are represented in the method signature and documentation.

### Raw manual contract

The outbound ABI can be executed, but metadata does not prove one or more of:

- output ownership;
- allocator or cleanup;
- count/capacity/actual relationship;
- handle ownership;
- typed interface InOut old/new ownership and replacement semantics;
- opaque pointee meaning;
- nested pointer lifetime; or
- externally supplied pointee storage.

Stage 2 emits these methods only with closed high-level strategy arguments.
Missing facts are never filled with a guessed default.

Examples:

```text
UnsafeOutput.comOwned(iid)
UnsafeOutput.coTaskMem(layout)
UnsafeOutput.borrowedPointer(size)
UnsafeInterfaceReplacement.consumesOld(value)
UnsafeInterfaceReplacement.preservesOld(value)
UnsafePointee.required(memoryOrPointer)
UnsafePointee.nullable(memoryOrPointer)
```

Each parameter requirement is derived independently from the shared classifier.
If a semantic fact cannot be elevated safely, that individual parameter or
method requires `UnsafeRawCall`; IID, slot, signature, registration, and native
argument ordering remain generated.

### Raw runtime blocked

Do not emit an executable method when the current target cannot express its ABI.
Record the method and blocker in `support.json`.

An interface may receive a partial outbound unsafe companion containing only
callable methods. Partial companions are never used to implement a COM
interface.

## Call versus implementation

Outbound calls may be generated per method because invoking one known vtable
slot does not require the runtime to execute other slots.

JavaScript COM implementation requires a complete contiguous callback vtable:

```text
outbound unsafe call  -> method-level capability is allowed
interface implement   -> complete interface validation remains mandatory
```

Generated unsafe companions do not add raw callback implementation.

## Generated API shape

An unsafe companion wraps an existing managed native value:

```ts
export declare class IWbemServicesUnsafe {
  private constructor();

  static from(
    value: DynWinRtValue | { readonly nativeValue: DynWinRtValue },
  ): IWbemServicesUnsafe;
  static readonly iid: WinGuid;
  static readonly support: UnsafeInterfaceSupport;
  readonly nativeValue: DynWinRtValue;

  /**
   * @unsafe Metadata-complete outbound ABI.
   */
  openNamespace(
    namespace: string,
    flags: number,
    context: DynWinRtValue,
    workingNamespace: DynComRawMemory | DynComRawPointer,
    callResult: DynComRawMemory | DynComRawPointer,
  ): void;

  release(): void;
}
```

The implementation internally owns:

- interface registration;
- method signature construction;
- architecture-specific aggregate descriptors;
- native argument conversion; and
- invocation of the correct absolute slot.

Scalar, Boolean, `bigint`, GUID, enum, BSTR, and HSTRING inputs use their exact
natural projections where available. Pointer-shaped caller storage is
`DynComRawMemory | DynComRawPointer`; by-value aggregates remain branded
`DynWinRtValue` values created by the Phase 1 layout API. Native argument order
is preserved. Out and InOut slots remain caller-visible even when HRESULT
failure throws.

Native `isize` and `usize` are distinct generated conversions. Inputs require
`bigint` and use `DynCom.isize()` / `DynCom.usize()`. Direct results use
`DynCom.toIsizeBigint()` / `DynCom.toUsizeBigint()`, which accept only I32/U32
on i686 and I64/U64 on 64-bit targets. Generated code never converts a
pointer-width value through a JavaScript `number`.

`from()` performs QueryInterface and owns the resulting `+1`. `release()` is
deterministic and idempotent; finalization remains a fallback. The companion
does not silently adopt or clean up caller-owned raw slots.

## Generated strategy runtime

Every unsafe package contains shared, byte-identical `runtime.js` and
`runtime.d.ts`. The unsafe CJS/ESM/declaration barrels export:

- `UnsafePointee.required/nullable(memoryOrPointer)` for explicit external
  storage and owner retention;
- `UnsafePointerOutput.unclassified`, `borrowed`, `comOwned`, `coTaskMem`,
  `bstr`, `localAlloc`, `globalAlloc`, and `rawResponsibility`;
- `UnsafeHandleOutput.borrowed`, `closeHandle`, `destroyIcon`, `deleteObject`,
  and `rawResponsibility`;
- `UnsafeInterfaceReplacement.consumesOld`, `preservesOld`, and `unchanged`;
- `UnsafeCountedBuffer.required/nullable`;
- `UnsafeRawCall.value` for a raw parameter and
  `UnsafeRawCall.acknowledge` for a method-level fallback; and
- `UnsafeOwnedPointer` / `UnsafeInterfaceReplacementResult` result owners.

All constructors are private and every mode has a named factory. There are no
user-provided string modes or allocator defaults.

Strategy state is held only in module-private `WeakMap`s. Instances, public
prototypes, and constructors are frozen; factories always construct the exact
base class rather than `new this`. Generated calls use private branded
prepare/span/finish/failure helpers that are not exported through public
barrels or declarations. Subclasses, proxies, plain objects, prototype swaps,
field mutation, and method overrides therefore fail before native dispatch.
Preparation returns an empty frozen opaque record; a second private `WeakMap`
associates that record with the real state. Internal CommonJS helper properties
are non-writable and non-configurable, and companions capture those verified
functions at module load. Production runtime output contains no finalizer or
owner test hooks; tests request a separate test-only rendering.

Required pointee/count/output/replacement strategies resolve their native
pointer before invocation and reject literal null, `DynComRawPointer.null()`,
zero addresses, released storage, undersized pointer slots, and misaligned
pointer slots. Nullable factories intentionally admit a null pointer.

Every generated pointee requirement records its native `in`, `out`, or
`inout` direction, metadata nullability, and target-specific pointee
size/alignment when the layout is known. An input-only `UnsafePointee` retains
the existing bounded-memory or raw-pointer contract. A writable pointee
requires live bounded `DynComRawMemory`; known layouts enforce minimum size
and alignment, while an unknown layout uses the caller's complete bounded
memory range as its explicit writable span.

Preparation also returns exact writable native spans using checked `bigint`
address arithmetic. Generated invocation rejects duplicate or overlapping
output, handle, replacement, counted-buffer, count, and actual-length storage
before dispatch, including writable pointees and aliases made through separate
bounded views. Ordinary writable raw/count slots in a manual method receive
private preparation records too, so they participate in the same overlap
graph without changing their public declaration.

Pointer and handle output strategies zero and prepare an exact pointer-sized
slot, validate required/null results, and consume once. A COM-owned strategy
requires a native `WinGuid` at factory creation. Result extraction first uses
QueryInterface without consuming the output slot, then clears the slot and
releases its original transferred `+1` without repeating the IID conversion.
An IID mismatch therefore leaves the original output available for exact
transaction cleanup. CoTaskMem, BSTR, Local, Global, CloseHandle, DestroyIcon,
and DeleteObject results return an `UnsafeOwnedPointer` whose idempotent
`release()` uses only the selected cleanup. Raw/borrowed results remain
unowned.

Generated methods wrap invocation in a cleanup boundary. If native HRESULT
handling throws after writing a dirty output, every prepared output strategy
runs its selected failure cleanup in reverse order before the original error is
rethrown. If cleanup also fails, an `AggregateError` contains both failures.
Strategy type mismatch and reuse fail before native dispatch.

The raw runtime tracks dispatch at the core call boundary immediately after
all target and argument validation and immediately before the native executor.
Generated code does not infer dispatch from a JavaScript call attempt. This
distinguishes a pre-dispatch validation failure, which may roll back a
consumes-old activation, from an HRESULT or conversion failure after native
code ran.

Invocation and result extraction are one transaction. Direct conversion and
every strategy finish occur under the same rollback boundary. A first, middle,
or last extraction failure cleans all unfinished dirty outputs and releases
already extracted managed COM values, owned pointers/handles, and replacement
results in reverse order. Raw extracted pointers remain caller-owned and are
attached to the thrown error rather than silently released.

`unclassified`, `borrowed`, and `rawResponsibility` preserve dirty failure
pointers. The thrown error receives frozen `unsafeOutputs` metadata and the
strategy exposes one-shot `takeFailurePointer()`; taking twice fails. Automatic
cleanup modes continue to clean dirty failure output.

`UnsafeOwnedPointer` uses private cleanup state plus `FinalizationRegistry` as
a fallback for abandoned CoTaskMem, BSTR, Local, Global, HANDLE, HICON, and GDI
owners. Explicit `release()` unregisters only after successful cleanup; failure
keeps the owner retryable. Finalizers cannot throw, so a cleanup failure there
is conservatively leaked rather than retried with unknown partial effects.
Managed COM values continue to use their existing apartment-aware native
owners and are never registered with this finalizer.

Interface replacement strategies use existing
`DynComRawOwnedComPointer.transferTo`/`assumeTransferred` primitives. The
caller explicitly asserts consumes-old, preserves-old, or unchanged behavior;
the result owns exactly the surviving old/new references and releases each at
most once. A generated parameter uses this strategy only when its own
classifier reason is exactly `missing_interface_replacement_contract` and its
ABI shape is an InOut `IFoo**`. Exact method evidence may suppress that reason
only when it records the old/new ownership semantics. Ordinary `IFoo*`, typed
Out parameters, deeper pointers, and interface parameters on methods manual
for unrelated reasons retain their native pointer depth.
Consumes-old transfer is the last activation phase. If a later pre-dispatch
check fails, activated replacements roll back in reverse order, re-adopt the
slot reference, and remain retryable; callee-consumed reconciliation begins
only after the core dispatch marker is set.
Preserves-old and unchanged calls retain a private independent `+1` through
native dispatch. Reentrant JavaScript release therefore becomes logically
visible without allowing the native method to observe a freed object; if the
original owner was released, the retained reference moves into the successful
replacement result.

The same `DynComRawOwnedComPointer` object cannot back two prepared replacement
strategies in any mode. Validation rejects consumes/consumes,
preserves/unchanged, and mixed aliases before any slot write or owner transfer.
Callers must create one independent `+1` owner per native slot with
`DynComRawOwnedComPointer.addRef` or `queryInterface`.

`UnsafeCountedBuffer` is likewise selected only when that exact parameter has
`missing_count_relation`. A method-level or unassigned count reason requires
`UnsafeRawCall.acknowledge()` and never changes an unrelated pointer
parameter.

`IWbemServices::OpenNamespace` is an exact exception to the generic typed
interface InOut rule. Win32 metadata 71.0.14 marks both output slots InOut, but
the Windows SDK `WbemCli.h`/`WbemIdl.idl` declaration and Microsoft Learn
syntax declare `[out] IWbemServices**` and `[out] IWbemCallResult**`. The
override is applied atomically only when namespace, interface IID, method,
absolute slot, parameter count/index/name, optional flag, and complete typed
pointer shape match. Both native outputs are optional and exactly one may be supplied. Generated
code hides `pCtx` and always passes native null. Synchronous options require
`lFlags: 0` plus an initially null working-namespace slot. Semisynchronous
options require exact `lFlags: 16` (`WBEM_FLAG_RETURN_IMMEDIATELY`), a
native-null working argument, and an initially null result slot. The requested
successful output must be non-null and is adopted into
`DynComRawOwnedComPointer`; dirty failure output is nulled and released before
rethrowing.

## Registration

The current chained API remains the lowest-level escape hatch:

```js
DynComUnsafe.registerIUnknownInterface(name, iid).addMethodAt(
  slot,
  methodName,
  signature,
);
```

Generated code may initially emit this form. The preferred runtime evolution is
an atomic descriptor API:

```js
DynComUnsafe.registerRawInterface({
  name,
  iid,
  root,
  metadata,
  methods,
});
```

Atomic registration must:

- validate every included slot before publication;
- reject duplicate or conflicting slots;
- reject an incompatible registration for an existing IID;
- freeze published descriptors;
- retain metadata and signature fingerprints; and
- publish no partial interface after an error.

## Metadata and signature fingerprints

Every generated unsafe companion records:

- every emission, reference, auto-detected, and sibling winmd actually loaded;
- basename, package/version where known, and SHA-256 for each file;
- a deterministic metadata-set SHA-256 over the sorted, deduplicated file
  identities;
- the defining file when one exact TypeDef owner can be identified;
- interface IID;
- declaring IID for each method;
- absolute vtable slot; and
- a canonical native signature hash.

Stage 1 has no user-supplied contracts. Fingerprints make generated support
reports and stale artifacts auditable; Stage 2 contracts can use them for
explicit drift rejection.

No local absolute path is serialized. If the defining file cannot be
identified uniquely, `definingFile` is `null` and the complete metadata set is
still recorded rather than attributing the interface to the first command-line
path.

## Support manifest

Generation emits `generated/com/unsafe/support.json`:

```json
{
  "schemaVersion": 10,
  "interfaces": [
    {
      "schemaVersion": 10,
      "metadata": {
        "setSha256": "...",
        "files": [
          {
            "file": "Windows.Win32.winmd",
            "package": "Microsoft.Windows.SDK.Win32Metadata",
            "version": "71.0.14-preview",
            "sha256": "..."
          }
        ],
        "definingFile": {
          "file": "Windows.Win32.winmd",
          "package": "Microsoft.Windows.SDK.Win32Metadata",
          "version": "71.0.14-preview",
          "sha256": "..."
        }
      },
      "interfaceName": "Windows.Win32.System.Wmi.IWbemServices",
      "interfaceIid": "9556dc99-828c-11cf-a37e-00aa003240c7",
      "root": "IUnknown",
      "baseIids": [],
      "unsafeClass": "IWbemServicesUnsafe",
      "modulePath": "Windows/Win32/System/Wmi/IWbemServicesUnsafe",
      "methods": [
        {
          "name": "OpenNamespace",
          "projectedName": "openNamespace",
          "declaringIid": "9556dc99-828c-11cf-a37e-00aa003240c7",
          "absoluteSlot": 3,
          "signatureFingerprint": "...",
          "status": "raw_metadata_complete",
          "reasons": [],
          "strategyRequirements": [],
          "exactInterfaceOutputs": [
            {
              "entryId": "wmi.conditional-output.entry.windows-win32-system-wmi.iwbemservices.9556dc99828c11cfa37e00aa003240c7.opennamespace.slot-3.v1",
              "familyId": "wmi.conditional-output.v1",
              "contractKind": "conditional-output",
              "parameterIndex": 3,
              "parameterName": "ppWorkingNamespace",
              "interfaceIid": "9556dc99-828c-11cf-a37e-00aa003240c7",
              "argumentOptional": true,
              "nullableOnSuccess": false,
              "reason": "exact [out] owned +1 contract",
              "citation": "Microsoft Learn and Windows SDK WbemCli.h/WbemIdl.idl"
            },
            {
              "entryId": "wmi.conditional-output.entry.windows-win32-system-wmi.iwbemservices.9556dc99828c11cfa37e00aa003240c7.opennamespace.slot-3.v1",
              "familyId": "wmi.conditional-output.v1",
              "contractKind": "conditional-output",
              "parameterIndex": 4,
              "parameterName": "ppResult",
              "interfaceIid": "44aca675-e8fc-11d0-a07c-00c04fb68820",
              "argumentOptional": true,
              "nullableOnSuccess": false,
              "reason": "exact [out] owned +1 contract",
              "citation": "Microsoft Learn and Windows SDK WbemCli.h/WbemIdl.idl"
            }
          ],
          "exactInterfaceOutputCall": {
            "entryId": "wmi.conditional-output.entry.windows-win32-system-wmi.iwbemservices.9556dc99828c11cfa37e00aa003240c7.opennamespace.slot-3.v1",
            "familyId": "wmi.conditional-output.v1",
            "contractKind": "conditional-output",
            "sourceFingerprint": "EA3628EB9E45E1A0BAA0BC9F6DA1FD82FE938091EF1730E25E3CCEEA9EFD316B",
            "flagsParamIndex": 1,
            "contextParamIndex": 2,
            "synchronousOutputParamIndex": 3,
            "semisynchronousOutputParamIndex": 4,
            "synchronousFlags": 0,
            "semisynchronousFlagValue": 16,
            "flagsOptionName": "lFlags",
            "synchronousOutputOptionName": "workingNamespace",
            "semisynchronousOutputOptionName": "result",
            "reason": "exact OpenNamespace mode contract",
            "citation": "Microsoft Learn and Windows SDK WbemCli.h/WbemIdl.idl"
          },
          "targets": {
            "x64": {
              "classification": "raw_metadata_complete"
            }
          }
        }
      ]
    }
  ]
}
```

The manifest is deterministic, metadata-pinned, and generated from the same
classifier as the capability census.

## Command behavior

The command line remains unchanged:

```powershell
dynwinrt-codegen generate `
  --winmd Windows.Win32.winmd `
  --namespace Windows.Win32.System.Wmi `
  --class-name IWbemServices `
  --output generated
```

Possible outcomes:

| Result                                      | Exit behavior                               |
| ------------------------------------------- | ------------------------------------------- |
| Safe class emitted                          | Success                                     |
| At least one callable unsafe method emitted | Success, with an explicit unsafe summary    |
| Only support report emitted                 | Report is committed, then the command fails |

`--dry-run` reports safe, unsafe metadata-complete, manual-contract, and blocked
method counts and exact reasons without writing files. A report-only dry run
also returns nonzero.

Example output:

```text
[dry-run] Would generate IWbemServicesUnsafe (metadata-complete: 23, manual: 0, blocked: 0)
[dry-run] Report-only MFASYNCRESULTUnsafe {"metadataComplete":0,"manual":0,"blocked":5,"reasons":["missing_interface_iid"]}
```

Report-only generation transactionally merges `support.json`, removes only
stale callable files owned by that interface root, emits no class `.js` or
`.d.ts`, finalizes package metadata, and then returns the documented nonzero
result.

## Unified locking and output transaction

Every non-dry generation acquires the same exclusive OS-backed lock for its
output root before migration, snapshot, cleanup, projection output, or
manifest/barrel/package changes. This includes WinRT-only, safe COM, unsafe
COM, mixed, report-only, and Python generation. Python does not share a
language package with JavaScript by design, but using the same lock prevents
two commands pointed at the same filesystem root from racing.

The lock file is a sibling of the replaceable output directory, so replacing
the root cannot change the locked inode. Closing or crashing the process
releases the kernel lock; the persistent file is not a stale `create_new`
sentinel. Contention is retried for up to 120 seconds and then reported as a
generation error.

The complete existing output is copied to a sibling stage directory while the
lock is held. All affected root files, the entire `com` subtree, support and
generation manifests, barrels, package files, cleanup, and validation happen
only in that stage. Commit renames the old root to a backup and the completed
stage to the final path. Successful stage-to-final publication is the commit
point. Failures before publication restore the untouched backup; after
publication the complete new final root is authoritative and is never replaced
with a backup whose cleanup may have partially succeeded.

Snapshot uses `symlink_metadata` plus Windows reparse attributes before any
directory decision. Supported file symlinks, directory symlinks, and junctions
are recorded by validated relative path and are never traversed or copied.
After the old root is renamed to backup, commit moves each retained link
directory entry itself from backup into stage; this preserves its target
without requiring symlink recreation or developer-mode privileges. A failure
while moving links or before publication moves them back in reverse order
before restoring the backup. After publication, the links are part of the
authoritative final root.

Backup cleanup failure is a nonfatal warning. It retains the new committed root
and any orphan backup residue. The next generation retries deterministic
residue cleanup while holding the same output lock. Recovery handles:

- an old final root with no residue;
- backup-only interruption before publication, which restores the untouched
  backup;
- final plus backup, which always keeps final and removes only backup residue;
- complete failed-output residue without final, which is preferred over a
  possibly partial backup; and
- abandoned staging residue, which is never published.

Stage, backup, failed-output, and orphan-backup cleanup recursively inspects
entries without following reparses. Link entries are removed with
`remove_file`/`remove_dir`; their targets, including targets outside the output
root, are never deleted. Reparse entries that `std::fs::read_link` cannot
identify as supported filesystem links fail closed.

## Windows path identity and retained ownership

All generated and retained COM paths use one canonical Windows key: validated
ASCII segments, `/` normalization, and ASCII case folding. Validation rejects
absolute/rooted paths, drive or ADS colons, NUL, empty/`.`/`..` segments,
trailing dots/spaces, and Windows device names (`CON`, `PRN`, `AUX`, `NUL`,
`CLOCK$`, `CONIN$`, `CONOUT$`, `COM1`-`COM9`, and `LPT1`-`LPT9`), including
device names with extensions.

Retained schema-9 `modulePath` is never trusted. Codegen rederives it from the
validated qualified interface identity and exact `<Interface>Unsafe` class
name, requires an exact match, and then checks the case-insensitive path key.
Case-only namespaces or type names therefore fail instead of aliasing on
Windows.

Before overwriting a staged path owned by any manifest root outside the current
update set, codegen requires the retained and planned path identities to match,
requires the staged file to exist, and byte-compares its contents. Shared
ownership is admitted only for the exact same public path and bytes; different
content, missing files, case aliases, traversal, or reserved names abort before
publication and leave the prior root and manifest unchanged.

Unmanaged retained links use the same canonical Windows path key. A link is
rejected if it aliases another link by case, conflicts with a generated stage
file or descendant, overlaps manifest ownership, or uses transaction
lock/residue naming. Ordinary parent directories required to contain a link
remain allowed.

## Compatibility invariants

- Existing safe-complete generated output is byte-for-byte unchanged.
- Existing safe output paths and package exports are unchanged.
- Unsafe symbols never appear in a safe barrel or safe declaration file.
- Existing WinRT generation is unchanged.
- Existing Classic COM safe generation never changes a class into an unsafe
  class with the same name.
- A generated unsafe companion never claims complete-interface implementation
  support.
- Regenerating one root updates only files owned by that root.
- Concurrent processes targeting the same output serialize the complete
  incremental merge, including successful classes and report-only entries.
- The entire output root is replaced transactionally. COM is never published
  before root compatibility barrels and package metadata are ready.

## Validation

Required tests include:

1. unchanged safe snapshots and package boundaries;
2. automatic raw-metadata-complete companion generation;
3. manual-contract and blocked methods omitted and reported;
4. partial outbound companions containing only metadata-complete methods;
5. metadata/signature fingerprint drift;
6. duplicate IID/slot registration;
7. CJS, ESM, and `.d.ts` output;
8. no unsafe symbol leakage;
9. x64 and i686 live raw calls;
10. ARM64 compile/gate behavior;
11. deterministic support manifest generation; and
12. complete safe COM and WinRT regression suites.

The Stage 1 generated-companion integration fixture uses official
`IWbemServicesUnsafe::queryObjectSink` (IID
`9556dc99-828c-11cf-a37e-00aa003240c7`, absolute slot 5). It was selected
because the real metadata-complete signature combines generated scalar
conversion with an explicit interface-pointer output slot but does not require
a live WMI service. The test-hook object implements real IUnknown
QueryInterface/AddRef/Release plus the exact slot-5 ABI, writes a deterministic
pointer value, and exposes refcount/call statistics. CI generates the actual
deep module, loads CJS and ESM forms, type-checks its emitted declaration, calls
`.from()` and `queryObjectSink`, and checks idempotent/post-release behavior.

Stage 2 extends the same generated-artifact test with official
`IAudioClientUnsafe::isFormatSupported` and `getService` vtable slots. It covers
CoTaskMem closest-format success/failure cleanup, BSTR/Local/Global dirty
failure cleanup, COM-owned output cleanup, required/nullable output, strategy
mismatch before dispatch, one-shot reuse, handle/raw/count strategies, and all
three interface-replacement modes.

## Rollout plan

### Stage 1

- Generate `*Unsafe` companions for `raw_metadata_complete` methods.
- Hide all manual registration details.
- Emit deterministic support manifests.

Stage 1 is implemented. A method is executable only when the shared classifier
reports `raw_metadata_complete` for x64, i686, and ARM64; this keeps one
generated package deterministic across target machines. If safe projection
fails and no method meets that rule, generation publishes only the support
report and then fails without a partial class. Missing/zero interface IID,
unsupported root, and non-addressable identity blockers are applied to every
method before selection, so generated code can never evaluate
`WinGuid.parse('')`. Runtime registration still uses the existing immutable
chained interface API internally.

### Stage 2

Stage 2 is implemented:

- `raw_manual_contract` methods are executable with required closed strategy
  arguments;
- parameter requirements come from the same per-target classifier analysis;
- runtime-blocked methods remain omitted;
- support schema 9 records every parameter index/name, strategy type, exact
  reason, native direction/nullability, and known target pointee layouts; and
- a raw pointer is never substituted for missing ownership.

For official 71.0.14 metadata, **1,550 of 1,554** x64 manual-contract interfaces
have at least one portable executable generated high-level method. **1,549**
have an executable manual method, one retains only metadata-complete methods,
and four have no portable executable method because every candidate is blocked
on another generated target. There are **6,343** portable executable manual
methods, **0** remaining portable manual-classified methods omitted, and
**1,163** cross-target runtime-blocked methods still omitted.

### Stage 3

- Add atomic `registerRawInterface`.
- Add more dedicated high-level resource wrappers.
- Move audited manual contracts into the safe semantic registry where possible.
