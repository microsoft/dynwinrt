# Classic COM Contract Evidence Registry

This document defines how dynwinrt records, validates, and measures native COM
contract facts that are not completely represented by Windows metadata.

## Problem

`Windows.Win32.winmd` describes interface identities, vtable methods, native
types, parameter directions, and part of the native layout and array model. It
does not completely describe:

- ownership transfer;
- allocator and cleanup operations;
- conditional nullability;
- relationships between flags, HRESULT values, and outputs;
- count, capacity, and actual-length relationships;
- borrowed handle lifetime;
- context-dependent payload interpretation; or
- failure-time cleanup.

dynwinrt supplements metadata with COM standard rules and audited exact
registries. Grouped JSON contracts and the remaining code-defined families
share typed provenance and reproducible dependency counts. These are
method-contract evidence records, not OS registration or per-interface native
implementations.

## Evidence tiers

Every semantic fact records one of these sources:

```text
Metadata
ComStandard
ExactRegistry
UserUnsafe
```

### Metadata

The fact is present in the loaded metadata or one of its explicit attributes,
for example:

- interface IID and inheritance;
- parameter type and pointer depth;
- parameter direction;
- native layout fields;
- `NativeArrayInfo`;
- `FreeWith`; or
- `CanReturnMultipleSuccessValues`.

### COM standard

The fact follows a universal COM rule rather than a method-specific exception:

- QueryInterface returns a new `+1`;
- a typed interface output returns an owned `+1`;
- an interface input is borrowed for the call;
- `AddRef` creates one reference obligation;
- `Release` consumes one reference obligation; and
- a failed HRESULT is an error unless a documented semantic HRESULT contract
  applies.

### Exact registry

The fact comes from an authoritative method-specific contract not fully
represented by metadata. An exact entry contains a complete selector,
fingerprint, semantic contract, citation, and tests.

### User unsafe

The fact is asserted by an application through a generated `*Unsafe` strategy
or the raw API. It never contributes to safe support.

## Safe evidence classification

Literal “metadata-only COM interface” is not a useful interface-level category:
every COM wrapper relies on IUnknown identity and reference-counting rules.

Safe interfaces are therefore classified as:

```text
standard-derived
  WinMD + universal COM rules only

exact-registry-dependent
  At least one required fact comes from an exact registry entry
```

For detailed analysis, the census also reports individual fact counts from:

```text
metadata attributes
COM standard rules
exact entry IDs and aggregation-only family IDs
```

The two interface categories are mutually exclusive and sum to
`safe_complete`.

## Data layout

Contracts live under:

```text
tools/dynwinrt-codegen/contracts/classic-com/
├── schema.json
├── manifest.json
├── conditional-outputs.json
├── ownership-outputs.json
├── safearrays.json
├── null-inputs.json
├── parameter-directions.json
├── borrowed-handles.json
└── enumerators.json
```

The files are grouped by semantic contract kind, not by renderer or API family.
They are compiled into dynwinrt-codegen and are not loaded from an application
directory at runtime. The manifest allowlists the compiled files, kinds, and
families and pins each file's SHA-256. Other exact families remain in their
existing registries; this migration does not externalize every COM contract.

## Entry format

Each file has a `schemaVersion` and a `contracts` array. For example, this is
the existing nullable runtime-ID contract from `safearrays.json`:

