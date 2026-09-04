# dynwinrt 0.1.0-preview.22

This unified preview improves Python-native WinRT use, expands Classic COM
generation, and adds practical WinUI setup examples. The npm version is
`0.1.0-preview.22`; its Python PEP 440 version is `0.1.0rc22`.

## Highlights

- Generated Python async operations now work with `asyncio.create_task()` and
  `TaskGroup.create_task()`, while retaining direct `await`, progress,
  cancellation, and explicit blocking support.
- `IBuffer` projections provide safe copied byte conversion, and
  `unboxObject()`/`unbox_object()` explicitly convert supported boxed
  `IPropertyValue` results without changing generic object projection.
- Classic COM codegen now emits validated safe bindings plus generated unsafe
  companions and raw ABI support. Complete safe generation covers 5,681 of
  7,929 eligible interfaces in the current Win32 metadata baseline.
- Python projected-lifetime scopes now enforce thread affinity across asyncio
  tasks, worker threads, and foreign-thread callbacks.
- New JavaScript and Python WinUI samples demonstrate Windows App SDK
  initialization and repeatable WinApp CLI setup.

## Action required

- Node.js 18 or later is required.
- Regenerate existing Classic COM bindings. Generated modules now use canonical
  `com/` and `com/unsafe/` paths; legacy flat paths are not emitted.
- Import Classic COM through the explicit `@microsoft/dynwinrt/com` entrypoint.
  The package root remains WinRT-only, with lower-level access isolated under
  `@microsoft/dynwinrt/com/unsafe` and `@microsoft/dynwinrt/com/unsafe/raw`.

## Install

```bash
npm install @microsoft/dynwinrt@0.1.0-preview.22
npm install -D @microsoft/dynwinrt-codegen@0.1.0-preview.22
python -m pip install --pre "dynwinrt==0.1.0rc22" "dynwinrt-codegen==0.1.0rc22"
```

## Packages/platforms

- `@microsoft/dynwinrt`: JavaScript/TypeScript runtime with Windows x64 and
  ARM64 native addons for Node.js 18 or later.
- `@microsoft/dynwinrt-codegen`: typed WinRT and supported Classic COM
  generation for JavaScript/TypeScript.
- `dynwinrt`: CPython 3.11-3.14 runtime wheels for Windows x64 and ARM64.
- `dynwinrt-codegen`: standalone Windows x64 and ARM64 wheels that include a
  prebuilt generator and require no Rust installation.

**Detailed changes:** [v0.1.0-preview.21...v0.1.0-preview.22](https://github.com/microsoft/dynwinrt/compare/v0.1.0-preview.21...v0.1.0-preview.22)
