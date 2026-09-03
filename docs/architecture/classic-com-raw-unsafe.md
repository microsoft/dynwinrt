# Classic COM Raw Unsafe Design

This document defines the phase-one raw Classic COM ABI layer implemented at
`@microsoft/dynwinrt/com/unsafe/raw`.

## Motivation

The safe Classic COM generator validates complete interface inheritance, ABI
layout, parameter relationships, ownership, and cleanup before emitting an
ordinary JavaScript API. The existing `/com/unsafe` entry point lets experts
manually declare signatures while still selecting known dynwinrt ownership
models such as COM `Release`, BSTR, and CoTaskMem.

Some Windows.Win32 COM methods cannot use either layer because JavaScript has
no general representation for aligned native memory, pointer slots, pointer
arithmetic, opaque records, or caller-managed replacement semantics. The raw
layer fills that gap. Its responsibility is equivalent to a windows-rs
`unsafe fn`: faithfully execute a caller-declared ABI while assigning all
native semantic and lifetime responsibility to the caller.

## Public layering

```text
@microsoft/dynwinrt
  WinRT-only public API

@microsoft/dynwinrt/com
  Safe COM initialization and managed value types

generated/com
  Complete metadata-validated COM projections

@microsoft/dynwinrt/com/unsafe
  Manual signatures with known semantic ownership plans

@microsoft/dynwinrt/com/unsafe/raw
  Aligned native memory, raw pointers, pointer slots, and raw vtable calls
```

Safe generation must never fall back to either unsafe layer.

## Phase 1A: complete foundation

Phase 1A targets outbound method calls on an existing in-process COM
interface pointer:

- aligned owned native memory;
- owner-retaining pointers and checked pointer offsets;
- byte and primitive integer/floating-point reads and writes;
- pointer-width reads and writes;
- explicit external pointer construction;
- conversion to a borrowed `DynWinRtValue` for existing raw method invocation;
- raw vtable registration through the existing `DynComMethodSig` and
  `addMethodAt()` infrastructure; and
- re-export of the existing semantic unsafe signature, invocation, adoption,
  and cleanup tools when their exact contracts apply.

This is enough to express caller storage for `T*`, `T**`, opaque structs,
unions, arrays, and unclassified InOut slots without teaching the safe
generator their semantics.

Phase 1A is complete and review-validated. Its owned allocation, provenance,
thread-affinity, logical release, physical invocation lease, and isolated
package-surface invariants remain the foundation for later phases.

## Phase 1B: current block

The first Phase 1B block adds:

- explicitly unsafe, bounded, non-owning external memory views;
- raw-only architecture-specific struct and union layout descriptors;
- recursive nested struct fields, fixed field counts, GUIDs, and pointers;
- validated struct by-value, pointer input, output, and InOut calls through the
  existing method signature and executor;
- direct validated struct returns through libffi dynamic return storage; and
- a closed recursively described C-POD by-value/nested union subset.

The standard cleanup and managed/raw COM pointer bridge block adds retained
borrowed pointers, dedicated raw +1 owners, atomic managed adoption, explicit
detach/transfer, and fixed standard cleanup functions. The final ABI block adds
closed by-value union classification without extending JavaScript callbacks.

## Non-goals

Phase one does not provide:

- automatic metadata fallback;
- automatic pointer ownership inference;
- arbitrary foreign-memory bounds validation;
- arbitrary DLL export invocation;
- general foreign-thread synchronous JavaScript callbacks;
- cross-apartment marshaling or Global Interface Table integration;
- COM aggregation;
- registered in-process or out-of-process COM servers; or
- custom marshaling.

## Native memory model

`DynComRawMemory` represents either an owned allocation or an explicitly
unsafe bounded external view with an exact byte length and power-of-two
alignment. Owned allocation is zero-initialized and released exactly once.
`allocate(size, alignment?)` accepts a non-negative safe integer or `bigint`
that fits `usize`. Size zero is rejected. The default alignment is the host
pointer alignment; an explicit alignment must be a nonzero power of two and
the size/alignment pair must form a valid platform `Layout`. Allocation
failure throws instead of aborting the process.