```json
{
  "schemaVersion": 2,
  "contracts": [
    {
      "entryId": "automation.safearray.entry.windows-win32-ui-accessibility.irawelementproviderfragment.f7063da88359439c9297bbc5299a7d87.getruntimeid.slot-4.param-0-pretval.v1",
      "familyId": "automation.safearray.v1",
      "kind": "safearray",
      "reason": "Microsoft SDK IDL and API documentation define IRawElementProviderFragment::GetRuntimeId parameter pRetVal as SAFEARRAY(int), requiring VT_I4",
      "selector": {
        "interface": {
          "namespace": "Windows.Win32.UI.Accessibility",
          "name": "IRawElementProviderFragment",
          "iid": "f7063da8-8359-439c-9297-bbc5299a7d87"
        },
        "declaringIid": "f7063da8-8359-439c-9297-bbc5299a7d87",
        "method": "GetRuntimeId",
        "absoluteSlot": 4,
        "parameterCount": 1,
        "parameters": [
          {
            "index": 0,
            "name": "pRetVal",
            "nativeType": "Windows.Win32.System.Com.SAFEARRAY",
            "pointerDepth": 2,
            "direction": "out",
            "optional": false,
            "constness": "mutable",
            "constAttribute": false
          }
        ],
        "sourceFingerprint": "10CDC63F3FED0D67BA0B735F93246E1745D42DBC11FCDF2CF73B6CF7BF82B347",
        "sourceShape": {
          "format": "raw-method-shape-v1",
          "value": "GetRuntimeId@4(pRetVal:out:required:noconstattr:Windows.Win32.System.Com.SAFEARRAY[Struct]/ptr2/Mutable)->Windows.Win32.Foundation.HRESULT[Struct]/ptr0/Unspecified/underlying=i32/ptr0/Unspecified:plain_hresult:not_enumerator_next"
        }
      },
      "contract": {
        "parameterIndex": 0,
        "element": { "vartype": "VT_I4", "interfaceIid": null },
        "ownership": "owned-output",
        "cleanup": "SafeArrayDestroy",
        "nullability": {
          "kind": "required-output-cell",
          "pointee": {
            "kind": "nullable-on-success",
            "reason": "Microsoft permits a NULL SAFEARRAY result on success; the receiving SAFEARRAY** cell remains required.",
            "evidence": [
              {
                "kind": "microsoft-learn",
                "url": "https://learn.microsoft.com/windows/win32/api/uiautomationcore/nf-uiautomationcore-irawelementproviderfragment-getruntimeid",
                "file": null
              }
            ]
          }
        }
      },
      "evidence": [
        {
          "kind": "microsoft-learn",
          "url": "https://learn.microsoft.com/windows/win32/api/uiautomationcore/nf-uiautomationcore-irawelementproviderfragment-getruntimeid",
          "file": null
        }
      ],
      "validatedMetadata": [
        {
          "package": "Microsoft.Windows.SDK.Win32Metadata",
          "version": "71.0.14-preview",
          "sha256": "B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D"
        }
      ]
    }
  ]
}
```

A non-conditional CoTaskMem output uses the smaller closed contract:

```json
{
  "parameterIndex": 0,
  "ownership": "cotaskmem-owned",
  "cleanup": "CoTaskMemFree"
}
```

Only these literal ownership and cleanup values are accepted; the file cannot
name an arbitrary allocator or cleanup function.

## Selector requirements

An exact contract never matches by method name alone. The selector validates:

- namespace and interface name;
- interface IID;
- declaring interface IID;
- method name;
- absolute vtable slot;
- complete parameter count and order;
- every native type and pointer depth;
- direction, optionality, and constness;
- array, `FreeWith`, SAFEARRAY, and exact-contract metadata;
- return type and HRESULT convention; and
- an independently pinned full-method fingerprint or source shape.

Any drift disables the entry and restores the ordinary fail-closed or unsafe
classification.

### Fingerprint formats

The existing canonical SHA-256 convention is unchanged for ownership,
conditional outputs, null inputs, and parameter-direction corrections:
`sourceFingerprint` hashes `canonical_raw_method` before that contract is
attached. In particular, parameter-direction selectors retain the original
WinMD `inout` direction, not the corrected `out` direction.

SAFEARRAY entries retain their original `raw_method_shape` strings verbatim
in `sourceShape.value`. Borrowed-handle and enumerator entries now also pin
complete shapes captured from the same independently checked metadata.
`sourceShape.format` is the closed `raw-method-shape-v1` format;
`sourceFingerprint` must equal the SHA-256 of that exact UTF-8 string. The
loader checks the stored hash, and semantic validation checks the full shape
and every parameter selector against the declaration. Enumerator source
shapes retain their established array and semantic-HRESULT annotations.

These storage-side shape hashes do not replace the existing canonical
fingerprints in the evidence catalog or generated unsafe descriptors.
Thin Rust adapters preserve the raw evidence representation, so those
fingerprints and generated files remain unchanged. An inherited method uses
its declaring interface's selector and ID.

