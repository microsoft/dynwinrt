# @microsoft/dynwinrt

**Call Windows Runtime (WinRT) APIs from JavaScript — without writing a native addon.**

`dynwinrt` is a runtime library that lets your Node.js or Electron code call modern Windows APIs (WinAppSDK, Windows AI, notifications, file pickers, sensors, storage, networking, …) **directly from JavaScript / TypeScript**, with full IntelliSense, no MSBuild step, no C++ or C# project, and no per-Windows-version recompile.

## Why use this?

If you've ever tried to call a Windows API from an Electron or Node app, you've probably run into one of these:

- **Writing a C++ `node-addon-api` addon.** Needs `node-gyp`, MSVC, Python, the right Windows SDK, and a CI matrix per Electron version.
- **Writing a C# addon via `node-api-dotnet`.** Needs the .NET SDK, a separate `csproj` build step, and a manually-maintained C# wrapper for every API surface you want to expose.
- **Waiting for a typed projection.** Some Windows APIs ship `.winmd` metadata months before any JavaScript-friendly projection appears in a published package.

`dynwinrt-codegen` reads the `.winmd` metadata that ships with the Windows SDK
and WinAppSDK ahead of time. The generated JavaScript registers the required
interface signatures, and `dynwinrt` resolves and invokes the COM vtables
dynamically at runtime. There is no native build step in the consuming Electron
or Node project. The same generated bindings can work across compatible
metadata revisions without rebuilding a native addon.

The runtime primarily targets **data-style WinRT APIs** (AI, storage, notifications, networking, globalization, …). It also supports WinUI `Application + Window` hosting through the generated `Application.create()` helper when the caller supplies an STA UI thread, an initialized Windows App SDK runtime, and application lifecycle. Unpackaged callers can initialize the runtime with `initWinappsdk()`; the helper resolves the framework resources from that package graph. It also enables Per-Monitor V2 DPI awareness on the UI thread.

### System DispatcherQueue loading

The system DispatcherQueue helper resolves
`CoreMessaging.dll!CreateDispatcherQueueController` only when it needs to create
a current-thread queue, loading the DLL from the Windows system directory.
Capturing an existing queue skips that resolution. A successful resolution
retains the module for the process lifetime so controllers and callbacks remain
valid; DLL or export failures are reported at queue creation and can be retried.
Importing the root or `/com` entrypoint does not load CoreMessaging for this
helper. Apartment, message-pumping, and shutdown requirements are unchanged.
This is not a general older-Windows compatibility or addon feature-isolation
guarantee.

`npm run test:imports` checks every `.node` file in `dist` directly, including
x64 and ARM64 artifacts, without loading it or requiring Visual Studio discovery
or `dumpbin`. It rejects CoreMessaging DLL imports (including ordinal imports)
and named CreateDispatcherQueueController imports in both ordinary and
RVA-based delay-import tables. Malformed tables, legacy VA-based delay
descriptors, and unknown delay-descriptor flags fail explicitly rather than
being ignored. The separate optional-Win32 eager-import check remains
ordinary-import-only.

The same test measures root and `/com` imports in separate fresh processes with
a 10-second limit. Loaded-DLL reports omit network enumeration and DNS on Node
versions that support `process.report.excludeNetwork`; failures include
report/import phase timings.

## Quick start

`@microsoft/dynwinrt` is the **runtime**. You generate the typed bindings ahead of time with [`@microsoft/dynwinrt-codegen`](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen), then import them at runtime:

```bash
npm install @microsoft/dynwinrt
npm install -D @microsoft/dynwinrt-codegen

# Generate a binding for Windows.Foundation.Uri
npx dynwinrt-codegen generate \
  --namespace Windows.Foundation \
  --class-name Uri \
  --output ./generated
```

```js
const { roInitialize } = require('@microsoft/dynwinrt');
const { Uri } = require('./generated');

roInitialize(1);                                      // MTA
const uri = new Uri('https://example.com/path?q=1');
console.log(uri.host);                                // "example.com"
console.log(uri.port);                                // 443
```

### Generated module layout

Generated WinRT implementations use canonical namespace paths. Prefer the root
barrel for concise imports:

```js
const { Uri, Button } = require('./generated');
```

Use the canonical path when a deep import is needed:

```js
const { Uri } = require('./generated/windows/foundation/Uri.js');
const { Button } = require(
  './generated/microsoft/ui/xaml/controls/Button.js'
);
```

Legacy flat paths such as `./generated/Uri.js` are not generated. When metadata
contains duplicate short names, each canonical module keeps its native symbol
name while the root barrel uses a namespace-qualified name, such as
`AIFoundationEmbeddingVector` or `SemanticSearchEmbeddingVector`.

### Standalone WinRT interface implementations

Metadata-generated standalone WinRT interfaces expose `implementation(handlers)`
and `implement(handlers, ...additionalDescriptors)`. The latter creates a native
instance and returns `DynWinRtImplementationHandle<GeneratedInterface>`:

```js
const impl = IBackgroundTask.implement(taskHandlers, {
  interfaces: [[IStringable, textHandlers]],
})
try {
  impl.value.run(instanceView)
} finally {
  impl.dispose()
}
```

`.value` is a stable, lazily created typed primary view, managed by the handle.
`release()` releases its primary view and owner without disconnecting other
native references; `dispose()` additionally disconnects the object. Neither
allows `.value` to recreate a view afterward. The common pattern does not need
`releaseProjected(impl.value)`.
`GeneratedInterface.fromImplementation(impl)` remains the advanced,
independently owned view path; positional descriptors and raw native owners
remain supported. Low-level users must select an interface with
`value.cast(iid)` before invoking that interface's method handles: `toValue()`
returns the canonical `IInspectable`, not an arbitrary interface's vtable.
These are local, synchronous, non-agile WinRT objects, not COM class
registrations or OS background-task registrations.

The npm root also exports the low-level primitives for metadata-driven callers:

```ts
DynWinRtInterfacePlan.create(name, interfaceType, [
  { name: 'ToString', vtableIndex: 6, signature },
], requiredIids)

DynWinRtImplementation.create(plans, (interfaceIndex, vtableIndex, args) => {
  return [DynWinRtValue.hstring('implemented in JavaScript')]
}, runtimeClassName)
```

Plans must describe each complete interface, including every contiguous native
slot starting at 6. Required interfaces need separate plans. The dispatcher
receives `DynWinRtValue[]` and must return an array of **all** outputs in signature
order (`[]` for void). Fill-array inputs are capacities; their returned arrays
must have exactly that length. Async functions, promises, and thenable results
are unsupported. Dispatch errors fail the native call; `takeError()` returns and
clears the latest diagnostic, or `null` when there is none.

In this low-level callback API, incoming reference values own retained native
references. A callback that rejects an input without retaining it should release
that `DynWinRtValue`, including on a throwing path. Waiting for garbage collection
can keep a delegate's event-loop resource alive. Conversely, a value deliberately
retained before throwing stays valid until explicitly released or collected.
This is not a requirement to release hidden arguments of generated typed
handlers: the generated adapters retain their own conversion/lifetime contract.

Low-level owner `release()` and garbage collection drop only its native reference.
Separately retained native values keep their callbacks alive. `disconnect()`
closes every view while retaining the owner reference; `dispose()` disconnects
and releases it, and both are idempotent. Disposing during a callback allows that
active invocation to finish safely. `isClosed` describes object-wide callback
disconnection, not merely whether this owner was released.

**Always dispose deterministically when handlers capture their owner or a native
view.** Such cross-runtime cycles cannot be collected by JavaScript alone.
Disposal breaks native callback roots; final reference cleanup runs on a later
event-loop turn. Cleanup TSFNs do not keep Node alive. Environment shutdown
disconnects all live implementations before releasing handler references, and
callbacks from other threads fail rather than touching the JS environment.

Native delegates received by an implementation can be invoked with
`value.invokeDelegate(iid, signature, args)`. For repeated calls, cache a
`DynWinRtDelegateMethod.create(iid, signature)` and use its `invoke(value, args)`
method instead. Both query the metadata's concrete delegate IID, retain the
managed values for the call, and return all outputs in signature order (`[]` for void). Delegate
`Invoke` occupies `IUnknown` slot 3; do not register it as an ordinary slot-6
WinRT interface method. The IID and full signature must come from delegate
metadata, including concrete type arguments for generic delegates. This helper
uses only the WinRT call planner and performs no COM registration.
Its arguments follow the existing outbound `invokeAll()` contract: a fill-array
input is a preallocated, correctly typed `DynWinRtArray.toValue()`, not a U32
capacity value. Only reverse-entry implementation handlers receive capacities.
Generated callable delegate bridges must allocate this outbound storage before
invoking the helper.

