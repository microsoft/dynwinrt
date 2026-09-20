# JavaScript binding internals

The JavaScript binding uses one native N-API binary. Its root WinRT entrypoint,
Classic COM facades and flat Win32 facades remain separate public APIs.
`bindings/js/src/lib.rs` only declares modules and re-exports their existing
registrations; it does not interpret native contracts.

## JavaScript storage

`js_storage.rs` owns behavior-neutral backing-store primitives:

- native TypedArray inspection, element widths and byte views;
- shared/detached backing checks, bounded offsets, alignment and view identity;
- the existing napi-rs `Uint8Array` owner and explicit N-API TypedArray reference
  retention strategies;
- stable Rust-owned byte/word storage used by synchronous calls; and
- owner-environment Node Buffer retention and its original view for asynchronous
  copy-back.

These primitives do not decide whether bytes denote a handle, an address, a
string, a COM reference or an owned native allocation. They do not select native
allocators, infer nullability or establish apartments. Domain adapters provide
their existing diagnostics and sequence the relevant checks.

The domains deliberately retain different policies. Classic COM counted buffers
accept supported TypedArray widths, while its pointer helper accepts byte views.
Win32 requires unsigned-byte views and validates their native backing extent
against its 64 MiB limit. Shared-backed native storage is rejected at the
respective domain boundary. Fixed views are not rejected merely because an
unrelated part of their backing grows: revalidation retains the existing
pointer/length rules. WinRT `fromBuffer` still copies into an owned `IBuffer`;
this refactor introduces no new borrowed-storage policy or size cap there.

Retaining a JS view prevents collection, not detachment or resizing. Validation
must still occur at the existing call or copy-back boundary. The two synchronous
retention strategies are not substituted for each other, and no reference is
moved into a core native operation.

## Value carriers

Generated WinRT collection factories convert each element (and both map keys
and values) using the same metadata-directed argument projection as ordinary
methods. Native `createVector`/`createMap` receive managed `DynWinRtValue`
carriers, not unconverted JavaScript primitives. Conversion does not expand
the native producer's supported ABI or ownership boundary.
Runtime-class and collection-reference conversions preserve managed null
carriers without querying an interface; non-null carriers still query the
declared interface before being passed to native code.

With the JS addon built, run `npm run test:collection-factories` from
`bindings/js` for generated SDK factory roundtrips. `DYNWINRT_CODEGEN` selects
a prebuilt generator, `DYNWINRT_JS_PACKAGE` selects a built runtime package,
and `DYNWINRT_WINDOWS_WINMD` overrides the SDK metadata path. Missing inputs
fail explicitly.

`value.rs` defines `DynWinRTValue` with named WinRT data, independent call storage,
and the private `com_value.rs` sidecar. Callers use constructors and accessors,
not positional tuple fields.

`ComValueState` groups native pointer owner/provenance, an exclusive
`None`/native-struct/buffer/Automation payload, apartment identity and input
bookkeeping. Payload exclusivity does **not** make the other groups exclusive:
a counted buffer can retain JS storage, and an Automation or container payload
can also retain managed input identities. Taking a payload does not implicitly
consume its pointer provenance, backing owner or input bookkeeping.

Ordinary WinRT construction creates no COM apartment binding, performs no
canonical-identity query and does not initialize COM. Interface-bearing WinRT
values retain the existing pointer-free deferred input bookkeeping, which only
claims apartment ownership when it reaches a COM boundary. Casts and container
snapshots preserve the existing shared identity slots and independent native
references. Wrong-thread and post-WinUI teardown handling, native-output adoption,
retryable explicit cleanup and Automation destruction remain COM-local.

Win32 byte/string pointers use `win32_storage::PointerStorage` directly, without
a WinRT/COM value carrier. `RetainedNativePointer` distinguishes that storage
from an explicitly managed COM input. Win32 resource ownership, aggregate field
cleanup, ordered owner leases and successful input consumption remain in their
Win32-specific types and completed call plans.

The Node-only Win32 retention and aggregate adapters use `Rc`; they cannot be
sent to a native worker. Their core aggregate buffers, resources and immutable
call plans retain their existing `Arc` ownership and synchronization.

## Module responsibilities and lifetimes

| Modules | Responsibility |
|---|---|
| `initialization.rs` | Existing WinAppSDK initialization and thread-local WinUI loop state. |
| `winrt_types.rs`, `winrt_methods.rs` | The single lazy metadata table, type/signature wrappers, method invocation and fast getters. |
| `value.rs`, `winrt_array.rs`, `winrt_struct.rs`, `property_value.rs` | Existing WinRT value, container and explicit property-value conversions. |
| `com_value.rs`, `com_input.rs` | COM carrier state, ownership and apartment/input lifetime checks. |
| `js_storage.rs`, `js_numbers.rs` | JS storage facts and exact numeric boundary checks, without native semantic inference. |
| `direct_callback.rs`, `winrt_delegate.rs`, `winrt_element_factory.rs` | Existing callback resources, delegates and the synchronous WinUI element factory. |
| `system.rs`, `benchmarks.rs` | Existing system-information and static benchmark exports. |
| `win32_io.rs`, `async_promise.rs`, `com_completion.rs`, `scheduled_start.rs` | Their distinct asynchronous adapters; these are not unified into IOCP. |

The metadata table remains a single process-wide lazy instance. N-API class
names, export names, callback registrations and per-environment lifecycle hooks
are unchanged by module placement.

The IOCP adapter retains its `RetainedBuffer` on the Node owner thread until the
existing terminal delivery/copy-back or teardown point. Core I/O owns only
native storage and completion records. Cancellation is not completion, and
queued results continue holding their resource occupancy and quota until they
are consumed or discarded.
