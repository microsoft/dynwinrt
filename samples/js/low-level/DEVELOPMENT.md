# Developing the low-level examples

For running the examples, see the [README](README.md). The checks below are
maintenance tools; they are not prerequisites for calling the low-level API.

## Typechecking and tests

Install development dependencies if you previously used `--omit=dev`:

```powershell
npm install
npm run check
npm test
```

`check` type-checks all five examples and their helpers. The tests cover
cleanup, initialization decisions, cancellation results, and process exits
with native addon loading blocked. They do not open the picker or call WinRT.
Use the individual example commands for native execution.

## Handwritten contracts

[`contracts.ts`](contracts.ts) declares each interface in metadata order,
including methods that the examples do not call. Preserve the full signatures;
do not insert empty methods just to reach a particular vtable slot.

Async results must retain their closed type, such as
`IAsyncOperation<StorageFile>` or `IAsyncOperation<IRandomAccessStream>`.
Declaring them as `Object` loses the contract needed by `toPromise()`.
`PropertyValue.CreateInt32Array`, however, really does return `Object`; that
example queries the returned box for `IPropertyValue`.

The common [`example.ts`](example.ts) helper releases owned values in reverse
order, attempts the remaining cleanup after an error, and prints the result
only after cleanup succeeds.

## Optional metadata checks

These checks use `@microsoft/dynwinrt-codegen` to generate reference declarations
for comparison. The generated wrappers are not used by the examples.

From this directory, prepare the local codegen package with the repository
helper, which builds both the runtime and generator:

```powershell
..\prepare-local.ps1
npm install
npm run check:contracts
```

The OS-only check compares 10 interfaces and 129 method signatures against
Windows SDK 26100. It does not require Windows App SDK metadata.
The default input is
`C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd`;
`DYNWINRT_WINDOWS_WINMD` can select another explicit file.

### Picker metadata

The separate picker check includes the OS contracts and the three picker
interfaces, for 13 interfaces and 140 methods:

```powershell
npm run check:picker-contracts
```

It additionally requires these restored SDK inputs:

| Package | Version | Metadata |
| --- | --- | --- |
| Microsoft.WindowsAppSDK.Foundation | 1.8.251104000 | `metadata\Microsoft.Windows.Storage.Pickers.winmd` |
| Microsoft.WindowsAppSDK.InteractiveExperiences | 1.8.251104001 | `metadata\10.0.18362.0\Microsoft.UI.winmd` |

The second file supplies the `WindowId` struct. The NuGet cache is resolved
through `NUGET_PACKAGES` or `$HOME\.nuget\packages`; `DYNWINRT_PICKER_WINMD` and
`DYNWINRT_UI_WINMD` can select explicit files. Missing metadata fails the check
rather than installing an SDK.

Both checks compare IDs, method order, parameter directions, and type shapes
without loading the native addon. Temporary projections and lock files are
removed afterward. To retain a report, explicitly choose an output path:

```powershell
npm run check:picker-contracts -- --evidence "$env:TEMP\dynwinrt-picker-contracts.json"
```

## Picker hosts

An unpackaged process initializes MTA and bootstraps Windows App SDK 1.8.
A full packaged host instead needs a manifest dependency on
`Microsoft.WindowsAppRuntime.1.8`; the sample skips bootstrap in that case.
The runtime does not expose a matching JavaScript `roUninitialize()` API.