## Closed contract kinds

The grouped-file schema admits only kinds with implemented typed payloads:

```text
ownership
conditional-output
borrowed-handle
enumerator-next
safearray
null-input
parameter-direction
```

Each kind maps to a typed semantic IR. Data files cannot inject JavaScript,
Rust code, arbitrary cleanup functions, or renderer fragments.

Unknown contract kinds and unknown fields fail validation.
The schema binds each kind to its exact family and payload, not just an
unrelated union of allowed fields. Other catalog kinds remain available to
their existing Rust or packaged-contract implementations; listing a catalog
kind does not make an unimplemented JSON payload valid.

### Five migrated evidence families

| File | Records | Existing contract preserved |
| --- | ---: | --- |
| `safearrays.json` | 209 | 69 borrowed inputs and 140 owned outputs; exact VARTYPE/IID, shape, cleanup, and pointee nullability |
| `null-inputs.json` | 2 | Reserved native-NULL `IStorage` inputs |
| `parameter-directions.json` | 3 | Exact `IMFAttributes` InOut-to-Out corrections |
| `borrowed-handles.json` | 22 | Borrowed HWND outputs through required cells, with no cleanup or ownership transfer |
| `enumerators.json` | 97 | Count/value/fetched roles, raw directions and optionality, exact element identity, and `S_OK`/`S_FALSE` behavior |

Of the 333 records, 309 are exact entries and 24 enumerator declarations
remain **COM-standard**. `enumerators.json` records this distinction with
`evidenceSource`: `com-standard` uses `com.enumerator-next.generic.v1`;
`exact-registry` uses the existing selector-derived exception entry.
The storage IDs keep their existing selector-derived namespace, but a
COM-standard row is not registered or counted as an exact entry or an exact
family dependency. The fetched/value direction tables and optional-fetched
lists are now data in the complete parameter selectors, not name-based Rust
exceptions.

`VT_UNKNOWN` means Automation interface elements, not an unknown ABI. It
requires an exact `interfaceIid`; scalar, BSTR, and VARIANT arrays require
`interfaceIid: null`. The supported set remains `VT_I4`, `VT_UI1`, `VT_UI4`,
`VT_R8`, `VT_BSTR`, `VT_VARIANT`, and `VT_UNKNOWN`. Borrowed inputs cannot
request destruction; owned outputs require `SafeArrayDestroy`.

Only four existing SAFEARRAY outputs allow a null contained pointer:
`IRawElementProviderFragment.GetRuntimeId`,
`IRawElementProviderFragment.GetEmbeddedFragmentRoots`,
`ITextProvider.GetSelection`, and `IDragProvider.GetGrabbedItems`.
Their `nullability` is `required-output-cell` with a `nullable-on-success`
pointee, reason, and citation. The `SAFEARRAY**` argument remains required,
mutable, Out, and pointer-depth two. Other outputs use a `required` pointee;
inputs use `required-input`. The consumer reads this validated field only
after exact evidence matching; it does not recognize UIA names or citations.

The five old registry modules now contain derived typed adapters, exact
lookups, and tests, not fixed evidence arrays. Matching, HRESULT handling,
ownership lowering, native layout validation, and runtime cleanup remain
Rust logic. Newly pinned borrowed/enumerator source shapes are checked after
existing semantic validation, preserving the more specific allocator and
ownership diagnostics for already-unsupported inputs. The raw/unsafe
classification entrypoint and exact-entry catalog collection use the same
source-shape guard, so fallback generation or census accounting cannot
consume drifted evidence. This guard is limited to the migrated declarations;
it does not expand raw classifier support.

## Registry versus renderer

The required flow is:

```text
exact evidence entry
  -> validated ComMethodContract
  -> projected semantic IR
  -> generic JavaScript renderer
  -> runtime call plan
```

Production renderers do not import the registry and never compare interface or
method names.

## Registry validation

Every entry requires:

1. exact selector and source fingerprint;
2. at least one authoritative citation;
3. schema validation with unknown-field rejection;
4. official metadata match;
5. mutation tests for every selector field;
6. success, null, failure, and cleanup tests;
7. target architecture validation;
8. a live Windows test when deterministic and practical; and
9. a stable unique ID.

