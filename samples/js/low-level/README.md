# Low-level JavaScript runtime examples

This directory separates direct runtime API demonstrations from the application
samples that use metadata-generated bindings. Low-level examples register
interfaces with `DynWinRtType` and `DynWinRtMethodSig`, then call method handles
through `invoke()`.

These demonstrations are not the automated test suite in
[`bindings/js/__test__`](../../../bindings/js/__test__/).
For application code, prefer generated bindings, which derive interface IDs,
method ordering, and closed async result types from the selected metadata.

## OS-only examples

| Example | Demonstrates |
| --- | --- |
| [`uri.ts`](uri.ts) | Activation factory, QueryInterface via `cast()`, method lookup and synchronous string/integer results |
| [`async-file.ts`](async-file.ts) | Closed `IAsyncOperation<StorageFile>` and `IAsyncOperation<IRandomAccessStream>` results, `toPromise()`, and deterministic stream/file cleanup |
| [`struct-geopoint.ts`](struct-geopoint.ts) | Named `BasicGeoposition` struct layout, by-value input, and struct output |
| [`array-property-value.ts`](array-property-value.ts) | Int32 pass/receive arrays, empty arrays, signed boundaries, boxing and an explicit `IPropertyValue` cast |

These four examples use system Windows Runtime APIs, not Windows App SDK APIs.
They need Windows 10/11 and native ARM64 or x64 Node.js **24 or newer**, but no
WinApp CLI, package identity, bootstrap DLL, network, picker UI, or special
hardware. The URI example does not make an HTTP request. Geopoint constructs
fixed coordinates; it does not read the device's location.

From the repository root:

```powershell
cd samples\js\low-level
..\prepare-local.ps1
npm install
npm run check
npm run check:contracts
npm test

npm run uri
npm run async-file
npm run struct-geopoint
npm run array-property-value
```

The repository's runtime and codegen are consumed through local `file:`
dependencies, as in the other JavaScript samples. `prepare-local.ps1` builds
those packages; use `-Architecture arm64` or `-Architecture x64` to match Node.
The examples themselves import only `@microsoft/dynwinrt`, without generated
application wrappers. Each runs independently with fixed inputs and assertions.

`async-file.ts` creates its own `dynwinrt-low-level-*` temporary directory,
writes `input.txt`, opens it through WinRT, verifies the stream's byte length,
closes the stream, and removes the directory. It does not accept or modify a
user-supplied file.

Each example calls `roInitialize(1)` explicitly. The small
[`example.ts`](example.ts) helper releases owned values in reverse order,
continues cleanup after an error, and returns a nonzero exit status for either
operation or cleanup failure. Results are printed only after cleanup succeeds.
The JavaScript runtime does not expose a matching `roUninitialize()` API.

### Why the complete interface tables are separate

[`contracts.ts`](contracts.ts) contains the handwritten registrations, so the
four small example files can focus on invocation. Every registered IID has a
complete metadata-ordered method table, including methods the example does not
call. Do not replace unused methods with empty signatures to reach a slot.

The async example's two closed result types must stay distinct: declaring
either output as `Object` loses the async contract required by `toPromise()`.
Conversely, `PropertyValue.CreateInt32Array` really does return `Object` in
metadata; the array example explicitly queries the returned box for
`IPropertyValue`.

### Native-free checks

`npm run check` strictly type-checks the examples (including the picker) and their helpers.
`npm test` runs the local Node test runner with native addon loading blocked,
covering cleanup, error propagation, and process exits. Neither command runs
the examples' native calls or requires Windows App SDK initialization.

`npm run check:contracts` uses the installed Windows SDK 26100
`Windows.winmd` and local dynwinrt-codegen to independently generate reference
declarations into a fresh temporary output directory. It compares **all 10
interface IDs and 129 method signatures**, including parameter directions,
named types, struct fields, enum members, and closed async/array element types.
Only registration declarations are evaluated against a data-only recorder;
no native addon or WinRT call is executed. Temporary projections and their lock
files are created under the OS temporary directory and removed afterward; the
check does not leave a `generated` directory in the sample.