The implementation uses `std::alloc::alloc_zeroed` and records the exact
`Layout` used for `dealloc`. `release()` removes the physical-storage owner
from the logical allocation controller immediately, making every subsequent
read, write, pointer conversion, and dispatch validation fail. An invocation
lease acquired before native dispatch retains only the physical storage until
that invocation returns. This permits a synchronous callback to call
`release()` reentrantly without a use-after-free or mutex deadlock: the
callback observes the allocation as released, while deallocation waits for
the outer call's lease. Stale child pointer objects retain only the released
controller and cannot prolong the physical allocation. Finalizer cleanup and
explicit release remain one-shot. All offsets use checked `usize` conversion
and checked `offset + width` arithmetic before a native address is
dereferenced.

`fromUnsafeAddress(address, size, alignment)` and
`fromUnsafePointer(pointer, size, alignment)` create a non-owning view. The
name is intentionally explicit: the caller guarantees that the complete
external range remains live and writable for every access and invocation.
Forged or stale addresses can corrupt memory or terminate the process.
dynwinrt never deallocates an external view.

External view construction rejects:

- null with nonzero length;
- zero or non-power-of-two alignment;
- a base address that violates the declared alignment;
- non-lossless JavaScript numbers or values wider than `usize`;
- `address + length` overflow;
- impossible platform `Layout` size/alignment pairs; and
- a view beyond the known remainder of an owned `DynComRawPointer`.

Zero-length external views, including a null zero-length view, are permitted
but cannot service any nonzero-width access. All existing bounds, offset,
primitive, pointer-slot, release, thread, and invocation checks apply. Releasing
an external view is logical and idempotent; active leases retain only the view
state, not ownership of the caller's bytes.

Primitive access is native-endian and supports unaligned field offsets. This
matches the Windows host ABI while avoiding aligned Rust references into
caller-described storage. The complete memory API is:

| API                                                       | Result                                                     |
| --------------------------------------------------------- | ---------------------------------------------------------- |
| `DynComRaw.pointerSize()`                                 | Host pointer width in bytes (`4` or `8`)                   |
| `DynComRawMemory.allocate(size, alignment?)`              | Nonzero aligned zeroed owned allocation                    |
| `fromUnsafeAddress`, `fromUnsafePointer`                  | Bounded non-owning external view                           |
| `size`, `alignment`, `released`, `release()`              | Allocation metadata and deterministic cleanup              |
| `readBytes(offset, length)`, `writeBytes(offset, Buffer)` | Checked byte copies                                        |
| `readI8/U8/I16/U16/I32/U32/I64/U64`, matching `write*`    | Checked native-endian integers; 64-bit values use `bigint` |
| `readF32/F64`, matching `write*`                          | Checked native-endian floating-point values                |
| `readIsize/Usize`, matching `write*`                      | Pointer-width signed/unsigned `bigint` slots               |
| `readPointer`, `writePointer`                             | Pointer-width bit slots with no ownership transfer         |
| `pointer(offset?)`                                        | Owner-retaining pointer into the allocation                |

`writeI8` through `writeU32` reject fractional and out-of-range numbers.
Pointer-sized and 64-bit inputs use exact `bigint` conversion. Finite `f32`
inputs outside the representable finite range are rejected.

`DynComRawPointer` is a pointer capability:

- an owned pointer retains the backing `DynComRawMemory`;
- an external pointer has no owner and no bounds;
- checked offsets on owned memory cannot escape the allocation;
- exporting an address is explicit;
- converting to a call value preserves the owner through the native call; and
- raw pointer values are never implicitly adopted as COM references.

`DynComRawPointer.fromAddress(bits)` and `null()` construct explicit unowned
external pointers. Their `address` and `isNull` accessors expose only bits.
External pointers can be converted with `toValue()` and passed to native
calls, but the unbounded pointer object deliberately provides no dereference
or offset API. Dereference requires an explicit bounded
`fromUnsafeAddress`/`fromUnsafePointer` view. `offset(byteOffset)` is accepted
only for an owned memory pointer and cannot escape its allocation.
`readPointer()` always returns an unowned external pointer, and
`writePointer()` copies only bits; neither operation transfers or retains
native resource ownership.

