# Classic COM API Support Plan

This document is the user-facing capability map and roadmap for the Classic COM
interface portion of `Windows.Win32.winmd`.

It answers three practical questions:

1. Which COM APIs work today?
2. Which APIs work only for a limited native contract or through an unsafe
   declaration?
3. Which APIs are not currently possible, and what work would unlock them?

For ABI details and implementation architecture, see
[Classic COM support](../architecture/classic-com-support.md). For raw unsafe
design, see
[Classic COM raw unsafe](../architecture/classic-com-raw-unsafe.md). For usage
examples, see the
[Classic COM JavaScript guide](../guides/windows/classic-com-usage.md).
Automatic high-level unsafe companion generation is specified in
[Generated high-level unsafe COM companions](../architecture/classic-com-generated-unsafe.md).

## Current baseline

The current census was produced from
`Microsoft.Windows.SDK.Win32Metadata` **71.0.14-preview**:

```text
Windows.Win32.winmd SHA-256:
B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D
```

| Result                                               | Interfaces | Percentage |
| ---------------------------------------------------- | ---------: | ---------: |
| Externally addressable Classic COM interfaces        |      7,929 |       100% |
| Complete safe generation                             |      5,697 |     71.85% |
| Rejected because at least one contract is incomplete |      2,232 |     28.15% |

The denominator contains addressable COM interface identities, not flat Win32
DLL exports. A complete interface means that its full inherited vtable can be
generated without guessing ABI, layout, count relationships, ownership, or
cleanup.

The 5,697 figure is semantic codegen coverage, not a claim that every interface
has a dedicated live Windows test or can be activated on every machine.

Reproduce the census with:

```powershell
npx dynwinrt-codegen com-census `
  --winmd C:\path\to\Windows.Win32.winmd `
  --json
```

## How we describe support

| Label           | What users can expect                                                                                                                                                         |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Verified live   | Generated code has been exercised against a real Windows COM implementation.                                                                                                  |
| Safe generated  | The complete interface passes metadata, ABI, layout, ownership, and cleanup validation. It may still require hardware, software, or an application-specific acquisition path. |
| Semantic unsafe | The caller manually declares a trusted IID and signature while selecting a dynwinrt-managed ownership plan such as COM, BSTR, or CoTaskMem.                                   |
| Raw unsafe      | The caller supplies native memory, pointers, pointer slots, layout, and cleanup. dynwinrt guarantees only ABI execution.                                                      |
| Not supported   | The required calling convention, threading, marshaling, server, or runtime primitive is unavailable.                                                                          |

Safe generation is determined per complete interface and metadata version. A
safe wrapper is not generated from only the methods an application happens to
call, and safe generation never falls back to an unsafe layer.

## APIs users can rely on today

These APIs represent the strongest support level because they combine complete
generation with real runtime coverage.

| Area                 | API examples                                                          | What is covered                                                                                                                                                         |
| -------------------- | --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| COM fundamentals     | `IUnknown`, `IClassFactory`                                           | Inheritance, QueryInterface, dynamic-IID output, owned `+1` references, class factory acquisition, and server locking.                                                  |
| Memory and streams   | `IMalloc`, `ISequentialStream`, `IStream`                             | Direct scalar/pointer/void returns, allocator identity, byte buffers, actual lengths, seek, `STATSTG`, interface outputs, and failure cleanup.                          |
| Binding and errors   | `IBindCtx`, `ICreateErrorInfo`, `IErrorInfo`                          | Validated POD layout, acquisition helpers, wide strings, BSTR output, thread-local error state, and deterministic release.                                              |
| Shell links          | `IShellLinkW`, `IPersistFile`                                         | Coclass activation, wide strings, enums, nested POD output, persistence interfaces, and QueryInterface views.                                                           |
| Shell UI             | `TaskbarList`, `ITaskbarList3`, `FileOperation`, `FileOpenDialog`     | Coclass construction, inherited slots, HWND input, unsigned flags, `u64`, and option/state round trips.                                                                 |
| Property System      | `IPropertyStore`                                                      | `PROPERTYKEY`, owned PROPVARIANT values, Set/Get/Commit, and cleanup.                                                                                                   |
| Automation core      | `IDispatch`                                                           | `GetIDsOfNames`, natural-order `DISPPARAMS`, optional Invoke outputs, EXCEPINFO, VARIANT results, and HRESULT failure information.                                      |
| Imaging              | `IWICImagingFactory`                                                  | Explicit CLSID activation and typed interface outputs.                                                                                                                  |
| WinRT interop        | `IDataTransferManagerInterop`, `ISystemMediaTransportControlsInterop` | HWND plus REFIID bridges and owned WinRT interface output.                                                                                                              |
| JavaScript COM sinks | `IFileDialogEvents`, `IDropTarget`                                    | Generated JavaScript implementations, real `Advise`/`Unadvise`, static and libffi callbacks, POD/InOut arguments, multiple interfaces, and canonical IUnknown identity. |

