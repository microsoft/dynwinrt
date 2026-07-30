---
name: classic-com-abi
description: Use when implementing or reviewing Classic COM, Windows.Win32.winmd, native ABI, pointer, handle, ownership, libffi, or COM codegen changes in dynwinrt.
---

# Classic COM ABI development

Use this skill for changes under:

- `crates/dynwinrt/src/com.rs`, `signature.rs`, `native_call.rs`, or `call.rs`;
- `bindings/js/src/com.rs`;
- `tools/dynwinrt-codegen/src/com_metadata.rs`;
- `tools/dynwinrt-codegen/src/codegen/com/`; or
- Classic COM runners in `tests/runners/com/`.

Read [`docs/classic-com-support.md`](../../../docs/classic-com-support.md)
before changing supported types or claiming support for an interface.

## Core principle

Start from the native ABI type **and parameter contract**, never from the
desired JavaScript/Python representation.

```text
Windows.Win32.winmd facts
  -> COM-local semantic ABI model
  -> validation and ownership plan
  -> libffi call plan
  -> language projection
```

`Buffer`, `bigint`, `string`, and generated wrappers are projection choices.
They must not determine native semantics.

## Required semantic model

Preserve these facts before rendering:

- native type name and underlying type;
- pointer depth;
- const/mutability;
- `In`, `Out`, or `InOut`;
- nullable/required state;
- struct/union size, alignment, packing, and fields;
- count/capacity/actual-length parameter relationships;
- ownership transfer;
- allocator or cleanup function;
- interface IID and reference ownership; and
- return convention: HRESULT, semantic HRESULT, direct value, pointer, or
  `void`.

Do not erase these facts into a generic `Object` or pointer before validation.

## Semantic categories

Model at least these categories explicitly:

```text
Scalar
Enum
NativeStruct
NativeUnion
HandleValue
DataPointer
StringPointer
Bstr
ComInterface
CountedBuffer
SafeArray
Variant
FunctionPointer
Unknown
```

Unknown or incomplete categories must fail closed.

## Layer boundaries

1. Keep Classic COM metadata and projected types COM-local.
2. Do not add Classic COM concepts to the existing WinRT metadata model or
   `DynWinRt*` public surface.
3. Sharing private libffi storage and vtable dispatch is allowed.
4. Keep the npm root WinRT-only; generated COM bindings import
   `@microsoft/dynwinrt/com`.
5. Renderers consume validated semantic IR. They must not infer ABI semantics
   from names, JavaScript values, or struct shape.

### Required runtime architecture

```text
WinRT metadata -> signature.rs (WinRT planner) --------\
                                                        -> native_call.rs -> call.rs -> native method
COM metadata   -> com.rs (COM planner and method table) /
```

Keep these source-level responsibilities distinct:

| Component | Required responsibility |
|---|---|
| `signature.rs` | WinRT-only signature facade preserving existing `In`, `Out`, fill-array, HRESULT, and out-value behavior. |
| `com.rs` | COM-local `Type`, `MethodSignature`, `Interface`, `MethodHandle`, interface roots, method registry, pointer/InOut semantics, and native return conventions. |
| `native_call.rs` | Private lowering backend for completed signatures: parameter/output indexing, value validation and coercion, array ABI expansion, fast-path selection, libffi CIF preparation, and result coordination. |
| `call.rs` | Private executor: vtable lookup, stable ABI storage, libffi argument construction and invocation, and decoding raw output slots according to the plan. |

Apply these rules:

- WinRT methods stay in the WinRT `MetadataTable`; COM methods stay in the
  COM-local registry.
- Only the WinRT planner may define WinRT signature behavior. Do not add raw
  pointers, `InOut`, direct native returns, or `void` returns to its public
  model.
- Only the COM metadata/projection and planner layers may interpret pointer
  categories, parameter direction, return convention, and ownership.
- A by-value GUID is not REFIID. Dynamic-IID output adoption requires
  pointer-shaped metadata plus an explicit `iid`/`riid` semantic parameter.
- `native_call.rs` may validate and lower an already-described call, but must
  not infer metadata semantics, allocator ownership, or language projection.
- `call.rs` must execute the completed plan without inferring metadata,
  ownership, or projection contracts from the caller or language-level value.
- Native methods published through a shared registry must be fully constructed
  and immutable. Any manual `Send`/`Sync` implementation requires a documented
  libffi read-only safety argument and compile-time trait tests.
- Exact identity checks are required for structs. Preserve established
  ABI-compatible WinRT projection aliases such as Char16/U16 and enum/I32
  arrays.

## Projection responsibility

Keep these responsibilities separate:

| Layer | Responsibility |
|---|---|
| Runtime / ABI | Faithfully and safely execute a fully described native call: storage, libffi types, vtable dispatch, HRESULT, ownership, and cleanup. |
| Codegen semantic projection | Turn validated COM semantics into an idiomatic language API: Buffer/string/bigint choices, camelCase, overloads, optional arguments, hidden ABI parameters, and projected return values. |
| Renderer | Serialize the projection decision into JavaScript and declarations. It must not discover or guess native semantics. |

Electron/Node conveniences belong in the JavaScript projection. The runtime may
provide a small, centralized safety primitive such as `handleValue()`, but it
must not decide that an arbitrary Buffer represents a handle.

## WinRT compatibility invariant

Classic COM work must not change existing WinRT semantics.

- Do not add COM-only types, directions, ownership, pointers, or return
  conventions to the public WinRT model.