Raw memory controllers and their owned pointers are bound to the thread that
creates them. Access, explicit release, pointer conversion, and pre-invocation
owner validation reject a different thread. Unowned external pointers expose
bits only; a bounded view created from them is bound to its creating thread.
Finalizer deallocation may occur on another thread because the Rust global
allocator is thread-safe and no COM or JavaScript state is touched. The
allocation and pointer types therefore need no manual `Send` or `Sync`
implementation.

## Raw aggregate descriptors

`DynComRawStructLayout.fromDescriptor(json)` and
`DynComRawUnionLayout.fromDescriptor(json)` select the current architecture
from an exact descriptor:

```json
{
  "name": "Contoso.Native.Packet",
  "x86": {
    "size": 12,
    "alignment": 4,
    "fields": [
      { "name": "tag", "offset": 0, "count": 1, "type": { "kind": "u32" } },
      { "name": "items", "offset": 4, "count": 2, "type": { "kind": "u16" } },
      { "name": "data", "offset": 8, "count": 1, "type": { "kind": "pointer" } }
    ]
  },
  "x64": {
    "size": 16,
    "alignment": 8,
    "fields": [
      { "name": "tag", "offset": 0, "count": 1, "type": { "kind": "u32" } },
      { "name": "items", "offset": 4, "count": 2, "type": { "kind": "u16" } },
      { "name": "data", "offset": 8, "count": 1, "type": { "kind": "pointer" } }
    ]
  },
  "arm64": {
    "size": 16,
    "alignment": 8,
    "fields": [
      { "name": "tag", "offset": 0, "count": 1, "type": { "kind": "u32" } },
      { "name": "items", "offset": 4, "count": 2, "type": { "kind": "u16" } },
      { "name": "data", "offset": 8, "count": 1, "type": { "kind": "pointer" } }
    ]
  }
}
```

Supported field kinds are `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`,
`u64`, `f32`, `f64`, `isize`, `usize`, `guid`, `pointer`, and recursively
inline `struct` and `union` descriptors. `count` expresses fixed arrays.
Struct offsets are explicit and non-overlapping; union fields overlap at
offset zero. Layout validation checks qualified identity, nonzero size,
alignment, field bounds, overflow, overlap, duplicate names, exact computed
alignment, recursive name cycles, and a bounded recursion depth.

`byValueType()` performs a second strict schema pass over every supplied
architecture before constructing a libffi type. The only admitted keys are:

| Descriptor object | Exact keys |
| --- | --- |
| Struct root | `name`, `x86`, `x64`, `arm64`, optional `initializers` |
| Union root | `name`, `x86`, `x64`, `arm64` |
| Struct layout | `size`, `alignment`, `fields` |
| Union layout | `size`, `alignment`, `fields`, `complete` |
| Struct field | `name`, `offset`, `count`, `type` |
| Union field | `name`, `count`, `type` |
| Scalar/pointer/GUID type | `kind` |
| Nested aggregate type | `kind`, `name`, `layout` |
| `sizeOfLayout` initializer | `kind`, `field` |

Unknown keys and explicit packed/bitfield/vector/HVA/flexible-array/
over-aligned/opaque/nontrivial/selected-member/incomplete markers are rejected.
Misspelled `kind`, `count`, `offset`, or `layout` keys therefore cannot be
silently ignored. Every nested by-value union requires `complete: true`.

`fromDescriptor()` and `pointerType()` intentionally retain the raw
pointer-storage behavior: extra caller metadata may be ignored while selecting
the recognized host layout because no aggregate register classification is
granted. The original descriptor is retained, and a later `byValueType()` call
always performs the strict recursive pass; pointer-only parsing cannot be
promoted implicitly.

Core RawOutbound by-value validation also requires natural C struct layout,
recursively through structs nested in structs or unions. Fields are ordered by
their declared offsets; each offset must equal the previous field end rounded
up to the field's natural alignment. Fixed arrays contribute
`elementSize * count`, the declared alignment must equal the maximum field
alignment, and the declared size must equal the final field end rounded up to
that alignment. Empty layouts, overflow, cycles, unexplained internal gaps,
inflated trailing padding, and non-natural nested aggregates fail before CIF
construction. Intentional reserved bytes must be represented as explicit
fixed `u8` fields.

