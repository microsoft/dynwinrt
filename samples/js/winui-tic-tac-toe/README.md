# JavaScript WinUI Tic-Tac-Toe

A playable Mica/Fluent 3x3 Tic-Tac-Toe sample using generated WinUI 3
projections. It loads the visual tree with `XamlReader`, converts raw XAML
objects with `projectAs()`, and uses generated events, window APIs, and the
composable `MicaBackdrop` constructor.

## Prerequisites

- Windows 11 on x64 with Node.js 18 or newer.
- An unpackaged x64 WinAppSDK 2.3 fixture: `Microsoft.UI.Xaml.winmd`, a
  newline-separated reference-WinMD list, and the matching x64
  `Microsoft.WindowsAppRuntime.Bootstrap.dll`.
- Matching local `dynwinrt` and `dynwinrt-codegen` builds.

Build the JavaScript runtime from the repository root:

```powershell
cd bindings\js
npm install
npm run build
```

Generate and run the sample:

```powershell
cd samples\js\winui-tic-tac-toe
npm install
.\generate.ps1 `
  -WinuiWinmd C:\fixtures\winappsdk\metadata\Microsoft.UI.Xaml.winmd `
  -RefList C:\fixtures\winappsdk\winmd-reference-list.txt `
  -BootstrapDll C:\fixtures\winappsdk\x64\Microsoft.WindowsAppRuntime.Bootstrap.dll `
  -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
npm start
```

Closing the window exits the WinUI application and releases the projected
objects and event subscriptions.