- Do not change existing `DynWinRt*` behavior or the
  `@microsoft/dynwinrt` root surface.
- Do not change generated WinRT constructors, method signatures, imports,
  naming, ownership, or output files as a side effect of COM support.
- Shared ABI/libffi helpers must remain private and behavior-neutral for WinRT.
- Route Classic COM through COM-local metadata and projection before any
  language renderer.
- Require WinRT snapshot, package, runtime, and live E2E regression coverage
  for every shared-infrastructure change.

## Pointer and Buffer rules

A Node Buffer can have different native meanings:

| Semantic type | Buffer meaning | Projection |
|---|---|---|
| Handle value | Pointer-width bytes containing a numeric handle | Explicit `DynCom.handleValue()` |
| Data pointer | Native data stored in the Buffer | `DynCom.pointer(buffer)` passes and retains its address |
| String pointer | Encoded, terminated string bytes | Pass the backing address with encoding validation |
| BSTR | Length-prefixed Automation allocation | Dedicated BSTR allocation/conversion |
| COM interface | Reference-counted interface pointer | Managed COM wrapper, never a Buffer |

Never apply one Buffer interpretation to every pointer-shaped typedef.

For Electron HWND input:

- accept Buffer/Uint8Array only for a confirmed `HWND` input;
- require exactly `size_of::<usize>()` bytes;
- decode little-endian handle bits in the centralized runtime helper;
- keep HWND output aliases numeric; and
- keep PSID, security descriptors, structs, and strings on address semantics.

Do not infer `HandleValue` merely because a Win32 struct has one `Value`
pointer field. Use metadata attributes and an explicit conservative mapping.
Examples:

- `HANDLE`: `RAIIFree(CloseHandle)`;
- `HKEY`: `RAIIFree(RegCloseKey)`;
- `HICON`: `RAIIFree(DestroyIcon)`;
- `HWND`: `AlsoUsableFor(HANDLE)`;
- `BSTR`: `RAIIFree(SysFreeString)`;
- `PSID`: data pointer, not a handle value.

## Ownership rules

- `CoCreateInstance`, QueryInterface, and typed interface out-parameters return
  owned `+1` references.
- Managed COM values release automatically; explicit `release()` is only
  deterministic early release.
- Interface inputs are borrowed unless the callee AddRefs them for retention.
- `adoptComPointer()` accepts only a native output known to transfer `+1`.
- Numeric and Buffer-backed pointers are borrowed and cannot be adopted.
- Pair BSTR with `SysFreeString`.
- Pair HSTRING ownership with `WindowsDeleteString`; never project HSTRING as a
  numeric pointer.
- Pair CoTaskMem allocations with `CoTaskMemFree`.
- Win32 handles are not COM references; cleanup is resource-specific.
- Unknown allocator or ownership contracts fail closed.

## Metadata evidence

Before supporting an interface:

1. Parse the actual configured `Windows.Win32.winmd`.
2. Walk its full interface inheritance chain.
3. Inspect every method, not only the method intended for a sample.
4. Record `NativeArrayInfo`, `FreeWith`, `Const`, parameter direction, and
   pointer depth.
5. Record `CanReturnMultipleSuccessValuesAttribute` before deciding whether an
   HRESULT is throw-or-void or a semantic result.
6. Resolve every referenced interface IID from the loaded metadata. Require
   callers to provide external definitions through `--ref`.
7. Check Microsoft API documentation for ownership that metadata does not
   encode.
8. Generate with `--dry-run` and verify unsupported methods stop the whole
   unsafe interface projection.

Do not claim general interface support when only a manually described runtime
subset works.

## Fail-closed requirements

Reject generation when any required fact is unknown, including:

- native struct/union layout;
- writable caller-sized buffers without a modeled count relationship;
- untyped output pointers without ownership;
- unsupported interface in/out replacement;
- BSTR arrays or unknown string allocation;
- VARIANT, PROPVARIANT, SAFEARRAY, FORMATETC, or STGMEDIUM without dedicated
  models;
- unsupported direct native returns; or
- interface parameters whose IID or PIID cannot be resolved from loaded
  metadata;
- parameterized or async interfaces without a computed closed IID;
- delegates without a managed callback projection;
- native arrays without explicit count and element-ownership contracts;
- incomplete inherited vtable layout.

An error during generation is safer than plausible generated code with the
wrong ABI.

## Validation

Every new semantic type or ownership rule needs:

1. a pure unit test for mapping and rendering;
2. a real `Windows.Win32.winmd` regression test;
3. a runtime test covering storage and cleanup;
4. a fail-before/fail-closed test for the nearest unsupported shape;
5. x64 and i686 compile validation for pointer-sized ABI;
6. WinRT regression coverage proving the existing generator and runtime did
   not change; and
7. a live stock-Windows E2E when the API is deterministic and requires no
   optional software, network, or user interaction.

Prefer tests that add a new ABI shape. Do not add many interfaces that only
repeat activation.

## Review checklist

- Does the change start from metadata facts rather than JS convenience?
- Is the semantic type explicit?
- Are pointer depth and direction preserved?
- Is storage correctly sized before native invocation?
- Is ownership explicit on success and failure?
- Are x86 and x64 widths correct?
- Can a borrowed pointer become a second owner?
- Can Buffer contents be confused with Buffer address?
- Does an InOut path use the same conversion and helper availability as In?
- Does the renderer contain ABI heuristics that belong in projection?
- Does unsupported metadata fail during generation?
- Did any WinRT model, output, or root API change?