This natural-layout requirement applies only to RawOutbound by-value
capability, including direct returns. Semantic explicit-layout types and raw
pointer-only struct/union storage retain their existing behavior.

Raw descriptor construction also applies explicit practical denial-of-service
limits before allocating value bytes or building a libffi type:

| Limit                                                      |         Maximum |
| ---------------------------------------------------------- | --------------: |
| Aggregate byte size                                        | 1,048,576 bytes |
| Recursive nesting depth                                    |   32 aggregates |
| Expanded fixed fields, including nested counts             |          65,536 |
| Flattened libffi elements, including explicit/tail padding |          65,536 |

An individual fixed count is also limited to 65,536. These limits are raw-only;
they do not change existing metadata-validated safe POD descriptors or the
semantic-unsafe descriptor API. Raw struct and union value creation uses
fallible reservation and checked resize even after a descriptor passes the
limits. Oversized size, padding-only expansion, nested multiplication, count
overflow, and allocation failure are reported before native dispatch.

The struct API is:

| API                                                | Purpose                                                  |
| -------------------------------------------------- | -------------------------------------------------------- |
| `qualifiedName`, `descriptor`, `size`, `alignment` | Validated identity and host layout                       |
| `createValue(bytes?)`                              | Create an exact branded zeroed or copied aggregate value |
| `readValueBytes(value)`                            | Copy bytes after exact layout identity validation        |
| `byValueType()`                                    | Type for `addIn`, `addOut`, `addInOut`, or `returns`     |
| `pointerType(nullable?)`                           | Type for an existing native struct pointer input         |

The direction still determines the ABI: `addIn(byValueType())` passes the
aggregate by value, while `addOut` and `addInOut` pass validated caller storage.
`pointerType()` represents an already pointer-shaped input. No second call
executor or renderer inference is involved.

Raw-outbound aggregate capability remains attached to the core type through
signature completion. Semantic `DynCom.nativeStructType()` and
`nativeStructPointerType()` reject any recursively union-containing layout.
Raw-outbound aggregates and all structs containing unions are rejected from
callback lowering; existing semantic union-free POD callbacks are unchanged.

Direct struct return uses the same recursively constructed libffi type as
by-value input and supplies an aligned runtime return buffer with
`call_return_into`. Private Win64 aggregate argument/return storage is aligned
to at least 16 bytes.

`DynComRawUnionLayout` exposes `createValue(activeField, bytes?)`,
`readValueBytes`, `assertActiveField`, `pointerType(nullable?)`, and
`byValueType()`. Nullable union pointer types preserve native null through
validation and invocation; non-nullable union pointer types reject it before
dispatch.
`activeField` is a caller interpretation only and never participates in ABI
classification. Direct returns and Out/InOut values carry branded bytes with
unknown active field until `assertActiveField` is called explicitly.

Nested aggregate parsing remains raw-only. The semantic
`DynCom.nativeUnionPointerType()` factory accepts only flat union alternatives
and directs recursive struct/union descriptors to `/com/unsafe/raw`.

By-value union descriptors must set `complete: true` for every
architecture-specific union layout and describe every alternative. The closed
subset accepts only natural C-POD scalar, pointer, GUID, fixed-array, recursive
struct, and recursive union fields. It rejects vector/HVA, packed, bitfield,
flexible-array, over-aligned, opaque, nontrivial, and selected-member-only
forms.

Classification uses every alternative:

- ARM64 homogeneous analysis accepts only one recursive `f32` or `f64` base,
  with effective count 1 through 4 and exact `size == base_size * count`.
  Struct children sum counts; union alternatives take the maximum count.
- Homogeneous unions lower as a libffi struct containing the repeated base
  scalar.
- Nonhomogeneous unions lower as an aggregate carrier whose first integer
  matches validated alignment (`u8`, `u16`, `u32`, or `u64`) followed by exact
  byte padding. This prevents accidental HFA classification.
- Prepared-CIF tests verify the resulting size and alignment exactly.