`DynWinRtDelegate.release()` deterministically drops the creator's native
reference, without disconnecting references retained by a native event source.
Event registration code must release its temporary `toValue()` result and the
creator in `finally` after the add call, on success and failure. A successful
event source owns the delegate independently until removal. Retaining a creator
in the same closure context as its native callback can otherwise form a
cross-runtime cycle and keep the delegate's referenced TSFN alive after
unsubscribe. Releasing the creator twice is harmless; `toValue()` after release
throws. This does not unref active event callbacks or force process exit.

For metadata whose native type is HRESULT, use `DynWinRtValue.fromHResult(code)`
(also available as `hresult(code)`)
or `DynWinRtArray.fromHresultValues(codes)` to preserve its semantic type,
especially for array parameters. Codes must be signed 32-bit integers (for
example, `0x80004005 | 0`). These factories do not change the existing `i32()`
or `fromI32Values()` behavior. Read results with `toNumber()` or `toI32Vec()`.

### Classic COM

Classic COM is a preview under active development. It uses a separate subpath
from the same package, keeping the WinRT root API unchanged:

```js
const { initializeCom } = require('@microsoft/dynwinrt/com');
initializeCom(1); // MTA
```

COM interface values returned by activation, `QueryInterface`, or typed
interface out-parameters own one reference and release it when their
`DynWinRtValue` is released or collected. Manual interface registration,
native signatures, and raw pointer ownership are isolated under
`@microsoft/dynwinrt/com/unsafe`. `adoptOwnedComPointer()` there consumes one
explicit caller-supplied `+1` reference; Buffer backing addresses are not
accepted. Win32 handles are not COM references and require their own
type-specific cleanup function.

See the repository's
[Classic COM JavaScript usage guide](../../docs/guides/windows/classic-com-usage.md) for
codegen, current coverage and limitations, GUID/IID/CLSID, lifetime, Automation,
and `/com/unsafe` examples.

### Flat Win32 DLL exports

Generated Win32 wrappers use `@microsoft/dynwinrt/win32` to invoke DLL exports
described by `Windows.Win32.winmd`. They support validated native structures,
counted buffers, managed handle cleanup, and IOCP-backed asynchronous file I/O.
Manual raw ABI operations remain isolated under
`@microsoft/dynwinrt/win32/unsafe`; the package root stays WinRT-only.

Winsock, GDI+, and Media Foundation functions that require initialization take
an explicit subsystem context. Keep that context for the required native
lifetime and call `close()` when finished. Lifecycle DLLs load on initialization,
not package import; missing components fail the requested initialization without
adding an eager dependency to ordinary WinRT/COM imports. MAPI utility
initialization is available only on the unsafe subpath and requires a configured
provider.

When a failed call cannot close an owned result, both Win32 entrypoints throw
`DynWin32CallError`, an `Error` with the original invocation message.
Its read-only `cleanupFailures` array contains each native `target`, the original
cleanup `error` (`code`, a signed HRESULT, and `message`), and a managed
`DynWin32Resource`. Keep the error or a resource alias until cleanup succeeds.
After correcting the native condition, call `error.retryCleanup()` or close the
individual resources. Retry does not invoke the original function again.
Failed retries retain the owners; successful retries and `close()` are
idempotent, and the original records remain available with `resource.closed`
set to `true`. Aliases share that close state.

If projecting a recoverable error or its records fails, the runtime attempts
to create an `AggregateError` retaining the native error in `cause` and
`errors[0]`, alongside the projection failure in `errors[1]`.
It also exposes `cleanupFailures` and `retryCleanup()`
for recovery without discarding the native owner. Recovery property descriptors
have no prototype, so inherited descriptor fields cannot break decoration.
If constructing or decorating that aggregate also fails, the original native
error is rethrown unchanged. It can remain unprojected or non-extensible and
need not satisfy `instanceof DynWin32CallError`; use
`DynWin32CallError.getCleanupFailures(error)` and
`DynWin32CallError.retryCleanup(error)` to recover without modifying its
prototype. These static methods validate the native carrier, not `instanceof`.
Errors without cleanup failures retain their existing behavior.

