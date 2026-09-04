# Python AppNotification

This sample uses Windows App SDK `AppNotificationBuilder` and
`AppNotificationManager` to build and display a notification from an unpackaged
Python process. It demonstrates:

- Windows App SDK bootstrap;
- a fluent generated builder API;
- an activation event delivered to Python; and
- asynchronous cleanup of the notification.

Unlike PyWinRT's
[toast activation sample](https://github.com/pywinrt/pywinrt/blob/main/samples/toast_activation.py),
this version uses the Windows App SDK registration API and does not require a
custom `comtypes` local server or manual registry keys.

## Prerequisites

- WinApp CLI 1.0 or newer;
- `dynwinrt` installed in the selected Python interpreter; and
- `dynwinrt-codegen` on `PATH`, or passed with `-Codegen`.

## Run

```powershell
winapp restore
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe

.\run.ps1 -Python C:\path\to\python.exe -Major 2 -Minor 3
```

`winapp restore` honors the standard NuGet configuration and prepares the
pinned SDK metadata and bootstrap binaries under `.winapp\`. `generate.ps1`
uses that metadata to generate Python bindings and copies the selected
architecture's bootstrap DLL to `.runtime\`.

`Major` and `Minor` default to `2` and `3`. They must exactly match the Windows
App SDK product version represented by the metadata, bootstrap DLL, and
installed runtime.

The sample subscribes before registration, displays the notification, waits
for activation, removes the notification, and clears all sample registration
data before exiting. Registration cleanup still runs if notification removal
fails.

Use `-Smoke` to validate support and build the notification payload without
registering or displaying it.
