# Setup and troubleshooting

For the normal build and launch commands, start with the [README](README.md).
This guide covers SDK configuration and alternative setup paths.

## SDK and runtime versions

[`winapp.yaml`](winapp.yaml) pins the metadata used to generate the bindings:

| Package | Version |
| --- | --- |
| Microsoft.Windows.SDK.CPP | 10.0.26100.6901 |
| Microsoft.WindowsAppSDK.Foundation | 1.8.251104000 |
| Microsoft.WindowsAppSDK.InteractiveExperiences | 1.8.251104001 |
| Microsoft.WindowsAppSDK.AI | 1.8.39 |

WinApp CLI 0.6.2 is a local development dependency; no global npm installation
is required. The runtime and codegen packages use repository-local `file:`
dependencies.

The [package manifest](Package.appxmanifest.in) requires
`Microsoft.WindowsAppRuntime.1.8` version **8000.675.1142.0 or newer**. This is
the framework minimum for the pinned SDK release. Restoring NuGet packages
provides build inputs; it does not replace installing the Windows App SDK
runtime or preparing an AI model.

## Generate from cached metadata

If `npm run restore` cannot reach NuGet, such as during a TLS or proxy failure,
do not disable certificate verification. With the pinned packages already in
the NuGet cache and Windows SDK 26100 installed, generate the bindings directly.

Run these commands from `samples\js\ocr`, after `prepare-local.ps1` and
`npm install`:

```powershell
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

npm run check
npm test
npm run start:generated -- --image "C:\images\sample.png"
```

Keep sibling metadata in its original package directories for type resolution.
InteractiveExperiences supplies the `WindowId` struct required by the picker.
Resolve any generation warnings before running the sample.

`start:generated` explicitly uses the existing bindings without WinApp CLI's
generation preflight. It is not an automatic fallback from a failed restore.
Regenerate after changing SDK inputs or the runtime/codegen packages.

Direct generation does not create WinApp CLI's managed-output marker. Before
switching back to `npm run restore` or `npm run generate`, move the directly
generated `.winapp\bindings` directory aside so WinApp CLI can create its own
managed output.

## Package identity

Both `npm start` and `npm run start:generated` launch through WinApp CLI with
the manifest's framework dependency and `systemAIModels` capability. These
commands do **not** need `WINAPPSDK_BOOTSTRAP_DLL_PATH`.

Running `node main.ts` in an ordinary terminal does not grant package identity.
If you use a custom packaged host, it must declare the same framework
dependency and AI capability. Setting a bootstrap DLL path does not grant
package identity or the AI capability.

If Windows AI reports access denied, check the package capability, supported
hardware, Windows version, and model availability against Microsoft's
[setup requirements](https://learn.microsoft.com/windows/ai/apis/get-started).

## Clean up an interrupted launch

Each launch uses a unique `.winapp\dynwinrt-ocr-<UUID>` host directory. Failed
launches retain that directory and print its path for diagnosis.

If registration or launch was interrupted, close that sample process and
unregister only its generated manifest:

```powershell
.\node_modules\.bin\winapp.cmd unregister `
  --manifest ".\.winapp\dynwinrt-ocr-<the-printed-UUID>\Package.appxmanifest"
```

After confirming the package is no longer registered or running, remove that
specific host directory. Do not use `--force`, unregister another sample, or
delete the entire `.winapp` directory as cleanup.
