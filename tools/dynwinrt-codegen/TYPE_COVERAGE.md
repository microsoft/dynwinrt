# Type Coverage & Implementation

This document describes how each WinRT type is handled across the three layers:
**codegen** (tools/dynwinrt-codegen), **JS binding** (bindings/js), and **core runtime** (crates/dynwinrt).

## Overview

The codegen reads WinRT metadata (.winmd) and generates TypeScript bindings. Each WinRT type needs to be handled in four directions:

1. **Method parameter** (JS -> WinRT): `wrap_arg()` wraps JS value into `DynWinRtValue`
2. **Method return** (WinRT -> JS): `convert_return()` extracts JS value from `DynWinRtValue`
3. **Struct field read** (WinRT -> JS): `struct_field_getter()` reads from `DynWinRtStruct`
4. **Struct field write** (JS -> WinRT): `struct_field_setter()` writes to `DynWinRtStruct`

Plus two annotation functions: `ts_param_type()` and `ts_return_type()` for TypeScript type signatures.

---

## Primitive Types

| WinRT Type | TS Type | wrap_arg | convert_return | Struct getter | Struct setter |
|---|---|---|---|---|---|
| Boolean | `boolean` | `boolValue(x)` | `.toBool()` | `getU8(i) !== 0` | `setU8(i, x ? 1 : 0)` |
| Int8 | `number` | `i32(x)` | `.toNumber()` | `getI8(i)` | `setI8(i, x)` |
| UInt8 | `number` | `i32(x)` | `.toNumber()` | `getU8(i)` | `setU8(i, x)` |
| Int16 | `number` | `i32(x)` | `.toNumber()` | `getI16(i)` | `setI16(i, x)` |
| UInt16 | `number` | `i32(x)` | `.toNumber()` | `getU16(i)` | `setU16(i, x)` |
| Char16 | `number` | `i32(x)` | `.toNumber()` | `getU16(i)` | `setU16(i, x)` |
| Int32 | `number` | `i32(x)` | `.toNumber()` | `getI32(i)` | `setI32(i, x)` |
| UInt32 | `number` | `i32(x)` | `.toNumber()` | `getU32(i)` | `setU32(i, x)` |
| Int64 | `number` | `i64(x)` | `.toI64()` | `getI64(i)` | `setI64(i, x)` |
| UInt64 | `number` | `i64(x)` | `.toI64()` | `getU64(i)` | `setU64(i, x)` |
| Single | `number` | `f32(x)` | `.toF64()` | `getF32(i)` | `setF32(i, x)` |
| Double | `number` | `f64(x)` | `.toF64()` | `getF64(i)` | `setF64(i, x)` |

### Notes
- Bool uses `getU8`/`setU8` in structs because WinRT Boolean is 1 byte in struct layout
- Bool uses `boolValue()` for method params, creating `WinRTValue::Bool` (correct ABI width)
- I64/U64 return uses `.toI64()` (not `.toNumber()` which truncates to i32)
- F32/F64 return uses `.toF64()` (not `.toNumber()` which truncates to i32)
- Small integer params (I8, U8, I16, U16) wrapped as `i32()` — the runtime truncates at ABI call time

## String & GUID

| WinRT Type | TS Type | wrap_arg | convert_return | Struct getter | Struct setter |
|---|---|---|---|---|---|
| String (HSTRING) | `string` | `hstring(x)` | `.toString()` | `getHstring(i)` | `setHstring(i, x)` |
| Guid | `string` | `guid(WinGuid.parse(x))` | `.toString()` | `getGuid(i).toString()` | `setGuid(i, WinGuid.parse(x))` |

### Notes
- HSTRING in struct fields is non-blittable. `getHstring`/`setHstring` use `get_field_struct`/`set_field_struct` internally with proper refcount management (duplicate on read, release old + duplicate new on write)
- GUID is 16 bytes, blittable. `getGuid`/`setGuid` use `get_field::<GUID>()`/`set_field()` directly

## Enum

| WinRT Type | TS Type | wrap_arg | convert_return | Struct getter | Struct setter |
|---|---|---|---|---|---|
| Enum | `EnumName` | `i32(x)` | `.toNumber()` | `getI32(i)` | `setI32(i, x)` |

Generated as TypeScript `enum` with numeric values. ABI is always Int32.

## Struct (Value Type)

| WinRT Type | TS Type | wrap_arg | convert_return |
|---|---|---|---|
| Struct | `StructName` (interface) | `_packXxx(x).toValue()` | `_unpackXxx(expr)` |
| HResult | `number` | `i32(x)` | `.toNumber()` |

### Implementation

For each non-HResult struct used in method signatures, the codegen generates:

```typescript
// 1. TypeScript interface with camelCase field names
interface Point { x: number; y: number; }

// 2. Unpack: DynWinRtValue -> plain JS object
function _unpackPoint(v: DynWinRtValue): Point {
    const s = v.asStruct();
    return { x: s.getF32(0), y: s.getF32(1) };
}

// 3. Type constant for struct creation
const _Point_Type = DynWinRtType.registerStruct('Windows.Foundation.Point', [DynWinRtType.f32(), DynWinRtType.f32()]);

// 4. Pack: plain JS object -> DynWinRtStruct
function _packPoint(v: Point): DynWinRtStruct {
    const s = DynWinRtStruct.create(_Point_Type);
    s.setF32(0, v.x);
    s.setF32(1, v.y);
    return s;
}
```

### Supported struct field types
- All primitives (Bool, I8..U64, F32, F64) via typed getters/setters
- Enum via `getI32`/`setI32`
- String via `getHstring`/`setHstring` (non-blittable, refcounted)
- Guid via `getGuid`/`setGuid` (16-byte blittable)
- Nested struct via `getStruct`/`setStruct` + recursive `_unpack`/`_pack`
- IReference\<T\> (COM pointer) via `getObject`/`setObject` (rare, fallback to DynWinRtValue)

### Dependency ordering
Struct helpers are generated in post-order (inner structs before outer). E.g., if `ManipulationDelta` contains a `Point` field, `_unpackPoint` is generated before `_unpackManipulationDelta`.

### DynWinRtStruct import
The `DynWinRtStruct` import is only added to files that actually use structs, to avoid unused import warnings.

## Array

### As method return (WinRT -> JS)

The codegen auto-converts array returns to native JS arrays:

| Element Type | TS Return Type | Conversion |
|---|---|---|
| I8 | `number[]` | `.asArray().toI8Vec()` |
| U8 | `number[]` | `.asArray().toU8Vec()` |
| I16 | `number[]` | `.asArray().toI16Vec()` |
| U16 / Char16 | `number[]` | `.asArray().toU16Vec()` |
| I32 / Enum | `number[]` | `.asArray().toI32Vec()` |
| U32 | `number[]` | `.asArray().toU32Vec()` |
| I64 | `number[]` | `.asArray().toI64Vec()` |
| U64 | `number[]` | `.asArray().toU64Vec()` |
| F32 | `number[]` | `.asArray().toF32Vec()` |
| F64 | `number[]` | `.asArray().toF64Vec()` |
| Boolean | `boolean[]` | `.asArray().toValues().map(v => v.toBool())` |
| String | `string[]` | `.asArray().toStringVec()` |
| Guid | `string[]` | `.asArray().toValues().map(v => v.toString())` |
| Struct | `StructName[]` | `.asArray().toValues().map(v => _unpackXxx(v))` |
| RuntimeClass | `ClassName[]` | `.asArray().toValues().map(v => new ClassName(v))` |
| Object | `DynWinRtValue[]` | `.asArray().toValues()` |

### As method parameter (JS -> WinRT)

Input arrays use `DynWinRtArray` type. Users construct arrays via factory methods:

```typescript
DynWinRtArray.fromI32Values([1, 2, 3])
DynWinRtArray.fromStringValues(["a", "b"])
```

Available `fromXxxValues`: i8, u8, i16, u16, i32, u32, f32, f64, i64, u64, string.

### Array out parameters

WinRT ReceiveArray/FillArray patterns (array as `[out]` parameter, no retval) are handled identically — the codegen detects the array out param, determines the element type, and applies the same conversion.

## Reference Types

| WinRT Type | TS Type | wrap_arg | convert_return |
|---|---|---|---|
| Object (IInspectable) | `DynWinRtValue` | `(x as any)._obj ?? x` | raw (no conversion) |
| RuntimeClass | `ClassName` | `(x as any)._obj ?? x` | `new ClassName(expr)` |
| Interface | `InterfaceName` | `(x as any)._obj ?? x` | `new InterfaceName(expr)` |
| Delegate | `DynWinRtValue` | `(x as any)._obj ?? x` | raw (no conversion) |
| Parameterized | `ConcreteName` | `(x as any)._obj ?? x` | `new ConcreteName(expr)` |

### Notes
- `(x as any)._obj ?? x` extracts the underlying `DynWinRtValue` from wrapper classes, or passes raw if already a `DynWinRtValue`
- RuntimeClass/Interface returns are wrapped in generated constructor only when the type is in `known_types` (has a generated .ts file)
- Unknown reference types fall through to `DynWinRtValue`

## Async

| WinRT Type | TS Return Type | Conversion |
|---|---|---|
| IAsyncAction | `Promise<void>` | `.toPromise()` |
| IAsyncOperation\<T\> | `Promise<T>` | `(await expr.toPromise())` then convert inner T |
| IAsyncActionWithProgress\<P\> | `Promise<void>` | `.toPromise()` |
| IAsyncOperationWithProgress\<T,P\> | `Promise<T>` | `(await expr.toPromise())` then convert inner T |

The async unwrap is recursive: `convert_return` first unwraps the Promise, then applies the inner type's conversion. E.g., `IAsyncOperation<String>` generates `(await expr.toPromise()).toString()`.

