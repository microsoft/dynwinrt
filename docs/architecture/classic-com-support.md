# Classic COM support

`dynwinrt` supports a deliberately limited subset of Classic COM. It is not a
general Automation or native Win32 projection.

> **Status: preview, under active development.** The current CI baseline against
> `Microsoft.Windows.SDK.Win32Metadata` 71.0.14-preview is 5,567 complete safe
> interface projections out of 7,929 eligible interfaces (70.21%). Earlier
> inventory and demand-snapshot sections retain the metadata versions and dates
> stated in those sections.

For installation and application-facing examples, including GUID/IID/CLSID
and `/com/unsafe`, see
[Classic COM JavaScript usage](../guides/windows/classic-com-usage.md).

The design keeps the existing WinRT API separate:

```js
import { DynWinRtType, DynWinRtValue } from '@microsoft/dynwinrt';
import { DynComVariant, initializeCom } from '@microsoft/dynwinrt/com';
```

Both entrypoints use the same native N-API binary and private libffi call
machinery. Classic COM metadata, generated wrappers, ownership rules, and
public APIs remain separate from the WinRT projection.

Language ergonomics belong to codegen projection, after native semantics have
been validated. The runtime executes a faithful ABI plan; the JavaScript
projection chooses Buffer/string/bigint, naming, hidden ABI parameters, and
return shapes; the renderer only serializes those decisions. Classic COM work
must not change existing WinRT metadata, generated output, ownership, runtime
behavior, or the `@microsoft/dynwinrt` root API.

## Runtime call architecture

WinRT and Classic COM have separate semantic planners. They share only the
private native-call backend and executor:

```text
WinRT metadata -> signature.rs (WinRT planner) --------\
                                                        -> native_call.rs -> call.rs -> native method
COM metadata   -> com.rs (COM planner and method table) /
```

| Layer | Responsibility |
|---|---|
| `signature.rs` | WinRT-only signature facade. It preserves the existing `In`, `Out`, fill-array, HRESULT, and out-value conventions. It must not expose raw pointers, `InOut`, native direct returns, or other Classic COM semantics. |
| `com.rs` | Classic COM types, method signatures, interface roots, vtable slot numbering, method registry, and method handles. It owns raw-pointer, `InOut`, direct-return, and `void` call semantics without registering methods in the WinRT `MetadataTable`. |
| `native_call.rs` | Private lowering backend. It converts a completed WinRT or COM signature into parameter/output slots, validates input values, expands array ABI parameters, chooses a fast path or prepares a libffi CIF, and coordinates result conversion. It does not own metadata, language projection, or public interface registries. |
| `call.rs` | Private native executor. It reads the vtable function pointer, creates stable ABI storage and libffi arguments, performs the call, and decodes raw output storage according to the completed plan. It must not infer WinRT, Classic COM, ownership, or JavaScript semantics. |

This separation is semantic, not a duplication of the native executor. WinRT
and Classic COM may both lower primitive and struct layout information through
the same private backend, but only their respective semantic layers may decide
what a type, parameter direction, return convention, or ownership contract
means.

In particular:

- WinRT interface methods remain in the WinRT `MetadataTable` and begin at
  `IInspectable` slot 6.
- Classic COM maintains its own interface method table and selects slot 3 or 6
  from its `IUnknown` or `IInspectable` root.
- shared native methods are fully built before publication and are immutable
  during concurrent invocation;
- exact struct identity is validated before native dispatch, while established
  WinRT ABI aliases such as Char16/U16 and enum/I32 arrays remain compatible;
  and
- language-friendly choices remain a codegen responsibility after the COM
  planner has validated the native contract.

## Code generation architecture

Code generation is organized by semantic domain before target language:

```text
codegen/
├── winrt/
│   ├── shared/
│   ├── javascript/
│   └── python/
└── com/
    ├── ir.rs
    ├── model/
    ├── project/
    │   ├── interop.rs
    │   ├── legacy_diagnostics.rs
    │   └── legacy_types.rs
    └── javascript/
        ├── types.rs
        └── render.rs
```

The Classic COM flow is:

```text
raw Windows.Win32 facts
  -> validated SemanticComInterface / ComMethodContract
  -> COM language projection
  -> validated ComType / ProjectedComMethod
  -> JavaScript and declaration renderer
```

`ComType` is a closed set of supported ABI semantics: primitives, transparent
scalar typedefs, pointer-sized scalars, BOOL/HRESULT, GUID, HSTRING, enums,
explicitly classified handle/data/string pointers, BSTR, raw input pointers,
validated native POD values/pointers, explicitly tagged native-union pointers,
VARIANT pointer contracts, input-only VARIANT-by-value contracts, SAFEARRAY,
PROPVARIANT, and managed interfaces with resolved IIDs.
Typed counted buffers carry their validated element ABI and an explicit
input-count, caller-capacity/actual-length, or callee-allocation relation.
One authoritative element count may also group exactly one borrowed
NUL-terminated UTF-16/ANSI string-pointer input array with one caller-owned
plain scalar/enum output array. Generated JavaScript accepts `string[]`, hides
the count and output storage, and returns a numeric/enum array.
Parameter direction, return convention, result ownership, cleanup, buffer
relationships, activation, and dynamic-IID behavior are encoded in the
projected IR.
An optional implementation plan is encoded only for an `IUnknown`-rooted
interface whose complete contiguous vtable maps to the validated callback
subset. That subset includes scalar/enum/GUID/handle/interface values,
HRESULT/void/direct-scalar returns, BSTR/HSTRING and borrowed string pointers,
POD values/pointers, basic InOut, and authoritative plain counted-buffer
contracts. Each registered method owns both its outbound `ComCallPlan` and
full inbound `CallbackMethodPlan`. The runtime chooses a static thunk for a
common signature or a cached libffi closure for every other supported
signature; the renderer never serializes backend shapes.

Implemented objects may expose multiple independently generated interface
views. QueryInterface routes each derived and base IID to its frozen view,
every view shares one reference count, and QueryInterface for `IUnknown`
always returns the canonical identity. Generated implementation descriptors
compose these views without exposing handwritten signatures.

libffi allocates executable closure memory. A process mitigation such as
`ProhibitDynamicCode` can therefore reject a signature that has no static fast
path; object creation reports that failure instead of publishing a partial
vtable. Prepared closures are cached for the process lifetime so a callback
that performs the final reentrant `Release` cannot free the machine-code page
currently executing.
Production projection reads those decisions only from the validated semantic
contracts; the shared compatibility metadata supplies names, documentation,
and enum member values, not ABI meaning.
The legacy TypeMeta projector is retained only to reproduce established
unsupported-interface diagnostics and to build synthetic renderer fixtures;
successful production generation discards it entirely.

General arrays outside that shared-count subset, parameterized and async
interfaces, delegates, unknown
layouts, unclassified pointer typedefs, unresolved IIDs, unknown allocators,
and unsupported ownership transfers fail during projection. Fixed primitive
arrays are accepted only as fields of a completely validated POD layout. The
renderer cannot see `TypeMeta` or metadata attributes and has no default
pointer/Buffer fallback; it only serializes the validated projected IR with
exhaustive type matches.

## Size of Windows.Win32.winmd

The counts below are exact for
`Microsoft.Windows.SDK.Win32Metadata` **69.0.7-preview**
`Windows.Win32.winmd`, read with `windows-metadata` 0.59.0. `<Module>` is
excluded.

There is no single canonical definition of an "API" in ECMA-335 metadata. For
callable entries, the most useful count is:

```text
17,760 flat P/Invoke functions
+46,233 declared interface methods
=63,993 callable entries
```

| Metadata entity | Count |
|---|---:|
| Namespaces | 324 |
| Type definitions | 35,055 |
| Flat P/Invoke functions | 17,760 |
| Interfaces | 7,971 |
| `IUnknown`-rooted interfaces | 7,878 |
| `IInspectable`-rooted interfaces | 43 |
| Other/no-root interfaces | 50 |
| Declared interface methods | 46,233 |
| Structs | 15,944 |
| Enums | 7,784 |
| Enum members | 67,587 |
| Delegates | 3,002 |
| Classes/API containers | 316 |
| Metadata attributes | 38 |
| Non-enum literal constants | 88,931 |

These numbers describe the metadata, not dynwinrt support:

- The current Classic COM work targets interface methods. It does **not**
  project the 17,760 flat DLL exports.
- An interface declaration may describe a caller-implemented callback rather
  than an OS object that can be activated and called.
- The interface count includes graphics, media, WMI, Automation, Shell, and
  other families whose native types are not all supported.
- Methods inherited by a derived interface are counted once where they are
  declared, not repeated for every derived interface.

The largest flat-function modules in this metadata version include
`KERNEL32.dll` (1,407), `USER32.dll` (767), `gdiplus.dll` (629),
`ADVAPI32.dll` (619), `GDI32.dll` (431), `OLEAUT32.dll` (405),
`OLE32.dll` (273), and `SHELL32.dll` (244).

## Type-system problem map

The following counts come from all 46,233 declared interface methods, not only
the 30-interface frequency sample. Nested pointee types are included in type
occurrence counts.

| Signature characteristic | Count |
|---|---:|
| Parameters | 79,181 |
| Input parameters | 47,289 |
| Output parameters | 28,058 |
| In/out parameters | 3,834 |
| Optional parameters | 5,362 |
| `HRESULT` returns | 44,309 |
| Direct `void` returns | 1,018 |
| Direct value returns | 906 |
| Mutable pointer occurrences, depth 1 | 36,321 |
| Mutable pointer occurrences, depth 2 | 1,492 |
| Mutable pointer occurrences, depth 3 | 8 |
| Parameters with `NativeArrayInfo` | 2,973 |
| Parameters with `FreeWith` metadata | **13** |
| Unique referenced interfaces | 2,875 |
| Unique referenced structs | 1,491 |
| Unique referenced enums | 1,739 |
| Unique referenced delegates | 71 |
| BSTR occurrences | 7,697 |
| VARIANT-family occurrences | 3,586 |
| SAFEARRAY occurrences | 238 |
| PROPVARIANT occurrences | 156 |
| PROPERTYKEY occurrences | 138 |
| Representative audio-format struct occurrences | 69 |
| FORMATETC/STGMEDIUM occurrences | 27 |

The implementation should therefore be planned around the following problems,
not around one-off interface fixes.

### 1. Native layout engine

**Problem:** A named native type is not enough to call a method. The ABI needs
its exact size, alignment, packing, field offsets, nested layout, architecture
variation, and whether it is a struct or union.

This is the largest general blocker: 1,491 distinct structs appear in interface
signatures. It affects Direct3D, DXGI, Shell, drag-and-drop, streams, WMI,
audio, and the Property System.

Required model:

- sequential and explicit layout;
- nested structs and unions;
- fixed arrays and bitfields;
- x86/x64/ARM64 size and alignment;
- by-value, pointer-to, out, and in/out forms; and
- safe construction and field access in each language binding.

### 2. Pointer depth and pointee semantics

**Problem:** `T*`, `T**`, and `T***` are not interchangeable. A pointer may
mean a borrowed object, optional value, caller storage, callee allocation,
array, null-terminated string, interface reference, or opaque token.

The metadata contains 37,821 pointer occurrences, including 1,500 with depth
greater than one.

Required model:

- pointee type and pointer depth;
- const versus writable storage;
- nullable versus required;
- interface pointer versus data pointer;
- input, output, and replacement/in-out semantics; and
- storage size before a native call is allowed.

### 3. Counted buffers and native arrays

**Problem:** A pointer plus count is one logical value. Allocating one scalar
for a writable `BYTE*` is a memory overwrite.

There are 2,973 `NativeArrayInfo` parameters. The projection needs:

- which parameter supplies the count;
- whether the count is bytes or elements;
- capacity versus actual returned length;
- caller-allocated, callee-allocated, and two-call sizing patterns;
- string termination and encoding; and
- partial writes and failure cleanup.

The supported subset includes primitive typed input buffers, caller-owned
outputs with exact capacity/actual-length relationships, byte-counted
`ISequentialStream::Read`/`Write`, exact bounded two-call sizing contracts, and
known-allocator callee outputs. A distinct exact `IEnum*::Next` plan supports
`IEnumGUID` and `IEnumConnectionPoints`: requested capacity and fetched count
remain separate, `S_FALSE` is successful partial completion, and generated
`next(count)` returns only the fetched GUIDs or managed interface wrappers.
Counts are never inferred from names or adjacency. Other owning
BSTR/interface/string-pointer elements and unknown allocators remain fail
closed.

### 4. Ownership and allocator contracts

**Problem:** The type and pointer depth do not identify who owns memory or how
to release it.

Only 13 parameters in this metadata carry `FreeWith`, despite thousands of
owned-output contracts. Metadata alone is therefore insufficient.

The ABI/projection needs explicit ownership such as:

- borrowed;
- COM `AddRef`/`Release`;
- BSTR / `SysFreeString`;
- `CoTaskMemFree`;
- `LocalFree`;
- allocator/interface-specific release;
- Win32 resource-specific cleanup; and
- custom or unknown ownership, which must fail closed.

### 5. Discriminated unions: Automation and Property System

**Problem:** `VARIANT` and `PROPVARIANT` combine a type tag, a union payload,
and nested ownership. Treating either as an opaque pointer is not a complete or
safe projection.

The implemented Automation subset now provides:

- VARIANT empty/null, signed/unsigned integers, float/double, exact
  `VARIANT_BOOL`, BSTR, IUnknown, IDispatch, and supported SAFEARRAY values;
- PROPVARIANT scalar numeric/bool values, LPWSTR, CLSID, FILETIME, blob, and
  supported vectors;
- `VariantInit`/`VariantClear` and zero-init/`PropVariantClear`;
- distinct VARIANT pointer and input-only by-value ABI categories. By-value
  calls use a call-local `VariantCopy`, pass the real aggregate through
  libffi, and clear the copy on every exit path; required aggregates never
  fabricate null or `VT_EMPTY`;
- dedicated `DISPPARAMS` input storage that deep-copies and reverses natural
  arguments into stable contiguous `VARIANTARG` storage;
- dedicated `EXCEPINFO` output storage with exact BSTR cleanup and one-shot
  deferred-fill handling;
- dedicated COM-only JavaScript wrappers and range-checked conversions; and
- fail-closed BYREF/InOut, unknown VARTYPE, and unsupported nested ownership.

Complete inherited `IDispatch` now projects from real metadata.
`GetIDsOfNames` retains its natural shared-count array surface, and `Invoke`
takes `DynComDispatchParams` plus explicit LCID, flags, IID, and optional-output
request options. XML Automation and Task Scheduler still stop at their own
unsupported BYREF/InOut or nested-ownership contracts; support for `IDispatch`
inheritance does not imply those derived interfaces are complete.

### 6. SAFEARRAY

**Problem:** SAFEARRAY is a descriptor, not a pointer to a flat JavaScript
array. It carries rank, bounds, element type, locks, ownership, and potentially
non-blittable elements.

The runtime supports ranks 1 through 8, signed/non-zero lower bounds, typed
scalar/VARIANT_BOOL/BSTR/IUnknown/IDispatch/VARIANT elements, overflow and
length validation, and exact element cleanup. It uses SafeArrayCreate,
SafeArrayCopy, SafeArrayGetVartype/dimension/bounds/element-size APIs, and
SafeArrayAccessData/UnaccessData; descriptor bytes are never reinterpreted.

