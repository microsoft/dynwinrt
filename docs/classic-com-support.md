# Classic COM support

`dynwinrt` supports a deliberately limited subset of Classic COM. It is not a
general Automation or native Win32 projection.

The design keeps the existing WinRT API separate:

```js
import { DynWinRtType, DynWinRtValue } from '@microsoft/dynwinrt';
import { DynCom, DynComMethodSig } from '@microsoft/dynwinrt/com';
```

Both entrypoints use the same native N-API binary and private libffi call
machinery. Classic COM metadata, generated wrappers, ownership rules, and
public APIs remain separate from the WinRT projection.

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

Recognized UTF-16 output-buffer shapes are supported today. General writable
native arrays remain fail closed.

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

Required support:

- scalar and interface alternatives;
- BSTR and other owned strings;
- nested VARIANT values;
- SAFEARRAY and vector alternatives;
- `VariantInit`/`VariantClear` and `PropVariantClear`;
- language conversion and range checking; and
- DISPPARAMS argument order, named arguments, and EXCEPINFO.

This unlocks `IDispatch`, XML Automation, Task Scheduler, `IPropertyStore`, and
many scripting/management APIs.

### 6. SAFEARRAY

**Problem:** SAFEARRAY is a descriptor, not a pointer to a flat JavaScript
array. It carries rank, bounds, element type, locks, ownership, and potentially
non-blittable elements.

Required support includes multidimensional bounds, lower bounds, element
cleanup, interface/BSTR/VARIANT elements, and safe lock/unlock behavior.

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

### 8. Semantic HRESULT values

**Problem:** Most HRESULTs are throw-or-success, but methods such as
`IPersistFile::IsDirty` use `S_OK` versus `S_FALSE` as their actual result.
Discarding every successful HRESULT loses information.

The projection needs an explicit PreserveSig/semantic-HRESULT classification
instead of globally treating every non-negative HRESULT as `void`.

### 9. Apartment affinity and marshaling

**Problem:** A valid COM reference is not necessarily callable from every
thread. STA objects require the owning apartment or a marshaled proxy.

Required support includes:

- tracking the apartment where a value was acquired;
- preventing unsafe cross-thread calls;
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

1. General native struct/union layout.
2. Pointer-depth plus counted-buffer contracts.
3. Explicit allocator/ownership metadata.
4. VARIANT/PROPVARIANT and semantic HRESULT handling.
5. SAFEARRAY.
6. Arbitrary COM sink/interface implementation.
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
| Method return conventions | Supports normal HRESULT methods plus native direct scalar, direct pointer at the runtime layer, and direct `void` returns. |
| Basic parameter direction | Supports input, output, and scalar in/out parameters without reducing in/out to out-only. |
| Primitive ABI types | Signed/unsigned integers, floats, BOOL, HRESULT, GUID, enums, and `char16`. |
| Pointer-sized values | `ISize`/`USize` select the correct x86/x64 ABI width and JavaScript uses `bigint`. |
| GUID ABI | Full 16-byte GUID output storage plus GUID value and REFIID/REFGUID pointer patterns. |
| Unsigned enum values | COM-local enum metadata preserves unsigned values, including 32-bit high-bit flags and 64-bit `bigint` literals. |
| Standard COM references | `CoCreateInstance`, QueryInterface, and typed interface outputs carry an owned `+1` reference and release automatically. |
| Ownership provenance | Borrowed numeric/TypedArray pointers cannot be re-adopted as a second COM owner. Native owned outputs are consumed once. |
| Backing-storage lifetime | Buffer/TypedArray owners are retained and detached ArrayBuffers are rejected before native use. |
| Common string ownership | Scalar BSTR output uses `SysFreeString`; supported `PWSTR`/`PSTR` allocations use `CoTaskMemFree`. |
| Common interop pattern | Supports HWND + REFIID + `void**` bridges and adopts the returned interface reference. |
| Explicit COM initialization | Activation no longer silently chooses MTA; callers select STA or MTA with `DynCom.initialize()`. |
| Fail-closed generation | Unsupported structs, arrays, pointer outputs, ownership, and in/out shapes stop generation with a targeted error. |
| Consumable output | COM-only generation emits index declarations and package exports and preserves them across incremental generation. |

### Partially implemented

