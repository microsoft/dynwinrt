# Electron System Media Controls

This sample uses dynwinrt to exercise both Windows media-control surfaces from
one Electron process:

- **SMTC** (`Windows.Media.SystemMediaTransportControls`) publishes the Electron
  window as a media source for hardware keys and the Windows media UI.
- **GSMTC** (`Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager`)
  discovers that session and controls it as another media controller would.

The sample covers:

- acquiring SMTC for an Electron `BrowserWindow` HWND through
  `ISystemMediaTransportControlsInterop`;
- metadata and `SystemMediaTransportControlsDisplayUpdater`;
- playback state and timeline updates;
- play, pause, next, previous, seek, playback-rate, shuffle, and repeat requests;
- SMTC and GSMTC event subscriptions; and
- media-property, playback-info, and timeline round trips.

## Prerequisites

- Windows 10 version 2004 or later;
- Node.js 22.12 or later and Rust/Cargo;
- a Windows SDK containing `Windows.winmd`;
- `Microsoft.Windows.SDK.Win32Metadata` in the NuGet global cache (or set
  `DYNWINRT_WIN32_WINMD`); and
- Developer Mode, because the sample registers a sparse debug identity with
  the restricted `globalMediaControl` capability.

## Run

Build the local JavaScript runtime once from the repository root:

```powershell
cd bindings\js
npm install
npm run build
```

Then install, generate, and start the sample:

```powershell
cd samples\js\electron-smtc
npm install
npm run generate
npm start
```

To launch the UI and immediately populate it with a complete successful
loopback run:

```powershell
npm run demo
```

## Automated loopback validation

```powershell
npm run check
```

The check registers the Electron debug identity, runs a hidden Electron window,
publishes an SMTC session, discovers it through GSMTC, issues real transport
requests, verifies the resulting callbacks and state, prints the result as
JSON, and exits nonzero on any failure.