To retain a machine-readable metadata/contract report, explicitly supply an
output path outside the sample:

```powershell
npm run check:contracts -- --evidence "$env:TEMP\dynwinrt-os-contracts.json"
```

The default metadata location is
`C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd`.
Set `DYNWINRT_WINDOWS_WINMD` to check an explicitly selected SDK instead. Missing
metadata, generation warnings, or a contract mismatch fail the check; they
must be investigated rather than bypassed with placeholder signatures.

## Windows App SDK picker

[`picker.ts`](picker.ts) uses the same runtime imports, handwritten
[`contracts.ts`](contracts.ts), and resource/error helper as the OS-only
examples, but calls `Microsoft.Windows.Storage.Pickers.FileOpenPicker`.
It preserves interactive selection and prints the returned `PickFileResult.Path`.

Unlike the four OS-only examples, this one requires the **Windows App SDK 1.8
runtime** and, when running unpackaged, its architecture-matching bootstrap DLL.
It does not require package identity, AI capabilities, or an AI model. It never
installs a runtime, downloads a model, or registers an identity.

For an unpackaged Node process, set the bootstrap path before running. The
following uses the pinned Foundation package already restored in the NuGet
cache; if it is absent, restore/install the matching Windows App SDK through
your normal SDK setup first:

```powershell
# From samples\js\low-level, after the runtime build and npm install above.
$nuget = if ($env:NUGET_PACKAGES) { $env:NUGET_PACKAGES } else { Join-Path $HOME ".nuget\packages" }
$arch = node -p "process.arch"
$env:WINAPPSDK_BOOTSTRAP_DLL_PATH = Join-Path $nuget "microsoft.windowsappsdk.foundation\1.8.251104000\runtimes\win-$arch\native\Microsoft.WindowsAppRuntime.Bootstrap.dll"
npm run picker -- --no-ui
npm run picker
```

The sample explicitly initializes MTA before bootstrap. A full packaged host
instead needs a manifest dependency on `Microsoft.WindowsAppRuntime.1.8`;
it must not call `MddBootstrapInitialize2` unconditionally.
Missing bootstrap setup or native errors are reported with exit code 1.

The picker uses the actual `WindowId` struct (`UInt64 Value`) with zero for an
unowned console dialog. Its complete factory/picker/result tables preserve the
correct IID, getter/setter ordering, enum types, and closed async types.
View mode, start location, and commit-button text are set and read back through
those tables. `--no-ui` stops after this native initialization/property check:
it **does not test interactive selection**.

Normal execution opens the picker. Selecting a file prints its path after
cleanup and exits 0; no file returned (cancelled or unavailable) prints a
distinct message and exits 2. The sample does not open or modify the selected
file's contents.

### Checking the picker contracts

The OS-only `check:contracts` command remains independent of Windows App SDK
metadata. The separate picker command additionally compares the three picker
interfaces (11 methods), for **13 interfaces / 140 methods** in total:

```powershell
npm run check:picker-contracts
```

Its reference inputs are Foundation `1.8.251104000`'s
`Microsoft.Windows.Storage.Pickers.winmd`, InteractiveExperiences
`1.8.251104001`'s `metadata\10.0.18362.0\Microsoft.UI.winmd` (for `WindowId`),
and Windows SDK 26100. The NuGet cache is resolved through `NUGET_PACKAGES` or
`$HOME\.nuget\packages`. `DYNWINRT_PICKER_WINMD` and `DYNWINRT_UI_WINMD` can
select explicit metadata files. Missing inputs fail rather than triggering an
SDK install. This is also a native-free check and accepts `--evidence PATH`.

For the generated-binding version of this picker, see the
[`Windows AI OCR sample`](../ocr/README.md). Its OCR engine has additional
identity, capability, and hardware requirements separate from the picker.
