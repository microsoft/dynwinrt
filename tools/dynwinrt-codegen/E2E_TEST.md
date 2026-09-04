# End-to-End Test: FileOpenPicker

This guide walks through a full end-to-end test of the dynwinrt-codegen
pipeline, from cloning the repo to opening a file picker dialog. Codegen emits
CommonJS implementation modules (`.js`), ESM facades (`.mjs`), and ambient
TypeScript declarations (`.d.ts`).

## Prerequisites

- Windows 10/11 with Windows SDK installed
- WinAppSDK 1.8 runtime installed (for `Microsoft.Windows.Storage.Pickers`)
- Rust toolchain (stable)
- Node.js 18+
- Environment variable `WINAPPSDK_BOOTSTRAP_DLL_PATH` set to the WinAppSDK Bootstrap DLL path

**Finding the WinAppSDK winmd:**

The WinAppSDK `.winmd` files are typically found under a NuGet restore or a `winapp` cache:

```
~/.nuget/packages/microsoft.windowsappsdk.foundation/<version>/metadata/
~/.winapp/packages/Microsoft.WindowsAppSDK.<Component>.<version>/metadata/
```

## Step 1: Build everything

```bash
cd dynwinrt

# Build the codegen tool
cargo build -p dynwinrt-codegen --release

# Build the JS native binding
cd bindings/js
npm install
npm run build
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
  },
  "devDependencies": {
    "tsx": "^4.21.0"
  }
}
```

```bash
npm install
```

## Step 3: Generate bindings

Windows SDK `Windows.winmd` is auto-detected from `C:\Program Files (x86)\Windows Kits\10\UnionMetadata\`. Only the target WinAppSDK winmd needs to be specified. `--lang js` is the default and emits CommonJS `.js`, ESM `.mjs` facades, and `.d.ts`:

```bash
cd ../../dynwinrt

cargo run -p dynwinrt-codegen --release -- generate \
  --winmd "<path-to>/Microsoft.Windows.Storage.Pickers.winmd" \
  --namespace "Microsoft.Windows.Storage.Pickers" \
  --class-name "FileOpenPicker" \
  --output ../test-winmd/test-picker/generated
```

Each generated symbol produces both a CommonJS `.js` module and a matching
`.d.ts` (for example, `FileOpenPicker.js` + `FileOpenPicker.d.ts`). The package
also contains CommonJS `index.js`, ESM `index.mjs`, and `index.d.ts` facades.

Parameterized collection interfaces (e.g. `IVector<String>`) are automatically instantiated from `Windows.winmd` as concrete types like `IVector_String.js` / `.d.ts`.

## Step 4: Write test script

Create `test_picker.ts` in the test project. `tsx` reads the adjacent
declarations and interoperates with the generated CommonJS modules:

```typescript
import { initWinappsdk, roInitialize, DynWinRtValue } from '@microsoft/dynwinrt'
import { FileOpenPicker } from './generated/microsoft/windows/storage/pickers/FileOpenPicker.js'
import { PickerViewMode } from './generated/microsoft/windows/storage/pickers/PickerViewMode.js'

async function main() {
    initWinappsdk(1, 8)
    roInitialize(1)

    // Create picker (hwnd=0 for console app)
    const picker = new FileOpenPicker(DynWinRtValue.i64(0))
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
    if (result) {
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
| **dynwinrt (Rust)** | Dynamic COM vtable dispatch, parameterized type outputs, and `IAsyncOperation` |
| **@microsoft/dynwinrt (napi)** | JS-to-Rust bridge: `invoke()`, `toPromise()`, `toNumber()`, `toString()`, type marshalling |
| **WinAppSDK runtime** | Bootstrap initialization, FileOpenPicker activation factory, IFileOpenPickerFactory.CreateInstance |
| **Collection types** | `IVector_String.append()`, `.size`, `.getAt()` — from winmd parameterized interface instantiation, not hardcoded |

## Automated E2E suite

The repository's canonical cross-language specs are in
[`tests/e2e/e2e_specs.json`](../../tests/e2e/e2e_specs.json). The orchestrator
generates temporary bindings and runs the JavaScript/TypeScript, Python, or
Classic COM runners. From the repository root:

```powershell
.\tests\e2e\e2e_test.ps1 -Lang ts
.\tests\e2e\e2e_test.ps1 -Lang py
.\tests\e2e\e2e_test.ps1 -Lang com
```

The Classic COM suite also requires `DYNWINRT_WIN32_WINMD` or an installed
`Microsoft.Windows.SDK.Win32Metadata` package.
