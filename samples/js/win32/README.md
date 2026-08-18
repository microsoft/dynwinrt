# Flat Win32 JavaScript samples

These samples generate and call safe flat Win32 bindings:

- `system-info.mjs` reads the 64-bit Windows uptime and fills a branded
  `SYSTEMTIME` native struct.
- `registry-product-name.mjs` demonstrates a two-phase caller-owned buffer and
  deterministic `HKEY` cleanup.
- `overlapped-file.mjs` opens a file with `FILE_FLAG_OVERLAPPED`, writes and
  reads it with generated Promises, and passes an `AbortSignal`.

Neither sample requires administrator privileges, network access at runtime,
or the unsafe Win32 entrypoint.

## Prerequisites

- Windows 10 or later with Node.js 18+, Rust, and NuGet available.
- `Microsoft.Windows.SDK.Win32Metadata` containing `Windows.Win32.winmd`.

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
