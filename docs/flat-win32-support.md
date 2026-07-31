# Flat Win32 support

Flat Win32 APIs are DLL exports described by `[DllImport]` methods in
`Windows.Win32.winmd`. They do not use WinRT activation or COM vtables.

```text
Windows.Win32.winmd
  -> flat Win32 metadata model
  -> validated ABI and projection plan
  -> generated JavaScript and declarations
  -> @microsoft/dynwinrt/win32
  -> System32 DLL export through libffi
```

## Generate bindings

```powershell
dynwinrt-codegen generate `
  --winmd C:\path\to\Windows.Win32.winmd `
  --namespace Windows.Win32.System.Registry `
  --class-name Apis `
  --output .\generated
```

The namespace is isolated so multiple `Apis` containers can share one output:

```text
generated/
  package.json
  win32/
    Windows.Win32.System.Registry/
      Apis.js
      Apis.d.ts
      index.js
      index.d.ts
      package.json
```

Generated modules import the dedicated
`@microsoft/dynwinrt/win32` entrypoint. The npm package root remains WinRT-only,
and `@microsoft/dynwinrt/com` remains Classic COM-only.

## Current supported subset

- x64 and ARM64 system DLL exports;
- fixed-arity functions;
- signed and unsigned integers from 8 through 64 bits;
- `float`, `double`, `BOOL`, and 32-bit enums;
- explicitly classified Win32 handle values;
- UTF-16 input strings;
- caller-encoded ANSI byte strings;
- single-level scalar and handle out/in-out parameters;
- caller-owned buffers with explicit element-count or byte-count metadata;
- direct handle and function-pointer returns; and
- atomic `GetLastError` capture when metadata marks an export accordingly.

Metadata DLL names are loaded from System32 with
`LOAD_LIBRARY_SEARCH_SYSTEM32`. Arbitrary DLL paths are rejected.

## Fail-closed behavior

An individual export is omitted with a diagnostic when its complete ABI cannot
be represented safely. This includes:

- variadic functions;
- architecture-specific overloads that differ between x64 and ARM64;
- by-value structs or unions without a native layout model;
- unbounded writable pointers and string buffers;
- pointer returns without known lifetime or ownership;
- nested or unsized native arrays;
- JavaScript callbacks without a managed native thunk;
- BSTR, SAFEARRAY, VARIANT, and other allocator-sensitive values; and
- enums whose underlying ABI cannot be represented faithfully.

Generated bindings are a safe subset of the requested `Apis` container, not a
claim that every function in a namespace is supported.
