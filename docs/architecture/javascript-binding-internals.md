# JavaScript binding internals

The JavaScript binding uses one native N-API binary. Its root WinRT entrypoint,
Classic COM facades and flat Win32 facades remain separate public APIs.
`bindings/js/src/lib.rs` only declares modules and re-exports their existing
registrations; it does not interpret native contracts.

## Native class identity

The binding pins `napi` 3.12.0 and `napi-derive` 3.6.1 (backend 6.1.0)
and explicitly enables `napi8`. This is required for the upstream native
class type-tag checks; enabling only `napi7` leaves those checks inactive.
Node.js 18 already supports N-API 8. Building this binding requires Rust 1.88
or later. The guarantee applies to the shipped Windows native addons, not
the upstream WebAssembly fallback.

napi-rs stamps the actual Rust class's tag in constructors, factories and
`ToNapiValue`/instance publication. Ref, mutable-ref, `ClassInstance`,
`Reference`, method-receiver and accessor-receiver conversions check that tag
before a typed Rust cast/reference/dereference. Some upstream paths first
retrieve an opaque pointer with `napi_unwrap`; that does not interpret the
Rust payload. Nested `Vec`/`Option` inputs and handwritten `FromNapiValue`
bridges use the same generated class conversions.

Handwritten bridges use `native_class_ref::with_ref` / `with_mut` for the
upstream tag-checked conversion and synchronous use. The higher-ranked closure
cannot return a reference to the payload; metadata/value copies and independent
COM owners are returned instead.

This paired release contains the complete native type-tag fix from
[napi-rs/napi-rs#3405](https://github.com/napi-rs/napi-rs/pull/3405).
The backend is pinned explicitly because its otherwise-compatible version
range also admits later borrow-tracking code. That separate policy change
rejects existing reentrant delegate release/`once` behavior and needs its own
compatibility work. This change does not claim to solve general Rust aliasing
or addon-unload safety.

These checks must not be replaced with constructor names, JavaScript
`instanceof`, or calls to public methods. In particular, a Node receiver
signature describes a JavaScript template, not every possible Rust payload
publication path. `winrt_implementation::require_instance` uses the native
tag for its explicit descriptor and callback-output checks.

`native_class!` only adds the upstream `type_tag` attribute to a class
declaration. It does not wrap methods or implement argument conversion.
`build_support/native_branding.rs` computes its salt from sorted, relative native
source paths and bytes, the binding/core/contract manifests, the lockfile,
target/configuration/features, and `rustc -vV`. Cargo watches the files and
directories so additions and edits invalidate the salt. The upstream
derivation still includes each Rust module and class name: classes never
share a single build-wide tag. Object-shaped metadata records are not native
classes and do not use this macro.

The salt is a conservative native-build compatibility boundary, not a
cross-library Rust ABI guarantee. Different source, dependencies, compiler,
target or feature configuration must not reinterpret one another's objects,
even when package versions coincide. The current npm release pipeline sets
JavaScript package versions after the native build; the Python preparation
helper updates the Python/codegen Cargo manifests, not this binding.
All facades of one published runtime still share the same native addon and
class identities. Existing Win32 carriers retain their separate, module-local
N-API tags and finalizers; they are not stamped again as napi-rs classes.
Identity validation does not replace COM apartments, released-state checks,
resource cleanup contracts or core WinRT value validation.

Run `npm run test:native-classes` against a production addon for bounded
child-process collection, descriptor, accessor, callback and worker checks.
The owned process resource must remain usable after rejection and close once.
After an explicit `test-hooks` build, `npm run test:native-classes:hooks`
also checks mutable/owned/aliased references, setters, and layout-identical
untagged or differently branded controls. Counters verify rejected inputs
never enter native bodies, and controlled payloads finalize once.

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

Typed map construction uses the same key equality as `Insert`. For duplicate
keys, the last value wins while the first key and its insertion position are
retained. All keys and values are validated before publishing the map, including
values that will be replaced. This policy is shared by the Rust, JavaScript and
Python typed constructors.

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
