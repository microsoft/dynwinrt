# End-to-End Test: FileOpenPicker

This guide walks through a full end-to-end test of the dynwinrt-codegen pipeline, from cloning the repo to opening a file picker dialog in TypeScript.

## Prerequisites

- Windows 10/11 with Windows SDK installed
- WinAppSDK 1.8 runtime installed (for `Microsoft.Windows.Storage.Pickers`)
- Rust toolchain (stable)
- Node.js 18+
- Environment variable `WINAPPSDK_BOOTSTRAP_DLL_PATH` set to the WinAppSDK Bootstrap DLL path

**Finding the WinAppSDK winmd:**

The WinAppSDK `.winmd` files are typically found at:
```
~/.nuget/packages/microsoft.windowsappsdk.foundation/1.8.*/metadata/
```
or if you have lazy-winrt checked out:
```
lazy-winrt/packages/Microsoft.WindowsAppSDK.Foundation.1.8.*/metadata/
```

## Step 1: Build everything

```bash
cd dynwinrt

# Build the codegen tool
cargo build -p dynwinrt-codegen --release

# Build the JS native binding
cd bindings/js
npx napi build --no-const-enum --platform --release -o dist
cd ../..
```

## Step 2: Create test project

```bash
mkdir -p ../test-winmd/test-picker
cd ../test-winmd/test-picker
```

Create `package.json`:
```json
{
  "private": true,
  "dependencies": {
    "@microsoft/dynwinrt": "file:../../dynwinrt/bindings/js"
  }
}
```

```bash
npm install
```

## Step 3: Generate TypeScript bindings

Windows SDK `Windows.winmd` is auto-detected from `C:\Program Files (x86)\Windows Kits\10\UnionMetadata\`. Only the target WinAppSDK winmd needs to be specified:

```bash
cd ../../dynwinrt

cargo run -p dynwinrt-codegen --release -- generate \
  --winmd "<path-to>/Microsoft.Windows.Storage.Pickers.winmd" \
  --namespace "Microsoft.Windows.Storage.Pickers" \
  --class-name "FileOpenPicker" \
  --lang ts \
  --output ../test-winmd/test-picker/generated
```

Expected output:
```
Auto-detected Windows SDK: C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd
Generated .../IVector_String.ts
Generated .../IVectorView_PickFileResult.ts
Generated .../IVectorView_String.ts
Generated .../PickerLocationId.ts
Generated .../PickerViewMode.ts
Generated .../FileOpenPicker.ts
Generated .../PickFileResult.ts
Generated .../_collections.ts
Generated .../index.ts
Done. 2 class(es) + 3 interface(s) + 2 enum(s) generated
```

Parameterized collection interfaces (e.g. `IVector<String>`) are automatically instantiated from `Windows.winmd` as concrete types like `IVector_String.ts`.

## Step 4: Write test script

Create `test_picker.ts` in the test project:

```typescript
import { initWinappsdk, DynWinRtValue } from '@microsoft/dynwinrt'
import { FileOpenPicker } from './generated/FileOpenPicker'
import { PickerViewMode } from './generated/PickerViewMode'

async function main() {
    initWinappsdk(1, 8)

    // Create picker (hwnd=0 for console app)
    const picker = FileOpenPicker.createInstance(DynWinRtValue.i64(0))
    console.log('FileOpenPicker created')

    // Set properties
    picker.viewMode = PickerViewMode.Thumbnail
    console.log('ViewMode:', picker.viewMode)

    picker.commitButtonText = 'Select File'
    console.log('CommitButtonText:', picker.commitButtonText)

    // Add file type filters — fully typed, no DynWinRtValue wrapping needed
    const filter = picker.fileTypeFilter
    filter.append('.png')
    filter.append('.jpg')
    filter.append('.txt')
    console.log('FileTypeFilter size:', filter.size)
    console.log('Filter[0]:', filter.getAt(0))
    console.log('Filter[1]:', filter.getAt(1))
    console.log('Filter[2]:', filter.getAt(2))

    // Open file picker dialog
    console.log('Opening file picker dialog...')
    const result = await picker.pickSingleFileAsync()
    if (result && result._obj) {
        console.log('Selected file path:', result.path)
    } else {
        console.log('User cancelled the picker')
    }

    console.log('ALL PASS')
}

main().catch(e => console.error('Error:', e))
```

## Step 5: Run

```bash
cd ../test-winmd/test-picker
npx tsx test_picker.ts
```

Expected output:
```
FileOpenPicker created
ViewMode: 1
CommitButtonText: Select File
FileTypeFilter size: 3
Filter[0]: .png
Filter[1]: .jpg
Filter[2]: .txt
Opening file picker dialog...
Selected file path: C:\Users\...\some_file.png
ALL PASS
```

A file picker dialog will open. Select a file (filtered to .png/.jpg/.txt) to complete the test.

## What this tests

| Layer | What's verified |
|---|---|
| **dynwinrt-codegen** | Generates correct interface registrations, method signatures, factory methods, enum values from `.winmd` metadata |
| **dynwinrt-codegen (generics)** | Parameterized interfaces (IVector\<String\>, IVectorView\<PickFileResult\>) instantiated from `Windows.winmd` with concrete types, auto-detected Windows SDK path |
| **dynwinrt (Rust)** | Dynamic COM vtable dispatch, parameterized type out-params, async operation (IAsyncOperation), RawPtr out-buffer for COM pointers |
| **@microsoft/dynwinrt (napi)** | JS-to-Rust bridge: `invoke()`, `toPromise()`, `toNumber()`, `toString()`, type marshalling |
| **WinAppSDK runtime** | Bootstrap initialization, FileOpenPicker activation factory, IFileOpenPickerFactory.CreateInstance |
| **Collection types** | `IVector_String.append()`, `.size`, `.getAt()` — from winmd parameterized interface instantiation, not hardcoded |

## Additional E2E tests

| Test | Namespace | What it covers |
|---|---|---|
| **test-http** | `Windows.Foundation` + `Windows.Web.Http` | Uri properties, HttpClient.getStringAsync (async with progress), response status code, content.readAsStringAsync |
| **test-geo** | `Windows.Devices.Geolocation` | Struct pass-by-value (BasicGeoposition → Geopoint.create), struct out-param (Geopoint.position) |


