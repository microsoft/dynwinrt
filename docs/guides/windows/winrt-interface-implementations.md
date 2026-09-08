# Implement WinRT interfaces in Node.js and Python

Generated WinRT interfaces can create standalone native objects backed by
synchronous JavaScript or Python handlers. The objects support IInspectable,
QueryInterface, and real native vtable calls. They do not need WinUI
composition, a package identity, or background-task registration.

Use a runtime and code generator from the same version. Generate the complete
interfaces you need, including their required interfaces. The generated
handler declaration describes exactly which methods and results to provide.

## Construction and typed interface views

`Interface.implementation(handlers)` describes one implemented interface.
`Interface.implement(handlers, ...additional)` creates a native owner from one
or more descriptors. The returned `DynWinRtImplementation` (Node) or
`DynWinRTImplementation` (Python) is distinct from a typed interface view.

Use `Interface.fromImplementation(owner)` in Node, or
`Interface.from_implementation(owner)` in Python, to obtain a public typed view.
This keeps the owner valid and creates an independently owned interface
reference. The view is accepted by ordinary generated methods taking that
interface; no type assertion or internal `_fromNative` helper is needed.
Repeated conversions to the same IID return independent views. Python's new
conversion remains tracked by the current projected lifetime scope, but does
not reuse or replace the ordinary `from_value` identity cache.

### Node.js

Generate IStringable and IClosable:

```powershell
cargo run -p dynwinrt-codegen -- generate `
  --class-name "Windows.Foundation.IStringable,Windows.Foundation.IClosable" `
  --output .\generated
```

```js
import { roInitialize } from '@microsoft/dynwinrt';
import { IClosable, IStringable, releaseProjected } from './generated/index.js';

roInitialize(1);
const owner = IStringable.implement(
    { toString: () => 'Implemented in JavaScript' },
    IClosable.implementation({ close: () => console.log('Native Close call') }),
);
const text = IStringable.fromImplementation(owner);
const closable = IClosable.fromImplementation(owner);
try {
    console.log(text.toString()); // Outbound native call, then reverse callback.
    owner.release();             // These two interface references remain valid.
    closable.close();
} finally {
    owner.dispose();            // Disconnect every retained view's callbacks.
    releaseProjected(closable);
    releaseProjected(text);
}
```

Use the generated `IStringableHandlers` and other handler declarations
to type handler objects in TypeScript. Handlers must be actual synchronous
functions; an `async` function or a returned Promise/thenable is not a WinRT
synchronous result.

### Python

Generate the same interfaces with `--lang py`:

```powershell
cargo run -p dynwinrt-codegen -- generate `
  --class-name "Windows.Foundation.IStringable,Windows.Foundation.IClosable" `
  --lang py --output .\generated
```

```python
from dynwinrt import RoApartment, release_projected
from generated.windows.foundation import IClosable, IStringable

class TextHandler:
    def to_string(self) -> str:
        return "Implemented in Python"

class CloseHandler:
    def close(self) -> None:
        print("Native Close call")

with RoApartment(1):
    owner = IStringable.implement(
        TextHandler(), IClosable.implementation(CloseHandler())
    )
    text = IStringable.from_implementation(owner)
    closable = IClosable.from_implementation(owner)
    try:
        print(text.to_string())  # Crosses the native vtable in both directions.
        owner.release()
        closable.close()
    finally:
        owner.dispose()
        release_projected(closable)
        release_projected(text)
```

Python handler objects may use ordinary bound methods. Generated `.pyi`
declarations describe their protocol. Coroutine functions and returned
awaitables are rejected, not run in the background or awaited on the callback
thread. The implementation owner's context-manager exit performs **dispose**,
not merely reference release.

Python implementations currently require the main CPython interpreter;
subinterpreter-owned callbacks are rejected at construction. Captured
`contextvars` follow the existing Python callback-context convention.

## Supported contracts

Support is determined from the entire interface, not only from the method an
application intends to call. Missing handlers, unresolved required interfaces,
duplicate views, invalid slots, or unsupported signatures fail before a native
object is published.

| Contract | Support |
| --- | --- |
| Ordinary methods | Typed inputs, void results, return values, and multiple outputs |
| Properties | Explicit synchronous getter and setter handlers |
| Events | Add/remove handlers, native delegate values, and registration tokens |
| Primitive values | Booleans, integer widths, floating-point values, Char16, enums, HRESULT values |
| GUID and HSTRING | Managed values with native layout/string ownership |
| WinRT structs | Known layouts, including supported nested fields |
| Object/interface/class references | Nullable managed references; output QueryInterface checks the declared IID |
| Closed generic and nullable-reference values | Established value conversions, not generic implementations |
| Arrays | Input, receive, and fill arrays of supported elements |
| Native async objects | Pass-through where the existing projection supports the declared type |

Handler accessor names and output field names are given in the generated
handler type. Do not infer them from a property's displayed name or a
language object's shape. When a method has multiple logical outputs, the
generated result has named fields; every field must be present and valid.
The ABI order is explicit output parameters in metadata order followed by the
method's logical return value.

A fill-array handler receives a **capacity**, not a writable borrowed native
buffer. Return exactly that many elements. Receive-array storage and element
references are allocated and transferred by the runtime. An invalid element,
wrong length, missing result field, or failed output QueryInterface fails the
whole call before any output ownership is published.

Required interfaces have their own handler descriptors and native views.
For example, IMemoryBufferReference requires IClosable. Supply its descriptor
alongside the memory-reference handlers; do not merge an IClosable method into
the memory-reference vtable.

