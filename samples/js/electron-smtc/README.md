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
- a three-track playlist with generated artwork, rich music metadata, and
  `SystemMediaTransportControlsDisplayUpdater`;
- a live playback clock with track transitions, seek, rate, shuffle, and repeat;
- play, pause, toggle, stop, next, previous, record, fast-forward, rewind, and
  channel requests;
- SMTC sound-level, transport, and request events;
- GSMTC manager, media-property, playback-info, and timeline events;
- global session discovery and current-session highlighting; and
- the complete GSMTC playback capability matrix.

The UI presents the SMTC publisher and GSMTC controller side by side so each
request and resulting state change is visible. A small Web Audio synthesizer
generates original notes locally and follows the published playback state,
track, position, and rate; no media files or network downloads are used. The
window uses native Windows 11 Mica plus the system light/dark theme, accent
color, and Fluent-style controls.

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

Use the GSMTC panel to control the published session, or use Windows hardware
media keys and the system media UI. The session list also shows other media
applications visible to GSMTC.

To launch the UI and immediately populate it with a successful loopback run:

```powershell
npm run demo
```

## Automated loopback validation

```powershell
npm run check
```

The check registers the Electron debug identity, runs a hidden Electron window,
publishes an SMTC session, discovers it through GSMTC, issues every supported
transport request, verifies artwork and metadata, exercises the live timeline
and playlist, validates the capability matrix and event round trips, prints the
result as JSON, and exits nonzero on any failure. Synthesized audio is disabled
in this validation mode.