### 7. Interface in/out and callback implementations

**Problem:** Replacing `IFoo*` through `IFoo**` requires precise release and
AddRef behavior. Event APIs additionally require dynwinrt to implement an
arbitrary caller-defined COM interface, not merely invoke one.

Required support:

- release of the old in/out reference when the contract requires it;
- ownership of the replacement reference;
- generated sink vtables;
- QueryInterface identity and reference counting for implemented objects;
- callback threading/apartment dispatch; and
- conversion of callback failures to HRESULT.

The dynamic implementation backend now provides generated vtables, canonical
multi-interface identity, static fast paths plus libffi closures, owner-thread
dispatch, and fail-closed output validation. Interface InOut replacement
remains unsupported because its old/new reference ownership is not encoded
strongly enough.

### 8. Semantic HRESULT values

**Problem:** Most HRESULTs are throw-or-success, but methods such as
`IPersistFile::IsDirty` use `S_OK` versus `S_FALSE` as their actual result.
Discarding every successful HRESULT loses information.

Windows.Win32 metadata marks these methods with
`CanReturnMultipleSuccessValuesAttribute`. The COM projection preserves the
numeric successful HRESULT for marked methods while still throwing failed
HRESULTs. Exact documented exceptions such as `IPersistFile::GetCurFile`,
whose metadata omits the marker, are classified explicitly. Other unmarked
HRESULT methods retain the normal throw-or-`void` behavior.

### 9. Apartment affinity and marshaling

**Problem:** A valid COM reference is not necessarily callable from every
thread. STA objects require the owning apartment or a marshaled proxy.

Generated Classic COM JavaScript wrappers now record the creating thread and
reject wrong-thread invocation or explicit release. A wrong-thread finalizer
leaks rather than calling `Release` in the wrong apartment. WinRT values remain
unbound and retain their existing behavior.

Remaining apartment work includes:

- agile-object detection;
- Global Interface Table or COM marshaling integration; and
- deterministic callback dispatch to the correct apartment.

### 10. Acquisition and flat-function boundary

**Problem:** many common interfaces are not created with `CoCreateInstance`.
Examples include `CoGetMalloc`, `CreateBindCtx`, `D2D1CreateFactory`,
`DWriteCreateFactory`, `D3D11CreateDevice`, and shell helper functions.

The current Classic COM layer can invoke an acquired interface, but a separate
flat-Win32 layer is needed for the 17,760 DLL exports, their calling
conventions, `GetLastError`, callbacks, and handle cleanup.

### Recommended implementation order

1. Extend validated POD layout to unions, bitfields, and owned/non-POD fields.
2. Pointer-depth plus counted-buffer contracts.
3. Explicit allocator/ownership metadata.
4. VARIANT/PROPVARIANT and semantic HRESULT handling.
5. SAFEARRAY.
6. Broaden generated COM sink/interface implementation beyond the initial
   same-thread interface-input subset.
7. Apartment-aware marshaling.
8. Separate flat-Win32 acquisition/invocation layer.

## What the current PR handles

The PR establishes a safe Classic COM subset and rejects the rest. It should
not be described as solving every problem in the map above.

### Implemented

| Problem | Current implementation |
|---|---|
| WinRT/Classic COM separation | Separate COM metadata/codegen path and `@microsoft/dynwinrt/com` public entrypoint. The WinRT generator and root runtime API remain unchanged. |
| Interface root and vtable layout | Distinguishes `IUnknown` slot 3 from `IInspectable` slot 6 and walks inherited Classic COM interfaces before assigning slots. |
| Method return conventions | Supports normal HRESULT methods, semantic HRESULT values marked with `CanReturnMultipleSuccessValuesAttribute`, native direct scalar, direct pointer at the runtime layer, and direct `void` returns. |
| Basic parameter direction | Supports input, output, scalar in/out, and documented Automation BSTR replacement parameters without reducing in/out to out-only. |
| Primitive ABI types | Signed/unsigned integers, floats, BOOL, HRESULT, GUID, enums, and `char16`. |
| Pointer-sized values | `ISize`/`USize` select the correct x86/x64 ABI width and JavaScript uses `bigint`. |
| Validated native POD layout | Architecture-specific sequential layouts and authoritative non-overlapping explicit layouts support primitive, enum, GUID, pointer-sized, nested POD, and fixed primitive-array fields. Layout computation checks packing, alignment, bounds, overflow, overlap, and recursive cycles for x86, x64, and ARM64. JavaScript uses branded native-struct objects exposing a copied `.bytes` Buffer; qualified identity and exact size are checked before calls. |
| Native unions | Architecture-specific overlapping fields are validated at offset zero. Only scalar/GUID/pointer/nested POD struct fields are accepted. JavaScript uses branded `DynComNativeUnion` values whose constructor requires an explicit active field. Union pointer inputs are supported; by-value unions, outputs without a discriminant contract, nested unions, bitfields, flexible arrays, and nested owned fields fail closed. |
| VARIANT | Dedicated `DynComVariant` values support VT_EMPTY, VT_NULL, I1/UI1/I2/UI2/I4/UI4/I8/UI8/INT/UINT, R4/R8, exact VARIANT_BOOL, BSTR, UNKNOWN, DISPATCH, and supported `VT_ARRAY` values. Pointer-shaped input/output contracts retain their existing path. Required input-only `VARIANT`/`VARIANTARG` values may also lower by value: windows-rs establishes size/alignment 24/8 on x64 and 16/8 on i686, libffi receives an aggregate rather than a pointer, and a call-local `VariantCopy` protects shared storage and balances nested BSTR/interface ownership. Optional by-value metadata, output/InOut, BYREF, and unknown flags/tags fail before dispatch. |
| SAFEARRAY | Dedicated `DynComSafeArray` values support ranks 1–8, explicit signed lower bounds, typed scalar/bool/BSTR/interface/VARIANT elements, exact length/overflow/VARTYPE/element-width validation, lock/unlock, copy, and destruction. JavaScript preserves bounds rather than silently forcing zero-based arrays. |
| PROPVARIANT | Dedicated `DynComPropVariant` values support empty/null, scalar integer/float/bool, LPWSTR, CLSID, FILETIME, BLOB, and vectors of numeric/bool/string/GUID/FILETIME values. Storage is zero-initialized and cleared exactly once with `PropVariantClear`; nested VT_VECTOR\|VT_VARIANT and unknown combinations fail closed. |
| DISPPARAMS | Dedicated `DynComDispatchParams` owns deep-copied VARIANTARG values, reverses natural argument order for `IDispatch`, preserves named DISPIDs, and keeps descriptor/array pointers stable through the call. |
| EXCEPINFO | Dedicated output storage is zero-initialized, invokes `pfnDeferredFillIn` at most once, validates reserved fields, extracts source/description/help-file/context/scode, and frees every BSTR on all success and failure paths. |
| GUID ABI | Full 16-byte GUID output storage plus GUID value and REFIID/REFGUID pointer patterns. |
| Unsigned enum values | COM-local enum metadata preserves unsigned values, including 32-bit high-bit flags and 64-bit `bigint` literals. |
| Standard COM references | `CoCreateInstance`, QueryInterface, and typed interface outputs carry an owned `+1` reference and release automatically. |
| Ownership provenance | Borrowed numeric/TypedArray pointers cannot be re-adopted as a second COM owner. Native owned outputs are consumed once. |
| Backing-storage lifetime | Buffer/TypedArray owners are retained and detached ArrayBuffers are rejected before native use. |
| Common string ownership | By-value BSTR input is a caller-owned call-local allocation borrowed by the callee; scalar BSTR output and final BSTR replacement results are caller-owned and use `SysFreeString`. Supported `PWSTR`/`PSTR` allocations use `CoTaskMemFree`. |
| HSTRING ownership | Classic COM methods that explicitly use HSTRING project strings through owning HSTRING values; outputs release with `WindowsDeleteString`. |
| External interface metadata | Interface parameters require a resolvable IID. Missing referenced metadata fails generation with a `--ref` diagnostic instead of degrading an owned interface to a raw pointer. |
| WinRT runtime-class references | A resolved runtime class lowers through its default interface IID and remains a managed COM value. Missing defaults fail closed. |
| Common interop pattern | Supports HWND + REFIID + `void**` bridges and adopts the returned interface reference. |
| Explicit COM initialization | Activation no longer silently chooses MTA; callers select STA or MTA with `initializeCom()`. Generated implementation files use the isolated unsafe runtime internally. |
| Dynamic JavaScript COM implementations | Any `IUnknown`-rooted interface whose complete contiguous vtable maps to the validated callback subset receives `static implement()` and `static implementation()`. Static fast-path thunks cover common signatures; cached libffi closures cover arbitrary supported parameter counts, scalar widths, POD layouts, outputs, and native return conventions using the platform COM calling convention. Objects support derived/base IID aliases, multiple interface views, canonical IUnknown identity, shared atomic AddRef/Release, synchronous owner-thread JS dispatch, required Out initialization/validation, allocator-correct transfer, and panic/exception containment. Count/capacity values are read according to their In/InOut ABI direction, fixed outputs without an actual-length slot require exact size, and typed interface outputs are queried to the declared IID. QI references, BSTR/HSTRING values, and CoTaskMem buffers remain RAII-owned until every output is prepared; only then are all native output slots committed, so preparation failure leaves owned outputs null and releases every temporary owner. Wrong-thread HRESULT methods return `RPC_E_WRONG_THREAD`; direct returns are zeroed and void methods do nothing because those native ABIs have no error channel. `IFileDialogEvents` is live-tested with `Advise`/`Unadvise`; `IDropTarget` exercises libffi, POD/InOut, generated multi-interface composition, and QueryInterface. |
| Fail-closed generation | Unknown/unsafe layouts, untagged/by-value/output unions, bitfields, flexible arrays, nested owned fields, unsupported VARTYPE/BYREF/SAFEARRAY/PROPVARIANT combinations, unsupported arrays, pointer outputs, ownership, and in/out shapes stop generation with a targeted error. |
| Consumable output | Classic COM files live under `com/`, with `./com` and `./com/*` package exports. Mixed and incremental generation preserve the WinRT-only root barrel; COM-only output retains its legacy root entrypoint. |
| Explicit vtable registration | Every generated method is registered with `.addMethodAt(vtableIndex, name, signature)`, keyed by its actual metadata-derived vtable slot. Methods are never deduplicated by name, so same-name overloads at different slots both register correctly. |
| Same-name overload projection | Overloads (e.g. `IDCompositionEffectGroup::SetOpacity`) are grouped once during projection (not by renderer heuristics). A single public JS method dispatches to a private per-slot implementation using only a validated, mutually-distinguishable arity/shape key (`typeof`-based: boolean/number/bigint/string/object); ambiguous groups fail generation closed with a diagnostic naming the interface, method, and reason. The `.d.ts` emits one TypeScript overload signature per branch, contiguously. |
| Lifecycle ergonomics | Generated interface wrappers declare a protected constructor (`protected constructor(obj: unknown);`) so only generated coclasses can subclass them. Coclasses expose a public zero-argument constructor, and every wrapper provides an idempotent `release()` that delegates to the managed native value. Factory-activated interop wrappers retain `static create()` with JSDoc reminding callers to initialize COM first. |
| Doc-link rendering | When win32metadata attaches a `DocumentationAttribute` (a `learn.microsoft.com` URL) to a method, the generator renders it as an `@see {@link ...}` comment in both `.js` and `.d.ts`. No raw metadata is imported into the renderer — the URL is threaded through `ProjectedComMethod.doc`, populated once during projection. |
| Acronym-aware parameter casing | Parameter names are lowered using the same acronym-run-aware rule as method names, so a Hungarian-prefixed trailing acronym like `hwndMDI` projects as `mdi` (not the previous naive `mDI`). |
| Wide/ANSI string pointer split | `PointerAliasKind::StringPointer` now carries a `StringEncoding` (`Wide`/`Ansi`). Generated wrappers call the semantically distinct `DynCom.wideStringPointer(value)` / `DynCom.ansiStringPointer(value)` constructors instead of one unqualified pointer helper; the renderer never infers encoding — it only renders the encoding decision already made during projection. |
| Output ownership provenance | Signature expressions for pointer-shaped `[out]` values are chosen from `ProjectedComResult` ownership/conversion facts, not from type names: `DynCom.ownedComPointerType()` for dynamic-IID `+1` COM outputs, `DynCom.coTaskMemPointerType()` for `CoTaskMem` `PWSTR`/`PSTR` outputs, dedicated `DynCom.bstrType()` for BSTR outputs/replacements, and plain `pointerType()` (never consumable as an owned value) for everything unclassified/borrowed. |
| Invocation validation | The completed native-call plan validates the exact argument count and ABI-compatible value shape before selecting a direct or libffi path. Native pointers reject scalar/object values, mismatched widths fail before dispatch, and established WinRT JS aliases (`I32` projected as `i8`/`u8`/`char16`) are range-checked and converted to exact ABI storage. |
| Failure-path cleanup | Each output parameter carries an explicit cleanup plan. If a callee writes an owned value and then returns a failing HRESULT, interface references are released, HSTRING/BSTR values are deleted with their matching APIs, and CoTaskMem outputs are freed on both direct and libffi paths. BSTR InOut uses one boxed call-local slot, so unchanged, replaced, and nulled values remain safely cleanable on success and failure without exposing shared storage. Unconsumed successful outputs retain the same allocator-specific ownership until converted, adopted, released, or garbage-collected. |
| Typed counted buffers | COM-local plans support `[in] T* + count`, caller-owned `T* + capacity`, separate or in/out actual lengths, bounded exact two-call sizing, and known-allocator `T** + count` output. Generated JavaScript derives hidden ABI counts, accepts `Buffer`/TypedArray storage, and returns only initialized bytes. |
| Exact fixed-capacity byte output | `IMFAttributes::GetBlob` is registered by declaring IID, qualified interface, slot, and complete parameter/return signature. Generated `getBlob(guidKey, capacity)` allocates exclusive zeroed runtime storage, hides `pBuf`/`cbBufSize`/`pcbBlobSize`, bounds the allocation, and returns only the successful actual byte range. |
| Enumerator partial arrays | Exact IID/name/slot/type registries validate standard `IEnum*::Next(ULONG, T*, ULONG*)`. Runtime storage is exclusive, aligned, stable, bounded, and zeroed. `fetched > capacity` is rejected without an out-of-bounds read. Interface slots transfer each fetched `+1` reference once; failed HRESULTs, conversion errors, overflow, and unused initialized capacity release every remaining non-null slot. Exact canonical IUnknown elements use `DynWinRtValue[]` directly without generating an `IUnknown.js` wrapper. Generated `next(count: number): T[]` rejects zero, fractions, and values above `u32::MAX`; `pceltFetched` is hidden and omitted only where exact metadata permits it for `count == 1`. |
| Exact borrowed HWND outputs | Twenty-two Microsoft-documented declarations are registered by namespace, interface IID, method, slot, parameter count/index/name, and full native shape. Runtime storage is zeroed and pointer-sized, successful null is `0n`, and neither success nor failure calls `Release`, `DestroyWindow`, or another cleanup routine. |
| Shared caller-sized arrays | One authoritative input count may group a borrowed NUL-terminated string-pointer input array with one caller-owned plain scalar/enum output array. Runtime-owned encoded strings and pointer tables remain stable through dispatch; aligned output storage is zeroed, count agreement is checked before dispatch, and failed HRESULTs return no array. |
| Buffer backing safety | Node backing storage is retained and revalidated before invocation. Detached, moved, resized, width-mismatched, non-integral-length, or misaligned storage is rejected; caller output storage is zeroed before the call. Failed HRESULTs never return a partial success-shaped buffer result. |
| Failure-time stream progress | `ISequentialStream` may write a partial count before returning failure. The runtime cleans initialized owned elements exactly, but generated wrappers throw and deliberately discard failure-time partial data rather than returning a success-shaped buffer. |
| Authoritative count-param detection | Relationships come solely from `NativeArrayInfo(CountParamIndex)` or an exact cited override registry. Documented overrides cover `ISequentialStream::Read`/`Write`, `IDiscRecorder::GetRecorderGUID`, `IOpcSignatureCustomObject::GetXml`, and the fixed-capacity/actual-byte `IMFAttributes::GetBlob` contract. The previous substring/adjacency heuristic remains removed. |
| Private-data ownership guard | Actual metadata contains seven declaring `GetPrivateData(REFGUID, UINT*, void*)` interfaces. Their documentation permits returning an AddRef'd interface set by `SetPrivateDataInterface`; Direct3D 10 also documents destructive NULL behavior, and DXGI does not mark the data pointer optional. They remain exact, cited fail-closed hazards rather than being projected as leaking `Buffer` methods. |
| Atomic multi-interface writes | When a single `generate` invocation projects several COM interfaces, every interface is projected into memory first; files (and the `com/` index/package barrel) are only written once the whole batch has projected successfully. A later interface's projection failure no longer leaves an earlier interface's files partially written to disk. |
| Coclass projection | GUID-bearing Classic COM coclasses such as `TaskbarList`, `FileOperation`, and `FileOpenDialog` generate as independently constructible JS classes (`new TaskbarList()`). Interface wrappers remain non-publicly constructible and expose an IID descriptor plus `_fromNative` for runtime QueryInterface views. |
| Interface views | Generated coclasses expose `as(InterfaceClass)`, `tryAs(InterfaceClass)`, and `supports(InterfaceClass)`. These execute real QueryInterface calls; `tryAs` returns `null` only for `E_NOINTERFACE`, while other errors remain visible. |
| Conservative primary interface | Windows.Win32 coclass TypeDefs do not carry `InterfaceImpl`/`DefaultAttribute` rows. The generator associates only exact metadata naming candidates, constructs the real interface inheritance graph, and selects a primary only when there is one unique most-derived leaf. Multiple unrelated leaves fail closed rather than choosing by numeric suffix. |
| CommonJS and ESM | COM implementation files use the same CommonJS format as generated WinRT files. `com/index.js` is the CommonJS barrel, while `com/index.mjs` is the ESM facade. Package exports provide explicit `require` and `import` conditions for the COM barrel and deep imports. |
| Explicit raw opt-in | Manual ABI declarations and caller-supplied COM pointers are isolated under `@microsoft/dynwinrt/com/unsafe`. The default COM facade exports only initialization and managed value/layout wrappers—not `DynCom`, signatures, interfaces, native types, or `DynComUnsafe`; generated safe bindings continue to fail closed. |

