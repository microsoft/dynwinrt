# Standalone WinRT interface implementations

The WinRT implementation facility creates real, independently constructible
`IInspectable` objects from runtime metadata. JavaScript and Python implement
their methods. It does not require a composable WinUI base, an aggregated inner
object, a per-interface native adapter, or activation registration.

## Planning and dispatch

```text
WinRT InterfaceMeta + MethodMeta
  -> shared, validated WinRT implementation projection
  -> generated typed handler adapters and interface descriptors
  -> WinRtImplementationPlan
  -> immutable IInspectable identity and independent interface views
  -> private native_callback static thunks / cached libffi closures
  -> owner-thread language callback
  -> prepared, owned outputs
  -> native caller
```

`crates/dynwinrt/src/winrt_implementation/plan.rs` consumes WinRT
`MethodSignature` descriptions, not Classic COM contracts. The signature
facade exposes only WinRT input, output, and fill-array directions. The first
interface method is slot **6**, after all six IUnknown/IInspectable methods.
Methods must be contiguous, and the plan records their full parameter
expansion and output order before an object can be published.

Required WinRT interfaces are separate QueryInterface views. Their methods
are not appended to a derived interface, and incompatible vtable prefixes are
never aliased. The whole-object plan rejects absent requirements, cycles,
duplicate interface IIDs, reserved infrastructure IIDs, invalid slots, and
unsupported types. The public interface set and every view's vtable are frozen
for the object's entire lifetime.

The existing outbound WinRT and Classic COM planners remain independent.
Private scalar-validation, array-element identity rules, and native callback
storage are shared where their contracts are identical. The WinRT host never
registers its methods in the Classic COM registry.

## Native object model

The canonical identity is an IInspectable-rooted allocation. Every interface
view has its own immutable vtable and a pointer to that allocation. All views
share a single atomic reference count. QueryInterface for IUnknown or
IInspectable returns the canonical identity regardless of the source view.

GetIids allocates the actual implemented public IID list with CoTaskMemAlloc;
the caller owns that allocation. GetRuntimeClassName returns an owned HSTRING
for the supplied descriptive name, or an empty string when none was supplied.
This name does not make a class activatable. GetTrustLevel returns BaseTrust.

The private identity adapter maps each native `+1` to one `Arc<Host>` strong
reference and implements native weak references using `Weak<Host>`. Upgrading
a weak reference is atomically synchronized with final destruction. Its
temporary pin and ordinary native Release use the same Arc destruction path,
including when QueryInterface rejects an IID. No state lock is held across
QueryInterface, final destruction, or callback-resource cleanup. This avoids
depending on the pinned windows-core internal weak refcount's temporary
decrement to trigger destruction of this separately allocated dynamic host.

Native weak references expire after the last strong reference is released.
IWeakReferenceSource is infrastructure, not a user-supplied interface plan.
The target host does not expose IAgileObject or a free-threaded marshaler. The
separate native weak-reference object is agile; its optional marshaler uses a
pinned windows-core helper encapsulated in `identity.rs`.

## Threading and lifetime

The only supported callback policy is synchronous, same-owner-thread
dispatch. The native host compares thread identity before entering a language
binding. A foreign-thread call returns `RPC_E_WRONG_THREAD`, even if the caller
has an otherwise valid native reference. Identity, reference counting,
IInspectable metadata queries, and weak-reference operations do not enter
the language runtime.

Each method invocation acquires an extra native reference before accessing its
dispatch plan. It also snapshots the callback outside the callback-state
mutex. Reentrant disposal or the caller's final Release cannot destroy an
executing callback's object, plan, or language roots.

Dynamic libffi closures use the existing process-lifetime signature/slot cache.
Their executable pages cannot be freed by a final reentrant Release. Common
no-argument and pointer-only signatures use static thunks. If the process
cannot allocate executable closure memory for another supported signature,
creation fails before publishing any interface.

The owner/controller holds one native reference. Releasing or collecting it
does not disconnect independently retained native references. Explicit
`dispose` disconnects callbacks object-wide and drops the controller's
reference. Already-dispatched callbacks may finish, including returning their
fully validated outputs; later calls return `RO_E_CLOSED`.