The live Classic COM suite currently contains 17 runners and is available with:

```powershell
$env:DYNWINRT_WIN32_WINMD = "C:\path\to\Windows.Win32.winmd"
.\tests\e2e\e2e_test.ps1 -Lang com
```

## Native contracts supported by the safe generator

An API not named above may still be safe when all of its methods use these
modeled contracts.

| Native contract      | Current support                                                                                                                             |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Scalars and enums    | Signed and unsigned integers, floating point, BOOL, HRESULT, pointer-sized values, enums, and flags.                                        |
| GUIDs and interfaces | GUID values, REFIID/REFGUID, typed interface input/output, QueryInterface, dynamic-IID `void**`, and owned `+1` output.                     |
| Native POD structs   | Validated x86, x64, and ARM64 layouts with primitive, enum, GUID, pointer-sized, nested POD, and fixed primitive-array fields.              |
| Native unions        | Tagged, validated pointer-input unions with safe POD fields. By-value, output, nested-owning, and undiscriminated unions are not supported. |
| Strings              | BSTR, HSTRING, supported PWSTR/PSTR inputs and outputs, embedded NUL BSTR values, and matching allocator cleanup.                           |
| Counted buffers      | Typed input buffers, caller-owned output buffers, capacity/actual-length patterns, exact bounded sizing, and known CoTaskMem outputs.       |
| Automation values    | Defined subsets of VARIANT, SAFEARRAY, PROPVARIANT, DISPPARAMS, and EXCEPINFO.                                                              |
| Return conventions   | Ordinary and semantic HRESULT, direct scalar, direct void, and exact registered direct-pointer contracts such as `IMalloc`.                 |
| COM object semantics | Coclass activation, inherited interfaces, QI views, automatic release, and idempotent explicit release.                                     |

## JavaScript interface implementation

dynwinrt can dynamically implement an interface in JavaScript when:

- the interface is rooted in `IUnknown`;
- its complete inherited vtable is contiguous and validated;
- every callback argument, output, and return value has a completed inbound ABI
  plan;
- output ownership can be prepared under RAII and committed atomically; and
- callbacks execute synchronously on the thread that created the implementation.

Generated implementations support multiple interface views, canonical
`IUnknown` identity, shared reference counting, common-signature static thunks,
dynamic libffi closures, common native values, and failure containment. They do
not provide general cross-apartment callback dispatch, `IInspectable`
implementation, aggregation, or custom marshaling.

## APIs not supported by the safe layer today

