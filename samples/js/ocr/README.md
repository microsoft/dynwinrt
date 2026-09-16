# Windows AI OCR (JavaScript/TypeScript)

The [`main.ts`](main.ts) entry uses
`Microsoft.Windows.AI.Imaging.TextRecognizer`, **not** `Windows.Media.Ocr`.
With no arguments it opens the Windows App SDK image picker. `--image` bypasses
the picker for deterministic, read-only processing of a specified image.

All WinRT calls use generated JavaScript and TypeScript declarations. The
sample has no handwritten IIDs, vtable offsets, or untyped async outputs.
This directory contains the entry, setup, implementation, and focused tests;
it does not change the other standalone examples.

## Prerequisites

- Windows 11 on hardware supported by
  [Windows AI text recognition](https://learn.microsoft.com/windows/ai/apis/text-recognition),
  including a supported NPU. An ordinary Windows SDK installation alone is
  **not** sufficient.
- Native ARM64 or x64 Node.js **24 or newer** (built-in TypeScript support).
- The matching Windows App SDK **1.8 runtime**, framework version
  **8000.675.1142.0 or newer**, installed. See Microsoft's
  [Windows AI setup requirements](https://learn.microsoft.com/windows/ai/apis/get-started).
  Restoring NuGet metadata is not the same as installing the runtime or model.
- Developer Mode already enabled for the WinApp CLI development-package flow.
  This sample does not enable it, change system policy, or install certificates.
- For a source build, the repository's Rust/MSVC prerequisites.

The SDK inputs are pinned in `winapp.yaml`: Windows SDK CPP
`10.0.26100.6901`, Foundation `1.8.251104000`, InteractiveExperiences
`1.8.251104001`, and AI `1.8.39`. InteractiveExperiences supplies the actual
`Microsoft.UI.WindowId` struct used by the picker constructor.
The framework minimum comes from `WindowsAppSDK-VersionInfo.json` in Runtime
`1.8.251106002`, the runtime paired with these SDK packages by the
`Microsoft.WindowsAppSDK` `1.8.251106002` package.

## Restore, generate, and run

From the repository root:

```powershell
cd samples\js\ocr
..\prepare-local.ps1
npm install
npm run restore
npm run check
npm test
npm start
```

`prepare-local.ps1` builds the repository's production runtime and codegen npm
packages. Both are consumed through local `file:` dependencies. Use its
`-Architecture arm64` or `-Architecture x64` option when necessary to match Node.
No global npm installation is needed.

WinApp CLI **0.6.2** is pinned locally. `npm run restore` restores the configured
SDKs, copies the bootstrap DLL into `.winapp\bin\<architecture>`, and generates
bindings through `winapp.jsBindings`. `npm run generate` regenerates from an
existing restore. Normal `npm start` also runs this generation preflight: a
missing/stale restore inventory is a nonzero error, not permission to launch
old generated files. Generated files and dependencies are ignored. Do not edit
generated JavaScript or declarations.

To process a specific image, including a path containing spaces:

```powershell
npm start -- --image "C:\my-test-images\owned image.png"
```

`npm start` runs `launch.ts`, which makes a **private copy** of Node under a
uniquely named `.winapp\dynwinrt-ocr-<UUID>` directory. WinApp CLI generates local
assets and launches that copy with the `Package.appxmanifest.in` template's
`systemAIModels` capability. The `.in` suffix prevents WinApp CLI from
auto-discovering the source template as an already-created package. The launch
uses an execution alias so the process retains console output,
and requests `--unregister-on-exit`. Each launch has a unique package/alias;
neither the installed `node.exe` nor another application's identity is modified.
No junction to the Node installation, Electron, or sandbox override is involved.
The manifest declares `Microsoft.WindowsAppRuntime.1.8` as a framework
dependency, so Windows supplies the package graph before the application runs.

The sample entry can also be run directly from this directory:

```powershell
node main.ts --help
node main.ts --image "C:\my-test-images\owned image.png"
```

The second command needs a full packaged host with the same framework
dependency and AI capability, not just any package identity. A normal,
identityless Node process needs bootstrap setup and then fails the AI identity
check; setting the bootstrap environment variable does **not** supply package
identity or AI capabilities.
Use `npm start` for the identity-aware development launch.

### Model readiness and explicit preparation

By default the sample only checks readiness. It never invokes
`EnsureReadyAsync` or silently downloads a model. A `NotReady` result stops with
an actionable nonzero error.

Only run the following if you explicitly consent to Windows preparing and
potentially downloading the OCR model:

```powershell
npm start -- --image "C:\my-test-images\owned image.png" --ensure-ready
```

The sample awaits the typed readiness result, checks its status and extended
error, and rechecks `Ready` before creating a recognizer. Unsupported hardware
and user-disabled states remain errors, even with this option. The sample never
changes those settings or replaces TextRecognizer with a different OCR engine.

### Offline generation from already restored SDKs

If WinApp CLI cannot reach NuGet (for example, a TLS/proxy failure), do not
disable certificate verification. With the **pinned packages already present**
in the NuGet cache and Windows SDK 26100 installed, the supported codegen CLI
can generate the same selected API closure directly:

```powershell
# Run from samples\js\ocr after prepare-local.ps1 and npm install.
$nuget = if ($env:NUGET_PACKAGES) { $env:NUGET_PACKAGES } else { Join-Path $HOME ".nuget\packages" }
$windows = "${env:ProgramFiles(x86)}\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd"
$ai = Join-Path $nuget "microsoft.windowsappsdk.ai\1.8.39\metadata\Microsoft.Windows.AI.Imaging.winmd"
$picker = Join-Path $nuget "microsoft.windowsappsdk.foundation\1.8.251104000\metadata\Microsoft.Windows.Storage.Pickers.winmd"
$ui = Join-Path $nuget "microsoft.windowsappsdk.interactiveexperiences\1.8.251104001\metadata\10.0.18362.0\Microsoft.UI.winmd"
$classes = "Windows.Storage.StorageFile,Windows.Graphics.Imaging.BitmapDecoder,Microsoft.Windows.Storage.Pickers.FileOpenPicker,Microsoft.Graphics.Imaging.ImageBuffer,Microsoft.Windows.AI.Imaging.TextRecognizer"

.\node_modules\.bin\dynwinrt-codegen.cmd generate `
  --winmd "$windows;$ai;$picker" --ref $ui `
  --class-name $classes --output .\.winapp\bindings
if ($LASTEXITCODE -ne 0) { throw "OCR binding generation failed." }

$arch = node -p "process.arch"
$env:WINAPPSDK_BOOTSTRAP_DLL_PATH = Join-Path $nuget "microsoft.windowsappsdk.foundation\1.8.251104000\runtimes\win-$arch\native\Microsoft.WindowsAppRuntime.Bootstrap.dll"
npm run check
npm test
npm run start:generated -- --image "C:\my-test-images\owned image.png"
```

Keep sibling metadata in its original package directories for dependency
discovery. Missing types or generation warnings must be resolved by providing
the correct metadata, not by substituting `Object` or copying old method tables.
Direct generation does not create WinApp CLI's managed-output marker. Before
switching back to `npm run restore`/`npm run generate`, move the directly
generated `.winapp\bindings` directory aside so WinApp CLI can create its own
managed output. Neither flow installs the Windows AI model.
`start:generated` is an explicit opt-in to these already generated bindings and
prints that it does **not** validate WinApp CLI SDK restore. It is not an
automatic fallback from a failed restore. Regenerate and rerun the checks after
changing any SDK inputs or the codegen/runtime packages.

## Results, cancellation, and lifetime

The pipeline is `StorageFile.OpenAsync` -> `BitmapDecoder.CreateAsync` ->
`GetSoftwareBitmapAsync(Bgra8, Premultiplied)` ->
`ImageBuffer.CreateForSoftwareBitmap` -> `TextRecognizer`.
The text printed is **`RecognizedText.Lines` / each line's `Text`**, in order,
not `IStringable.ToString()`.

`roInitialize(1)` (MTA) is explicit on both paths. The private full packaged
host uses its manifest's
[static framework dependency](https://learn.microsoft.com/windows/apps/windows-app-sdk/deploy-packaged-apps).
An identityless process explicitly calls `initWinappsdk(1, 8)` using the
configured bootstrap DLL. Do not unconditionally bootstrap a full packaged
process: `MddBootstrapInitialize2` rejects that use with `0x80070032`.
The standalone picker uses a metadata-typed zero `WindowId` for an unowned dialog.
Streams and projected references are released after use; the bitmap, image
buffer, and recognizer are closed even on failure. The JavaScript runtime keeps
bootstrap alive for the process lifetime; it does not expose a matching
`roUninitialize`/bootstrap-shutdown API.

| Exit | Meaning |
| --- | --- |
| 0 | Help, or completed recognition (an empty text result is valid) |
| 1 | Argument, setup, identity/capability, readiness, decoding, recognition, launch, or cleanup error |
| 2 | No file selected: picker cancelled or did not return a file; no OCR success is printed |
| 130 | Ctrl+C cancellation |

A `Ready` log is **only a readiness check**, not proof that inference succeeded.
The recognized-text heading is printed only after recognition and cleanup
complete. Access denied from a readiness/call stage can still indicate missing
capability grants or Windows AI prerequisites even when package identity exists.
See the linked Microsoft prerequisites; do not bypass policy to force success.

Successful private launches remove their temporary host files. Failed launches
retain the printed host directory for diagnosis. If WinApp CLI was interrupted
during registration/launch, close that sample process and unregister **only
that generated manifest**, without `--force`:

```powershell
.\node_modules\.bin\winapp.cmd unregister `
  --manifest ".\.winapp\dynwinrt-ocr-<the-printed-UUID>\Package.appxmanifest"
```

After confirming it is no longer registered/running, remove only that specific
host directory. Never unregister another sample, delete the entire `.winapp`
tree as cleanup, or modify the system Node installation.

## Focused checks

`npm run check` checks the sample against its actual generated declarations
with strict TypeScript and without `skipLibCheck`. `npm test` uses the existing
Node test runner and blocks native addon loading. It covers arguments, actual
entry-process exit codes, cancellation, readiness/download gating, typed text
extraction, cleanup failure propagation, private launch arguments/manifest, and
metadata-derived async/slot regressions. These are pure checks, **not** a claim
of live OCR success or hardware/model availability.