CI validates every entry against the pinned Win32Metadata package. It fails
when:

- an entry no longer matches;
- metadata already contains an equivalent fact;
- two entries conflict;
- a citation or ID is missing;
- an entry references an unsupported contract kind; or
- an entry is unused by every loaded interface.

When authoring these files, validate against `schema.json`, update the
manifest hash from the exact file bytes, and run
`cargo test -p dynwinrt-codegen --lib registry` with the pinned
`DYNWINRT_WIN32_WINMD`. The regression checks all 333 migrated records,
including the 24 COM-standard rows, against real metadata. Unknown fields,
missing required fields, kind/family mismatches, duplicate IDs/selectors,
unsupported semantics, invalid hashes, and metadata drift fail closed.
Do not replace an expected fingerprint with one computed from drifted input
merely to make validation pass.

## Upstream policy

If a fact is universal and representable in Win32Metadata, prefer contributing
it upstream. Keep a local exact entry when:

- metadata cannot express the conditional relationship;
- the rule is projection-specific;
- the contract depends on multiple parameters and HRESULT states; or
- an upstream metadata release containing the fix is not yet the supported
  baseline.

When metadata begins carrying an equivalent fact, CI identifies the local entry
as redundant so it can be removed.

## Dependency census

`com-capability-census` reports:

```json
{
  "safeEvidence": {
    "safeComplete": 0,
    "standardDerived": 0,
    "exactRegistryDependent": 0,
    "metadataFactOccurrences": 0,
    "comStandardFactOccurrences": 0,
    "registeredExactEntries": 0,
    "metadataMatchedExactEntries": 0,
    "safeConsumedExactEntries": 0,
    "exactEntryInterfaceDependencies": 0,
    "exactFamilyInterfaceDependencies": 0,
    "byContractKind": {},
    "byEntryId": {},
    "byFamilyId": {},
    "exactEntryStatus": {}
  }
}
```

For every safe-complete interface, the interface inventory records:

```text
evidence_class
COM standard rule IDs
exact entry IDs
exact family IDs
exact contract kinds
```

Counts have two meanings:

- **dependency count**: interfaces whose current safe plan uses an entry;
- **net contribution**: interfaces that cease to be safe-complete when that
  entry or contract family is disabled.

Dependency counts are computed directly from semantic provenance. Net
contribution requires a controlled ablation census and is not inferred from
dependency counts.

## Remaining registry migration scope

The five-family data migration above is behavior-preserving and bounded.
Other method-specific contract implementations retain their existing
locations, including:

- counted-buffer and sizing overrides;
- semantic HRESULT exceptions;
- `IWbemServices::OpenNamespace`;
- `IDispatch::Invoke` compound behavior;
- `STATSTG` and allocator-specific outputs;
- `IDataObject::GetDataHere` caller-allocated, non-replacing STGMEDIUM InOut;
- `IDataObject::SetData` and `IOleCache::SetData` caller-retained STGMEDIUM ownership;
- `IDataObject::GetCanonicalFormatEtc` output validity and ignored `tymed`;
- `IAudioClient` shared/exclusive format negotiation and CoTaskMem format
  outputs;
- `IMMDevice::Activate` typed null-parameter activation and
  `IMMDevice::GetId` CoTaskMem string ownership;
- `IMDSPDeviceControl::Record` and `IWMDMDeviceControl::Record` nullable
  audio-format inputs selecting the device default; and
- exact fail-closed hazards such as `GetPrivateData`.

Any further migration or promotion is complete only when generated safe snapshots, the
5,721/7,929 safe census, generated unsafe manifests, and all live tests agree
with the exact evidence dependencies.

## User contracts

User-supplied unsafe strategies and future contract files remain separate from
the built-in registry:

```text
built-in exact registry
  authoritative evidence
  may contribute to safe support

user unsafe contract
  caller assertion
  only contributes to *Unsafe/raw execution
```

User entries cannot override or weaken a built-in safe contract.

## Implemented registry and evidence accounting

