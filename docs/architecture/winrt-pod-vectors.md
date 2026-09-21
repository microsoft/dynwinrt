# Checked WinRT POD vectors

**Post-release phase one of #161.** This feature branch is not part of the
current release candidate. It implements checked POD **vectors**, not the
whole struct-collection issue. Maps, owning structs, and unrelated top-level
scalar expansion remain out of scope. There is no permanent feature flag:
release isolation is the branch boundary.

## Validation and ABI planning

The existing generated JavaScript array and Python sequence conversions call
`create_vector_from_values`. Neither projection infers an ABI from a language
value's shape. `CollectionElementPlan::for_vector` validates the supplied
WinRT metadata first:

- a closed type signature and the exact six collection IIDs;
- recursively complete, nonempty, naturally aligned POD layouts, with checked
  field offsets, total size, alignment, and nesting depth;
- scalar/enum/GUID or recursively POD struct fields, with no HSTRING,
  interface, reference, array, or other owning/unknown fields; and
- exact `TypeHandle` identity for every supplied struct, not just its size.

The existing integer-word, HSTRING, reference, and single-word aggregate
paths remain in use where their native ABI is valid. Char16/U16 and enum/I32
aliases and reference QueryInterface/ownership/null behavior are unchanged.
Maps still call the original word-ABI planner; extending vector admission
does not expose a new map key/value ABI.

Other validated structs use the metadata's recursive libffi type graph.
`vector_pod.rs` prepares `IndexOf`, `SetAt`, `InsertAt`, and `Append`, plus
both live and snapshot view `IndexOf`, through the existing private
`native_callback` cache. The system calling convention is used, including
stdcall on i686. libffi, not a hand-written register/stack adapter, handles
ARM64 HFAs, multi-register aggregates, indirect x64 aggregates, and i686
stack values. Dispatch receives addresses of complete argument values.

All other published slots have element-independent pointer/scalar argument
ABIs: `GetAt`, `Current`, `GetMany`, `ReplaceAll`, view/iterator creation,
size, removal, clearing, and observable registration reuse the existing
entrypoints. These slots copy the full planned element size, not a machine
word. Empty input is never permission to publish an incompatible by-value
slot or substitute a null reference.

The table cache is keyed by the closed recursive signature; natural WinRT
layout is deterministic for that signature on the running target. It owns
immutable tables only, not metadata handles or objects. The shared callback
cache owns prepared CIFs, type graphs, contexts and executable pages for the
process lifetime. Failure to prepare/allocate executable callbacks returns
an error before any COM object is published.

This is WinRT-local planning. It does not change COM/Win32 registries,
contracts, generated output, or public subpaths. See
[Classic COM support](classic-com-support.md) for the shared private ABI
boundary.

## Storage, equality, and lifetime

New POD storage consists of individually owned allocations with the validated
native `Layout`. The existing vector's word slots address those allocations;
they are never treated as inline words or COM references. Prepared inputs have
RAII owners until all inputs are validated. Mutation copies borrowed native
values into owned storage; removal and destruction deallocate with the same
layout. No closure or vtable is allocated per element.

Vector, live view, and observable views share canonical identity and storage.
`GetView` and `First` retain the existing independent snapshot semantics.
Snapshots and iterators own their copied POD allocations and remain usable
after the source vector or metadata table is released. Pointer-only iterator
slots need no dynamic table; their copied layout is sufficient for output and
cleanup. Vector/snapshot objects retain their immutable table plans.

`GetMany` and `ReplaceAll` use native packed element strides and caller
capacity/count contracts, not the internal pointer stride. Field equality
recurses over metadata leaves and ignores internal/tail padding. Floating
leaves compare numerically: signed zeros match and NaNs never match. Stored
field bits are not normalized.

Mutation unlocks before notifying observers. Notification retains the sender
while invoking the snapshotted handler set, so unsubscribe, reentrant mutation,
and releasing the caller's last reference cannot destroy the sender mid-event.
Dynamic dispatch runs inside the centralized panic boundary; its Rust method
bodies do not cross a second non-unwinding FFI boundary.

The metadata-free unsafe constructors retain their original requirements and
packed-byte comparison. In particular, `create_value_vector` still only
permits its documented single-word or empty-indirect subset. This work does
not silently make those APIs checked or broaden their native ABI.

## Coverage and limits

SDK-typed tests cover `Point`, `Size`, `PointInt32`, `RectInt32`,
`BasicGeoposition`, nested `ManipulationDelta`, and nested-HFA
`ManipulationVelocities`. They cover all vector/view/iterator slots, identity,
bulk buffer guards, snapshots, metadata lifetime, numerical equality,
observable reentrancy/final release, and unchanged map/rejection boundaries.
Additional metadata-layout tests cover nested padding. These names are test
examples, not a production allowlist.

The standard E2E specs exercise generated Point arrays/lists through
`InkStrokeBuilder.CreateStroke` and BasicGeoposition arrays/lists through all
three `Geopath` factory and `GeoboundingBox.TryCompute` overloads. An empty
`TryCompute` input returns native null; that is distinct from the non-null
empty input collection. An empty producer does not imply that every native
consumer accepts an empty collection (for example, `Geopath.Create`).

`tests/e2e/pod_hint_specs.json` is an additional, explicitly selected metadata
fixture. Generate `Microsoft.Windows.AI.Imaging.ImageObjectExtractorHint`
from the configured AI Imaging WinMD, with Windows SDK metadata as a reference,
then run the existing `ts_runner.ts` / `py_runner.py` with that specs file and
the isolated generated directory. It exercises the generated RectInt32 vector
factory and all its mutation/bulk operations without activating the hint, AI,
WinAppSDK bootstrap, UI, or a model. It does **not** qualify AI hint execution.

Native ARM64 and Windows x64/i686 emulation are separate evidence categories;
emulated results are not native x64/i686 hardware qualification. Generated
consumers must run against the matching locally built runtime. No gallery,
installed consumer, published package, tag, or release workflow is replaced
by this work.