| API or family                                              | Current blocker                                                                                      | Raw direction                                                                                     |
| ---------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Non-HGLOBAL/device-specific clipboard and drag-and-drop    | `DVTARGETDEVICE` and the remaining `STGMEDIUM` alternatives have distinct layout and release rules. | Opaque aligned storage and explicit type-specific cleanup until each semantic alternative exists. |
| DXGI/D3D `GetPrivateData`                                  | The same payload may be bytes or an AddRef'd interface pointer.                                      | Caller-selected byte storage or pointer slot with explicit adoption.                              |
| Generic typed interface `InOut`                            | Replacing an interface pointer requires explicit old/new reference transfer.                         | `UnsafeInterfaceReplacement` with one independent `+1` owner per slot.                            |
| Advanced Automation                                        | BYREF VARIANTs, additional alternatives, replacement arrays, and nested ownership are incomplete.    | Raw storage can expose ABI reach; safe support still requires dedicated value and cleanup models. |
| Custom allocators and resource handles                     | The runtime lacks the exact cleanup operation.                                                       | Raw pointer plus explicit cleanup primitive where one exists.                                     |
| Cross-apartment callbacks                                  | JavaScript is tied to its V8 owner thread while COM callbacks can be synchronous on foreign threads. | Not solved by raw ABI; requires marshaling and dispatch architecture.                             |
| COM aggregation, custom marshaling, and registered servers | These change object identity, activation, and process semantics rather than only method ABI.         | Separate architecture work.                                                                       |
| Flat Win32 DLL exports                                     | They are not COM vtable methods.                                                                     | Separate Win32 raw function layer.                                                                |

## How users check a specific API

Run a dry run against the exact metadata version used by the application:

```powershell
npx dynwinrt-codegen generate `
  --winmd C:\path\to\Windows.Win32.winmd `
  --class-name Windows.Win32.System.Com.IStream `
  --output .\generated `
  --dry-run
```

Interpret the result as:

```text
Success = the complete interface is safe generated for this metadata version.
Failure = inspect the diagnostic, then decide whether the missing fact belongs
          in semantic unsafe, raw unsafe, or unsupported architecture work.
```

## Unsafe layers

`@microsoft/dynwinrt/com/unsafe` remains the preferred manual layer when the
caller knows the contract and dynwinrt already has a matching semantic type:

- COM-owned interface output;
- BSTR output;
- CoTaskMem output;
- borrowed handle or pointer;
- scalar/POD InOut; or
- authoritative counted buffers.

`@microsoft/dynwinrt/com/unsafe/raw` is the lower layer for aligned native
memory, pointer slots, opaque records, and caller-managed replacement. It does
not infer ownership or call cleanup automatically. Phase 1A is complete:
nonzero zero-initialized aligned allocations, checked native-endian primitive
and pointer-width access, owner-retaining in-allocation pointers, unowned
external pointers, borrowed conversion into the existing COM invocation path,
and reentrant-release-safe invocation leases.

The first Phase 1B block adds explicitly unsafe bounded external views plus
architecture-specific raw struct and closed C-POD union descriptors. Validated
recursive structs support scalar, GUID, pointer, nested struct, and fixed-array
fields; existing signatures cover by-value input, pointer input, output,
InOut, and direct struct return. External views never free caller memory.
The next block adds retained borrowed managed pointers, dedicated raw +1 COM
owners, atomic managed adoption, explicit detach/transfer, caller-managed
interface InOut reconciliation, and fixed standard cleanup for COM task/local/
global/BSTR/SAFEARRAY/VARIANT/PROPVARIANT/STGMEDIUM/handle/icon/GDI resources.
RawCom invocation leases retain a temporary +1 across every dispatch path;
detached pointers remain RAII-owned until adoption, and slot transfer writes
before disarming ownership.
Closed, fully described recursive unions support by-value input, Out, InOut,
and direct return on x64 and i686. ARM64 classification compiles but remains
runtime-gated pending an executable oracle. Win64 3/5/6/7-byte by-value
aggregates are rejected at the top-level CIF boundary for the bundled libffi
3.5.2 irregular-size argument passing/copy defect; valid naturally laid out
outer aggregates may contain an odd-sized union.

RawOutbound by-value structs additionally require recursive natural C field
offsets, alignment, and final size. Unexplained gaps or inflated tail padding
are rejected; intentional reserved bytes require explicit fixed `u8` fields.
Semantic explicit layouts and raw pointer-only storage are unchanged.