Stage 1 is implemented for the pinned
`Microsoft.Windows.SDK.Win32Metadata` 71.0.14-preview input. The embedded
registry is compiled from strict serde models under
`tools/dynwinrt-codegen/contracts/classic-com/`; unknown fields, unknown
contract kinds, duplicate IDs/selectors, missing citations, malformed
fingerprints/hashes, unsupported semantic fields, and unused entries fail
validation.

`wmi.conditional-output.entry.windows-win32-system-wmi.iwbemservices.9556dc99828c11cfa37e00aa003240c7.opennamespace.slot-3.v1`
is the first migrated standalone
contract. `conditional-outputs.json` is now the sole source for its full raw
method fingerprint, selector, exact flags, mutually exclusive outputs,
ownership, citations, and validated metadata hash. The old Rust constants were
removed.

`ownership-outputs.json` contains 148 parameter-specific CoTaskMem contracts
and one exact `HBITMAP`/`DeleteObject` contract. Attachment occurs only after the complete raw
method is built and only when its namespace, interface IID, method, absolute
slot, full parameter selector, and pre-contract fingerprint all match. These
entries promote complete interfaces without applying allocator or handle
ownership inference to any unrelated pointer output.

PR1 completes `IMMDevice` (IID
`d666063f-1587-4e43-81f1-b948e807363f`) with two exact `com.ownership.v1`
entries:

- `Activate`, absolute slot 3, source fingerprint
  `5025E3C25B95D89F92271233B0761D56DF1041F985D6C1D0BAFE0FC4DEDD26E3`:
  the code-defined contract validates the full native signature and owned
  dynamic-IID output. The safe projection fixes `CLSCTX_INPROC_SERVER` to `1`
  and `pActivationParams` to native NULL.
- `GetId`, absolute slot 5, parameter `0: ppstrId`, source fingerprint
  `38AF4F595BEEADA67D0D49EB1B04F2CC354C8A06813B286CCAE4B5EF44BFCD99`:
  the JSON ownership entry copies the returned UTF-16 string and pairs the
  allocation with `CoTaskMemFree`, yielding `getId(): string`.

The activation targets are named records in
`tools/dynwinrt-codegen/src/com_activation_registry.rs`, not an anonymous
fixed-length IID array in the language model. Each record carries its interface
identity, authoritative citation, reason, and the in-process/native-NULL/owned
result conditions. Semantic lowering rejects missing or ambiguous evidence and
passes only the validated context and target IIDs to the projected IR; the
renderer does not interpret evidence.

These records are part of the existing exact `IMMDevice::Activate` contract,
not additional safe-interface or per-entry census contributions. The migration
keeps the same five targets and does not add `IDeviceTopology` or general
activation. Activation-policy eligibility, wrapper completeness, and actual
device support remain separate requirements.

