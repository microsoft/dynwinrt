# Windows Hello

This Electron sample uses:

- the WinRT `Windows.Security.Credentials.UI.UserConsentVerifier` runtime class
  to check availability; and
- Classic COM `IUserConsentVerifierInterop` to associate the verification
  dialog with the Electron HWND.

It demonstrates the same WinRT API shape commonly wrapped by a custom Electron
native addon, without requiring sample-specific C++ or `node-gyp`.

## Prerequisites

- Windows 10 or 11;
- Windows Hello configured for the current user;
- Node.js and Rust/Cargo;
- a Windows SDK containing `Windows.winmd`; and
- `Microsoft.Windows.SDK.Win32Metadata` containing `Windows.Win32.winmd`
  for the Classic COM interop projection.

## Run

Build the local JavaScript runtime once from the repository root:

```powershell
cd bindings\js
npm install
npm run build
```

Generate the WinRT and COM projections and run the sample:

```powershell
cd samples\js\windows-hello
npm install
npm run generate
npm start
```

Set `DYNWINRT_WIN32_WINMD` to the metadata file when it is outside the default
`$HOME\.nuget\packages` cache.

Click **Verify identity**. Windows displays its native verification dialog
owned by the Electron window. Complete or cancel the prompt; the result is
shown in the application.

To check availability without displaying the verification dialog:

```powershell
npm run check
```
