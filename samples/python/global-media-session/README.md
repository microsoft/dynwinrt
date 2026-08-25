# Python global media sessions

This sample lists the media sessions exposed through
`GlobalSystemMediaTransportControlsSessionManager` (GSMTC). It is a compact
Python counterpart to the repository's Electron SMTC/GSMTC sample and
demonstrates:

- a WinRT async factory;
- projected collections and enums;
- media properties, playback state, and timeline values; and
- typed session-change subscriptions.

## Prerequisites

- Windows 10 version 2004 or later;
- Developer Mode, required to register the sparse debug identity;
- a Windows SDK containing `Windows.winmd`;
- `dynwinrt` installed in the selected Python interpreter; and
- `dynwinrt-codegen` on `PATH`, or passed with `-Codegen`.

GSMTC requires the restricted `globalMediaControl` capability. The sample's
`setup-identity.ps1` creates a local sparse package identity for the selected
Python executable. It removes and replaces only the
`dynwinrt-python-gsmtc-sample` registration.

## Run

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe
```

The command prints JSON for every visible media session. Start music or video
playback in another application if the list is empty.

Watch for session changes:

```powershell
.\run.ps1 -Python C:\path\to\python.exe -Watch -SkipIdentity
```

`-SkipIdentity` is useful after the identity has already been registered for
that Python interpreter.

Remove the sample identity when finished:

```powershell
.\remove-identity.ps1
```