The activation target must be a registered generated safe class with the IID
of `IAudioClient`, `IAudioEndpointVolume`, `IAudioMeterInformation`,
`IAudioSessionManager`, or `IAudioSessionManager2`. Other/custom targets,
asynchronous activation, and loopback or other parameterized activation are
outside this subset. The selectors cite Microsoft's
[`IMMDevice::Activate`](https://learn.microsoft.com/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-activate)
and
[`IMMDevice::GetId`](https://learn.microsoft.com/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-getid)
contracts. Metadata drift fails closed; neither entry introduces a
per-interface native adapter or changes the WinRT root API.

Three exact parameter-direction entries, two reserved-null input entries, one
flag-selected caller-buffer entry, one `IStorage::Stat` entry, and seven WMI
conditional-output entries reuse closed semantic models. Together with the
ownership batch, the high-value promotion pass raises safe-complete coverage
from 5,651 to 5,681 interfaces.

Every external evidence path now exposes a selector-derived per-entry ID, a
typed aggregation-only family ID, and a closed contract kind.
`RawEvidence::ExactRegistry` distinguishes exact
registry provenance from `MetadataAttribute`; validated and projected semantic
interfaces retain only dependencies consumed by their successful plan.
Universal COM facts use stable typed rule IDs.

The safe-complete evidence census is:

| Evidence class | Safe interfaces |
| --- | ---: |
| `standard_derived` | 5,355 |
| `exact_registry_dependent` | 366 |
| **Total** | **5,721** |

The registry contains **532 declared entries**, all 532 match the pinned
metadata, and 431 distinct entries are consumed by complete safe plans. These plans have
694 entry/interface dependencies and 425 family/interface dependencies.
Per-interface dependency-set totals also include 6,012 metadata-attribute
dependencies and 26,247 COM-standard-rule dependencies.

PR4's explicit overload names promote 24 previously raw-metadata-complete
interfaces: 23 standard-derived and one exact-registry-dependent.
`IGenericDescriptor2` now consumes the existing `IGenericDescriptor::GetBody`
slot-6 ownership entry, bringing that entry's interface dependencies from one
to two. No entries are added: registered/matched and distinct safe-consumed
entry counts remain 532 and 431. The new names select already validated
normal methods; they do not supply missing ABI, ownership, or lifetime facts.

| Exact contract kind | Safe-interface dependencies |
| --- | ---: |
| `ownership` | 183 |
| `parameter-direction` | 45 |
| `bounded-two-call` | 16 |
| `conditional-output` | 10 |
| `flag-selected-buffer` | 3 |
| `null-input` | 4 |
| `safearray` | 263 |
| `enumerator-next` | 74 |
| `borrowed-handle` | 54 |
| `counted-buffer` | 16 |
| `semantic-hresult` | 3 |
| `compound-dispatch` | 1 |
| `borrowed-storage` | 15 |
| `contextual-effect` | 7 |

Family rollups deliberately count each interface once per family:

| Exact family ID | Registered entries | Safe-used entries | Family/interface dependencies |
| --- | ---: | ---: | ---: |
| `automation.safearray.v1` | 209 | 181 | 121 |
| `windows.borrowed-hwnd-output.v1` | 22 | 18 | 45 |
| `com.enumerator-next-exception.v1` | 73 | 73 | 74 |
| `com.sequential-stream-buffer.v1` | 2 | 2 | 7 |
| `buffers.counted-buffer.v1` | 3 | 2 | 2 |
| `buffers.bounded-two-call.v1` | 2 | 2 | 16 |
| `com.ownership.v1` | 169 | 118 | 124 |
| `com.parameter-direction.v1` | 3 | 3 | 15 |
| `com.reserved-null-input.v1` | 2 | 2 | 1 |
| `com.nullable-input.v1` | 2 | 2 | 2 |
| `com.semantic-hresult.v1` | 2 | 2 | 3 |
| `automation.idispatch-invoke.v1` | 1 | 1 | 1 |
| `graphics.private-data-hazard.v1` | 7 | 0 | 0 |
| `shell.flag-selected-string.v1` | 1 | 1 | 3 |
| `wmi.conditional-output.v1` | 7 | 7 | 1 |
| `audio.conditional-output.v1` | 1 | 1 | 3 |
| `audio.context-effect.v1` | 3 | 3 | 3 |
| `buffers.borrowed-copy.v1` | 23 | 13 | 4 |

The new selectors are defined in
[`com_borrowed_metadata.rs`](../../tools/dynwinrt-codegen/src/com_borrowed_metadata.rs).
They preserve full method fingerprints, exact declaring IID/slot, citations,
and the pinned Win32Metadata SHA256. Three effects observe actual successful
Audio initialization/GetService calls; 23 entries validate the storage and
complete vtable evidence used by the copy plans. WIC's previously complete
interface remains complete, now with an exact-evidence copy augmentation.
The Audio render/capture and linear MF copy-only facades remain **excluded**
from the complete-interface count. Their useful copy operations are not a
claim that their unrestricted native methods are safely projected.

Universal rule dependencies are:

| COM standard rule ID | Safe-interface dependencies |
| --- | ---: |
| `com.activation.output-plus-one.v1` | 976 |
| `com.automation.bstr-output-owned-sysfreestring.v1` | 1,231 |
| `com.automation.bstr-replacement.v1` | 99 |
| `com.enumerator-next.generic.v1` | 25 |
| `com.handle.borrowed-no-cleanup.v1` | 45 |
| `com.hresult.failure.v1` | 5,603 |
| `com.interface.input-borrow.v1` | 1,908 |
| `com.interface.typed-output-plus-one.v1` | 3,480 |
| `com.iunknown.identity-refcount.v1` | 5,721 |
| `com.query-interface.output-plus-one.v1` | 5,721 |
| `com.standard-cleanup.matching-allocator.v1` | 1,438 |

These are dependency counts: an inherited contract can be consumed by several
interfaces, and one interface can consume several IDs or kinds. They are not
net safe-coverage contributions. Stage 1 does not claim ablation results.
The complete per-ID, per-kind, metadata-attribute, and COM-standard-rule maps
are retained in `docs/status/generated/classic-com-capability-summary.json`;
the compact interface support CSV records each interface's complete-safe state,
evidence class, and first stable reason code. Full exact dependency sets remain
available in the CI capability artifact.

Generic scalar BSTR Out ownership/SysFreeString behavior now uses
`com.automation.bstr-output-owned-sysfreestring.v1`; supported BSTR replacement
uses `com.automation.bstr-replacement.v1`. Twenty-four generic JSON-registry
enumerator entries use `com.enumerator-next.generic.v1` after exact signature
validation (25 safe interfaces consume the rule because of inheritance).
ISequentialStream `Read` and `Write` have distinct exact entries in
`com.sequential-stream-buffer.v1`; their missing buffer relationships are not
universal metadata-complete rules. IDispatch `Invoke` is likewise an exact
compound contract in `automation.idispatch-invoke.v1`.
Method-specific SAFEARRAY, borrowed-handle, enumerator exception, ownership,
hazard, and conditional-output registries remain exact.

Counted-buffer, the remaining ownership/cleanup declarations, semantic-HRESULT,
IDispatch, STATSTG/IMalloc, activation policies, native completion, and hazard
registries retain their existing implementations. Borrowed-copy/runtime
recipes are not part of this five-family migration. Each exact path continues
to expose its stable typed provenance ID/kind and participates in the
dependency census only when consumed. Further grouped-file migration and
controlled contract-family ablation remain separate work.

The strict contract data schema and its registry `manifest.json` remain
version 2. The capability summary is version 3, and generated unsafe support
manifests are version 12.
The seven grouped files contain 489 records: 465 exact entries and 24
COM-standard enumerator declarations. The exact subset includes the original
seven WMI conditional-output and 149 output-ownership entries plus the 309
migrated exact entries. Of the other 67 registered exact entries, 26
borrowed-copy/context entries live in the native-independent packaged
`dynwinrt-com-contracts` JSON registry, and 41 remain code-defined. Their
locations and dependency classification are unchanged.
All entries use the same selector-derived `entryId`, typed `familyId`,
selector/fingerprint/citation catalog, and pinned-metadata validation path.

The five-family migration preserves the 5,721/7,929 complete-safe census,
all 532 registered/matched exact IDs, 431 safe-consumed entries, and all
entry/family dependency sets and totals. A mechanical baseline comparison
also retained all per-record fields and compared 285 declaring/inherited
roots: 1,366 generated safe file hashes, unsafe output/support records, and
4,247 canonical method fingerprints were identical. No generated-bindings
manifest or runtime/API version change is needed for this data migration.

Separately, PR1 raised the generated COM file-ownership manifest
`com/.dynwinrt-com-manifest.json` from version 2 to version 3; borrowed-copy
context effects introduced version 4, and version-2 typed copy recipes now
require manifest version 5. This changes neither evidence IDs nor support
counts. Generated safe
classes must register descriptors for the public runtime `/com` `projectAs`
entrypoint and typed `IMMDevice.activate` through private `/com/unsafe`
helpers. Delete existing generated bindings and completely regenerate every
selected root with matching updated runtime/codegen versions; there is no
in-place migration or support for mixed old/new generated classes. The
contract-data schema is unaffected, as are other generated `.as(...)` paths.

PR4 requires no further manifest change. Already-supported safe output,
including distinguishable overload dispatch, remains byte-identical;
previously rejected overload groups had no valid safe surface to migrate.
