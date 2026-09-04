# WinRT type coverage

This document summarizes the current language projections across:

- the shared Rust ABI runtime in `crates/dynwinrt`;
- the JavaScript N-API binding in `bindings/js`;
- the Python PyO3 binding in `bindings/py`; and
- the JavaScript/TypeScript and Python generators in
  `tools/dynwinrt-codegen`.

Classic COM has a separate semantic type system and safety boundary. See
[Classic COM support](../../docs/architecture/classic-com-support.md) instead
of applying the WinRT mappings below to `Windows.Win32.winmd`.

## Scalar and foundation types

| WinRT type | JavaScript / TypeScript | Python | ABI notes |
|---|---|---|---|
| `Boolean` | `boolean` | `bool` | One-byte WinRT boolean storage |
| `Int8`–`UInt32`, `Char16` | `number` | `int` | Inputs are range checked at the language boundary |
| `Int64`, `UInt64` | `bigint` | `int` | JavaScript accepts safe integers where documented, but generated APIs preserve the full range with `bigint` |
| `Single`, `Double` | `number` | `float` | `Single` is narrowed to the native `f32` ABI |
| `String` / `HSTRING` | `string` | `str` | Runtime duplicates and releases owned HSTRING values |
| `Guid` | `string` | `uuid.UUID` | Native 16-byte GUID layout |
| `HRESULT` value | `number` | `int` | Method failures become language exceptions; semantic HRESULT returns remain values |
| Enum | frozen numeric object + declaration type | `enum.IntEnum` | Unknown Python enum values remain integers |
| Flags enum | frozen numeric object + declaration type | `enum.IntFlag` | Native storage follows metadata, normally `Int32` |
| `DateTime` | generated struct value | `datetime.datetime` | Python values are normalized to UTC WinRT ticks |
| `TimeSpan` | generated struct value | `datetime.timedelta` | Python converts to and from 100-nanosecond ticks |

## Reference and nullable types

| WinRT type | JavaScript / TypeScript | Python |
|---|---|---|
| Runtime class | generated wrapper | generated wrapper |
| Interface | generated interface wrapper | generated structural interface wrapper |
| `Object` / `IInspectable` | `DynWinRtValue` | `DynWinRTValue` |
| `IReference<T>` output | `T \| null` | `T \| None` |
| `IReference<T>` input | native value, `null`, or generated compatibility wrapper | native value, `None`, or generated compatibility wrapper |
| Delegate input | generated delegate/callback surface | normal Python callable |

Returned runtime classes and interfaces own their native reference. Generated
wrappers provide explicit projection and interface-conversion helpers for
metadata that exposes only `Object`/`IInspectable`.

## Structs

Struct layout comes from metadata and includes native size, alignment, nested
fields, enums, fixed arrays, HSTRING fields, GUIDs, and interface-valued fields.
The core runtime clones and drops non-blittable fields with the required
reference counting.

JavaScript projects structs as typed plain objects and generates internal
pack/unpack helpers. Python emits one canonical generated value class per struct
and uses native Python values for GUID, `DateTime`, `TimeSpan`, and nullable
fields.

Unknown or incomplete layouts fail during generation instead of being treated
as an opaque object.

## Arrays and collections

WinRT pass, receive, and fill arrays are modeled separately so capacity,
actual-length, allocation, and element cleanup remain correct.

- JavaScript generated APIs accept and return typed arrays such as `number[]`,
  `bigint[]`, `string[]`, and generated wrapper arrays.
- Python generated APIs accept normal sequences. Byte arrays accept `bytes` and
  `bytearray`; outputs use native Python values where possible.
- Runtime-class, interface, HSTRING, struct, enum, GUID, and primitive elements
  preserve their native ownership and ABI representation.

Parameterized collections are generated as concrete interfaces:

- `IIterable<T>` and `IIterator<T>`;
- `IVector<T>` and `IVectorView<T>`;
- `IObservableVector<T>`;
- `IMap<K,V>`, `IMapView<K,V>`, and `IKeyValuePair<K,V>`.

JavaScript exposes the projected WinRT methods and convenience helpers. Python
implements the matching `collections.abc` sequence, mutable-sequence, mapping,
mutable-mapping, iterable, and iterator protocols.

## Async operations

| WinRT type | JavaScript / TypeScript | Python |
|---|---|---|
| `IAsyncAction` | `Promise<void>` | `WinRTAsync[None]` |
| `IAsyncOperation<T>` | `Promise<T>` | `WinRTAsync[T]` |
| `IAsyncActionWithProgress<P>` | Promise + progress callback | `WinRTAsyncWithProgress[None, P]` |
| `IAsyncOperationWithProgress<T,P>` | Promise + progress callback | `WinRTAsyncWithProgress[T, P]` |

Cancellation calls `IAsyncInfo.Cancel`. Python awaitables integrate with
`asyncio` and also expose an explicit blocking `wait()` API for non-async
hosts. Blocking waits are rejected on an STA or from a running event loop when
they could deadlock.

## Delegates and events

Generated event methods retain the WinRT token model. JavaScript and Python
bindings marshal callbacks through their host runtimes and report callback
failures as failing HRESULTs instead of unconditional success.

The shared dynamic WinRT delegate currently supports up to two ABI parameters.
This covers common handlers such as `TypedEventHandler<TSender,TArgs>`,
`EventHandler<T>`, and async completion/progress handlers. Delegates with more
than two ABI parameters remain unsupported.

## Activation, composition, and WinUI

Public `ActivatableAttribute` and `ComposableAttribute` metadata becomes normal
language constructors. Protected-only composition and system-returned classes
remain non-constructible but can still be wrapped when returned by Windows.

Both generators include specialized WinUI `Application + Window` support,
metadata-provider/resource helpers, composable controls, DispatcherQueue
integration, and projected lifetime management. The application must still
provide package identity or Windows App SDK bootstrap, the correct STA thread,
and lifecycle ownership.

## Validation and remaining limits

Coverage is enforced through:

- core ABI and ownership tests;
- JavaScript and Python binding tests;
- generated JavaScript, declaration, Python, and stub snapshots;
- TypeScript and mypy checks; and
- x64/ARM64 end-to-end tests against real Windows APIs.

Current limits that affect type coverage:

1. Dynamic WinRT delegates accept at most two ABI parameters.
2. JavaScript GUID and Boolean arrays use per-element conversion rather than a
   dedicated bulk fast path.
3. Python does not yet expose a zero-copy buffer protocol for WinRT buffers.
4. Classic COM, native Win32 pointers, Automation variants, SAFEARRAY, and
   ownership-specific outputs use the separate fail-closed COM model and
   currently generate only JavaScript/TypeScript.
