# Electron Share UI

This sample opens the Windows Share UI for an Electron `BrowserWindow`. It uses:

- the standard `electron-vite` main/preload/renderer TypeScript project structure;
- Classic COM `IDataTransferManagerInterop` to associate sharing with the Electron HWND;
- `projectAs(raw, DataTransferManager)` to project the returned WinRT object; and
- generated `DataRequested`, `DataPackage.properties.title`, and `DataPackage.setText()` APIs.

The sample explicitly releases every owned native value. The raw COM result is released immediately after
`projectAs`, temporary event objects are released after the request is populated, and the event subscription,
projected manager, and interop object are released when the window closes.

## Run

Build the local JavaScript binding once from the repository root:

```powershell
cd bindings\js
npm install
npm run build
```

Then generate the bindings and start Electron:

```powershell
cd samples\js\electron-share-ui
npm install
npm run generate
npm run dev
```

Classic COM generation needs `Windows.Win32.winmd`. The script uses
`$env:DYNWINRT_WIN32_WINMD` when set, otherwise it searches the local
`Microsoft.Windows.SDK.Win32Metadata` NuGet package cache.