The independent test-only C oracle is compiled by MSVC and executes live on
x64 and i686, covering register/stack positions, register and sret returns,
HFA1/HFA2/HFA4, mixed non-HFA, nested struct/union forms, sentinels, guarded
destination canaries, indirect U16 local-copy mutation, and input
immutability. Core tests additionally guard direct-return storage on both
sides and validate the guards immediately after dispatch; production storage
is unchanged. CI runs the exact test with `test-hooks` on x64 and live i686.
The same C/Rust oracle cross-compiles in CI for ARM64 with `--no-run`, but the
core runtime gate remains until native execution is available. Production
builds omit `test-hooks` and do not compile or link the C oracle. windows-rs
BITS and WHV union types provide additional layout evidence. Phase 1 is
therefore complete only for the documented x64/i686 in-process
current-apartment outbound subset, not cross-platform complete.

The final Phase 1 matrix executes representative raw `GetPrivateData`,
`IsFormatSupported`, `GetData`, and interface-InOut contracts with real fake
vtables. It also covers pointer depth through `T***`, required/nullable
pointers, every struct direction and return, caller/callee buffers, cleanup
failure paths, the managed/raw bridge, and all standard cleanup wrappers.
`IsFormatSupported` includes its real share-mode argument and documented
shared/exclusive output rules. `GetData` uses real `TYMED_HGLOBAL` cleanup both
with and without `pUnkForRelease`. Interface InOut has distinct
consume-old/replace, preserve-old/replace, and unchanged-slot tests.
This demonstrates raw reach only; the named interfaces remain unsafe and
absent from complete safe generation. Phase 1 is complete for the documented
x64/i686 in-process current-apartment outbound subset, with explicit aggregate
and ARM64 gates.

### Generated unsafe companions: Stage 1

The ordinary `generate` command now reuses the exact capability-census method
classifier after complete safe projection fails. When at least one method is
`raw_metadata_complete` on x64, i686, and ARM64, it emits a nonconstructible
`<Interface>Unsafe` companion under `generated/com/unsafe/` with natural method
names, exact hidden IID/slot/signature wiring, and explicit
`DynComRawMemory | DynComRawPointer` storage arguments. There is no new CLI
flag.

Only metadata-complete methods are executable in Stage 1.
`raw_manual_contract` and `raw_runtime_blocked` methods are omitted from the
class but retained with target status, reasons, declaring IID, absolute slot,
and signature fingerprint in deterministic `support.json`. Safe-complete
interfaces retain their byte-identical safe output and receive no duplicate
unsafe class. Safe barrels and the npm root, `/com`, and `/com/unsafe` surfaces
never export generated unsafe symbols.

Pointer-width inputs and direct results remain exact `bigint` values:
`NativeIsize` uses I32 on i686 and I64 on 64-bit targets; `NativeUsize` uses
U32 on i686 and U64 on 64-bit targets. Empty/zero interface IIDs, unsupported roots, and
non-addressable identities block every method before companion selection.
`MFASYNCRESULT` therefore produces no callable class.

When an interface has no executable method, generation transactionally commits
a schema-11 report containing every blocked/manual method and exact reasons,
emits no class `.js`/`.d.ts`, and then exits nonzero. The metadata record is a
sorted, deduplicated set of every loaded emission/reference/sibling winmd, with
per-file hashes, a set hash, and an optional exact defining file; it contains no
local paths. Concurrent incremental COM generation is serialized by a
crash-safe OS file lock shared by WinRT-only, COM-only, mixed, report-only, and
Python writers. The complete output root is staged and replaced as one
transaction, covering cleanup, manifest/support, barrels, package files, and
the `com` subtree. Pre-publication failure restores the previous root
byte-for-byte. Successful publication is the commit point; later backup
cleanup failure keeps the complete new root, warns, retains residue, and
retries cleanup on the next locked run. Owner-marked stage/backup residue from
process termination between publication renames is recovered under that same
lock; ambiguous or unowned residue fails closed.

Unmanaged file symlinks, directory symlinks, and junctions inside an existing
output are detected without following reparse targets. Their directory entries
move from backup to stage immediately before publication and move back on
pre-publication rollback. No-follow residue cleanup removes only the links,
never an external target. Case aliases, managed/staged/manifest ownership
conflicts, transaction-residue names, and unsupported reparse tags fail closed.