### Generated package layout

When WinRT and Classic COM are generated together, WinRT remains at the package
root and Classic COM uses a domain-specific CommonJS subpackage with an ESM
facade:

```text
bindings/
├── index.js
├── index.mjs
├── index.d.ts
├── Uri.js
├── com/
│   ├── package.json
│   ├── index.js
│   ├── index.mjs
│   ├── index.d.ts
│   ├── TaskbarList.js
│   └── ITaskbarList4.js
└── package.json
```

The root index never re-exports COM symbols. Generate types from different
namespaces in one invocation by using fully qualified names:

```powershell
dynwinrt-codegen generate `
  --winmd "C:\path\to\Windows.winmd;C:\path\to\Windows.Win32.winmd" `
  --class-name Windows.Foundation.Uri,Windows.Win32.UI.Shell.TaskbarList `
  --output .\.winapp\bindings
```

Generated coclasses follow the WinRT runtime-class convention:

```js
import { TaskbarList, ITaskbarList3, TBPFLAG } from './bindings/com/index.mjs';

const taskbar = new TaskbarList();
taskbar.setProgressState(hwnd, TBPFLAG.TBPF_NORMAL);

const v3 = taskbar.as(ITaskbarList3); // real QueryInterface
v3.release();
taskbar.release();
```

Separate WinRT and COM invocations may target the same output directory in
either order. The generated package manifest is rebuilt from both domains
without adding COM exports to the WinRT root. Legacy COM-only output is
relocated automatically when it is reused with the new generator.

winappCli project aliases use `#winapp/bindings/com` for the COM barrel and
`#winapp/bindings/com/InterfaceName` for deep imports. Existing source that
used `#winapp/bindings/InterfaceName` for COM should switch to the `com/`
path. Standalone COM-only packages continue to expose their legacy
package-name deep imports.

### Explicit unsafe/raw opt-in

APIs that cannot be proven from metadata remain absent from generated safe
bindings. A caller that independently knows the complete native contract may
opt in through the separate `@microsoft/dynwinrt/com/unsafe` entrypoint:

```js
import {
  DynCom,
  DynComMethodSig,
  DynComUnsafe,
  WinGuid,
} from '@microsoft/dynwinrt/com/unsafe';

const iid = WinGuid.parse('00000000-0000-0000-c000-000000000046');
const raw = DynComUnsafe.registerIUnknownInterface('Example.IRaw', iid)
  .addMethodAt(
    3,
    'ReadPointer',
    new DynComMethodSig()
      .addIn(DynCom.u32Type())
      .addOut(DynComUnsafe.coTaskMemOutputType()),
  );
```

The signature must explicitly declare every ABI type, direction, vtable slot,
count/capacity/actual relationship, return convention, and output cleanup;
the runtime applies the architecture-correct COM system calling convention.
Available raw output choices deliberately distinguish unclassified borrowed
pointer bits, borrowed handles, `Release`-owned COM pointers,
`CoTaskMemFree` allocations, and `SysFreeString` BSTRs. An unclassified
pointer output is never automatically adopted.

`DynComUnsafe.borrowComPointer(bits, iid)` treats the supplied pointer as
borrowed and obtains a new managed `+1` reference through `QueryInterface`.
`DynComUnsafe.adoptOwnedComPointer(bits, iid)` instead consumes exactly one
caller-supplied `+1` reference, including on IID-validation failure. Both
accept only explicit numeric pointer bits; Buffer backing addresses are
rejected to avoid contents-versus-address ambiguity.

This surface is intentionally unsafe: an invalid pointer, IID, slot, calling
convention, layout, direction, count relation, or ownership declaration can
crash the process or corrupt memory. It does not add inference or a fallback
to the default generator, and it remains outside both `@microsoft/dynwinrt`
and `@microsoft/dynwinrt/com`.

### Partially implemented

| Problem family | Supported subset | Remaining gap |
|---|---|---|
| Native pointers | Pointer width, depth preservation, borrowed pointers, handles, REFIID, known interface outputs, and 22 exact documented borrowed `HWND*` outputs | General nullable/required semantics, arbitrary pointee storage, unregistered HWND outputs, and all allocator contracts |
| Counted buffers | Plain primitive/GUID/enum/POD elements plus exact owning COM-interface, BSTR, VARIANT, and `IEnumString` PWSTR elements with authoritative relations; input, caller output, actual/fetched length, generated fixed-capacity bytes, exact bounded two-call sizing, and exact CoTaskMem callee output | PROPVARIANT/SAFEARRAY/unknown pointer/resource elements, nested-owning POD, owning callee-allocated outer arrays without exact allocators, undocumented sizing loops, and ANSI output decoding |
| Native layout | Metadata-driven POD structs plus tagged pointer-input unions with exact x86/x64/ARM64 layout; union fields overlap at offset zero and may contain only safe POD fields | By-value/output/nested unions without discriminant contracts, non-default or unknown packing, bitfields, flexible arrays, non-authoritative explicit offsets, and nested BSTR/interface/resource ownership |
| VARIANT | Empty/null, scalar integer/float/bool, BSTR, UNKNOWN, DISPATCH, supported SAFEARRAY values, and authoritative counted input/output arrays with exact `VariantCopy`/`VariantClear` ownership | Optional aggregate defaults, bare aggregate output/InOut, BYREF, DECIMAL/DATE/CY/ERROR/RECORD and other VARTYPEs, Automation replacement, and arrays without exact count/ownership contracts |
| DISPPARAMS / EXCEPINFO | Exact `IDispatch::Invoke` input/output contracts, dedicated JS wrappers, optional null outputs, deferred fill, and failure cleanup | Output/InOut DISPPARAMS, input/InOut EXCEPINFO, nested occurrences, and arbitrary deferred/function-pointer contracts |
| SAFEARRAY | Rank 1–8, signed bounds, typed scalar/bool/BSTR/interface/VARIANT elements, SafeArray API validation and cleanup | Unsupported element VARTYPEs, rank > 8, untyped arrays whose VARTYPE cannot be proven, and Automation InOut replacement |
| PROPVARIANT | Scalar numeric/bool, LPWSTR, CLSID, FILETIME, blob, and supported vectors with PropVariantClear | Nested VARIANT vectors, streams/interfaces, arrays, clipboard/storage types, BYREF, and unknown VARTYPEs |
| Allocator ownership | COM Release, BSTR output/replacement and array elements, VARIANT clear, CoTaskMem buffers/PWSTR elements, boxed GUID, retained JS buffers | LocalFree, custom allocators, allocator interfaces, unknown ownership |
| Interface pointers | Typed input/output interfaces, QueryInterface, dynamic IID output, and generated multi-interface callback objects with inherited IID aliases | Interface in/out replacement, aggregation, and `IInspectable` implementation |
| Apartments | Explicit initialization, non-agile owner-thread implementations, synchronous same-thread callbacks, and rejection before entering JS on a foreign thread | Cross-apartment marshaling, GIT/agility handling, and callback dispatch |
| Activation | In-process `CoCreateInstance` and `CoGetClassObject` | Aggregation, arbitrary CLSCTX, and other non-CoCreate factory functions |
| Direct pointer returns | Runtime signature plus exact `IMalloc` codegen | Other direct pointer returns remain fail-closed without exact ownership and cleanup evidence |

### Not implemented

- by-value/output/nested unions without an explicit discriminant and ownership contract;
- optional or output/InOut aggregate VARIANT, unsupported VARIANT alternatives,
  BYREF/InOut replacement, and arrays without authoritative count/ownership;
- DISPPARAMS/EXCEPINFO directions, nesting, or callback shapes outside the
  exact supported `IDispatch::Invoke` contract;
- unsupported PROPVARIANT alternatives and nested ownership;
- unsupported SAFEARRAY element types/ranks and InOut replacement;
- BSTR pointer nesting, scalar input `BSTR*`, callee-allocated outer arrays
  without exact allocators, and unknown/custom BSTR allocation contracts;
- FORMATETC and STGMEDIUM;
- callback methods containing unmodeled ownership, Automation, union, array,
  or interface-replacement contracts;
- cross-thread/apartment marshaling; and
- the general flat-Win32 DLL-export and handle-cleanup layer.

## Supported ABI surface

| Capability | Status | Notes |
|---|---|---|
| `IUnknown` and `IInspectable` roots | Supported | User methods begin at vtable slot 3 or 6 respectively. Full inherited Classic COM slot numbering is preserved. |
| `HRESULT` methods | Supported | Failed HRESULTs become errors. |
| Semantic `HRESULT` methods | Supported | `CanReturnMultipleSuccessValuesAttribute` preserves successful values such as `S_OK` and `S_FALSE`; failed values still become errors. |
| Native `void` returns | Supported | Used by interfaces such as `IMalloc`. |
| Direct scalar returns | Supported | Includes signed/unsigned integers, floating point values, and enums. |
| Direct pointer returns | Exact-contract support | `IMalloc` returns opaque allocator-bound values. Other direct pointer returns fail closed until ownership and cleanup are proven. |
| `[in]`, `[out]`, and `[in, out]` parameters | Supported for modeled types | Scalars and validated native POD storage are supported. Other composite in/out types fail generation. |
| Primitive integer and floating-point types | Supported | `i8` through `u64`, `f32`, `f64`, `BOOL`, and `HRESULT`. |
| `ISize` / `USize` | Supported | Projected with the target pointer width; verified by an i686 compile check. |
| GUID values and `REFIID`/`REFGUID` pointers | Supported | GUID out storage uses the full 16-byte layout. |
| Signed and unsigned enums/flags | Supported | Values up to unsigned 64-bit are preserved; 64-bit JavaScript values use `bigint`. |
| Validated native POD structs | Supported subset | Exact x86/x64/ARM64 layouts; primitive, enum, GUID, pointer-sized, nested POD, and fixed primitive-array fields; by-value, pointer input, output, and in/out calls. Generated values are branded `DynComNativeStruct` objects, and output storage is zero-initialized. |
| Validated native unions | Supported subset | Pointer inputs only, with exact architecture layout and an explicit active-field-branded `DynComNativeUnion`. Output/by-value/nested ownership shapes fail closed. |
| VARIANT | Supported subset | Dedicated `DynComVariant`; supported tags are VT_EMPTY, VT_NULL, I1/UI1/I2/UI2/I4/UI4/I8/UI8/INT/UINT, R4/R8, BOOL, BSTR, UNKNOWN, DISPATCH, and arrays of supported SAFEARRAY elements. Pointer contracts remain distinct from required input-only by-value aggregates. |
| SAFEARRAY | Supported subset | `DynComSafeArray` preserves rank/bounds and validates VARTYPE and element width through SafeArray APIs. Supported elements are the scalar integer/float family, VARIANT_BOOL, BSTR, IUnknown, IDispatch, and VARIANT. |
| PROPVARIANT | Supported subset | `DynComPropVariant` supports the scalar family, LPWSTR, CLSID, FILETIME, BLOB, and vectors of numeric/bool/string/GUID/FILETIME elements. |
| DISPPARAMS / EXCEPINFO | Supported for `IDispatch::Invoke` | `DynComDispatchParams` accepts natural-order `DynComVariant[]` plus optional named DISPIDs. `DynComExcepInfo` exposes code/source/description/helpFile/helpContext/scode. Optional Invoke outputs pass native null when not requested. |
| Typed interface parameters and outputs | Supported | Interface outputs carry an owned COM reference. |
| Opaque pointers and handle-shaped typedefs | Supported with limits | They are pointer values, not COM objects. Cleanup remains type-specific. |
| Borrowed HWND outputs | Supported only for exact registered declarations | Qualified `HWND* Out` metadata and Microsoft lifetime evidence must match exactly. JavaScript returns the natural numeric handle alias; runtime output storage is pointer-sized and has no cleanup. |
| NUL-terminated string pointer inputs | Supported | Callers pass a NUL-terminated `Buffer` or a borrowed numeric pointer. |
| Caller-owned UTF-16 output buffers | Supported for recognized shapes | The generator allocates and decodes the buffer when metadata identifies the count parameter. |
| Typed counted input buffers | Supported for validated plain elements | Generated wrappers accept `Buffer` or TypedArray storage and derive byte/element counts from its exact backing length. |
| Caller-owned typed output buffers | Supported for validated plain elements | Capacity is derived from supplied backing storage; separate or in/out actual lengths trim the returned `Buffer` to the initialized range. |
| Generated fixed-capacity byte output | Supported only for registered documented shapes | `IMFAttributes::GetBlob(guidKey, capacity)` allocates exclusive storage in the runtime; capacity is bounded to the projected Buffer limit and native counts remain hidden. |
| Exact two-call sizing | Supported only for registered documented shapes | Generated wrappers start with the documented null/zero query and retry at most twice. Continued size races fail explicitly. |
| Callee-allocated typed buffers | Supported for plain elements with known CoTaskMem ownership | The runtime copies the exact count, calls `CoTaskMemFree` once, and returns an owned Node `Buffer`. |
| Callee-allocated `PWSTR` / `PSTR` outputs | Supported | Generated code decodes and frees `CoTaskMem` storage. |
| By-value `[in] BSTR` | Supported | Generated JavaScript accepts `string` (or `null` only for metadata-proven optional input). The runtime creates a uniquely owned call-local `SysAllocStringLen` allocation, preserving embedded NUL and exact UTF-16 length; the callee only borrows it. |
| Scalar `[out] BSTR*` | Supported | Generated code converts the BSTR and releases it with `SysFreeString`. |
| Required scalar `[in, out] BSTR*` replacement | Supported | Generated JavaScript accepts and returns `string`. A boxed call-local slot owns the current BSTR; the original JavaScript string is immutable, and the final unchanged/replaced/null slot is cleaned or transferred exactly once on every HRESULT path. |
| Counted BSTR arrays | Supported subset | Exact input and caller-output count/capacity/actual contracts project as `string[]`; call-local input elements and every initialized output slot use `SysFreeString` exactly once. Callee-allocated outer arrays, unsupported pointer nesting, and unknown allocators fail closed. |
| HSTRING inputs and scalar outputs | Supported | JavaScript strings are converted to owning HSTRING values; returned HSTRING values are decoded and released automatically. |
| Referenced interface types | Supported when IID metadata is loaded | Missing external definitions fail closed and direct callers to pass the defining winmd with `--ref`. |
| Dynamic-IID `void**` outputs | Supported for explicit required REFIID shapes | The method must return ordinary HRESULT and contain exactly one required const `GUID*` named `iid`/`riid` plus one required mutable `void**`/Object** output with +1 COM ownership. Their explicit parameter indices may be non-adjacent/non-terminal. Optional, duplicate, array, FreeWith, InOut, by-value/mutable/deeper GUID, and wrong-depth output shapes fail closed. |
| Explicit apartment initialization | Supported | `initializeCom()` never silently chooses an apartment for the caller. |

