# Low-level WinRT examples

Small Node.js examples using `@microsoft/dynwinrt` directly: register an
interface, describe its methods, and invoke them with `DynWinRtValue` arguments.
The examples use handwritten declarations, not generated application bindings.

## Examples

| Example | Command | Demonstrates |
| --- | --- | --- |
| [`uri.ts`](uri.ts) | `npm run uri` | Create a URI and read string/integer properties |
| [`async-file.ts`](async-file.ts) | `npm run async-file` | Open a temporary file through typed WinRT async operations |
| [`struct-geopoint.ts`](struct-geopoint.ts) | `npm run struct-geopoint` | Pass and return a `BasicGeoposition` struct |
| [`array-property-value.ts`](array-property-value.ts) | `npm run array-property-value` | Box and read Int32 arrays |
| [`picker.ts`](picker.ts) | `npm run picker` | Open a file picker and print the selected path |

The first four use Windows' built-in WinRT APIs. They do not need Windows App
SDK, package identity, or a bootstrap DLL. The picker uses Windows App SDK 1.8
and has [additional setup](#file-picker).

## Run

Use Windows 10/11 with Node.js **24 or newer**, ARM64 or x64. Building the local
runtime also requires Rust and Visual Studio C++ build tools.

From the repository root, build the runtime and run an example:

```powershell
Push-Location bindings\js
npm install
npm run build
Pop-Location

cd samples\js\low-level
npm install --omit=dev
npm run uri
```

The other examples can be run with the commands in the table above.
There is no code generation step. `@microsoft/dynwinrt-codegen` is used only by
the optional development checks, not by the examples.

The URI example does not make a network request. The async-file example
creates and removes its own temporary file. The Geopoint example uses fixed
coordinates and does not read the device's location.

## File picker

Install the **Windows App SDK 1.8 runtime** and provide the matching bootstrap
DLL for an unpackaged Node process. For a Foundation package already restored
in the NuGet cache, run the following from this directory:

```powershell
$nuget = if ($env:NUGET_PACKAGES) { $env:NUGET_PACKAGES } else { Join-Path $HOME ".nuget\packages" }
$arch = node -p "process.arch"
$env:WINAPPSDK_BOOTSTRAP_DLL_PATH = Join-Path $nuget "microsoft.windowsappsdk.foundation\1.8.251104000\runtimes\win-$arch\native\Microsoft.WindowsAppRuntime.Bootstrap.dll"
npm run picker
```

If that package is not present, obtain the bootstrap DLL through your Windows
App SDK setup and set the environment variable to its actual path. The DLL
architecture must match Node.

Choose a file to print its path, or cancel the dialog. The sample does not read
or modify the selected file's contents and does not require package identity or
an AI model.

To check initialization and property calls without showing the dialog:

```powershell
npm run picker -- --no-ui
```

This still makes native calls. Successful selection or `--no-ui` exits 0,
errors exit 1, and no file returned (cancelled or unavailable) exits 2.

## Source layout

- [`contracts.ts`](contracts.ts) contains the handwritten interface declarations.
- [`example.ts`](example.ts) handles resource release and nonzero error exits.
- [`picker-support.ts`](picker-support.ts) handles the picker's SDK initialization.

For typechecking, tests, and optional metadata comparison, see
[Developing the examples](DEVELOPMENT.md).
