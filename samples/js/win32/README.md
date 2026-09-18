# Flat Win32 JavaScript samples

These examples use generated JavaScript bindings to call real Windows DLL
exports. `generate.ps1` reads `Windows.Win32.winmd` and emits `.js` modules and
`.d.ts` declarations; the examples load those modules through the locally built
`@microsoft/dynwinrt/win32` runtime. No TypeScript compilation or handwritten
native call signatures are required.

| Sample | Windows APIs | Demonstrates |
| --- | --- | --- |
| [system-info.mjs](system-info.mjs) | `GetTickCount64`, `GetSystemTime` | A `bigint` return and a native `SYSTEMTIME` output structure. |
| [registry-product-name.mjs](registry-product-name.mjs) | `RegOpenKeyExW`, `RegQueryValueExW`, `RegCloseKey` | Size probing, a caller-owned UTF-16 buffer, status handling, and deterministic `HKEY` cleanup. |
| [overlapped-file.mjs](overlapped-file.mjs) | `CreateFileW`, `ReadFile`, `WriteFile`, `CloseHandle` | IOCP-backed Promises, file offsets, retained buffers, and an `AbortSignal`. |

None of the samples requires administrator privileges, network access at
runtime, package identity, or WinAppSDK initialization. Application code uses
the generated safe wrappers, not the manual unsafe Win32 entrypoint.

## Prerequisites

- Windows 10 or later with a 64-bit Node.js 18+ process (x64 or ARM64).
  Flat Win32 invocation is not supported in a 32-bit process.
- Rust with the Windows MSVC build tools/SDK, PowerShell, and NuGet.
- `Microsoft.Windows.SDK.Win32Metadata` version `71.0.14-preview`, containing
  `Windows.Win32.winmd`.

The sample package references `bindings\js` in this checkout rather than a
published npm runtime. Dependency installation may require network access.

From the repository root, build the runtime and code generator:

```powershell
Push-Location bindings\js
npm install
npm run build
Pop-Location

cargo build -p dynwinrt-codegen --release
```

Restore the same metadata package used by CI if it is not already installed:

```powershell
$metadataRoot = Join-Path $env:TEMP "dynwinrt-win32metadata"
nuget install Microsoft.Windows.SDK.Win32Metadata `
  -Version 71.0.14-preview `
  -OutputDirectory $metadataRoot `
  -DirectDownload `
  -NonInteractive
$winmd = Get-ChildItem $metadataRoot -Filter Windows.Win32.winmd -File -Recurse |
  Select-Object -First 1
```

Generate the bindings and install the locally built runtime:

```powershell
Push-Location samples\js\win32
npm install
.\generate.ps1 `
  -Win32Winmd $winmd.FullName `
  -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
```

Run any sample:

```powershell
npm run system-info
npm run registry
npm run overlapped-file
Pop-Location
```

Generated files and `node_modules` are local artifacts and are not tracked.

## Generated modules

The three namespaces are kept in separate directories:

```text
generated\
  package.json
  win32\
    windows\win32\system\system-information\
    windows\win32\system\registry\
    windows\win32\storage\file-system\
```

Each namespace contains `Apis.js`, `Apis.d.ts`, namespace indexes, and any
required enum modules. The samples import `index.mjs`; unsuffixed names such
as `regOpenKeyEx` and `regQueryValueEx` are aliases of the Unicode `W` APIs.
`generate.ps1` replaces this sample's entire `generated` directory, so do not
put handwritten files there.

## Expected behavior

### System information

`npm run system-info` prints the system uptime in milliseconds and the current
UTC time. `createSYSTEMTIME()` allocates the native structure;
`getSystemTime()` fills it, and the sample reads its 16-bit fields.

Illustrative output; values change on every run:

```text
Windows uptime: 123456789 ms
System UTC time: 2026-09-11T10:00:00.000Z
```

### Registry product name

`npm run registry` opens
`HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows NT\CurrentVersion` with
`KEY_READ` and reads `ProductName`. It first passes `null` to obtain the
required byte count, allocates a `Buffer`, then decodes the filled UTF-16LE
data. The managed key is closed in `finally`; no registry values are changed.

Example output:

```text
Windows 10 Enterprise
```

The exact string depends on the machine's registry value and may still say
Windows 10 on a Windows 11 installation.

### Asynchronous file round trip

`npm run overlapped-file` creates a uniquely named file in the system temporary
directory with `FILE_FLAG_OVERLAPPED`. It writes and reads the same bytes at
offset `0n`, checks the transferred byte counts and payload, then closes the
handle and removes the file in `finally`.

```text
Hello from OVERLAPPED Win32 I/O
```

The sample passes an `AbortSignal` but does not deliberately abort the
operation. The generated helpers support cancellation through
`AbortController.abort()`; cancellation and detached-buffer behavior are
covered separately by the Win32 E2E runners.

## Common setup issues

- Missing `generated` modules: run `generate.ps1` before an npm sample command.
- Missing native addon or `win32` entrypoint: build `bindings\js` and run
  `npm install` in this sample directory, using a matching Node.js architecture.
- Metadata mismatch: use the pinned Win32Metadata version above rather than
  substituting the WinRT `Windows.winmd` file.
