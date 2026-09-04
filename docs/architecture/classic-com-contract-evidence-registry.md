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

Today, dynwinrt supplements metadata with COM standard rules and several
code-embedded exact registries. Those facts need one provenance model, one
data format, and reproducible dependency counts.

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
└── ownership-outputs.json
```

The files are grouped by semantic contract kind, not by renderer or API family.
They are compiled into dynwinrt-codegen and are not loaded from an application
directory at runtime. Other exact families remain typed Rust registries until
they are migrated to equally strict grouped JSON files.

## Entry format

```json
{
  "schemaVersion": 2,
  "entryId": "audio.conditional-output.entry.windows-win32-media-audio.iaudioclient.1cb9ad4cdbfa4c328b32e7f3216b7b3d.isformatsupported.slot-7.v1",
  "familyId": "audio.conditional-output.v1",
  "kind": "conditional-output",
  "selector": {
    "interface": {
      "namespace": "Windows.Win32.Media.Audio",
      "name": "IAudioClient",
      "iid": "..."
    },
    "declaringIid": "...",
    "method": "IsFormatSupported",
    "absoluteSlot": 7,
    "sourceFingerprint": "..."
  },
  "contract": {
    "discriminator": {
      "parameterIndex": 0,
      "cases": [
        {
          "value": 0,
          "outputArgument": "required-pointer-slot"
        },
        {
          "value": 1,
          "outputArgument": "native-null"
        }
      ]
    },
    "output": {
      "parameterIndex": 2,
      "ownership": "owned",
      "allocator": "CoTaskMem",
      "cleanup": "CoTaskMemFree"
    }
  },
  "evidence": [
    {
      "kind": "microsoft-learn",
      "url": "https://learn.microsoft.com/windows/win32/api/audioclient/nf-audioclient-iaudioclient-isformatsupported"
    },
    {
      "kind": "sdk-header",
      "file": "audioclient.h"
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
- a canonical full-method fingerprint.

Any drift disables the entry and restores the ordinary fail-closed or unsafe
classification.

## Closed contract kinds

The schema admits only implemented semantic kinds:

```text
ownership
conditional-output
counted-buffer
bounded-two-call
borrowed-handle
enumerator-next
safearray
semantic-hresult
compound-dispatch
hazard
```

Each kind maps to a typed semantic IR. Data files cannot inject JavaScript,
Rust code, arbitrary cleanup functions, or renderer fragments.

Unknown contract kinds and unknown fields fail validation.

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

## Existing registry migration

Current code registries are migrated without changing behavior:

- borrowed HWND outputs;
- SAFEARRAY element/ownership evidence;
- enumerator `Next` contracts;
- counted-buffer and sizing overrides;
- semantic HRESULT exceptions;
- `IWbemServices::OpenNamespace`;
- `IDispatch::Invoke` compound behavior;
- `STATSTG` and allocator-specific outputs; and
- exact fail-closed hazards such as `GetPrivateData`.

Migration or promotion is complete only when generated safe snapshots, the
5,681/7,929 safe census, generated unsafe manifests, and all live tests agree
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

## Stage 1 implementation

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

`ownership-outputs.json` contains 147 parameter-specific CoTaskMem contracts
and one exact `HBITMAP`/`DeleteObject` contract. Attachment occurs only after the complete raw
method is built and only when its namespace, interface IID, method, absolute
slot, full parameter selector, and pre-contract fingerprint all match. These
entries promote complete interfaces without applying allocator or handle
ownership inference to any unrelated pointer output.

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
| `standard_derived` | 5,326 |
| `exact_registry_dependent` | 355 |
| **Total** | **5,681** |

The registry contains **495 declared entries**, all 495 match the pinned
metadata, and 404 distinct entries are consumed by safe plans. Safe plans have
655 entry/interface dependencies and 404 family/interface dependencies.
Per-interface dependency-set totals also include 5,974 metadata-attribute
dependencies and 26,076 COM-standard-rule dependencies.

| Exact contract kind | Safe-interface dependencies |
| --- | ---: |
| `ownership` | 172 |
| `parameter-direction` | 45 |
| `bounded-two-call` | 16 |
| `conditional-output` | 7 |
| `flag-selected-buffer` | 3 |
| `null-input` | 2 |
| `safearray` | 263 |
| `enumerator-next` | 74 |
| `borrowed-handle` | 54 |
| `counted-buffer` | 16 |
| `semantic-hresult` | 2 |
| `compound-dispatch` | 1 |

Family rollups deliberately count each interface once per family:

| Exact family ID | Registered entries | Safe-used entries | Family/interface dependencies |
| --- | ---: | ---: | ---: |
| `automation.safearray.v1` | 209 | 181 | 121 |
| `windows.borrowed-hwnd-output.v1` | 22 | 18 | 45 |
| `com.enumerator-next-exception.v1` | 73 | 73 | 74 |
| `com.sequential-stream-buffer.v1` | 2 | 2 | 7 |
| `buffers.counted-buffer.v1` | 3 | 2 | 2 |
| `buffers.bounded-two-call.v1` | 2 | 2 | 16 |
| `com.ownership.v1` | 162 | 111 | 116 |
| `com.parameter-direction.v1` | 3 | 3 | 15 |
| `com.reserved-null-input.v1` | 2 | 2 | 1 |
| `com.semantic-hresult.v1` | 1 | 1 | 2 |
| `automation.idispatch-invoke.v1` | 1 | 1 | 1 |
| `graphics.private-data-hazard.v1` | 7 | 0 | 0 |
| `shell.flag-selected-string.v1` | 1 | 1 | 3 |
| `wmi.conditional-output.v1` | 7 | 7 | 1 |

Universal rule dependencies are:

| COM standard rule ID | Safe-interface dependencies |
| --- | ---: |
| `com.activation.output-plus-one.v1` | 976 |
| `com.automation.bstr-output-owned-sysfreestring.v1` | 1,231 |
| `com.automation.bstr-replacement.v1` | 99 |
| `com.enumerator-next.generic.v1` | 25 |
| `com.handle.borrowed-no-cleanup.v1` | 45 |
| `com.hresult.failure.v1` | 5,568 |
| `com.interface.input-borrow.v1` | 1,881 |
| `com.interface.typed-output-plus-one.v1` | 3,453 |
| `com.iunknown.identity-refcount.v1` | 5,681 |
| `com.query-interface.output-plus-one.v1` | 5,681 |
| `com.standard-cleanup.matching-allocator.v1` | 1,436 |

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
uses `com.automation.bstr-replacement.v1`. Twenty-four generic code-registry
enumerator entries use `com.enumerator-next.generic.v1` after exact signature
validation (25 safe interfaces consume the rule because of inheritance).
ISequentialStream `Read` and `Write` have distinct exact entries in
`com.sequential-stream-buffer.v1`; their missing buffer relationships are not
universal metadata-complete rules. IDispatch `Invoke` is likewise an exact
compound contract in `automation.idispatch-invoke.v1`.
Method-specific SAFEARRAY, borrowed-handle, enumerator exception, ownership,
hazard, and conditional-output registries remain exact.

Borrowed-handle, SAFEARRAY, enumerator, counted-buffer, the remaining
ownership/cleanup declarations, semantic-HRESULT, IDispatch, STATSTG/IMalloc,
and hazard registries remain code-defined in Stage 1, but each exposes the same
stable typed provenance ID/kind and participates in the dependency census only
when consumed. Moving those declarations into grouped JSON files, and adding
controlled contract-family ablation, remains later registry migration work.

The strict contract data schema and manifest are version 2. The capability
summary is version 3, and generated unsafe support manifests are version 11.
Seven WMI conditional-output entries and 148 output-ownership entries are
JSON-backed; the other 340 registered entries remain code-defined. All code and
data entries use the same selector-derived `entryId`, typed `familyId`,
selector/fingerprint/citation catalog, and pinned-metadata validation path.