Safe and unsafe modules use the canonical lowercase/kebab namespace layout, for
example `com/windows/win32/ui/shell/ITaskbarList3.js` and
`com/unsafe/windows/win32/ai/machine-learning/win-ml/IWinMLEvaluationContextUnsafe.js`.
The short class name remains `IWinMLEvaluationContextUnsafe`. A short barrel
export is emitted only when globally unique; ambiguous names are available only
through deep modules, and incremental order does not affect the resulting
barrel.

Generated and retained paths share an ASCII case-folded Windows identity.
Traversal, rooted/drive paths, trailing dots/spaces, reserved device names, and
case-only namespace/type collisions fail closed. Existing cross-root shared
ownership additionally requires the staged file to exist and exactly match the
planned bytes before overwrite.

CI also generates real official safe `IWbemServices`, `IDataObject`, and
`IAudioClient` wrappers plus an `IWinMLEvaluationContextUnsafe` companion. The
test-hook COM implementations measure QueryInterface/AddRef/Release,
conditional output selection, owned storage cleanup, and raw pointer-slot
mutation without requiring live WMI or audio hardware while testing the actual
generated CJS, ESM, and declaration artifacts.

Generated companions are outbound only. They do not infer output ownership,
provide callbacks or `implement()`, solve acquisition or apartment transfer, or
make raw storage safe.

Stage 2 is now implemented for portable `raw_manual_contract` methods. Shared
generated `runtime.js`/`.d.ts` provides closed pointee, pointer-output, handle,
interface-replacement, counted-buffer, owned-pointer, and raw-fallback
strategies. Generated methods validate strategy types before dispatch, clean
dirty HRESULT failure outputs according to the selected strategy, and report
exact per-parameter requirements in support schema 11.

Strategy capabilities are unforgeable frozen objects backed by private
WeakMaps. Private generated helpers perform prepare, writable-span overlap
validation, transactional finish/rollback, dirty-output cleanup, and extracted
owner release. Raw-responsibility failure pointers remain one-shot retrievable
and are attached to the error. Owned native cleanup results have a
FinalizationRegistry fallback; finalizer cleanup failure leaks conservatively
because finalizers cannot throw.

Preparation records are frozen and opaque; real records live in a second
private WeakMap, internal helper exports are immutable, and production output
contains no test hooks. Native `WinGuid` validation happens before COM output
or replacement slot mutation. The core marks actual native dispatch only
after fallible target/argument validation, allowing consumes-old activation to
roll back and remain retryable on pre-dispatch failure. Replacement selection requires the exact parameter-local
`missing_interface_replacement_contract` reason and InOut `IFoo**` depth;
exact method evidence can suppress it only by recording old/new ownership.
Replacement owners cannot alias across prepared slots in any mode.
Writable `UnsafePointee` requirements carry direction, nullability, and
per-target layout when known; they require bounded memory and join output,
replacement, counted-buffer, and ordinary writable count slots in one overlap
graph. Counted-buffer selection similarly requires that parameter's own
`missing_count_relation`.

`IWbemServices::OpenNamespace` is now covered by exact direction and ownership
evidence. Microsoft Learn and the Windows SDK `WbemCli.h`/`WbemIdl.idl`
declaration identify both pointer-to-interface parameters as `[out]`, not
InOut. A stored full-method fingerprint covers all five parameters, return and
HRESULT semantics, and absence of competing exact contracts. Generated options
hide `pCtx`, require exactly one empty output slot, enforce exact `lFlags` 0
versus 16 (`WBEM_FLAG_RETURN_IMMEDIATELY`), adopt the requested non-null `+1`, and
release dirty failure output. Generic interface InOut parameters remain manual
replacement contracts.

**1,428 of 1,432** x64 manual-contract interfaces now have at least one portable
executable generated high-level method. **1,427** have an executable manual
method, one retains only metadata-complete methods, and four have no portable
executable method because every candidate is blocked on another generated
target. Across the portable generated surface there are **6,048 executable
manual methods**, **0 remaining portable manual-classified methods omitted**,
and **1,160 runtime-blocked methods** still omitted.

### Safe contract evidence census

Stage 1 of the
[Classic COM contract evidence registry](../architecture/classic-com-contract-evidence-registry.md)
classifies all 5,697 safe-complete interfaces exactly once:

| Evidence class | Interfaces |
| --- | ---: |
| `standard_derived` | 5,338 |
| `exact_registry_dependent` | 359 |

The registry declares 499 selector-specific entries; all 499 match pinned
metadata, 408 distinct entries are safe-consumed, and safe plans contain 663
entry/interface plus 411 family/interface dependencies. They also consume
5,982 metadata-attribute and 26,134 universal COM-rule dependency sets.
Entry/interface dependencies by kind are SAFEARRAY 263, enumerator-next 74,
borrowed-handle 54, ownership 177, parameter-direction 45, bounded-two-call 16,
counted-buffer 16, conditional-output 10, flag-selected-buffer 3, null-input 2,
semantic-HRESULT 2, and compound-dispatch 1. Complete per-entry status,
per-family rollups, and per-interface entry IDs are retained in the summary
and interface CSV.
These are dependency counts, not net contribution; no ablation claim is made.

Generic scalar BSTR output/replacement and 24 `STANDARD_NEXT` enumerator
entries remain typed COM standard rules. Twenty-five safe interfaces consume
the generic enumerator rule because one contract is inherited.
ISequentialStream `Read`/`Write` and IDispatch `Invoke` are distinct exact
entries with full selectors, fingerprints, and citations.
`IAudioClient::IsFormatSupported` adds an exact shared/exclusive conditional
output contract, while `GetMixFormat` and
`IAudioClient3::GetCurrentSharedModeEnginePeriod` add exact CoTaskMem-owned
variable-format outputs.

The strict embedded registry lives in
`tools/dynwinrt-codegen/contracts/classic-com/`. JSON is the sole source for
seven selector-derived IWbemServices conditional-output contracts and 148
documented output-ownership contracts. Each entry includes its selector,
pre-contract fingerprint, evidence, and validated metadata hash.

An interface available only through an unsafe layer is not described as safe
supported. A wrong IID, slot, signature, pointer depth, layout, count relation,
allocator, or ownership declaration can corrupt memory or crash the process.

## Reproducible raw capability census

For `Microsoft.Windows.SDK.Win32Metadata` 71.0.14-preview
(`Windows.Win32.winmd` SHA-256
`B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D`),
the safe census is 5,697 of 7,929 interfaces. The separate outbound raw census
classifies the 2,232 safe-incomplete interfaces as:

| Target | Metadata-complete | Manual contract | Runtime-blocked |
| ------ | ----------------: | --------------: | --------------: |
| x64    |               412 |           1,432 |             388 |
| i686   |               411 |           1,409 |             412 |
| ARM64  |               412 |           1,432 |             388 |

Including safe-complete interfaces, x64 and ARM64 have 6,109
metadata-complete, 1,432 manual, and 388 blocked interfaces. i686 has 6,108
metadata-complete, 1,409 manual, and 412 blocked interfaces.

Pointer-shaped types are analyzed recursively. A missing pointee layout for an
external input pointer is manual-contract; the same missing layout for a
writable/readable `T*` caller-storage contract is runtime-blocked. Thus
`ID2D1Factory` matrix pointers are manual with external pointee storage, while
`ITypeComp` `BINDPTR*` output storage is blocked until its nested anonymous
layout is complete.

Cleanup availability is no longer represented by ambiguous booleans. Per
target, 2,237 interfaces require no cleanup, 4,533 use a Phase 1 standard
cleanup, none use a known external cleanup, and 1,159 have unknown cleanup.
Every missing output ownership/allocator contract has `cleanup_unknown`.
External pointer/callback requirements affect 1,089 x64/ARM64 interfaces and
1,090 i686 interfaces; 6,804 require external acquisition and all 7,929 retain
the current-apartment rule.

For all 5,697 safe-complete interfaces, cleanup is derived from the validated
projected result conversions rather than the raw analyzer. Pure values,
borrowed handles, caller buffers, and plain arrays are `none_required`.
Managed COM/dynamic-IID adoption, BSTR, HSTRING, CoTaskMem, VARIANT,
SAFEARRAY, PROPVARIANT, EXCEPINFO, STATSTG, owning/enumerator arrays, and
allocator wrappers are `standard_supported`. No safe-complete interface has
`cleanup_unknown` or `known_external`.