On Win64, raw struct and union `byValueType()` rejects aggregate sizes
3, 5, 6, and 7 because bundled libffi 3.5.2 has an irregular-size argument
passing/copy defect for those sizes. This is a top-level CIF gate: an odd-sized
union may be nested inside a naturally laid out supported outer aggregate.
Pointer storage remains available.

A test-only C oracle is independently compiled by MSVC behind the
`test-hooks` feature; production builds do not compile or link its symbols.
Live x64 and i686 runs cover U1/U2/U3/U4/U5/U6/U7/U8/U16/U24, first/fourth/
post-register arguments with COM `this` and sentinels, direct register and
sret returns, HFA1/HFA2/HFA4, mixed float/integer non-HFA, double HFA3,
union-in-union, homogeneous and mixed union-in-struct call/return, guarded
destination canaries, and an indirect U16 callee-local mutation that proves
the original input bytes and source canaries remain unchanged. Different
asserted active fields with identical bytes execute identically. windows-rs
`BITS_FILE_PROPERTY_VALUE`,
`BITS_JOB_PROPERTY_VALUE`, and `WHV_ACCESS_GPA_CONTROLS` provide additional
independent union layout evidence.

Core test builds surround direct aggregate return storage with 16-byte prefix
and suffix canaries, validate them immediately after `call_return_into`, and
include a corruption-detection regression. Non-test builds use a zero guard
size, preserving the production allocation size and behavior.

The Windows build workflow runs the exact
`com_raw::tests::msvc_c_union_oracle_executes_through_libffi` test with
`test-hooks` on x64 and as a live i686 process. The existing ARM64 build job
cross-compiles the same test binary and C source with `--no-run`; it is
compile-only evidence, not runtime execution. Production x64/ARM64 package
builds omit `test-hooks`, so `build.rs` does not compile or link the C oracle.
Public by-value unions, including structs containing unions, remain core-gated
on ARM64 until the oracle executes on native ARM64 hardware.

## ABI declarations

The raw layer reuses the existing immutable COM method registry and libffi call
executor. Native pointers use the existing pointer ABI type. Pointer depth is
represented by caller-allocated pointer slots rather than inferred by the
executor:

```js
const pointerSize = DynComRaw.pointerSize();
const slot = DynComRawMemory.allocate(pointerSize, pointerSize);
slot.writePointer(0, original);

const result = method.invoke(object, [slot.pointer().toValue()]);
const replacement = slot.readPointer(0);
```

The caller is responsible for declaring the exact absolute vtable slot,
argument order, return convention, and storage layout.

## Ownership

Raw memory ownership and native resource ownership are separate.

- `DynComRawMemory` frees only its own allocation.
- A pointer read from memory is borrowed and unclassified.
- COM `AddRef`/`Release`, BSTR, CoTaskMem, SAFEARRAY, PROPVARIANT, handles, and
  resource unions require explicit operations or a higher semantic wrapper.
- Pointer replacement never releases the old value or adopts the new value
  automatically.
- `toValue()` produces borrowed/unclassified pointer provenance. Even an owned
  memory pointer cannot be consumed by COM, CoTaskMem, or BSTR adoption APIs.

The existing semantic unsafe output types remain preferred when their contract
matches:

- `ownedComOutputType()`;
- `coTaskMemOutputType()`;
- `bstrOutputType()`; and
- `borrowedHandleOutputType()`.

### Managed/raw COM pointer bridge

The raw bridge uses explicit states rather than exporting managed wrapper
addresses through safe entrypoints:

