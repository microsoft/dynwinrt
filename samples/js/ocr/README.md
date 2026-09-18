# Windows AI OCR

A Node.js sample that extracts text from an image and prints it to the terminal.
Choose an image with the Windows file picker, or pass a file path on the command
line.

The sample uses `Microsoft.Windows.AI.Imaging.TextRecognizer` through generated
dynwinrt bindings. This is the Windows AI OCR API, not `Windows.Media.Ocr`.

## Prerequisites

- Windows 11 with an NPU supported by
  [Windows AI text recognition](https://learn.microsoft.com/windows/ai/apis/text-recognition).
- Node.js **24 or newer**, ARM64 or x64.
- The **Windows App SDK 1.8 runtime** installed.
- Windows Developer Mode enabled for the development-package launch.
- Rust and Visual Studio C++ build tools to build the repository's local packages.

See Microsoft's [Windows AI setup requirements](https://learn.microsoft.com/windows/ai/apis/get-started)
for supported hardware and Windows versions.

## Run

From the repository root:

```powershell
cd samples\js\ocr
..\prepare-local.ps1
npm install
npm run restore
npm start
```

`prepare-local.ps1` builds the local runtime and code generator.
`npm run restore` restores the pinned SDK metadata and generates bindings.
Neither step installs the OCR model.

Choose an image in the picker. Recognized lines are printed under
`=== Recognized Text ===`.

The launcher uses a private Node copy with the package identity and
`systemAIModels` capability required by Windows AI. It does not modify the
installed `node.exe`, and removes the temporary registration on normal exit.
Use `npm start` rather than launching `main.ts` directly.

## Use an image path

To skip the picker:

```powershell
npm start -- --image "C:\images\sample.png"
```

The image is read without modifying it. An image with no recognized text is a
valid result.

## Prepare the OCR model

If the model is not ready, the sample stops with an error. To explicitly allow
Windows to prepare and potentially download the model, add `--ensure-ready`:

```powershell
npm start -- --image "C:\images\sample.png" --ensure-ready
```

Without this option, the sample does not request model installation.
Unsupported hardware and user-disabled models still produce an error.

## Commands

| Command | Purpose |
| --- | --- |
| `npm start` | Open the image picker and recognize text |
| `npm start -- --help` | Show command-line options |
| `npm run restore` | Restore SDK metadata and generate bindings |
| `npm run generate` | Regenerate bindings after a successful restore |
| `npm run check` | Type-check the sample and generated declarations |
| `npm test` | Run the native-free tests |

Exit codes are `0` for recognition or help, `1` for an error, `2` when no image
is selected, and `130` for Ctrl+C cancellation.

For cached SDK generation, custom hosts, or cleanup after an interrupted
launch, see [Setup and troubleshooting](SETUP.md).