The ECMA TypeDef table contains 37,310 definitions excluding `<Module>`.
`windows_metadata::Index::all()` exposes 35,146 addressable definitions; the
older 35,055 number belongs to older metadata and is not reused. The category
table is only a 21-category semantic summary, not an exhaustive concrete type
list. Direct signature and recursively expanded occurrence counts are stored
separately. Specialized categories can arise after semantic projection:
`STATSTG` appears as `NativeStruct` in raw metadata, while the exact
`IStream::Stat` contract becomes semantic `StatStg`; therefore the raw
inventory's `StatStg` occurrence count is zero.

The COM-reachable inventory contains 6,924 definitions present in the pinned
file, 170 unresolved anonymous nested records, six external metadata
references, and one system/synthetic identity. External references are
`Windows.Foundation.IPropertyValue`,
`Windows.Graphics.Effects.IGraphicsEffectSource`,
`Windows.UI.Composition.CompositionGraphicsDevice`,
`CompositionTexture`, `Desktop.DesktopWindowTarget`, and
`ICompositionSurface`. Only identities reconciled against an actual pinned
TypeDef row are called `named_metadata_definition`.

Unresolved anonymous unions remain category `Unknown`; the appendix displays
their unique containing-path `identity_key` rather than an ambiguous empty
namespace/name. `NativeUnion = 1` represents the one recognized raw/projected
union occurrence. VARIANT and PROPVARIANT use dedicated semantic categories,
while outer STGMEDIUM-like records may appear as NativeStruct or contain an
unresolved anonymous field.

Retained evidence:

- [compact summary](generated/classic-com-capability-summary.json)
- [compact per-interface support index](generated/classic-com-interface-support.csv)

The support index contains namespace, name, IID, complete-safe state, safe
evidence class, and the first stable blocker/manual reason code. The complete
interface capability matrix, COM-reachable identity inventory, all-TypeDef
inventory, canonical type-shape report, and named-type appendix are generated
on every CI run and uploaded as the `classic-com-capability-report` artifact.
They remain reproducible locally but are not retained in Git. Arrays, detailed
reasons, layout facts, and target states use deterministic JSON cells inside
RFC 4180 CSV. Large JSON duplicates are emitted only with `--large-json`.

Regenerate with:

```powershell
cargo run -p dynwinrt-codegen -- com-capability-census `
  --winmd <Windows.Win32.winmd> `
  --output-dir docs/status/generated
```

The census is outbound-only. It does not count callbacks, servers,
aggregation, cross-apartment marshaling, acquisition, or cleanup convenience
as ABI support. The legacy `com-census --json` output remains unchanged.

## Roadmap

### Now

- Preserve safe generation and the 70% complete-interface target.
- Maintain completed Phase 1A allocation/provenance/lease invariants.
- Validate the first Phase 1B bounded-view and recursive-struct block without
  adding a safe-codegen fallback.
- Keep safe and raw capability census artifacts reproducible and separately
  versioned.

### Next

- Add higher-level owning `STGMEDIUM`, audio-format, handle, and
  interface-replacement values over the completed raw primitives.
- Expand authoritative array and allocator contracts.

### Later

- Expand Automation and native-layout semantic support.
- Design apartment marshaling and agility independently from raw ABI.
- Keep flat Win32 exports and COM server support as separate projects.

## Copyable user-facing statement

> dynwinrt supports the Classic COM interface portion of Windows.Win32
> metadata. With Win32Metadata 71.0.14-preview, 5,697 of 7,929 addressable COM
> interfaces pass complete safe generation. Safe symbols never fall back to an
> unsafe implementation; ordinary generation may instead emit an explicitly
> named `*Unsafe` outbound companion containing only metadata-complete methods.
> Experts may use the semantic unsafe layer for known ownership plans or the
> isolated raw unsafe layer for caller-managed native memory and pointers. Raw
> declarations guarantee only ABI execution and can
> corrupt memory or terminate the process when used incorrectly.