For its Closed event, the implementation handlers are `getCapacity`,
`addClosed`, and `removeClosed` in Node, or `get_capacity`, `add_closed`, and
`remove_closed` in Python. The add handler receives an owned, typed callable
native delegate. Retain it to deliver the event by calling
`handler(senderView, null)` / `handler(sender_view, None)`. Return a token
`{ value: 1n }` in Node or `EventRegistrationToken(value=1)` in Python. The remove
handler receives that token and should remove and release its stored delegate.
These calls invoke the delegate's native slot 3; they are not direct calls to
the original subscriber's language function.

Generated Node subscriptions release their creator and temporary delegate
references after the native add method returns. The native event source owns
the subscription reference until removal. Keeping an unsubscribe function
reachable after removal does not keep the native delegate alive. A caller
using `DynWinRtDelegate.create` directly must similarly release its delegate
owner and its separate `toValue()` reference when they are no longer needed.
Neither release disconnects a native-retained reference.

Existing metadata visibility rules remain in force: an `ExclusiveTo` static
or factory interface is an implementation detail of its runtime class, not a
new public implementation entrypoint. The array examples in the generated
integration suite use public IDataWriter, IPropertyValue, and IDataReader
interfaces rather than exposing private factory interfaces.

## Ownership, disposal, and errors

| Operation | Meaning |
| --- | --- |
| `toValue()` / `to_value()` | Create an owned low-level native value for explicit interop |
| Public typed view conversion | Create an owned typed view without consuming the owner |
| `releaseProjected(view)` / `release_projected(view)` | Release that view without disposing the controller or other views |
| `release()` on the implementation owner | Drop only that owner's native reference |
| `disconnect()` | Disconnect callbacks for the whole implemented object |
| `dispose()` | Disconnect the whole object, then release the owner's reference |
| `takeError()` / `take_error()` | Retrieve and clear the last contextual native callback error |

Native references keep handlers alive after the original wrapper leaves scope.
An independent view or native consumer's reference is not disconnected merely
because the owner was released or collected. Low-level values and typed views
have independent reference ownership and must follow their normal projected
lifetime rules. JavaScript's `releaseProjected` is exported by the generated
package; Python's `release_projected` is exported by `dynwinrt`.

Explicit disposal is different: every later method call through any retained
view fails with `RO_E_CLOSED`. A callback that had **already entered** before
disposal may finish and return its validated outputs. Its native object,
dispatch storage, and callback roots remain alive until it returns.

If a handler retains its own owner or native view, the resulting native/language
cycle may require explicit disposal. Python GC integration is conservative
when external native references exist: collecting a still-native-owned handler
would be incorrect. Avoid strong self/native cycles where practical, and use a
`try/finally` or the Python owner context manager for deterministic
disconnection. Environment/interpreter shutdown also closes callback state;
it does not leave callable pointers into a destroyed language runtime.

Python can collect an owner-only callback cycle when that owner holds the sole
native reference. Native aliases or native weak-reference use are treated
conservatively and may require explicit disposal. Native reference cleanup can
run on another thread without granting permission to invoke handlers there.

Language exceptions are reported as real failing HRESULTs. The owner's error
accessor provides interface/slot context; Python also reports the original
exception using its native-callback unraisable-exception convention. Outputs
are initialized on failure, and prepared strings, references, and buffers are
released if any output conversion fails.
Ordinary JavaScript handler exceptions map to `E_FAIL` (`0x80004005`);
ordinary Python handler exceptions map to `0xA0EE4005`. In particular, throwing
`NotImplementedError` does not request native `E_NOTIMPL`, and arbitrary
properties attached to a JavaScript Error do not override its HRESULT.

## Threading

Create and call implementations on their **owner thread**. They are explicitly
non-agile, including when created in an MTA. Native identity and reference-count
operations remain thread-safe, but a method callback from a foreign thread
fails with `RPC_E_WRONG_THREAD` before entering JavaScript or Python.

This release does not provide apartment dispatch, cross-thread callback
marshaling, implicit Promise/coroutine conversion, or event-loop blocking to
wait for a synchronous result. Existing native WinRT async objects and
deferrals may be used explicitly through their own APIs.

## IBackgroundTask and deployment

IBackgroundTask uses the same general implementation engine as IStringable,
IClosable, and other supported interfaces. A controlled native consumer can
call its Run method with a real IBackgroundTaskInstance interface without
registering an operating-system task.

The [Node sample](../../../samples/js/interface-implementation/README.md) and
[Python sample](../../../samples/python/interface-implementation/README.md)
provide runnable task-instance fixtures and demonstrate native Run calls,
nested native property callbacks, multiple interface views, and owner release.

Creating such an object does **not** register a background task or COM server,
make a CLSID activatable, configure a Windows trigger, install a package, or
grant a package identity. Windows background activation and deployment are
separate future work. GetRuntimeClassName is descriptive metadata, not
activation registration.

Arbitrary user-defined generic interface implementations, nested arrays,
unknown native layouts, and incomplete ownership contracts are not supported.
Where a signature requires a dynamic libffi closure, process restrictions on
executable-memory allocation can also make construction fail.

For implementation details, see
[the native architecture](../../architecture/winrt-interface-implementations.md).

The repository's focused generated/native scenarios can be run after building
the matching bindings and generator:

```powershell
.\tests\e2e\e2e_test.ps1 -SkipBuild -Suite implementations `
  -Lang py,ts -CargoProfile dev -Python .\bindings\py\.venv\Scripts\python.exe
```

They exercise real vtables, properties, native event delegates, pass/receive/
fill arrays, scalar/GUID/struct/reference outputs, named multiple outputs,
independent views, exception recovery, and retention/disposal. Use
`-Suite standard` for the existing WinRT/Classic COM compatibility scenarios.