| API/state                                                   | Ownership                                                                                        |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `DynComRawPointer.fromManagedBorrowed(value)`               | Retains an internal managed `IUnknown`; the exposed pointer remains borrowed and non-adoptable.  |
| `DynComRawOwnedComPointer.addRef(value)`                    | Owns one independent +1 for the same interface view.                                             |
| `DynComRawOwnedComPointer.queryInterface(value, iid)`       | Owns the exact +1 view returned by QueryInterface.                                               |
| `owner.pointer()`                                           | Borrowed pointer capability retaining the owner state.                                           |
| `owner.retain()`                                            | New independent +1 for the same raw interface pointer, used for invocation lifetime retention.   |
| `owner.query(iid)`                                          | New independent raw +1; the source remains owned.                                                |
| `owner.intoManaged(iid?)`                                   | Atomically consumes one +1. Optional QI consumes the original on both success and failure.       |
| `owner.detach()`                                            | Moves the +1 into an RAII detached pointer. Dropping it before adoption releases exactly once.   |
| `DynComRawOwnedComPointer.adoptTransferred(pointer, iid?)`  | Atomically moves an RAII detached +1 into a dedicated owner without AddRef.                      |
| `owner.transferTo(memory, offset?)`                         | Validates and writes a pointer slot, then disarms the owner without a fallible publication step. |
| `DynComRawOwnedComPointer.assumeTransferred(pointer, iid?)` | Explicitly adopts an external slot result that the caller asserts carries one +1.                |
| `owner.release()`                                           | Idempotently releases the owned +1.                                                              |

Managed inputs must be live interface objects. Null, released, scalar,
aggregate, and raw-pointer values are rejected. Raw COM owners are bound to
their creating apartment thread. Wrong-thread use fails; a wrong-thread or
post-WinUI finalizer leaks rather than invoking `Release` in an invalid
apartment, matching the existing managed Classic COM policy.

Detached provenance is distinct from ordinary borrowed provenance inside the
runtime. A detached pointer remains RAII-owned through N-API publication and
until `adoptTransferred` consumes it. Existing `adoptComPointer` paths reject
both retained borrowed and detached raw values. Ordinary external pointers
also cannot pass `adoptTransferred`; slot results require the deliberately
named `assumeTransferred` assertion. Pointer bits can still be duplicated by
the caller, so the raw layer cannot globally prevent double Release.

Raw interface InOut remains fully caller-managed:

```js
const oldOwner = DynComRawOwnedComPointer.addRef(managed);
const oldAddress = oldOwner.address;
const slot = DynComRawMemory.allocate(
  DynComRaw.pointerSize(),
  DynComRaw.pointerSize(),
);
oldOwner.transferTo(slot);

method.invoke(object, [slot.pointer().toValue()]);

const resultAddress = slot.readPointer(0);

// Apply exactly one branch after verifying the native method's contract:
if (calleeConsumesOldAndReturnsOwnedReplacement) {
  // The old +1 was consumed. Do not reconstruct or release oldAddress.
  const replacementOwner = DynComRawOwnedComPointer.assumeTransferred(
    resultAddress,
    replacementIid,
  );
  try {
    // Use replacementOwner, or call intoManaged() to transfer it.
  } finally {
    replacementOwner.release();
  }
} else if (calleePreservesOldAndReturnsOwnedReplacement) {
  // The caller still owns the old +1 even though the slot was replaced.
  const replacementOwner = DynComRawOwnedComPointer.assumeTransferred(
    resultAddress,
    replacementIid,
  );
  const preservedOld = DynComRawOwnedComPointer.assumeTransferred(
    DynComRawPointer.fromAddress(oldAddress),
  );
  try {
    // Use replacementOwner, or call intoManaged() to transfer it.
  } finally {
    replacementOwner.release();
    preservedOld.release();
  }
} else if (calleeLeavesOwnedSlotUnchanged) {
  // resultAddress and oldAddress are the same single +1. Adopt it only once.
  const unchangedOwner = DynComRawOwnedComPointer.assumeTransferred(
    resultAddress,
    originalIid,
  );
  try {
    // Use unchangedOwner, or call intoManaged() to transfer it.
  } finally {
    unchangedOwner.release();
  }
}
```

Whether the callee releases, preserves, or replaces the old pointer is part of
the independently verified native contract. The branches above are mutually
exclusive: reconstructing `oldAddress` after the callee consumed it is a double
Release, while adopting both addresses when the slot is unchanged creates two
owners for one +1. The fake regression uses the
preserve-old/addref-replacement contract. Every adopted owner must eventually
be released, converted into a managed wrapper, or transferred again; examples
must not rely on finalization. dynwinrt performs no automatic replacement or
reference reconciliation.