| Problem family | Supported subset | Remaining gap |
|---|---|---|
| Native pointers | Pointer width, depth preservation, borrowed pointers, handles, REFIID, and known interface outputs | General nullable/required semantics, arbitrary pointee storage, and all allocator contracts |
| Counted buffers | Recognized caller-owned UTF-16 output buffers and input Buffer pointers | General byte/element output arrays, two-call sizing, actual-length returns, ANSI output decoding |
| Native layout | Primitives, GUID, enum, handle-shaped typedefs, and manually described runtime structs | General metadata-driven struct/union/packing/bitfield layout |
| Allocator ownership | COM Release, BSTR, CoTaskMem, boxed GUID, retained JS buffers | LocalFree, custom allocators, allocator interfaces, unknown ownership |
| Interface pointers | Typed input/output interfaces, QueryInterface, dynamic IID output | Interface in/out replacement and arbitrary implemented sink interfaces |
| Apartments | Explicit initialization and same-thread invocation | Cross-apartment marshaling, GIT/agility handling, callback dispatch |
| Activation | In-process `CoCreateInstance` | `CoGetClassObject`, aggregation, arbitrary CLSCTX, and non-CoCreate factory functions |
| Direct pointer returns | Runtime signature supports them | Metadata codegen does not yet preserve raw-pointer direct-return semantics, so `IMalloc` generation fails closed |

### Not implemented

- general struct/union native layout;
- VARIANT, VARIANTARG, DISPPARAMS, and EXCEPINFO;
- PROPVARIANT and the Property System value model;
- SAFEARRAY;
- FORMATETC and STGMEDIUM;
- arbitrary COM event/callback sink generation;
- semantic `S_OK`/`S_FALSE` HRESULT projection;
- cross-thread/apartment marshaling; and
- the general flat-Win32 DLL-export and handle-cleanup layer.

## Supported ABI surface

| Capability | Status | Notes |
|---|---|---|
| `IUnknown` and `IInspectable` roots | Supported | User methods begin at vtable slot 3 or 6 respectively. Full inherited Classic COM slot numbering is preserved. |
| `HRESULT` methods | Supported | Failed HRESULTs become errors. |
| Native `void` returns | Supported | Used by interfaces such as `IMalloc`. |
| Direct scalar returns | Supported | Includes signed/unsigned integers, floating point values, and enums. |
| Direct pointer returns | Runtime supported; codegen partial | The runtime can describe a pointer return explicitly. Metadata codegen currently fails closed for interfaces such as `IMalloc` because it does not preserve the raw-pointer return kind. |
| `[in]`, `[out]`, and scalar `[in, out]` parameters | Supported | Unsupported composite in/out types fail generation. |
| Primitive integer and floating-point types | Supported | `i8` through `u64`, `f32`, `f64`, `BOOL`, and `HRESULT`. |
| `ISize` / `USize` | Supported | Projected with the target pointer width; verified by an i686 compile check. |
| GUID values and `REFIID`/`REFGUID` pointers | Supported | GUID out storage uses the full 16-byte layout. |
| Signed and unsigned enums/flags | Supported | Values up to unsigned 64-bit are preserved; 64-bit JavaScript values use `bigint`. |
| Typed interface parameters and outputs | Supported | Interface outputs carry an owned COM reference. |
| Opaque pointers and handle-shaped typedefs | Supported with limits | They are pointer values, not COM objects. Cleanup remains type-specific. |
| NUL-terminated string pointer inputs | Supported | Callers pass a NUL-terminated `Buffer` or a borrowed numeric pointer. |
| Caller-owned UTF-16 output buffers | Supported for recognized shapes | The generator allocates and decodes the buffer when metadata identifies the count parameter. |
| Callee-allocated `PWSTR` / `PSTR` outputs | Supported | Generated code decodes and frees `CoTaskMem` storage. |
| Scalar `[out] BSTR*` | Supported | Generated code converts the BSTR and releases it with `SysFreeString`. |
| Explicit apartment initialization | Supported | `DynCom.initialize()` never silently chooses an apartment for the caller. |