The generator emits native POD storage only after every architecture-specific
layout fact has been validated. A `Buffer` in this path represents the struct's
backing bytes/address; it is not interpreted as pointer-width handle bits.

Parameterized and async interfaces, delegates, and native arrays outside the
explicit count and element-ownership models remain fail-closed until the COM
projection can compute their complete IID, callback, count, and ownership
contracts. They must never fall back to `bigint | Buffer`.

## Unsupported types and shapes

The generator fails closed for unsupported signatures instead of emitting a
plausible but memory-unsafe binding.

The native type rows below come from real signatures in
`Windows.Win32.winmd`, including the 30-interface survey, plus the exact
fail-closed diagnostics produced by the current generator. The policy rows
describe known runtime/public-API boundaries. This is not an exhaustive scan
of every type in the 24 MB metadata file.

| Type or shape | Affected common APIs | Why it is unsupported | Basis |
|---|---|---|---|
| Unsupported `VARIANT` alternatives, aggregate directions, and BYREF/InOut | Automation APIs | Required input-only by-value VARIANT is supported. Optional aggregate defaults, bare aggregate output/InOut, DATE, DECIMAL, CY, ERROR, RECORD, unsupported flags, and every BYREF/InOut combination fail closed until their lifetime/replacement contracts are proven. | Runtime validation + Win32 winmd signatures |
| Unsupported `DISPPARAMS` / `EXCEPINFO` shapes | Automation APIs outside exact `IDispatch::Invoke` | Output/InOut DISPPARAMS, input/InOut EXCEPINFO, nested compounds, reinstalled deferred callbacks, and unrelated function-pointer contracts fail closed. | Runtime validation + Win32 winmd signature |
| Unsupported `PROPVARIANT` alternatives | Property System | Streams/interfaces, arrays, clipboard/storage alternatives, nested VT_VECTOR\|VT_VARIANT, BYREF, and unknown combinations are rejected. | Runtime validation + Win32 winmd signature |
| Native structs with nested owned pointers outside dedicated contracts | Storage and Shell APIs | `STATSTG` has a dedicated output-only model that adopts and frees its CoTaskMem name on every success and failure path. Arbitrary nested pointer structs still fail closed. | Runtime ownership tests + Win32 winmd signature |
| Unsupported `SAFEARRAY` shapes | Automation and Office-style COM APIs | Exact declaration-registry entries support documented `VT_I4`, `VT_UI1`, `VT_UI4`, `VT_R8`, `VT_BSTR`, `VT_VARIANT`, and `VT_UNKNOWN` plus an exact interface IID. Unknown VARTYPE, signature drift, input `SAFEARRAY**`, InOut replacement, unsupported records/dispatch contracts, rank > 8, inconsistent bounds/length/element width, and unproven nullable outputs are rejected. | Exact Microsoft citations + SafeArray API validation + Win32 winmd signatures |
| `FORMATETC` / `STGMEDIUM` | `IDataObject`, clipboard, drag-and-drop | `STGMEDIUM` is a union of handles and interfaces with type-specific release behavior. | Win32 winmd + codegen diagnostic |
| Untagged/by-value/output/nested unions, bitfields, flexible arrays, and nested owned-resource structs | `STGMEDIUM`, `STRRET`, `BINDPTR`, audio/media formats | Tagged pointer-input unions support only safely POD fields. Missing discriminants, nested unions, BSTR/interfaces/resources, bitfields, and flexible tails fail closed. | Win32 winmd + codegen diagnostics |
| Unknown packing or non-authoritative explicit offsets | Explicit/packed native records | Exact x86/x64/ARM64 size, alignment, and field offsets are mandatory. Missing facts fail closed rather than assuming the host compiler's defaults. | Layout validation policy |
| Writable caller-sized native arrays outside the modeled subset | Owning, pointer-element, native-POD, or incompletely described arrays | `IDispatch::GetIDsOfNames` is supported in the complete interface when one metadata count unambiguously groups borrowed string pointers with plain scalar output. Missing count direction, element layout/ownership, or an unambiguous group still fails closed. | Win32 winmd `NativeArrayInfo` + semantic validation |
| Unsupported BSTR pointer nesting, replacement arrays, or callee-allocated outer arrays | Automation collection APIs | Exact counted input/caller-output BSTR arrays are supported; deeper/replacement shapes still require authoritative outer allocation and per-element contracts. | Win32 winmd signature + ownership analysis |
| Caller-owned ANSI output buffers | `PSTR` output-buffer APIs | Safe sizing and decoding are not yet projected. | Win32 winmd signature + projection limitation |
| Private-data bytes or interface pointer | `IDXGIObject`, `ID3D10DeviceChild`, `ID3D10Device`, `ID3D11DeviceChild`, `ID3D11Device`, `ID3D12Object`, and `IDMLObject` `GetPrivateData` | The same GUID-keyed method may return ordinary bytes or an AddRef'd interface pointer. A `Buffer` projection would lose the interface ownership transfer; Direct3D 10 NULL calls are destructive, and DXGI's data parameter is required. | Exact Win32 winmd identities + Microsoft method documentation |
| Untyped output pointers without allocator/ownership | `IAudioClient::IsFormatSupported` and unrelated `void*` outputs | The runtime cannot infer whether the result is borrowed, COM-owned, `CoTaskMem`, or another allocator. | Win32 winmd + codegen diagnostics |
| Interface `[in, out]` ownership | Generic typed `IFoo**` InOut parameters | Replacing an existing interface pointer requires explicit release/AddRef transfer semantics. `IWbemServices::OpenNamespace` is not in this category: exact SDK evidence corrects its two flags to Out. | Win32 winmd + codegen diagnostic |
| Callback methods outside the validated implementation subset | Automation providers, custom marshaling, and resource-owning callbacks | The dynamic backend supports broad scalar/string/POD/buffer ABI shapes and multi-interface inheritance, but VARIANT/SAFEARRAY/PROPVARIANT callbacks, untagged unions, unknown pointers/allocators, interface replacement, and custom marshal contracts still fail the whole interface closed. | Runtime/codegen validation boundary |
| COM aggregation | `IClassFactory::CreateInstance` with `pUnkOuter` | The public activation helper always creates a non-aggregated in-process object. | Runtime/public-API boundary |
| General out-of-process activation controls | Custom `CLSCTX` scenarios | The unsafe runtime's `DynCom.coCreateInstance()` currently uses `CLSCTX_INPROC_SERVER`. | Runtime/public-API boundary |
| Flat Win32 DLL exports | `CreateFile`, registry functions, GDI, etc. | These are not COM interfaces and need a separate DLL-export/handle model. | Architecture boundary |

Consequently, `IDataObject` remains an important, widely encountered interface
that is not currently supported as a complete generated binding.
`IPropertyStore` and `IDispatch` are complete; derived Automation interfaces
still validate all of their additional methods independently.

## Complete-interface census after by-value VARIANT

A temporary full census against
`Microsoft.Windows.SDK.Win32Metadata` **71.0.14-preview** parsed 7,944
interfaces. The temporary census code and generated output are not retained in
the repository.

| Result | Before | After | Delta |
|---|---:|---:|---:|
| Complete interfaces | 4,773 | 5,188 | **+415** |
| Incomplete interfaces | 3,171 | 2,756 | **-415** |

All 415 interfaces whose first blocker was a bare required input VARIANT
became complete; no previously complete interface regressed. The remaining
first-blocker categories are:

| Category | Interfaces |
|---|---:|
| Ownership/cleanup | 1,113 |
| Arrays/buffers | 666 |
| Native layout | 437 |
| Other validated contracts | 252 |
| SAFEARRAY | 146 |
| Pointer semantics | 142 |

The largest exact first blockers are the cited `GetPrivateData` ownership
hazards: `ID3D12Object` (82), `ID3D11DeviceChild` (58), `IDXGIObject` (39),
and `ID3D10DeviceChild` (24), followed by PROPVARIANT InOut `GetItem`
contracts (19).

Automation remains intentionally partial. Across incomplete interfaces, 143
first fail on an unsupported SAFEARRAY element and 54 first fail on
VARIANT/PROPVARIANT/SAFEARRAY BYREF/InOut replacement. Eleven first fail on a
bare VARIANT aggregate in a non-input direction, twelve on the analogous
PROPVARIANT shape, and `IEnumVARIANT::Next` remains one caller-sized VARIANT
array blocker. Existing supported pointer-shaped VARIANT inputs and owned
outputs are unchanged.

Real metadata regressions lock two representative APIs:

```ts
IAccessible.accSelect(flagsSelect: number, varChild: DynComVariant): void;
IUIAutomation.createPropertyCondition(
  propertyId: UIA_PROPERTY_ID,
  value: DynComVariant
): DynWinRtValue;
```

`IAccessible` (IID `618736e0-3c3d-11cf-810c-00aa00389b71`) now generates
completely, including `accSelect` at slot 21.
`IUIAutomation::CreatePropertyCondition` (IID
`30cbe57d-d9d0-452a-ab13-7ac5ac4825ee`) projects in isolation at slot 23;
the complete interface now passes its exact runtime-ID SAFEARRAY methods and
stops later at `ElementFromPoint.pt` because `POINT` has not reached the
validated native-layout projection.

## Complete-interface census after safe dynamic-IID generalization

The same temporary census against
`Microsoft.Windows.SDK.Win32Metadata` **71.0.14-preview** parsed 7,944
interfaces. The temporary census code and generated output are not retained.

| Result | Staged baseline | Generalized | Net delta |
|---|---:|---:|---:|
| Complete interfaces | 5,188 | 5,200 | **+12** |
| Incomplete interfaces | 2,756 | 2,744 | **-12** |

Thirteen interfaces became complete because a required `iid`/`riid` and its
owned `void**` result may now be non-adjacent or non-terminal:

`IDCompositionSurface`, `IDCompositionTexture`, `IXpsDocumentConsumer`,
`IAMGraphStreams`, `IPSFactoryBuffer`, `IClassFactory2`,
`IRemoteSystemAdditionalInfoProvider`, `ISearchLanguageSupport`,
`ICompositionDrawingSurfaceInterop`, `ICompositionDrawingSurfaceInterop2`,
`ICompositionTextureInterop`, `ISurfaceImageSourceNativeWithD2D`, and
`ICustomDestinationList`.

`INetCfg` intentionally moved from complete to incomplete:
`QueryNetCfgClass.ppvObject` is optional in metadata, so it no longer receives
owned-COM adoption. No required-output interface regressed.

The remaining first-blocker categories are:

| Category | Interfaces |
|---|---:|
| Ownership/cleanup | 1,101 |
| Arrays/buffers | 666 |
| Native layout | 436 |
| Other validated contracts | 253 |
| SAFEARRAY | 146 |
| Pointer semantics | 142 |

The top exact blockers remain the cited `GetPrivateData` ownership hazards:
`ID3D12Object` (82), `ID3D11DeviceChild` (58), `IDXGIObject` (39), and
`ID3D10DeviceChild` (24), followed by PROPVARIANT InOut `GetItem` contracts
(19).

Forty-three interfaces (67 methods) now contain an ambiguous
dynamic-IID-like shape:
31 contain InOut/interface-array contracts, nine use optional `void**`
outputs, two combine the otherwise valid pair with hidden counted buffers,
and one (`IEnumObjects::Next`) marks the interface output with
`NativeArrayInfo`. `IDxcResult::GetOutput`,
`IDirectManipulationViewport2::GetTag`, and
`INetCfg::QueryNetCfgClass` are representative optional-output blockers;
`IMFTopologyServiceLookup::LookupService` remains blocked by its InOut
multi-interface count.

## Complete-interface census after BSTR normalization and replacement

The executable BSTR census used
`Microsoft.Windows.SDK.Win32Metadata` **71.0.14-preview** and selected 7,930
currently eligible interfaces (IUnknown-rooted interfaces plus the existing
`*Interop` selection). This is deliberately narrower than the 7,944 raw
interfaces reported by older census sections; before/after counts below use
the same 7,930-interface selector. Temporary census code and output are not
retained.

The deduplicated declared BSTR shapes were:

| Raw metadata shape | Occurrences |
|---|---:|
| Required by-value input | 4,201 |
| Optional by-value input | 27 |
| Scalar `BSTR*` input | 3 |
| Input BSTR array | 7 |
| Required scalar `BSTR*` InOut | 323 |
| Optional scalar `BSTR*` InOut | 4 |
| `BSTR**` InOut | 2 |
| Invalid by-value Out | 5 |
| Required scalar `BSTR*` Out | 3,099 |
| Out BSTR array | 5 |
| Optional scalar `BSTR*` Out | 27 |
| `BSTR**` Out | 6 |
| `BSTR**` Out array | 1 |

