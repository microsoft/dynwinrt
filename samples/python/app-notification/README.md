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

- `Microsoft.Windows.AppNotifications.winmd`;
- `Microsoft.Windows.AppNotifications.Builder.winmd`;
- a newline-separated reference-WinMD list for the matching Windows App SDK;
- the matching architecture's bootstrap DLL;
- `dynwinrt` installed in the selected Python interpreter; and
- `dynwinrt-codegen` on `PATH`, or passed with `-Codegen`.

## Run

```powershell
.\generate.ps1 `
  -AppNotificationsWinmd C:\fixtures\Microsoft.Windows.AppNotifications.winmd `
  -BuilderWinmd C:\fixtures\Microsoft.Windows.AppNotifications.Builder.winmd `
  -RefList C:\fixtures\winmd-reference-list.txt `
  -BootstrapDll C:\fixtures\x64\Microsoft.WindowsAppRuntime.Bootstrap.dll `
  -Codegen ..\..\..\target\release\dynwinrt-codegen.exe

.\run.ps1 -Python C:\path\to\python.exe
```

The sample subscribes before registration, displays the notification, waits
for activation, removes the notification, and unregisters before exiting.

Use `-Smoke` to validate support and build the notification payload without
registering or displaying it.