The runtime can manually describe some ABI shapes that the generator rejects.
For example, a carefully defined native struct can be called from Rust, but the
generator does not emit a struct until its native layout is known to be
correct.

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
| `VARIANT` / `VARIANTARG` | `IDispatch::Invoke`, Automation APIs | Requires a discriminated union with ownership rules for BSTR, interfaces, arrays, decimals, and nested values. | Win32 winmd signature |
| `DISPPARAMS` / `EXCEPINFO` | `IDispatch::Invoke` | Contains VARIANT arrays, BSTR fields, and nested pointer ownership. | Win32 winmd signature |
| `PROPVARIANT` | `IPropertyStore`, Windows Property System | Larger discriminated union with vector, string, stream, and interface ownership. | Win32 winmd signature |
| `PROPERTYKEY` and arbitrary native structs | `IPropertyStore::GetAt` | Native struct layout, alignment, and architecture must be modeled explicitly. | Win32 winmd + codegen diagnostic |
| `SAFEARRAY` | Automation and Office-style COM APIs | Requires rank, bounds, element type, locking, and element cleanup semantics. | Win32 winmd Automation signatures |
| `FORMATETC` / `STGMEDIUM` | `IDataObject`, clipboard, drag-and-drop | `STGMEDIUM` is a union of handles and interfaces with type-specific release behavior. | Win32 winmd + codegen diagnostic |
| Arbitrary unions, bitfields, and nested pointer-rich structs | `D3D11_COUNTER_INFO`, `STATSTG`, `STRRET`, `POINTL`, `BIND_OPTS`, audio/media formats | The current generator has no general native C layout engine. | Win32 winmd + codegen diagnostics |
| Writable caller-sized native arrays | `IDispatch::GetIDsOfNames`, counted byte/element output buffers | A scalar pointee is not sufficient storage. These are rejected unless a supported string-buffer projection applies. | Win32 winmd `NativeArrayInfo` + codegen diagnostic |
| `BSTR**` arrays and BSTR in/out arrays | Automation collection APIs | Each element has independent allocation and release semantics. | Win32 winmd signature + ownership analysis |
| Caller-owned ANSI output buffers | `PSTR` output-buffer APIs | Safe sizing and decoding are not yet projected. | Win32 winmd signature + renderer limitation |
| Untyped output pointers without allocator/ownership | `IDXGIFactory::GetPrivateData`, `IAudioClient::IsFormatSupported` | The runtime cannot infer whether the result is borrowed, COM-owned, `CoTaskMem`, or another allocator. | Win32 winmd + codegen diagnostics |
| Interface `[in, out]` ownership | `IWbemServices::OpenNamespace` | Replacing an existing interface pointer requires explicit release/AddRef transfer semantics. | Win32 winmd + codegen diagnostic |
| Arbitrary COM sink/interface implementation | Connection points and event sinks | `Advise` requires implementing a caller-defined COM interface, not only invoking one. | Runtime/public-API boundary |
| COM aggregation | `IClassFactory::CreateInstance` with `pUnkOuter` | The public activation helper always creates a non-aggregated in-process object. | Runtime/public-API boundary |
| General out-of-process activation controls | Custom `CLSCTX` scenarios | `DynCom.coCreateInstance()` currently uses `CLSCTX_INPROC_SERVER`. | Runtime/public-API boundary |
| Flat Win32 DLL exports | `CreateFile`, registry functions, GDI, etc. | These are not COM interfaces and need a separate DLL-export/handle model. | Architecture boundary |

