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

### Generated WinRT values

Unambiguous public WinRT activation metadata is projected as JavaScript constructors.
Parameterized and composable activations support idiomatic forms such as
`new Uri(base, relative)` and `new StackPanel()`. The generated static factory
methods remain available for compatibility.

Generated `IReference<T>` values use `T | null` in JavaScript. Native values,
`null`, and generated `IReference_*` wrappers are accepted as inputs.
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

## Platform support

- **Windows 10 / 11** — x64 and arm64 native binaries shipped via `napi-rs` prebuilds
- **Node.js** ≥ 18 (Electron, plain Node, VS Code extensions, …)

## Links

- 📦 [`@microsoft/dynwinrt-codegen`](https://www.npmjs.com/package/@microsoft/dynwinrt-codegen) — the typed-binding generator
- 🐛 [Source on GitHub](https://github.com/microsoft/dynwinrt) — issues, contributions, internal design docs

## License

[MIT](https://github.com/microsoft/dynwinrt/blob/main/LICENSE)