The four optional metadata-InOut occurrences are documentation-defined Out
parameters: `IPhotoAcquireDeviceSelectionDialog::DoModal.pbstrDeviceId` and
the three `IDiscRecorder::GetDisplayNames` strings. The exact Microsoft API
declarations are cited in the semantic override registry. No nullable generic
BSTR replacement contract is inferred.

Before this phase, 120 interfaces first failed on a scalar BSTR replacement
method; 38 were inherited descendants. The complete-interface result is:

| Result | Before | After | Net delta |
|---|---:|---:|---:|
| Complete interfaces | 5,188 | 5,289 | **+101** |
| Incomplete interfaces | 2,742 | 2,641 | **-101** |

The simulated bucket was larger because 18 replacement candidates expose another
unsupported contract after BSTR replacement becomes valid.
`IPhotoAcquireDeviceSelectionDialog` is one example: its BSTR is correctly
normalized to Out, but `pnDeviceType` remains an optional scalar InOut
contract without a safe nullable lowering. `IDiscRecorder` becomes complete
and returns its three display-name strings deterministically.
Separately, `IFileSearchBand` deliberately moves from complete to incomplete:
its scalar input `BSTR*` had previously degraded to a raw pointer even though
the pointee allocation/borrow contract is not proven.

The largest remaining exact first-error labels in this selector are
`unsupported COM semantic` (215), `invalid COM contract` (52),
PROPVARIANT `GetItem.pValue` (19), `GetWindow.phwnd` (14),
`MovedReferences` (11), and `GetKernelConnectionOptions` (9).