Consequently, `IDispatch`, `IPropertyStore`, and `IDataObject` are important
and widely encountered interfaces, but they are not currently supported as
complete generated bindings.

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
| 1 | `ID3D11Device` | 27,552 | 87 | Yes | Fail closed: native `D3D11_COUNTER_INFO` layout |
| 2 | `IDXGIFactory` | 17,432 | 83 | Yes | Fail closed: untyped output ownership |
| 3 | `IDataObject` | 10,648 | 44 | Yes | Fail closed: `STGMEDIUM`/union layout |
| 4 | `IMalloc` | 10,624 | 56 | Yes | Fail closed: direct raw-pointer return mapping; runtime tested |
| 5 | `IClassFactory` | 6,712 | 70 | Yes | Generates; acquisition helper and live test still needed |
| 6 | `IDispatch` via `IID_IDispatch` | 6,408 | 46 | Yes | Fail closed: counted arrays, VARIANT-family ABI |
| 7 | `IPersistFile` | 5,996 | 97 | Yes | Generates and live-tested |
| 8 | `IConnectionPoint` | 5,832 | 51 | Yes | Generates; implementing event sinks is not supported |
| 9 | `IWbemServices` | 5,680 | 76 | Yes | Fail closed: interface in/out ownership |
| 10 | `IWICImagingFactory` | 4,536 | 83 | Yes | Generates and live-tested |
| 11 | `IDropTarget` | 4,368 | 57 | Yes | Fail closed: native `POINTL` layout |
| 12 | `IShellFolder` | 4,056 | 33 | Yes | Fail closed: native `STRRET` union layout |
| 13 | `IFileDialog` | 4,048 | 98 | Yes | Generates; inherited methods tested through `IFileOpenDialog` |
| 14 | `IXMLDOMDocument` | 3,784 | 46 | Yes | Fail closed: inherits Automation/VARIANT ABI |
| 15 | `ID2D1Factory` | 3,752 | 92 | Yes | Generates; requires flat factory acquisition and native input structs |
| 16 | `IDWriteFactory` | 3,712 | 76 | Yes | Generates; requires flat factory acquisition |
| 17 | `IStream` via `IID_IStream` | 3,560 | 41 | Yes | Fail closed on `STATSTG`; safe runtime subset is live-tested |
| 18 | `IPropertyStore` | 3,400 | 77 | Yes | Fail closed: `PROPERTYKEY` and `PROPVARIANT` |
| 19 | `IShellItem` | 3,028 | 76 | Yes | Generates; acquisition/live test still needed |
| 20 | `IMMDeviceEnumerator` | 2,932 | 83 | Yes | Generates; live result depends on audio services/devices |
| 21 | `IBindCtx` | 2,660 | 42 | Yes | Fail closed: native `BIND_OPTS` layout |
| 22 | `IFileOpenDialog` | 2,536 | 92 | Yes | Generates and live-tested without showing UI |
| 23 | `IRunningObjectTable` | 2,532 | 50 | Yes | Fail closed: native `FILETIME` layout |
| 24 | `IAudioClient` | 2,500 | 82 | Yes | Fail closed: format pointer/output ownership |
| 25 | `IShellLinkW` | 2,128 | 67 | Yes | Generates and live-tested |
| 26 | `ITaskbarList3` | 1,672 | 87 | Yes | Generates and live-tested |
| 27 | `ICoreWebView2` | 1,608 | 35 | **No** | Defined in WebView2 metadata, not Windows.Win32.winmd |
| 28 | `IFileSaveDialog` | 1,188 | 96 | Yes | Generates; live test still needed |
| 29 | `IFileOperation` | 768 | 79 | Yes | Generates and live-tested |
| 30 | `ITaskService` | 461 | 72 | Yes | Fail closed: inherits Automation/VARIANT ABI |

### What the snapshot shows

- **29 of 30** candidates are defined as `IUnknown`-rooted interfaces in
  Windows.Win32.winmd. `ICoreWebView2` is the only external-metadata case.
- **14 of 29** Win32-metadata candidates pass complete codegen validation.
  **15 of 29** fail closed on an unsupported ABI or ownership shape.
- Among the **top 10** by `.cpp` hits, only `IClassFactory`,
  `IPersistFile`, `IConnectionPoint`, and `IWICImagingFactory` pass complete
  codegen. `IMalloc` has a tested runtime path but not a complete generated
  interface.
- The largest unsupported demand clusters are:
  - native structs/unions and layout (`D3D11`, `IDataObject`, Shell, streams);
  - Automation types (`IDispatch`, XML, Task Scheduler);
  - explicit output ownership (`DXGI`, audio);
  - interface in/out semantics (WMI); and
  - Property System types (`PROPERTYKEY`, `PROPVARIANT`).
- Seven frequency-survey candidates have generated live coverage:
  `IPersistFile`, `IWICImagingFactory`, `IFileDialog` through
  `IFileOpenDialog`, `IFileOpenDialog`, `IShellLinkW`, `ITaskbarList3`, and
  `IFileOperation`. `IMalloc` and `IStream` add runtime-only live coverage.

This means the current ten-interface suite provides useful ABI breadth, but it
does **not** cover every high-frequency interface. In particular,
`IDataObject`, `IDispatch`, `IPropertyStore`, graphics interfaces, WMI, and
audio remain material gaps.

## Engineering priority map

The frequency snapshot is only one input. Test priority also considers stock
Windows availability, deterministic behavior, whether an API requires UI or
hardware, and whether it adds a distinct ABI shape.