Every RawCom-backed invocation argument acquires a temporary independent +1
before conversion and dispatch. Reentrant `release`, `intoManaged`, QI
consumption, or finalization makes new work fail immediately, while the
temporary lease keeps the pointer valid until `invoke`, `invokeAll`,
`invokeDispatch`, WinRT invocation, or scheduled invocation returns. No COM
owner mutex is held during native dispatch.

### Standard cleanup

`DynComRawCleanup` exposes only fixed standard cleanup functions, not arbitrary
DLL invocation:

| API                                          | Native argument and behavior                                                                                                       |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `coTaskMemFree(pointer)`                     | Consumes an external pointer value; null is a no-op.                                                                               |
| `localFree(pointer)` / `globalFree(pointer)` | Consume on successful free; null is a no-op; native failure is thrown and leaves the pointer usable for retry.                     |
| `sysFreeString(pointer)`                     | Consumes a BSTR pointer value; null is a no-op.                                                                                    |
| `safeArrayDestroy(pointer)`                  | Consumes on successful HRESULT; null is a no-op; failure is thrown without consuming.                                              |
| `variantClear(memory, offset?)`              | Requires bounded, live, aligned `VARIANT` storage; HRESULT failure is thrown. Storage remains live and contains the cleared value. |
| `propVariantClear(memory, offset?)`          | Requires bounded, live, aligned `PROPVARIANT` storage; HRESULT failure is thrown.                                                  |
| `releaseStgMedium(memory, offset?)`          | Requires bounded, live, aligned `STGMEDIUM` storage. Calls `ReleaseStgMedium` and zeroes that structure afterward.                 |
| `closeHandle(pointer)`                       | Consumes a handle value only after successful BOOL result; failure is thrown.                                                      |
| `destroyIcon(pointer)`                       | Consumes an HICON only on success; failure is thrown.                                                                              |
| `deleteObject(pointer)`                      | Consumes a GDI object only on success; failure is thrown.                                                                          |

Pointer cleanup arguments are resource pointer/handle values, not addresses of
pointer slots. Aggregate cleanup arguments are addresses inside bounded
memory. Owner-backed memory pointers, retained COM pointers, and already
consumed pointers are rejected by pointer cleanup functions. These wrappers
prevent repeat cleanup through the same pointer object but cannot detect
duplicated or forged bits.

## Safety boundary

Importing the raw entry point is an explicit acknowledgement that incorrect
use can corrupt memory or terminate the process. Examples include:

- forged or stale addresses;
- out-of-bounds pointer arithmetic;
- wrong alignment;
- mismatched struct layout;
- wrong vtable slot or calling convention;
- reading an inactive union field;
- using a pointer after its backing memory is released;
- mismatched allocator and cleanup; and
- incorrect AddRef/Release accounting.

No raw API is exported by the package root, `/com`, or generated safe
declarations.

An owner-backed `DynWinRtValue` acquires an allocation lease for the complete
native invocation through the existing native pointer owner mechanism.
Explicit release invalidates that value and all child pointers for new work.
If release occurs reentrantly during an already-started invocation, its lease
keeps the physical bytes alive only until dispatch returns.

## Validation

Every raw primitive requires:

1. zero-size, overflow, alignment, and bounds tests;
2. x64 and i686 pointer-width compile coverage;
3. owner-retention and deterministic-release tests;
4. exact primitive and pointer round trips;
5. pointer-to-pointer fake-vtable invocation;
6. invalid-address operations that fail before dereference where validation is
   possible;
7. package-entrypoint isolation tests; and
8. unchanged WinRT and safe COM regression coverage.

Live API coverage should be added only when the operation is deterministic and
does not mutate persistent system state.

### Final Phase 1 outbound matrix

The matrix uses real synchronous fake vtables and the exact Windows
`extern "system"` ABI. It proves raw reach; none of these interfaces become
safe-generated:

| Scenario                             | Verified raw contract                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| DXGI/D3D `GetPrivateData`            | REFGUID, `UINT*` size query/update, owned and bounded external byte buffers, interface payload pointer slots, explicit +1 adoption, and explicit failure cleanup without bytes/interface inference.                                                                                                                                                                                                           |
| `IAudioClient::IsFormatSupported`    | Exact `(AUDCLNT_SHAREMODE, const WAVEFORMATEX*, WAVEFORMATEX**)` ordering; packed `cbSize` extension bytes; shared-mode required output slot with `S_OK`/`S_FALSE`; exclusive-mode null output; failure-null output; bounded returned storage and exact `CoTaskMemFree`. A separately named synthetic test covers dirty CoTaskMem output on failure without attributing that behavior to `IsFormatSupported`. |
| `IDataObject::GetData`               | windows-rs `FORMATETC` and `STGMEDIUM` layout with consistent `TYMED_HGLOBAL`; correct union `hGlobal`; delegated `pUnkForRelease` whose final Release frees HGLOBAL; null-owner direct GlobalFree; zeroing and exact one-time resource/reference cleanup.                                                                                                                                                    |
| Generic typed interface InOut style  | Three separately named interface InOut contracts: consume/release old then AddRef replacement; preserve old then AddRef replacement; unchanged slot without AddRef. Each uses atomic transfer, contract-specific adoption, and exact old/new reconciliation. `IWbemServices::OpenNamespace` is separately corrected to two exact Out slots in generated Stage 2.                                                                 |
| Pointer depth                        | Pointer, `T**`, and `T***` mutation plus scalar and pointer InOut.                                                                                                                                                                                                                                                                                                                                            |
| Pointer validity                     | Required-pointer rejection and nullable-pointer native execution.                                                                                                                                                                                                                                                                                                                                             |
| Memory                               | Aligned owned storage, bounded external views, overlapping memmove-safe writes, logical release, thread affinity, and active invocation leases.                                                                                                                                                                                                                                                               |
| Aggregates                           | Nested structs/unions, fixed arrays, GUID/pointer fields, by-value and pointer input, Out, InOut, and direct aggregate return for the documented closed C-POD subset.                                                                                                                                                                                                                                           |
| Returns                              | Scalar, pointer, void, ordinary HRESULT, semantic HRESULT, and validated direct aggregate returns.                                                                                                                                                                                                                                                                                                            |
| Ownership failures                   | Multiple owned outputs plus failed direct-aggregate conversion with exact COM/CoTaskMem cleanup.                                                                                                                                                                                                                                                                                                              |
| Buffers                              | Caller buffers, bounded external buffers, and callee CoTaskMem buffers.                                                                                                                                                                                                                                                                                                                                       |
| Bridge and cleanup                   | Managed/borrowed/raw +1 transitions, reentrant leases, RAII detach, slot transfer, and every `DynComRawCleanup` operation.                                                                                                                                                                                                                                                                                    |

The host x64 execution oracle checks windows-rs `WAVEFORMATEX`, `FORMATETC`,
`STGMEDIUM`, BITS union, and WHV union size/alignment evidence plus nested
aggregate and pointer-slot layouts. The independent C oracle executes live on
x64 and i686. ARM64 has C/Rust cross-compile evidence only and retains the
runtime gate. Public consumption examples live in
`bindings/js/__test__/raw-phase1.spec.ts`.

This provides broad in-process, current-apartment, outbound raw COM coverage
for completed scalar, pointer, memory, and struct contracts. It does not make
`IDataObject`, `IAudioClient`, private-data APIs, or WMI safe-generated.

Phase 1 can be called complete for the documented in-process,
current-apartment, outbound raw ABI subset on x64 and i686. The precise ABI
exclusions are top-level Win64 3/5/6/7-byte by-value aggregates, ARM64
by-value unions/structs containing unions until a runnable oracle is available,
and vector/HVA, packed, bitfield, flexible-array, over-aligned, opaque,
nontrivial, or incompletely described aggregates. It is therefore not yet
cross-platform-complete. Pointer-to-union storage, including `STGMEDIUM`, is
verified on all supported targets. Arbitrary foreign-memory liveness and
unmodeled custom allocators remain caller-supplied facts rather than inferred
layouts.

## Follow-up phases

After these Phase 1B blocks:

1. add dedicated higher-level owning `STGMEDIUM`, audio-format, handle, and
   allocator-specific values where they improve safety;
2. add raw owner-thread callback storage;
3. run the union ABI oracle on native ARM64 hardware and remove or preserve its
   runtime gate based on that result; and
4. separately design apartment marshaling and server support.