The supported ownership proof follows the Automation rules documented by
[Memory Management Rules](https://learn.microsoft.com/windows/win32/com/memory-management-rules)
and
[SysAllocStringLen](https://learn.microsoft.com/windows/win32/api/oleauto/nf-oleauto-sysallocstringlen):

- by-value input: the caller allocates/frees; the callee borrows;
- Out: the callee allocates; the caller owns and frees the result;
- InOut: the caller supplies the initial BSTR, the callee may free/replace it,
  and the caller frees the valid final slot;
- on failure, the InOut slot must remain unchanged or contain another safely
  cleanable value (including null).

Runtime storage is a boxed, uniquely owned call-local BSTR slot. Allocation
uses explicit UTF-16 lengths, never NUL-terminated inference. On success the
final slot transfers once to `takeBstr`; on failure RAII frees the valid final
slot. Fake-vtable tests cover embedded NUL input/output, unchanged/replaced/null
success and failure, original-string immutability, and exact allocation/free
counts.

At the end of that scalar-BSTR phase, before counted owning elements were
implemented, the following remained excluded:

- three scalar input `BSTR*` contracts (`IFileSearchBand::SetSearchParameters`
  and the two VSS `SaveAsXML` methods);
- all 13 native BSTR pointer-array occurrences because initialized-range
  element cleanup is incomplete; these are distinct from exact
  `SAFEARRAY(BSTR)` declarations, which are covered by the registry below;
- all nine `BSTR**` occurrences because pointer nesting/array ownership is not
  proven;
- five invalid by-value Out occurrences; and
- unknown/custom allocators or any BSTR shape not paired with exact Automation
  ownership evidence.

## Complete-interface census after exact SAFEARRAY subtype evidence

The executable census used
`Microsoft.Windows.SDK.Win32Metadata` **71.0.14-preview** and the same 7,930
eligible-interface selector as the BSTR phase. It deduplicated declarations at
their declaring interface, so inherited methods were not counted repeatedly.
Temporary census code and output are not retained.

The metadata contains 239 directly declared `SAFEARRAY` parameters. The exact
registry covers 209 declarations and leaves 30 direct declarations
unsupported. Every registry row keys the declaring namespace, interface IID,
method name, vtable slot, parameter index/name, and complete raw method shape.
Each row also records element VARTYPE, exact element IID when applicable,
borrowed-input or owned-output semantics, a reason, and a Microsoft citation.
An identity or raw-shape mismatch is a contract error, not a generic fallback.

| Documented element VARTYPE | Registry entries |
|---|---:|
| `VT_I4` | 25 |
| `VT_UI1` | 25 |
| `VT_UI4` | 3 |
| `VT_R8` | 3 |
| `VT_BSTR` | 40 |
| `VT_VARIANT` | 85 |
| `VT_UNKNOWN` plus exact interface IID | 28 |
| **Total** | **209** |

Of these entries, 69 are borrowed inputs and 140 are owned outputs. Major
families include UI Automation (61), File Server Resource Manager (50),
Mobile Broadband (25), IMAPI (25), Performance Logs and Alerts (20),
Component Services (9), Remote Desktop (8), tuner APIs (5), WMI (3), and
Camera UI (1).

Representative evidence:

- UI Automation runtime IDs are documented `SAFEARRAY(int)` (`VT_I4`);
  text selection/visible ranges are
  [`ITextRangeProvider*`](https://learn.microsoft.com/windows/win32/api/uiautomationcore/nf-uiautomationcore-itextprovider-getselection)
  arrays with IID `5347ad7b-c355-46f8-aff5-909033582f63`.
- Connected-client snapshots are documented
  [`IUIAutomationClientInfo*`](https://learn.microsoft.com/windows/win32/api/uiautomationcore/nf-uiautomationcore-iuiautomationclientinfosource-getconnectedclients)
  arrays with IID `b2e8a3f1-4c5d-4e7a-8f6b-3d2e1c9a0b8f`.
- PLA BSTR properties use the exact method sections in
  [MS-PLA](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-pla/);
  this covers alert thresholds, API tracing filters, configuration files and
  queries, collector-set keywords, and performance counters.
- `ITraceDataProvider::FilterData` is a documented
  [byte array](https://learn.microsoft.com/windows/win32/api/pla/nf-pla-itracedataprovider-get_filterdata)
  (`VT_UI1`).
- Registered workspace extensions and Camera UI selected-item paths are
  documented BSTR arrays by
  [`GetRegisteredFileExtensions`](https://learn.microsoft.com/windows/win32/api/workspaceax/nf-workspaceax-iworkspacerestyperegistry-getregisteredfileextensions)
  and
  [`GetSelectedItems`](https://learn.microsoft.com/windows/win32/api/camerauicontrol/nf-camerauicontrol-icamerauicontrol-getselecteditems).

`ITextProvider::GetSelection` and
[`IRawElementProviderFragment::GetEmbeddedFragmentRoots`](https://learn.microsoft.com/windows/win32/api/uiautomationcore/nf-uiautomationcore-irawelementproviderfragment-getembeddedfragmentroots)
have separate exact evidence that a successful output may be null. Both
project as `DynComSafeArray | null`; `GetVisibleRanges` and all other
registered outputs remain required.

The complete-interface result is:

| Result | Before | After | Net delta |
|---|---:|---:|---:|
| Complete interfaces | 5,289 | 5,399 | **+110** |
| Incomplete interfaces | 2,641 | 2,531 | **-110** |
| SAFEARRAY first blockers | 147 | 17 | **-130** |

The first-blocker decrease is larger than the complete-interface gain because
20 interfaces advance to another unsupported contract. The remaining 17
SAFEARRAY first blockers are:

- Mobile Broadband provisioned-context/device-service arrays (2);
- WinHTTP input `SAFEARRAY**` event data (1);
- Component Services module/query arrays, including inherited catalog
  amplification (3);
- IIS provider configuration records (1);
- Task Scheduler and Transaction Server metadata-InOut arrays (2);
- IME InOut arrays (1);
- Tablet PC nested/interface arrays (2);
- XAML Diagnostics record arrays inherited through three interfaces (3); and
- MSHTML event-listener and document-write arrays whose exact Automation
  VARTYPE contracts are not consistently documented (2).

Generated JavaScript keeps every supported result as `DynComSafeArray`, not a
natural array, so VARTYPE, exact interface IID, rank, and signed lower bounds
remain observable. Generated signatures use
`DynCom.safeArrayType(kind, iid?, nullable?)`;
owned results transfer through `DynCom.takeSafeArray` or, for the two proven
nullable methods, `DynCom.takeNullableSafeArray`. `DynComSafeArray.interface`
creates `VT_UNKNOWN` arrays with `SafeArrayCreateEx`; its `interfaceIid`,
`bounds`, `elementType`, and conversion methods preserve identity and shape.

Input descriptors are borrowed for the native call under a per-array lock and
remain owned by the caller. Output `SAFEARRAY**` storage starts null, adopts
the callee result once, validates VARTYPE, rank, bounds, element width, data
alignment, and every non-null typed-interface element with `QueryInterface`,
and calls `SafeArrayDestroy` on every mismatch or failed conversion. An exact
descriptor IID is accepted only when it matches the documented IID. A
descriptor carrying `IID_IUnknown`, or no `FADF_HAVEIID` identity at all, is
accepted only after every element passes that exact query; this is required by
stock UI Automation providers such as
[Microsoft Terminal](https://github.com/microsoft/terminal/blob/main/src/types/ScreenInfoUiaProviderBase.cpp),
which creates its documented text-range results with
`SafeArrayCreateVector(VT_UNKNOWN, ...)`.
[`SafeArrayGetIID`](https://learn.microsoft.com/windows/win32/api/oleauto/nf-oleauto-safearraygetiid)
documents the missing-`FADF_HAVEIID` `E_INVALIDARG` result. The wrapper records
the proven semantic IID after validation. Typed interface inputs are
QueryInterface-validated before creation; SafeArray element
insertion/removal provides the corresponding AddRef/Release behavior. There is
no second dispatch backend.

## Counted arrays with owning elements

An executable census against `Microsoft.Windows.SDK.Win32Metadata`
**71.0.14-preview** deduplicated declarations at their declaring interface.
It found 13 BSTR arrays (7 input, 5 caller output, 1 callee allocated),
262 interface arrays (113 input, 142 caller output, 7 callee allocated), and
13 VARIANT arrays (3 input, 10 caller output). The caller-output totals include
in/out count contracts. Exact `IEnum*::Next` shapes occur at slots 3, 4, and
8; inherited declarations are not counted again.

Supported generated surfaces use `string[]`, `DynComVariant[]`, or nominal
generated interface-wrapper arrays. Input storage is call-local: BSTR uses
`SysAllocStringLen`, including embedded NULs; VARIANT uses contiguous
`VariantCopy` storage; interface values are queried for the exact element IID
and borrowed while those wrappers remain alive. Shared authoritative counts
may describe parallel input/output arrays, and signed count ABIs are accepted
only through checked non-negative conversions.

Caller output storage is zeroed across the full capacity; VARIANT slots are
additionally `VariantInit`-initialized. Successful calls validate
`actual/fetched <= capacity`, transfer only that initialized range, and clean
unused slots. Failed HRESULTs return no partial values and scan the bounded
capacity. Interface slots release each remaining non-null `+1`; BSTR slots use
`SysFreeString`; VARIANT slots use `VariantClear`; `IEnumString` PWSTR slots
use `CoTaskMemFree`. Validation and conversion are transactional, including
null interface holes and unsupported/BYREF VARIANT tags.

`IEnumVARIANT` (IID `00020404-0000-0000-c000-000000000046`) and
`IEnumString` (IID `00000101-0000-0000-c000-000000000046`) follow their exact
element ABIs and preserve `S_OK`/`S_FALSE`. `pceltFetched` is nullable only
where the exact standard metadata contract permits it and only for a request
of one element. Incomplete element interfaces receive an opaque nominal
wrapper containing no projected methods, rather than weakening the owning
array API or projecting unsafe members.

Enumerator projection is keyed by exact namespace, interface IID, and element
type; an `IEnum*` name is not ownership evidence. The exact enumerator registry
contains 97 declarations, including `IEnumUnknown`, `IEnumVdsObject`,
`IEnumEventObject`, and `IEnumITfCompositionView`. Likewise, only
NUL-terminated `*STR` aliases are automatic strings. `PWCHAR`, `PCWCHAR`,
`LPWCH`, `LPCWCH`, `LPCH`, and `LPCCH` require an explicit counted-character
contract.

The complete-interface census moved from 5,399 to **5,531** of 7,930 eligible
interfaces: **+132 complete** and **-132 incomplete**. Seven owning-array first
blockers remain, all with unknown element semantics: XML issuer pointer lists,
DirectWrite target-family pointer lists, two debugger handle arrays, Shell
PIDL arrays, Text Services category arrays, and Text Services property arrays.
The broader top blockers remain untyped `GetPrivateData` ownership contracts,
PROPVARIANT replacement/InOut contracts, incomplete native layouts, and
unknown pointer/allocator semantics.

Excluded shapes remain fail closed: owning callee-allocated outer arrays
without an exact outer allocator and per-element contract; BSTR/COM
`T**`/`T***` arrays without exact allocation evidence; unknown pointer, handle,
PIDL, or nested-owning elements; unsupported VARIANT tags/BYREF; and any
count/capacity/actual/fetched relationship that cannot be proven.

## Borrowed HWND outputs and the final complete-interface census

The final safety phase used `Microsoft.Windows.SDK.Win32Metadata`
**71.0.14-preview**. The literal generator census contains 7,929 externally
addressable Classic COM interface identities. This is one fewer than older
7,930-interface census sections because the final selector excludes the
IID-less `Windows.Win32.UI.Controls.RichEdit.ITextHost2`; its HWND declaration
is still included in the raw audit below.

An HWND output is never accepted merely because its native type is `HWND`.
Each supported output must match an exact evidence-registry entry by declaring
namespace/interface/IID, method, absolute vtable slot, parameter count,
parameter index/name, and optionality. The raw contract must also be an
ordinary HRESULT method with a required mutable `[out]` qualified
`Windows.Win32.Foundation.HWND*`, pointer depth one, and no `FreeWith`,
array, SAFEARRAY, or InOut semantics. Any registry or signature drift fails
before generic projection.

The 22 accepted declarations are:

| Declaration | IID | Slot / parameter | Microsoft ownership evidence |
|---|---|---|---|
| `IWiaAppErrorHandler::GetWindow` | `6c16186c-d0a6-400c-80f4-d26986a0e734` | 3 / `0: phwnd` | [Returns the existing WIA error-handler dialog HWND](https://learn.microsoft.com/previous-versions/windows/desktop/wia/-wia-iwiaapperrorhandler-getwindow); it may be null and remains owned by the handler. |
| `IPhotoProgressDialog::GetWindow` | `00f246f9-0750-4f08-9381-2cd8e906a4ae` | 4 / `0: phwndProgressDialog` | [Retrieves the progress dialog box handle](https://learn.microsoft.com/windows/win32/api/photoacquire/nf-photoacquire-iphotoprogressdialog-getwindow); it does not create or transfer the dialog. |
| `IOverlay::GetWindowHandle` | `56a868a1-0ad4-11ce-b03a-0020af0ba770` | 8 / `0: pHwnd` | [Retrieves the existing clipping window associated with the overlay](https://learn.microsoft.com/windows/win32/api/strmif/nf-strmif-ioverlay-getwindowhandle). |
| `IMSVidCtl::get_Window` | `b0edf162-910a-11d2-b632-00c04f79498e` | 15 / `0: phwnd` | [Retrieves the existing video control window](https://learn.microsoft.com/windows/win32/api/msvidctl/nf-msvidctl-imsvidctl-get_window). |
| `IMSVidRect::get_HWnd` | `7f5000a6-a440-47ca-8acc-c0e75531a2c2` | 15 / `0: HWndVal` | [Retrieves the window represented by the existing video rectangle](https://learn.microsoft.com/windows/win32/api/segment/nf-segment-imsvidrect-get_hwnd). |
| `IMFPMediaPlayer::GetVideoWindow` | `a714590a-58af-430a-85bf-44f5ec838d85` | 31 / `0: phwndVideo` | [Retrieves the media player's current video window](https://learn.microsoft.com/windows/win32/api/mfplay/nf-mfplay-imfpmediaplayer-getvideowindow). |
| `IMFVideoDisplayControl::GetVideoWindow` | `a490b1e4-ab84-4d31-a1b2-181e03b1077a` | 10 / `0: phwndVideo` | [Retrieves the video window previously set on the display control](https://learn.microsoft.com/windows/win32/api/evr/nf-evr-imfvideodisplaycontrol-getvideowindow). |
| `IConsole::GetMainWindow` | `43136eb1-d36c-11cf-adbc-00aa00a80033` | 12 / `0: phwnd` | [Retrieves MMC's existing main frame window](https://learn.microsoft.com/windows/win32/api/mmc/nf-mmc-iconsole-getmainwindow). |
| `IOleWindow::GetWindow` | `00000114-0000-0000-c000-000000000046` | 3 / `0: phwnd` | [Retrieves an existing participant window](https://learn.microsoft.com/windows/win32/api/oleidl/nf-oleidl-iolewindow-getwindow); ownership remains with the participant that created it. |
| `ICoreWindowInterop::get_WindowHandle` | `45d64a29-a63e-4cb6-b498-5781d298cb4f` | 3 / `0: hwnd` | [Gets the HWND of the existing CoreWindow](https://learn.microsoft.com/windows/win32/api/corewindow/nf-corewindow-icorewindowinterop-get_windowhandle). |
| `IShareWindowCommandEventArgsInterop::GetWindow` | `6571a721-643d-43d4-aca4-6b6f5f30f1ad` | 3 / `0: value` | [Gets the window carried by the event arguments](https://learn.microsoft.com/windows/win32/api/sharewindowcommandsourceinterop/nf-sharewindowcommandsourceinterop-isharewindowcommandeventargsinterop-getwindow). |
| `IDesktopWindowXamlSourceNative::get_WindowHandle` | `3cbcf1bf-2f76-4e9c-96ab-e84b37972554` | 4 / `0: hWnd` | [Gets the existing parent UI-element HWND](https://learn.microsoft.com/windows/win32/api/windows.ui.xaml.hosting.desktopwindowxamlsource/nf-windows-ui-xaml-hosting-desktopwindowxamlsource-idesktopwindowxamlsourcenative-get_windowhandle). |
| `IUpdateInstaller::get_ParentHwnd` | `7b929c68-ccdc-4226-96b1-8724600b54c2` | 11 / `0: retval` | [Retrieves the existing configured parent window](https://learn.microsoft.com/windows/win32/api/wuapi/nf-wuapi-iupdateinstaller-get_parenthwnd). |
| `IUIAutomationElement::get_CachedNativeWindowHandle` | `d22108aa-8ac5-49a5-837b-37bbb3d7591e` | 68 / `0: retVal` | [Retrieves the cached native window handle of the existing element](https://learn.microsoft.com/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-get_cachednativewindowhandle). |
| `IUIAutomationElement::get_CurrentNativeWindowHandle` | `d22108aa-8ac5-49a5-837b-37bbb3d7591e` | 36 / `0: retVal` | [Retrieves the current native window handle of the existing element](https://learn.microsoft.com/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-get_currentnativewindowhandle). |
| `ICredentialProviderCredentialEvents::OnCreatingWindow` | `fa6fa76b-66b7-4b11-95f1-86171118e816` | 12 / `0: phwndOwner` | [Returns the Credential UI or Logon UI parent HWND](https://learn.microsoft.com/windows/win32/api/credentialprovider/nf-credentialprovider-icredentialprovidercredentialevents-oncreatingwindow), which providers borrow when parenting dialogs. |
| `ILaunchSourceViewSizePreference::GetSourceViewToPosition` | `e5aa01f7-1fb8-4830-8720-4e6734cbd5f3` | 3 / `0: hwnd` | [Gets the existing source application window](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ilaunchsourceviewsizepreference-getsourceviewtoposition). |
| `IFileIsInUse::GetSwitchToHWND` | `64a1cbf0-3a1a-4461-9158-376969693950` | 6 / `0: phwnd` | [Retrieves the existing application window to switch to](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileisinuse-getswitchtohwnd). |
| `IPreviewHandler::QueryFocus` | `8895b1c6-b41f-4c1c-a562-0d564250836f` | 8 / `0: phwnd` | [Returns the HWND observed by GetFocus](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ipreviewhandler-queryfocus). |
| `ITextInputPanel::get_AttachedEditWindow` | `6b6a65a5-6af3-46c2-b6ea-56cd1f80df71` | 3 / `0: AttachedEditWindow` | [Retrieves the edit window already attached to the text input panel](https://learn.microsoft.com/windows/win32/api/peninputpanel/nf-peninputpanel-itextinputpanel-get_attachededitwindow). |
| `ITfContextOwner::GetWnd` | `aa80e80c-2021-11d2-93e0-0060b067b86e` | 7 / `0: phwnd` | [Retrieves the existing owner window associated with the text context](https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-itfcontextowner-getwnd). |
| `ITfContextView::GetWnd` | `2433bf8e-0f9b-435c-ba2c-180611978c30` | 6 / `0: phwnd` | [Retrieves the existing window represented by the text context view](https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-itfcontextview-getwnd). |

At runtime, a borrowed HWND result uses zeroed pointer-width output storage and
`PointerOutputKind::None`, with no `Release`, `DestroyWindow`, or other cleanup
on success or failure. JavaScript converts the pointer bits with
`DynCom.asPointerBigint()` and exposes the existing natural
`HWND = bigint | number` alias; null is `0n`. It never exposes a Buffer whose
contents could be confused with its address.

Exact canonical `IUnknown` (`00000000-0000-0000-c000-000000000046`) elements
in owning counted/enumerator arrays now project directly as
`DynWinRtValue[]`. The runtime's existing per-element `+1` adoption and
`Release` cleanup are unchanged; only wrapper rendering changes. No
`IUnknown.js` file or import is generated, and unresolved non-IUnknown
interfaces remain nominal and fail closed as before. Real metadata regressions
cover `IEnumUnknown`, `IEnumVdsObject`, and `IEnumEventObject`.

### Complete HWND declaration audit

The raw metadata census found 58 unique declaring HWND-output parameters.
The 22 entries above are the only ones admitted to the borrowed registry.
Every other declaration remains excluded, even where documentation suggests
an observed/borrowed handle, because the final target was already met or the
complete method/interface contains another unmodeled contract.

| Declaration (IID) | Slot / parameter / shape | Documentation | Classification |
|---|---|---|---|
| `IDirectDrawClipper::GetHWnd` (`6c14db85-a733-11ce-a521-0020af0be560`) | 4 / `0: param0` / `HWND* InOut` | [Microsoft](https://learn.microsoft.com/windows/win32/api/ddraw/nf-ddraw-idirectdrawclipper-gethwnd) | Getter-like, but metadata is InOut; excluded before borrowed-output projection. |
| `IDXGIFactory::GetWindowAssociation` (`7b7166ec-21c7-44ae-b21a-c9ae321ae369`) | 9 / `0: pWindowHandle` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/dxgi/nf-dxgi-idxgifactory-getwindowassociation) | Documented borrowed identity; not registered because inherited DXGI interfaces remain blocked by `GetPrivateData` ownership. |
| `IDXGISwapChain1::GetHwnd` (`790a45f7-0d42-4876-983a-0a55cfe6f4aa`) | 20 / `0: pHwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgiswapchain1-gethwnd) | Documented borrowed identity; not registered because the complete DXGI interface remains blocked elsewhere. |
| `IAMDirectSound::GetFocusWindow` (`546f4260-d53e-11cf-b3f0-00aa003761c5`) | 10 / `0: param0` / `HWND* InOut` | [Microsoft](https://learn.microsoft.com/windows/win32/api/amaudio/nf-amaudio-iamdirectsound-getfocuswindow) | InOut contract; excluded. |
| `IFullScreenVideo::GetMessageDrain` (`dd1d7110-7836-11cf-bf47-00aa0055595a`) | 12 / `0: hwnd` / `HWND* Out` | No attached Microsoft documentation | Ownership/lifetime not proven; excluded. |
| `IFullScreenVideoEx::GetAcceleratorTable` (`53479470-f1dd-11cf-bc42-00aa00ac74f6`) | 21 / `0: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/amvideo/nf-amvideo-ifullscreenvideoex-getacceleratortable) | HWND is observed, but the method also returns an accelerator-table resource; excluded pending its complete ownership model. |
| `IOverlay::GetWindowHandle` (`56a868a1-0ad4-11ce-b03a-0020af0ba770`) | 8 / `0: pHwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/strmif/nf-strmif-ioverlay-getwindowhandle) | Registered exact borrowed identity; another interface contract still blocks completeness, so this row adds no complete interface. |
| `IMSVidCtl::get_Window` (`b0edf162-910a-11d2-b632-00c04f79498e`) | 15 / `0: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/msvidctl/nf-msvidctl-imsvidctl-get_window) | Registered exact borrowed property; completes `IMSVidCtl`. |
| `IMSVidRect::get_HWnd` (`7f5000a6-a440-47ca-8acc-c0e75531a2c2`) | 15 / `0: HWndVal` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/segment/nf-segment-imsvidrect-get_hwnd) | Registered exact borrowed property; completes `IMSVidRect`. |
| `IMSVidVRGraphSegment::get_Owner` (`dd47de3f-9874-4f7b-8b22-7cb2688461e7`) | 21 / `0: Window` / `HWND* Out` | No attached Microsoft documentation | Ownership/lifetime not proven; excluded. |
| `IMFPMediaPlayer::GetVideoWindow` (`a714590a-58af-430a-85bf-44f5ec838d85`) | 31 / `0: phwndVideo` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/mfplay/nf-mfplay-imfpmediaplayer-getvideowindow) | Registered exact borrowed video window; completes `IMFPMediaPlayer`. |
| `IMFVideoDisplayControl::GetVideoWindow` (`a490b1e4-ab84-4d31-a1b2-181e03b1077a`) | 10 / `0: phwndVideo` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/evr/nf-evr-imfvideodisplaycontrol-getvideowindow) | Registered exact borrowed video window; another interface contract still blocks completeness, so this row adds no complete interface. |
| `IWMPPluginUI::Create` (`4c5e8f9f-ad3e-4bf9-9753-fcd30d6d38dd`) | 4 / `1: phwndWindow` / `HWND* InOut` | [Microsoft](https://learn.microsoft.com/windows/win32/api/wmpplug/nf-wmpplug-iwmppluginui-create) | Creates a plug-in window and is InOut; lifecycle-owned/destroyable, excluded. |
| `IPhotoAcquireOptionsDialog::Create` (`00f2b3ee-bf64-47ee-89f4-4dedd79643f2`) | 4 / `1: phWndDialog` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/photoacquire/nf-photoacquire-iphotoacquireoptionsdialog-create) | Creates a dialog with a separate lifecycle; excluded. |
| `ISpThreadControl::StartThread` (`a6be4d73-4403-4358-b22d-0346e23b1764`) | 4 / `1: phwnd` / `HWND* Out` | No attached Microsoft documentation | Starts a thread/window lifecycle paired with stop behavior; not a generic borrowed getter. |
| `IAuthenticate::Authenticate` (`79eac9d0-baf9-11ce-8c82-00aa004ba90b`) | 3 / `0: phwnd` / `HWND* Out` | No attached Microsoft documentation | Parent-window ownership contract not proven from attached evidence; excluded. |
| `IAuthenticateEx::AuthenticateEx` (`2ad1edaf-d83d-48b5-9adf-03dbe19f53bd`) | 4 / `0: phwnd` / `HWND* Out` | No attached Microsoft documentation | Parent-window ownership contract not proven from attached evidence; excluded. |
| `IInternetSecurityMgrSite::GetWindow` (`79eac9ed-baf9-11ce-8c82-00aa004ba90b`) | 3 / `0: phwnd` / `HWND* Out` | No attached Microsoft documentation | Site-window lifetime not proven from attached evidence; excluded. |
| `IWindowForBindingUI::GetWindow` (`79eac9d5-bafa-11ce-8c82-00aa004ba90b`) | 3 / `1: phwnd` / `HWND* Out` | No attached Microsoft documentation | Binding-UI parent lifetime not proven from attached evidence; excluded. |
| `IActiveScriptSiteWindow::GetWindow` (`d10f6761-83e9-11cf-8f20-00805f2cd064`) | 3 / `0: phwnd` / `HWND* Out` | No attached Microsoft documentation | Site-window lifetime not proven from attached evidence; excluded. |
| `IWebApplicationHost::get_HWND` (`cecbd2c3-a3a5-4749-9681-20e9161c6794`) | 3 / `0: hwnd` / `HWND* InOut` | [Microsoft](https://learn.microsoft.com/windows/win32/api/webapplication/nf-webapplication-iwebapplicationhost-get_hwnd) | InOut metadata; excluded. |
| `IDataSourceLocator::get_hWnd` (`2206ccb2-19c1-11d1-89e0-00c04fd7a829`) | 7 / `0: phwndParent` / `HWND* Out` | No attached Microsoft documentation | Parent-window lifetime not proven; excluded. |
| `IUpdateInstaller::get_ParentHwnd` (`7b929c68-ccdc-4226-96b1-8724600b54c2`) | 11 / `0: retval` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/wuapi/nf-wuapi-iupdateinstaller-get_parenthwnd) | Registered exact borrowed configured-parent property; completes the four-member `IUpdateInstaller` family. |
| `IDesktopWindowTargetInterop::get_Hwnd` (`35dbf59e-e3f9-45b0-81e7-fe75f4145dc9`) | 3 / `0: value` / `HWND* Out` | No attached Microsoft documentation | Target-window lifetime not proven from attached evidence; excluded. |
| `IWindowGraphicsCaptureItemInterop::GetWindow` (`38e4c48b-94e6-4c44-9cfa-968193316c0c`) | 3 / `0: window` / `HWND* InOut` | No attached Microsoft documentation | InOut metadata; excluded. |
| `IIsolatedEnvironmentInterop::GetHostHwndInterop` (`85713c2e-8e62-46c5-8de2-c647e1d54636`) | 3 / `1: hostHwnd` / `HWND* Out` | No attached Microsoft documentation | Host-window lifetime not proven; excluded. |
| `IAccPropServices::DecomposeHwndIdentityString` (`6e26e776-04f0-495d-80e4-3330352e3169`) | 11 / `2: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/oleacc/nf-oleacc-iaccpropservices-decomposehwndidentitystring) | Decoded borrowed identity, but the multi-output contract was not needed for final coverage. |
| `IUIAutomationElement::get_CachedNativeWindowHandle` (`d22108aa-8ac5-49a5-837b-37bbb3d7591e`) | 68 / `0: retVal` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-get_cachednativewindowhandle) | Registered exact borrowed cached identity; together with the current-value row completes the nine-member `IUIAutomationElement` family. |
| `IUIAutomationElement::get_CurrentNativeWindowHandle` (`d22108aa-8ac5-49a5-837b-37bbb3d7591e`) | 36 / `0: retVal` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-get_currentnativewindowhandle) | Registered exact borrowed current identity; together with the cached-value row completes the nine-member `IUIAutomationElement` family. |
| `ITextHost2::TxGetWindow` (IID absent) | 43 / `0: phwnd` / `HWND* InOut` | [Microsoft](https://learn.microsoft.com/windows/win32/api/textserv/nf-textserv-itexthost2-txgetwindow) | IID-less callback declaration plus InOut metadata; excluded from projection and the final denominator. |
| `IActiveIMMApp::GetDefaultIMEWnd` (`08c0e040-62d1-11d1-9326-0060b067b86e`) | 26 / `1: phDefWnd` / `HWND* Out` | No attached Microsoft documentation | Lifetime not proven; excluded. |
| `IActiveIMMIME::CreateSoftKeyboard` (`08c03411-f96b-11d0-a475-00aa006bcc59`) | 73 / `4: phSoftKbdWnd` / `HWND* Out` | No attached Microsoft documentation | Creates a soft-keyboard window with a separate destroy lifecycle; excluded. |
| `IActiveIMMIME::GetDefaultIMEWnd` (`08c03411-f96b-11d0-a475-00aa006bcc59`) | 26 / `1: phDefWnd` / `HWND* Out` | No attached Microsoft documentation | Lifetime not proven; excluded. |
| `IBrowserService2::CreateViewWindow` (`68bd21cc-438b-11d2-a560-00a0c92dbfe8`) | 45 / `3: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/shdeprecated/nf-shdeprecated-ibrowserservice2-createviewwindow) | Creates a view window with explicit lifecycle; excluded. |
| `IBrowserService2::GetViewWindow` (`68bd21cc-438b-11d2-a560-00a0c92dbfe8`) | 47 / `0: phwndView` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/shdeprecated/nf-shdeprecated-ibrowserservice2-getviewwindow) | Borrowed existing view window; not needed for final coverage. |
| `IBrowserService2::v_MayGetNextToolbarFocus` (`68bd21cc-438b-11d2-a560-00a0c92dbfe8`) | 88 / `4: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/shdeprecated/nf-shdeprecated-ibrowserservice2-v_maygetnexttoolbarfocus) | Borrowed focus target, but deprecated multi-parameter semantics remain outside the registry. |
| `IFileIsInUse::GetSwitchToHWND` (`64a1cbf0-3a1a-4461-9158-376969693950`) | 6 / `0: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileisinuse-getswitchtohwnd) | Registered exact borrowed application window; another interface contract still blocks completeness, so this row adds no complete interface. |
| `IFolderFilter::GetEnumFlags` (`9cc22886-dc8e-11d2-b1d0-00c04f8eeb3e`) | 4 / `2: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifolderfilter-getenumflags) | Borrowed owner identity in a multi-output contract; not needed for final coverage. |
| `IShellBrowser::GetControlWindow` (`000214e2-0000-0000-c000-000000000046`) | 13 / `1: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellbrowser-getcontrolwindow) | Borrowed existing control window; not needed for final coverage. |
| `IShellMenu::GetMenu` (`ee1f7637-e138-11d1-8379-00c04fd918d0`) | 8 / `1: phwnd` / optional `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellmenu-getmenu) | Borrowed identity, but optional output is outside the required-output registry. |
| `IShellView::CreateViewWindow` (`000214e3-0000-0000-c000-000000000046`) | 9 / `4: phWnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellview-createviewwindow) | Creates a view window paired with `DestroyViewWindow`; excluded. |
| `IShellView3::CreateViewWindow3` (`ec39fa88-f8af-41c5-8421-38bed28f4673`) | 20 / `8: phwndView` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/shobjidl/nf-shobjidl-ishellview3-createviewwindow3) | Creates a view window paired with view destruction; excluded. |
| `ITextInputPanel::get_AttachedEditWindow` (`6b6a65a5-6af3-46c2-b6ea-56cd1f80df71`) | 3 / `0: AttachedEditWindow` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/peninputpanel/nf-peninputpanel-itextinputpanel-get_attachededitwindow) | Registered exact borrowed attached edit window; completes `ITextInputPanel`. |
| `ITextStoreACP::GetWnd` (`28888fe3-c2a0-483a-a3ea-8cb1ce51ff3d`) | 28 / `1: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/textstor/nf-textstor-itextstoreacp-getwnd) | Borrowed owner window; not needed for final coverage. |
| `ITextStoreAnchor::GetWnd` (`9b2077b0-5f18-4dec-bee9-3cc722f5dfe0`) | 26 / `1: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/textstor/nf-textstor-itextstoreanchor-getwnd) | Borrowed owner window; not needed for final coverage. |
| `ITfContextOwner::GetWnd` (`aa80e80c-2021-11d2-93e0-0060b067b86e`) | 7 / `0: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-itfcontextowner-getwnd) | Registered exact borrowed owner window; completes `ITfContextOwner`. |
| `ITfContextView::GetWnd` (`2433bf8e-0f9b-435c-ba2c-180611978c30`) | 6 / `0: phwnd` / `HWND* Out` | [Microsoft](https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-itfcontextview-getwnd) | Registered exact borrowed view window; completes `ITfContextView`. |
| `IEnumManagerFrames::Next` (`3caa826a-9b1f-4a79-bc81-f0430ded1648`) | 3 / `1: ppWindows` / `HWND** Out` | No attached Microsoft documentation | Ambiguous pointer-to-pointer array/ownership contract; excluded. |

The 22 registry declarations make 45 complete HWND-bearing interface
projections after inheritance. The 12 additions above recover 19 complete
interfaces: nine in the `IUIAutomationElement` family, four in the
`IUpdateInstaller` family, and six individual interfaces. `IOverlay`,
`IMFVideoDisplayControl`, and `IFileIsInUse` remain incomplete for unrelated
contracts and therefore add no complete-interface census entries. After
removing enum-name ownership inference, requiring distinct actual-length
parameters to be exact Out values, and separating counted character pointers
from terminated strings, the exact final literal census is
**5,567 / 7,929 = 70.210619%**. The result remains above the 70% target without
admitting any creator-owned, destroyable, InOut, undocumented, optional, or
`HWND**` shape.

CI reproduces this number with `dynwinrt-codegen com-census --json` and fails
if the denominator changes, complete generation drops below 5,567, or coverage
falls below 70%.

## Public-code frequency snapshot

There is no authoritative Microsoft ranking of COM interface usage. The table
below is a reproducible demand proxy based on public GitHub code, not runtime
telemetry.

The snapshot was collected on **2026-07-29** with GitHub code search:

```text
<TOKEN> extension:cpp
  NOT path:test
  NOT path:tests
  NOT path:third_party
  NOT path:vendor
  NOT path:external
  NOT path:generated
```

The survey selected 30 representative desktop COM interfaces across COM
infrastructure, Shell, OLE, graphics, audio, WMI, XML, and WebView2.
`IID_IDispatch` and `IID_IStream` were searched instead of their bare names to
reduce collisions with unrelated classes and C++ `std::istream`.

Two metrics are reported:

- **`.cpp` hits** is GitHub's total matching-file count after the best-effort
  path exclusions above.
- **Repos / first 100** is the number of distinct repositories represented in
  the first 100 matching files. It prevents one large repository from being
  mistaken for broad adoption, but it is not a count of every matching
  repository.

Vendored code can still appear under other directory names, search ranking and
repository contents change over time, and interfaces used through wrappers may
not mention the native symbol. Treat the numbers as relative prevalence only.

Each candidate was then checked against
`Microsoft.Windows.SDK.Win32Metadata` **69.0.7-preview**
`Windows.Win32.winmd`, and the current generator was run with `--dry-run`
against the resolved namespace.

| Rank | Interface/search token | `.cpp` hits | Repos / first 100 | In Win32 winmd | Current codegen |
|---:|---|---:|---:|---|---|
| 1 | `ID3D11Device` | 27,552 | 87 | Yes | Fail closed: untyped output ownership |
| 2 | `IDXGIFactory` | 17,432 | 83 | Yes | Fail closed: untyped output ownership |
| 3 | `IDataObject` | 10,648 | 44 | Yes | Fail closed: `STGMEDIUM` has unmodeled discriminant/resource ownership |
| 4 | `IMalloc` | 10,624 | 56 | Yes | Generates completely and is live-tested through `CoGetMalloc` |
| 5 | `IClassFactory` | 6,712 | 70 | Yes | Generates completely and is live-tested through `CoGetClassObject` |
| 6 | `IDispatch` via `IID_IDispatch` | 6,408 | 46 | Yes | Complete inherited interface generates; `Invoke` uses dedicated DISPPARAMS/EXCEPINFO and explicit optional-output requests |
| 7 | `IPersistFile` | 5,996 | 97 | Yes | Generates and live-tested |
| 8 | `IConnectionPoint` | 5,832 | 51 | Yes | Generates; callback objects passed to `Advise` must satisfy the complete validated same-thread implementation subset |
| 9 | `IWbemServices` | 5,680 | 76 | Yes | Fail closed: interface in/out ownership |
| 10 | `IWICImagingFactory` | 4,536 | 83 | Yes | Generates and live-tested |
| 11 | `IDropTarget` | 4,368 | 57 | Yes | Generates for client calls and dynamic JavaScript implementation; live E2E covers by-value `POINTL`, scalar/InOut callback ABI, libffi dispatch, and multi-interface QueryInterface |
| 12 | `IShellFolder` | 4,056 | 33 | Yes | Fail closed: untyped PIDL output ownership (and later `STRRET` union ABI) |
| 13 | `IFileDialog` | 4,048 | 98 | Yes | Generates; inherited methods tested through `IFileOpenDialog` |
| 14 | `IXMLDOMDocument` | 3,784 | 46 | Yes | Fail closed: inherited unsupported Automation shapes beyond scalar VARIANT |
| 15 | `ID2D1Factory` | 3,752 | 92 | Yes | Generates; requires flat factory acquisition and native input structs |
| 16 | `IDWriteFactory` | 3,712 | 76 | Yes | Generates; requires flat factory acquisition |
| 17 | `IStream` via `IID_IStream` | 3,560 | 41 | Yes | Generates completely; WIC live coverage exercises inherited buffers, seek, `STATSTG`, and Clone HRESULT propagation |
| 18 | `IPropertyStore` | 3,400 | 77 | Yes | Generates completely and is live-tested with an unsaved ShellLink |
| 19 | `IShellItem` | 3,028 | 76 | Yes | Generates; acquisition/live test still needed |
| 20 | `IMMDeviceEnumerator` | 2,932 | 83 | Yes | Generates; live result depends on audio services/devices |
| 21 | `IBindCtx` | 2,660 | 42 | Yes | Generates with exact `BIND_OPTS.cbStruct = sizeof(BIND_OPTS)` initialization and live `CreateBindCtx` coverage |
| 22 | `IFileOpenDialog` | 2,536 | 92 | Yes | Generates and live-tested without showing UI |
| 23 | `IRunningObjectTable` | 2,532 | 50 | Yes | Generates with POD `FILETIME`; acquisition/live test still needed |
| 24 | `IAudioClient` | 2,500 | 82 | Yes | Fail closed: format pointer/output ownership |
| 25 | `IShellLinkW` | 2,128 | 67 | Yes | Generates and is live-tested with nested/fixed-array POD `WIN32_FIND_DATAW` |
| 26 | `ITaskbarList3` / `TaskbarList` | 1,672 | 87 | Yes | Interface and newable coclass generate and are live-tested |
| 27 | `ICoreWebView2` | 1,608 | 35 | **No** | Defined in WebView2 metadata, not Windows.Win32.winmd |
| 28 | `IFileSaveDialog` | 1,188 | 96 | Yes | Generates; live test still needed |
| 29 | `IFileOperation` | 768 | 79 | Yes | Generates and live-tested |
| 30 | `ITaskService` | 461 | 72 | Yes | Fail closed: inherited unsupported Automation shapes |

### What the snapshot shows

- **29 of 30** candidates are defined as `IUnknown`-rooted interfaces in
  Windows.Win32.winmd. `ICoreWebView2` is the only external-metadata case.
- **23 of 29** Win32-metadata candidates pass complete codegen validation.
  **6 of 29** fail closed on an unsupported ABI or ownership shape.
- Among the **top 10** by `.cpp` hits, only `IClassFactory`,
  `IDispatch`, `IPersistFile`, `IConnectionPoint`, `IWICImagingFactory`, and
  `IMalloc` pass complete codegen.
- The largest unsupported demand clusters are:
  - discriminated resource unions and non-POD layout (`IDataObject`, parts of Shell and streams);
  - Automation contracts beyond the exact supported `IDispatch` compounds
    (XML and Task Scheduler BYREF/InOut and nested ownership);
  - explicit output ownership (`DXGI`, audio);
  - interface in/out semantics (WMI); and
  - unsupported PROPVARIANT alternatives in Property System APIs beyond
    `IPropertyStore`.
- Ten frequency-survey candidates have generated live coverage:
  `IPersistFile`, `IWICImagingFactory`, `IFileDialog` through
  `FileOpenDialog`, `TaskbarList`, `FileOperation`, `IShellLinkW`, `IStream`,
  `IPropertyStore`, `IMalloc`, and `IClassFactory`. `IBindCtx` also has live
  runtime coverage.

This means the current suite provides useful ABI breadth, but it
does **not** cover every high-frequency interface. In particular,
`IDataObject`, graphics interfaces, WMI, audio, and live real-object
`IDispatch::Invoke` coverage remain material gaps.

## Engineering priority map

The frequency snapshot is only one input. Test priority also considers stock
Windows availability, deterministic behavior, whether an API requires UI or
hardware, and whether it adds a distinct ABI shape.

| Interface | Typical use | Current status |
|---|---|---|
| `ISequentialStream` / `IStream` | OLE streams, imaging, shell, serialization | Complete generation includes documented `Read`/`Write` byte contracts and owned `STATSTG`; WIC live coverage exercises buffers, seek, stat, and Clone HRESULT propagation. |
| `IOpcSignatureCustomObject` | OPC signature custom XML | `GetXml` generates as a CoTaskMem-owned callee byte buffer; acquisition is application-specific. |
| `IDiscRecorder` | Legacy IMAPI recorder | Complete generation now includes the exact `GetRecorderGUID` two-call method and documentation-correct `getDisplayNames(): [string, string, string]` BSTR outputs. |
| `IMalloc` | COM task allocator | Complete generation is gated by exact IID/slot/shape evidence. Opaque values reject forged/stale addresses; destructive and size operations enforce allocator identity, while `DidAlloc` permits borrowed cross-allocator inspection. |
| `IPersistFile` | Loading and saving persistent COM objects | Core tests query it from `IShellLinkW`; Node activates the Shell Link coclass directly as `IPersistFile` and verifies `GetClassID`. |
| `IShellLinkW` | Shortcut creation and inspection | Core runtime tests cover strings, `u16`, enums, and scalar outputs; generated Node coverage round-trips `GetPath` with nested `FILETIME` and fixed WCHAR-array `WIN32_FIND_DATAW` POD storage. |
| `FileOpenDialog` | Desktop file selection | Node test covers coclass construction and option round-trip without showing UI. |
| `FileOperation` | Shell copy/move/delete operations | Node test covers coclass construction, unsigned flags, and state without modifying files. |
| `IWICImagingFactory` | Windows Imaging Component | Node test activates WIC and creates an interface-valued stream. |
| `TaskbarList` / `ITaskbarList3` | Taskbar progress and window state | Node test covers `new`, inherited vtable slots, HWND values, BOOL, enums, `u64`, and `as`/`tryAs`/`supports`. |
| `IDataTransferManagerInterop` | HWND-to-WinRT data-transfer bridge | Core and Node tests cover `IUnknown`-rooted interop and interface output. |
| `ISystemMediaTransportControlsInterop` | HWND-to-WinRT media controls | Node test covers `IInspectable`-rooted interop and use of the returned WinRT object. |
| `IClassFactory` | Low-level COM activation | Complete generation and public `CoGetClassObject` acquisition are live-tested with paired server locking and owned `CreateInstance` output. |
| `IBindCtx` / `IRunningObjectTable` | Monikers and object binding | Both generate. `BIND_OPTS` carries an exact size initializer, and explicit bytes with a zero or incorrect `cbStruct` fail before native dispatch. |
| `ICreateErrorInfo` / `IErrorInfo` | COM rich error information | Complete generation and acquisition are live-tested for GUID, wide strings, owned BSTR output, thread-local storage, and one-shot consumption. |
| `IMMDeviceEnumerator` | Audio endpoint discovery | Generates today, but live behavior depends on available audio endpoints. |
| `IAudioClient` | Low-level audio streaming | Fails closed because its format and output-pointer shapes are not fully modeled. |
| `IDispatch` | Automation and scripting | Complete inherited real-metadata generation passes. `GetIDsOfNames` projects as `string[] -> number[]`; `Invoke` accepts `DynComDispatchParams` and explicit result/excepInfo/argErr request options, returning dedicated owning wrappers. Derived Automation interfaces remain independently validated. |
| `IPropertyStore` | Shell/property metadata | Complete generation and live ShellLink `SetValue`/`GetValue`/`Commit` coverage pass with dedicated PROPVARIANT ownership. |
| `IDataObject` | Clipboard and drag-and-drop | Unsupported until FORMATETC and STGMEDIUM are modeled. |

## Automated coverage

Classic COM interfaces are exercised across core and generated Node coverage.
Core live tests are in
[`crates/dynwinrt/src/com.rs`](../../crates/dynwinrt/src/com.rs). The seventeen Node
runners are in
[`tests/e2e/runners/com`](../../tests/e2e/runners/com) and are generated
and executed by [`tests/e2e/e2e_test.ps1`](../../tests/e2e/e2e_test.ps1).

Automation coverage remains scoped to proven contracts rather than claimed as
general interface support. Local fake COM vtables copy pointer-shaped VARIANT,
SAFEARRAY, and PROPVARIANT inputs to owned outputs and verify conversion,
failure cleanup, typed-array rejection, interface AddRef/Release balance, and
native-union active-field validation. SAFEARRAY tests additionally cover
signed multidimensional bounds, typed scalar/bool/BSTR/VARIANT values, exact
interface IID creation/inspection/mismatch cleanup, generic-descriptor
per-element validation, nullable output, and descriptor VARTYPE/width/rank
validation. Dedicated BSTR fake-vtable tests pass
embedded-NUL input and output values and exercise unchanged, replaced, and
nulled InOut slots on both successful and failed HRESULT paths with exact
allocation/free counters. Separate x64 and i686 fake-vtable tests pass scalar,
BSTR, and interface VARIANT values by value and verify windows-rs layout,
libffi aggregate classification, deep-copy isolation, HRESULT failure cleanup,
panic-safe drop, wrong argument shape, unsupported tags, BYREF rejection, and
output/InOut signature rejection. Real Windows.Win32 metadata regressions
inspect `ITargetNotify2`, inherited `IExecAction2`, `IDiscRecorder`,
`IPhotoAcquireDeviceSelectionDialog`, BSTR arrays/double pointers/custom
cleanup, `IAccessible::accSelect`,
`IUIAutomation::CreatePropertyCondition`, IPropertyBag VARIANT pointer/InOut,
exact SAFEARRAY families across UI Automation, WMI, FSRM, PLA, Remote Desktop,
Camera UI, tuner, and Mobile Broadband APIs, IPropertyStore PROPVARIANT output,
and ITypeComp BINDPTR union facts. Complete interfaces still stop at their next
unrelated or unproven contract.

The exact `IDispatch::Invoke` contract uses a COM-local captured-HRESULT plan.
Successful calls expose only an optional `result` VARIANT. Failed calls discard
and clear `pVarResult`, finalize requested `EXCEPINFO` storage once, preserve a
meaningful `argErr`, and generate an `Error` with `hresult` plus optional
`excepInfo`, `argErr`, and deferred-fill `cause` fields.

| Interface | Test layer | Representative coverage |
|---|---|---|
| `IShellLinkW` | Core + Node E2E | Generated activation through its IID, wide strings, hotkeys/show command, and `GetPath` with zeroed 592-byte `WIN32_FIND_DATAW` POD storage. |
| `IPersistFile` | Core + Node E2E | `QueryInterface`/direct IID activation, owned returned reference, GUID output, and deterministic release. |
| `IMalloc` | Core + Node E2E | Exact opaque allocation projection, allocator identity for ownership-sensitive operations, borrowed `DidAlloc` inspection, automatic/explicit cleanup, resize, and direct scalar/void returns. |
| `IClassFactory` | Core + Node E2E | Public `CoGetClassObject`, owned factory reference, paired `LockServer`, dynamic-IID `CreateInstance`, and +1 output adoption. |
| `ICreateErrorInfo` / `IErrorInfo` | Core + Node E2E | Public acquisition, GUID/PWSTR setters, owned BSTR getters, thread-local isolation, and consume-on-read behavior. |
| `IStream` | Core + Node E2E | Typed counted byte input/output buffers, actual `u32` lengths, `i64` seek, owned `STATSTG`, `IStream**` clone ABI, and stock-WIC Clone HRESULT propagation. |
| `IPropertyStore` | Node E2E | Generated `PROPERTYKEY` POD and owned PROPVARIANT values against an unsaved ShellLink, including `Commit`. |
| `IBindCtx` | Core + Node | Exact multi-architecture `BIND_OPTS` layout, automatic `cbStruct`, pre-dispatch validation, and live `CreateBindCtx` round trip. |
| `TaskbarList` / `ITaskbarList3` | Node E2E | Coclass construction, inherited slots, runtime QI views, HWND, BOOL, enum, and `u64`. |
| `FileOperation` | Node E2E | Coclass construction, unsigned flags, and state query. |
| `FileOpenDialog` / `IFileDialogEvents` | Core + Node E2E | STA coclass construction, generated synchronous JS implementation, self-vtable callback dispatch, public native-value bridge, and real `Advise`/`Unadvise` without showing UI. |
| `IDropTarget` | Core + Node E2E | Dynamic libffi callbacks, interface/scalar/POD/InOut parameters, generated multi-interface composition, and QueryInterface to the additional view. |
| `IWICImagingFactory` | Node E2E | Explicit CLSID activation and typed interface output. |
| `IDataTransferManagerInterop` | Core + Node E2E | `IUnknown` base, HWND, REFIID, and WinRT interface output. |
| `ISystemMediaTransportControlsInterop` | Node E2E | `IInspectable` base and meaningful use of the returned WinRT projection. |

Additional regression tests cover:

- a test-only windows-rs ABI oracle for selected stable interfaces and native
  layouts: interface IIDs, host-target size/alignment, and field offsets for
  `RECT`, `THUMBBUTTON`, `WIN32_FIND_DATAW`, `DISPPARAMS`, `EXCEPINFO`, and
  the complete `IFileDialogEvents` callback vtable;
  windows-rs is not a production dispatch backend and does not replace
  semantic validation;
- rejection of duplicate ownership through exported pointer bits;
- generated COM sink Worker teardown, wrong-thread late invocation,
  JavaScript exception-to-HRESULT behavior, and callback-resource cleanup;
- detached TypedArray backing storage;
- BSTR exact-length allocation, replacement, null/failure cleanup, and
  `CoTaskMem` cleanup;
- exact SAFEARRAY registry identity/raw-shape drift, VARTYPE constants,
  windows-rs UI Automation IIDs, signed bounds, typed-interface descriptor and
  per-element QI validation, BSTR/scalar/interface cleanup, nullable output,
  and nearest unsupported input-`SAFEARRAY**`, InOut, and record-array shapes;
- x86 pointer width;
- x86/x64/ARM64 POD layout computation, nested structs, fixed arrays, and all
  value/pointer/out/in-out storage shapes;
- typed buffer zero initialization, actual-length slicing, bounded sizing,
  CoTaskMem transfer, failure suppression, detached backing, width, length,
  and alignment validation;
- exact fixed-capacity `IMFAttributes::GetBlob` metadata, natural GUID/capacity
  rendering, runtime-owned zeroed storage, successful actual-length slicing,
  capacity overflow, short-buffer failure, and ordinary HRESULT failure;
- exact cited rejection of all seven declaring `GetPrivateData` families whose
  payload may carry an AddRef'd interface, including mutation tests for IID,
  method/slot, REFGUID depth/constness, count direction/type, buffer
  direction/depth/optionality, actual count, and HRESULT return;
- deterministic fake-vtable `IEnum*::Next` partial-success, optional-fetched,
  fetched-overflow, interface transfer, unused-capacity, and failed-HRESULT
  cleanup, plus complete real-metadata generation of `IEnumGUID` and
  `IEnumConnectionPoints` (including the `IConnectionPoint` dependency);
- borrowed UTF-16/ANSI string-pointer arrays, stable pointer tables, shared
  count agreement, aligned zeroed scalar output arrays, and complete real
  `IDispatch` generation;
- fake-vtable DISPPARAMS reversal/named-ID/contiguous-storage checks and
  EXCEPINFO immediate/deferred success, callback failure, HRESULT failure,
  optional-null output, and conversion-failure cleanup;
- unsupported unions, bitfields, flexible arrays, unknown layouts, and nested
  owned fields;
- required parameter preservation;
- unsigned enum values;
- mixed, COM-only, and order-independent incremental package generation; and
- separation of `@microsoft/dynwinrt` from `@microsoft/dynwinrt/com`.

Run the live Classic COM suite with:

```powershell
$env:DYNWINRT_WIN32_WINMD = "C:\path\to\Windows.Win32.winmd"
.\tests\e2e\e2e_test.ps1 -SkipBuild -Lang com
```

## Reference counting and ownership

COM interface references and Win32 handles must not be treated the same.

| Value source | Ownership in dynwinrt | Cleanup |
|---|---|---|
| `CoCreateInstance` result | Owned `+1` COM reference | Automatic `Release` on `DynWinRtValue` drop/GC, or explicit `release()`. |
| `QueryInterface` / `cast()` result | Owned `+1` COM reference | Automatic `Release`, independently of the source wrapper. |
| Typed interface out parameter | Owned `+1` COM reference from the callee | Automatic `Release`. |
| Interface passed as `[in]` | Borrowed for the duration of the call | No ownership transfer unless the callee explicitly retains it with `AddRef`. |
| Numeric raw pointer | Borrowed | Never automatically released or freed. |
| Buffer/TypedArray pointer | Borrowed and owner-backed | Backing storage is retained and revalidated; it cannot be adopted as a COM owner. |
| Caller-owned counted output | Borrowed during the call; returned bytes are an owned exact-range copy | Native storage is zeroed first. Failed HRESULTs return no copied result. |
| `IEnum*::Next` interface element | Each non-null initialized slot is an owned `+1` reference | Fetched slots move once into managed values; slots are nulled on transfer. Failure, overflow, conversion error, and unused initialized capacity release remaining slots within the requested bound. |
| Callee-allocated counted `CoTaskMem` output | Allocation transfers once to the COM buffer plan | Exact bytes are copied and the original allocation is freed once with `CoTaskMemFree`. |
| `adoptComPointer()` input | Must be a native output carrying an existing `+1` reference | Ownership transfers to the returned wrapper. |
| Callee-allocated `CoTaskMem` string | Owned allocation | Generated conversion frees it with `CoTaskMemFree`. |
| By-value BSTR input | Caller-owned call-local allocation; borrowed by the callee | Runtime frees it after the call. |
| Scalar BSTR output | Owned allocation | Generated conversion frees it with `SysFreeString`. |
| Scalar BSTR InOut | Caller owns the initial and final slot; callee may free/replace the initial allocation | Runtime owns one unique call-local slot, transfers the successful final value once, and frees the valid final value on failure. |
| SAFEARRAY input | Caller-owned descriptor and elements, borrowed for the invocation | A per-array lock keeps the descriptor stable; no ownership transfer occurs. |
| SAFEARRAY output | Callee-owned descriptor transferred through `SAFEARRAY**` | Runtime adopts exactly once after HRESULT success and exact descriptor validation; RAII calls `SafeArrayDestroy` on rejection, conversion failure, or final drop. |
| Typed interface SAFEARRAY element | SafeArray-owned interface reference with an exact proven IID | Input creation and output adoption QueryInterface-validate each non-null value; descriptors with the exact IID, `IID_IUnknown`, or no IID are distinguished, and the latter two rely on element proof for the identity fallback. SafeArray APIs perform element AddRef/Release and `SafeArrayDestroy` releases remaining elements. |
| `HANDLE`, `HWND`, `HBITMAP`, etc. | Win32 resource value, not a COM reference | Use the resource-specific API such as `CloseHandle`, `DestroyWindow`, or `DeleteObject` when required. |

The JavaScript ownership provenance checks intentionally prevent turning a
borrowed numeric or TypedArray pointer into a second owner. This avoids two
wrappers releasing the same COM reference.

Every generated Classic COM class also exposes a public `release()` method
delegating to the wrapped `DynWinRTValue`'s own `release()`. Calling
`release()` more than once is safe (the underlying value is cleared to null
the first time).

## Test selection guidance

Prefer new CI tests that:

1. use stock Windows components;
2. require no network, optional software, or user input;
3. avoid persistent filesystem or system-state changes;
4. assert meaningful results rather than activation alone;
5. add a distinct ABI or ownership shape; and
6. clean up every COM reference, native allocation, and Win32 resource.

Interfaces that require Office, deprecated Internet Explorer automation,
active drag-and-drop, a populated clipboard, audio hardware, or an Explorer
desktop should remain optional or local-only tests.

## Related Microsoft documentation

- [Rules for managing COM reference counts](https://learn.microsoft.com/windows/win32/com/rules-for-managing-reference-counts)
- [IUnknown::QueryInterface](https://learn.microsoft.com/windows/win32/api/unknwn/nf-unknwn-iunknown-queryinterface(q))
- [IMalloc](https://learn.microsoft.com/windows/win32/api/objidl/nn-objidl-imalloc)
- [IStream](https://learn.microsoft.com/windows/win32/api/objidl/nn-objidl-istream)
- [IPersistFile](https://learn.microsoft.com/windows/win32/api/objidl/nn-objidl-ipersistfile)
- [IFileOperation](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation)
- [Windows Imaging Component overview](https://learn.microsoft.com/windows/win32/wic/-wic-about-windows-imaging-codec)