| Interface | Typical use | Current status |
|---|---|---|
| `IStream` | OLE streams, imaging, shell, serialization | Core live test covers counted buffers, seek, and interface output. |
| `IMalloc` | COM task allocator | Core live test covers direct pointer, pointer-sized, scalar, and void returns. |
| `IPersistFile` | Loading and saving persistent COM objects | Core and Node tests query it from `IShellLinkW` and verify `GetClassID`. |
| `IShellLinkW` | Shortcut creation and inspection | Core and Node tests cover strings, `u16`, enums, and scalar outputs. |
| `IFileOpenDialog` | Desktop file selection | Node test covers activation and option round-trip without showing UI. |
| `IFileOperation` | Shell copy/move/delete operations | Node test covers activation, unsigned flags, and state without modifying files. |
| `IWICImagingFactory` | Windows Imaging Component | Node test activates WIC and creates an interface-valued stream. |
| `ITaskbarList3` | Taskbar progress and window state | Node test covers inherited vtable slots, HWND values, BOOL, enums, and `u64`. |
| `IDataTransferManagerInterop` | HWND-to-WinRT data-transfer bridge | Core and Node tests cover `IUnknown`-rooted interop and interface output. |
| `ISystemMediaTransportControlsInterop` | HWND-to-WinRT media controls | Node test covers `IInspectable`-rooted interop and use of the returned WinRT object. |
| `IClassFactory` | Low-level COM activation | High-value next test; needs a public `CoGetClassObject` acquisition path. |
| `IBindCtx` / `IRunningObjectTable` | Monikers and object binding | High-value next test; needs acquisition helpers and validated native structs. |
| `ICreateErrorInfo` / `IErrorInfo` | COM rich error information | Good next test for GUID, wide strings, BSTR, and thread-local error state. |
| `IMMDeviceEnumerator` | Audio endpoint discovery | Generates today, but live behavior depends on available audio endpoints. |
| `IAudioClient` | Low-level audio streaming | Fails closed because its format and output-pointer shapes are not fully modeled. |
| `IDispatch` | Automation and scripting | Unsupported until VARIANT-family marshaling exists. |
| `IPropertyStore` | Shell/property metadata | Unsupported until PROPERTYKEY and PROPVARIANT are modeled. |
| `IDataObject` | Clipboard and drag-and-drop | Unsupported until FORMATETC and STGMEDIUM are modeled. |

## Automated coverage

Ten unique Classic COM interfaces are currently exercised.
Core live tests are in
[`crates/dynwinrt/src/com.rs`](../crates/dynwinrt/src/com.rs). The nine Node
runners are in [`tests/runners/com`](../tests/runners/com) and are generated
and executed by [`tests/e2e_test.ps1`](../tests/e2e_test.ps1).

| Interface | Test layer | Representative coverage |
|---|---|---|
| `IShellLinkW` | Core + Node E2E | Activation, wide strings, hotkeys, show command, and deterministic release. |
| `IPersistFile` | Core + Node E2E | `QueryInterface`, owned returned reference, and GUID output. |
| `IMalloc` | Core | Direct pointer return, `usize` return, direct `i32`, direct `void`, allocation cleanup. |
| `IStream` | Core | Counted byte buffer, `u32` output, `i64` seek, `u64` output, and `IStream**` clone. |
| `ITaskbarList3` | Node E2E | Inherited slots, HWND, BOOL, enum, and `u64`. |
| `IFileOperation` | Node E2E | Coclass activation, unsigned flags, and state query. |
| `IFileOpenDialog` | Node E2E | STA activation and get/set options without user interaction. |
| `IWICImagingFactory` | Node E2E | Explicit CLSID activation and typed interface output. |
| `IDataTransferManagerInterop` | Core + Node E2E | `IUnknown` base, HWND, REFIID, and WinRT interface output. |
| `ISystemMediaTransportControlsInterop` | Node E2E | `IInspectable` base and meaningful use of the returned WinRT projection. |

Additional regression tests cover:

- rejection of duplicate ownership through exported pointer bits;
- detached TypedArray backing storage;
- BSTR and `CoTaskMem` cleanup;
- x86 pointer width;
- unsupported native arrays and native struct layouts;
- required parameter preservation;
- unsigned enum values;
- COM-only package generation; and
- separation of `@microsoft/dynwinrt` from `@microsoft/dynwinrt/com`.

Run the live Classic COM suite with:

```powershell
$env:DYNWINRT_WIN32_WINMD = "C:\path\to\Windows.Win32.winmd"
.\tests\e2e_test.ps1 -SkipBuild -Lang com
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
| `adoptComPointer()` input | Must be a native output carrying an existing `+1` reference | Ownership transfers to the returned wrapper. |
| Callee-allocated `CoTaskMem` string | Owned allocation | Generated conversion frees it with `CoTaskMemFree`. |
| Scalar BSTR output | Owned allocation | Generated conversion frees it with `SysFreeString`. |
| `HANDLE`, `HWND`, `HBITMAP`, etc. | Win32 resource value, not a COM reference | Use the resource-specific API such as `CloseHandle`, `DestroyWindow`, or `DeleteObject` when required. |

The JavaScript ownership provenance checks intentionally prevent turning a
borrowed numeric or TypedArray pointer into a second owner. This avoids two
wrappers releasing the same COM reference.

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