See the [Win32 generation guide](../../tools/dynwinrt-codegen/README.md#contract-driven-flat-win32),
[JavaScript samples](../../samples/js/win32/README.md), and
[supported capabilities and ownership rules](../../docs/architecture/flat-win32-contracts.md).

### Generated WinRT values

Unambiguous public WinRT activation metadata is projected as JavaScript constructors.
Parameterized and composable activations support idiomatic forms such as
`new Uri(base, relative)` and `new StackPanel()`. The generated static factory
methods remain available for compatibility.

Generated `IReference<T>` values use `T | null` in JavaScript. Native values,
`null`, and generated `IReference_*` wrappers are accepted as inputs.
Collection factories take arrays of these inputs, typed as
`(T | null | IReference_*)[]`, not a scalar or a `null` container.
The same projection applies when `IReference<T>` appears inside a WinRT struct;
packing boxes the field automatically and unpacking returns the native value.

Generated `Windows.Storage.Streams.Buffer` and `IBuffer` projections provide
copied byte conversion. `fromBuffer()` accepts a Node.js `Buffer` or
`Uint8Array`, and `toBuffer()` returns a new `Buffer` containing exactly
`Length` bytes:

```js
const winrtBuffer = IBuffer.fromBuffer(Buffer.from([0, 1, 255]))
const bytes = winrtBuffer.toBuffer()
```

Both directions copy. Mutating the input or releasing the WinRT object does not
change the returned bytes, and no native buffer pointer is exposed.

Generated packages export `createProjectedLifetimeScope()`. WinUI/XAML hosts
can create a scope after Application and Window setup, then dispose it before
the native window and XAML core are destroyed. Active scopes retain projected
native values strongly for deterministic release; `releaseProjected()` removes
an individually released wrapper from its scope. Projects that never create a
scope do not retain projected values. Direct runtime users can release an
individual `DynWinRtValue` with `value.release()`.
`projectAs(value, Type)` is the public conversion for APIs whose metadata
returns `Object`/`IInspectable` even though the application knows the concrete
runtime class. It accepts either a raw projected value or an existing wrapper,
borrows the input, and creates a separately releasable projection:

```js
import {
  projectAs,
  StackPanel,
  XamlReader,
} from "./generated/index.mjs";

const raw = XamlReader.load(xaml);
if (raw === null) throw new Error("XamlReader returned no value");
const panel = projectAs(raw, StackPanel);
```

Use `wrapper.as(InterfaceClass)` when converting an existing runtime-class
wrapper to another interface view. Use `projectAs(raw, RuntimeClass)` when
converting a raw value to a generated runtime class. A failed QueryInterface is
reported as an error. Internal generated `_fromNative()` paths consume native
return values; application code should use `projectAs()` instead.

### Explicit boxed-value unboxing

Generic WinRT `Object`/`IInspectable` results remain raw `DynWinRtValue`
instances. Use `unboxObject()` only where the application expects a boxed
`Windows.Foundation.IPropertyValue`, such as values from
`DeviceInformation.properties`:

```js
import { unboxObject } from '@microsoft/dynwinrt'

const raw = deviceInformation.properties.get('System.Devices.DeviceInstanceId')
const instanceId = unboxObject(raw)
```

The helper borrows its argument. It maps supported numeric, Boolean, string,
character, GUID, and corresponding array property types to JavaScript values;
`Int64` and `UInt64` use `bigint`, GUIDs use strings, and `UInt8Array` uses
`Uint8Array`. `null` stays `null`. If the object does not implement
`IPropertyValue`, the exact same JavaScript object is returned, so identity and
later projection remain intact. Unsupported property types (including
`DateTime`, `TimeSpan`, geometry, inspectable, and other types) and native getter
failures throw.

Generated WinUI `IElementFactory` bindings expose `IElementFactory.create()`.
It creates a synchronous, UI-thread factory backed by JavaScript
`getElement`/`recycleElement` callbacks. Call `releaseCallbacks()` on the
returned factory after clearing its ItemsRepeater source so JavaScript item
state can be released before the native repeater is destroyed.

`DynWinRtValue.createVector()` objects implement `IObservableVector<T>` in
addition to `IIterable<T>`, `IVector<T>`, and `IVectorView<T>`. Mutations emit
the standard `VectorChanged` collection-change notifications. Generated
`IObservableVector<T>` projections expose `asVector()` for mutating collection
properties, and codegen emits the paired `IVector<T>` binding automatically.

### Creating WinRT collections

`DynWinRtValue.createVector(items, elementType)` and
`DynWinRtValue.createMap(keys, values, keyType, valueType)` keep their public
signatures. They validate element identity, native layout, ownership, and the
complete set of closed collection IIDs before creating a native object.
These limits apply to **producing** collections, including generated input
conversions, not to consuming collections returned by Windows.

| Element / key / value | x64 | ARM64 | i686 |
|---|---|---|---|
| Boolean, integers, Char16, enum, HRESULT | Supported | Supported | Up to 4 bytes; I64/U64 rejected |
| HSTRING; owned nullable interface/object references | Supported | Supported | Supported |
| POD structs in populated vectors or maps | Sizes 1, 2, 4, 8 | Up to 8 bytes, non-HFA only | Up to 4 bytes |
| Additional POD structs in **empty vectors only** | Larger than 8 bytes | Larger than 16 bytes, non-HFA only | None |
| Top-level F32/F64/GUID; structs recursively containing HSTRING or references | Rejected, even empty | Rejected, even empty | Rejected, even empty |

POD means a validated, reference-free native layout. HFA means an aggregate of
one to four same-width floating-point values, including nested aggregates.
Thus `Point`/`Size` work on x64 but are rejected on ARM64 even when empty;
`PointInt32` works on x64 and ARM64. ARM64 also rejects empty `RectInt32`,
`Rect`, and `BasicGeoposition` vectors. `ManipulationDelta` (20 bytes, five floats,
non-HFA) supports empty
vectors on x64 and ARM64. Large map keys/values are rejected even for empty maps.

Structs and typed enums require exact type identity, not a matching byte size
or shape. Scalars require the matching value variant, except U16 inputs for
Char16, I32 inputs for enum/HRESULT, and range-checked I32 inputs for
I8/U8/Char16. For example, I32 `255` is accepted for U8; I32 `257` throws instead
of wrapping. Reference inputs are retained and queried for the declared IID;
an incompatible interface throws, while a null reference remains null.

For admitted POD structs, vector `IndexOf` and map key operations compare
metadata-declared fields by value, including nested fields, and ignore padding.
Floating fields use numerical equality: `+0` and `-0` match, while NaN does not
match even the same NaN bits. This also applies to collection views; a
NaN-containing struct map key is not found by lookup. Stored field bits are
not normalized.
Map construction uses the same comparison as `Insert`: the last value for an
equal key wins, retaining the first key's field bits and iteration position.

The JavaScript helpers use validated typed core factories. Rust callers can use
the safe `create_vector_from_values` / `create_map_from_values` factories for
the same checks. The metadata-free Rust `create_value_vector`, `create_vector`,
and `create_map` constructors require `unsafe`: callers must prove the native
ABI, ownership, element types, and complete IID set themselves.
The metadata-free `create_value_vector` retains packed-byte equality, including
padding, rather than metadata-directed field equality.
Complete native struct collection support is tracked in
[microsoft/dynwinrt#161](https://github.com/microsoft/dynwinrt/issues/161).

## Platform support

- **Windows 10 / 11** — x64 and arm64 native binaries shipped via `napi-rs` prebuilds
- **Node.js** ≥ 18 (Electron, plain Node, VS Code extensions, …)
- **Native class validation** uses N-API 8 type tags. Keep runtime facades from
  the same installed package; native values from differently branded builds
  are rejected rather than reinterpreted.
- **Building from source** requires Rust 1.88 or later.

## Links

- 📦 [`@microsoft/dynwinrt-codegen`](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen) — the typed-binding generator
- 🐛 [Source on GitHub](https://github.com/microsoft/dynwinrt) — issues, contributions, internal design docs

## License

[MIT](https://github.com/microsoft/dynwinrt/blob/main/LICENSE)