## Events (Delegates)

| Pattern | TS Signature | Implementation |
|---|---|---|
| `add_EventName` | `onEventName(callback)` | `DynWinRtDelegate.create(IID, paramTypes, callback)` |
| `remove_EventName` | `offEventName(token)` | Pass token back to remove method |

Delegate IIDs and parameter types are generated as exports (`IID_DelegateName`, `DelegateName_PARAM_TYPES`) for use with `DynWinRtDelegate.create()`.

## DynWinRtStruct napi API

Complete field access API exposed to JavaScript:

| Method | Reads/Writes | Byte size |
|---|---|---|
| `getI8` / `setI8` | i8 | 1 |
| `getU8` / `setU8` | u8 | 1 |
| `getI16` / `setI16` | i16 | 2 |
| `getU16` / `setU16` | u16 | 2 |
| `getI32` / `setI32` | i32 | 4 |
| `getU32` / `setU32` | u32 | 4 |
| `getF32` / `setF32` | f32 | 4 |
| `getF64` / `setF64` | f64 | 8 |
| `getI64` / `setI64` | i64 | 8 |
| `getU64` / `setU64` | u64 | 8 |
| `getHstring` / `setHstring` | HSTRING | ptr (non-blittable) |
| `getGuid` / `setGuid` | GUID | 16 |
| `getStruct` / `setStruct` | nested struct | variable (non-blittable safe) |
| `getObject` / `setObject` | COM pointer | ptr (non-blittable) |
| `create(type)` | — | Allocates zero-initialized struct |
| `toValue()` | — | Wraps as DynWinRtValue for method calls |

### Memory safety
- `getHstring`/`getStruct`/`getObject` use `get_field_struct()` which clones non-blittable fields (AddRef/WindowsDuplicateString)
- `setHstring`/`setStruct`/`setObject` use `set_field_struct()` which releases old field values before writing + clones new values
- `ValueTypeData` implements `Clone` (deep copy with refcount increment) and `Drop` (release all non-blittable fields)

## DynWinRtArray napi API

### Read (WinRT -> JS)

| Method | Element type | Implementation |
|---|---|---|
| `toI8Vec()` | i8 → i32 | `as_typed_slice::<i8>` + widen |
| `toU8Vec()` | u8 | `as_typed_slice::<u8>` (zero-copy) |
| `toI16Vec()` | i16 → i32 | `as_typed_slice::<i16>` + widen |
| `toU16Vec()` | u16 → u32 | `as_typed_slice::<u16>` + widen |
| `toI32Vec()` | i32 | `as_typed_slice::<i32>` (zero-copy) |
| `toU32Vec()` | u32 | `as_typed_slice::<u32>` (zero-copy) |
| `toF32Vec()` | f32 | `as_typed_slice::<f32>` (zero-copy) |
| `toF64Vec()` | f64 | `as_typed_slice::<f64>` (zero-copy) |
| `toI64Vec()` | i64 | `as_typed_slice::<i64>` (zero-copy) |
| `toU64Vec()` | u64 → i64 | `as_typed_slice::<u64>` + cast |
| `toStringVec()` | HSTRING → String | Per-element `get()` + convert |
| `toValues()` | any → DynWinRtValue | Per-element `get()` (generic) |

### Write (JS -> WinRT)

| Method | JS input | WinRT value |
|---|---|---|
| `fromI8Values(i32[])` | number[] | WinRTValue::I8 |
| `fromU8Values(u8[])` | number[] | WinRTValue::U8 |
| `fromI16Values(i32[])` | number[] | WinRTValue::I16 |
| `fromU16Values(u32[])` | number[] | WinRTValue::U16 |
| `fromI32Values(i32[])` | number[] | WinRTValue::I32 |
| `fromU32Values(u32[])` | number[] | WinRTValue::U32 |
| `fromF32Values(f64[])` | number[] | WinRTValue::F32 |
| `fromF64Values(f64[])` | number[] | WinRTValue::F64 |
| `fromI64Values(i64[])` | number[] | WinRTValue::I64 |
| `fromU64Values(i64[])` | number[] | WinRTValue::U64 |
| `fromStringValues(String[])` | string[] | WinRTValue::HString |

## Known Limitations

1. **IReference\<T\> as struct field** — napi has `getObject`/`setObject`, codegen has fallback, but untested in practice (only `Windows.Web.Http.HttpProgress` uses this pattern in Windows.winmd)
2. **U64 precision** — JS number (f64) can only represent integers exactly up to 2^53. U64 values beyond that range lose precision. This is a JS language limitation.
3. **Guid array** — Uses `.toValues().map(v => v.toString())` (per-element), no native `toGuidVec()`. Low priority since Guid arrays are rare.
4. **Bool array** — Uses `.toValues().map(v => v.toBool())` (per-element), no native `toBoolVec()`.