Generated `implement` factories now wrap that unchanged low-level controller
in a language-neutral-in-semantics typed management facade. Its lazy `.value`
caches one owning primary QI view, rather than performing QI on every access.
Facade release clears its primary view and controller reference; disposal first
disconnects the native object. Neither path can recreate the primary view.
Independent-view helpers accept either facade or low-level controller.
Python outbound method handles retain their receiver natively without keeping
a PyO3 borrow across callback entry, so disposing the primary view from inside
its own callback remains valid.

Common Python reverse-conversion helpers live once in each generated package's
`_runtime.py`, shared by its interface modules. They are ordinary measured
generated code, not excluded coverage support. Method-specific ABI plans and
typed conversions remain with their interface.

Language bindings additionally gate dispatch on their environment/interpreter
lifetime. Callback roots belong to the native object, not merely the original
language wrapper. Shutdown disconnects language callback state before its
resources become unavailable. No synchronous callback is approximated by a
fire-and-forget thread-safe-function notification.

Explicit disposal is also the deterministic escape hatch for a handler that
strongly retains its own native object. Native references are external GC roots:
tracing them as ordinary language-only edges could collect an object that a
native consumer still owns. Python's GC integration is consequently
conservative in the presence of independently retained native references.
See the public guide for the binding-specific ownership rules.

The Python public implementation-view helper deliberately bypasses the
ordinary wrapper-identity cache, not the native initializer or projected
lifetime tracking. Thus two same-IID views have independently releasable QI
references, while ordinary `from_value` calls retain their existing identity
behavior.

Generated Node event registration releases the creator's delegate reference
and its call-local value after the native event source retains its own
reference. This prevents an unsubscribe closure's shared lexical environment
from keeping a removed delegate's referenced TSFN alive. It neither globally
unrefs callbacks nor changes cross-thread delegate dispatch.

## Value and output ownership

Incoming native interface pointers and HSTRINGs are borrowed. The reverse
marshaller retains or duplicates them before invoking user code, including
references in structs and arrays. The callback may retain these managed
values without retaining a pointer into the caller's stack or array buffer.

WinRT array directions have distinct plans:

| Direction | Native ABI | Callback contract |
| --- | --- | --- |
| Input/pass array | `UINT32 length, T* data` | Owned managed array of input elements |
| Output/receive array | `UINT32* length, T** data` | Owned array result; allocation uses CoTaskMem |
| Fill array | `UINT32 capacity, T* data` | Capacity input and exactly that many result elements |

Before dispatch, every supplied output location is initialized, including
output lengths, pointers, structs, and fill buffers. Null required storage
returns `E_POINTER`; zero-length arrays may have null data pointers. Byte-size
multiplication is checked against the target's address width.

All outputs are prepared before any is published. Preparation validates exact
struct/enum identity, established scalar/array aliases, array element values,
and the requested interface IID. It acquires owned interface references,
duplicates strings, and constructs zero-initialized, allocator-correct array
buffers. Prepared values are RAII-owned, so a later failed output conversion
releases all earlier preparation.

Publication only transfers already-prepared ownership. It does not run a
handler or QueryInterface halfway through a multi-output result. Strings are
owned HSTRINGs, references transfer a `+1`, and receive arrays transfer both
their element ownership and their CoTaskMem allocation. Structs retain their
exact metadata layout on 32-bit and 64-bit targets.

Language exceptions and Rust panics become failing HRESULTs and never
successful placeholder values. The controller retains a contextual diagnostic
containing the interface, slot, HRESULT, and message. Python additionally uses
its unraisable-exception convention at the native callback boundary.

## Boundaries

Generic interface implementations, uninstantiated generic types, nested
arrays, unknown layouts, ABI-only pointer/out-value descriptors, asynchronous
language handlers, and cross-thread language dispatch are outside this
release. Existing closed generic, delegate, nullable-reference, and native
async **values** can cross supported method contracts; this does not enable
user-defined generic implementations or automatic awaiting.

The Python binding rejects subinterpreters because its existing GIL attachment
path targets the main interpreter. Node callback state is scoped to its N-API
environment. These language-runtime gates supplement the native owner-thread
policy rather than replacing it.

The generated descriptor is a complete metadata contract, not permission to
invent an IID or substitute an unrelated vtable. Low-level descriptor
construction is intended for tools and complete, ABI-correct fixtures.

IBackgroundTask is an application of this same machinery. Windows triggering,
CLSID activation, COM server hosting, package identity/deployment, and task
registration are separate concerns and are not performed by this facility.

See [the Node.js and Python usage guide](../guides/windows/winrt-interface-implementations.md).
