# Python WinUI 3 Hello World

This is the smallest WinUI 3 sample in the repository. It complements the
Tic-Tac-Toe samples by showing only the setup required to:

- bootstrap the Windows App SDK;
- enter a single-threaded WinRT apartment;
- load a small XAML fragment;
- project raw XAML objects with `project_as()`;
- pass a generated `StackPanel` directly to the base `UIElement` API;
- create and activate a `Window`;
- subscribe to `Button.Click`; and
- release projected objects before leaving the apartment.

The flow is intentionally comparable to PyWinRT's
[WinUI 3 Hello World](https://github.com/pywinrt/pywinrt/blob/main/samples/winui3/hello_app.py),
while using dynwinrt's generated bootstrap and lifetime APIs.

## Prerequisites

The inputs are the same as the other Python WinUI samples:

- `Microsoft.UI.Xaml.winmd`;
- a newline-separated reference-WinMD list;
- the matching architecture's
  `Microsoft.WindowsAppRuntime.Bootstrap.dll`;
- `dynwinrt` installed in the selected Python interpreter; and
- `dynwinrt-codegen` on `PATH`, or passed with `-Codegen`.

## Run

```powershell
.\generate.ps1 `
  -WinuiWinmd C:\fixtures\winappsdk\metadata\Microsoft.UI.Xaml.winmd `
  -RefList C:\fixtures\winappsdk\winmd-reference-list.txt `
  -BootstrapDll C:\fixtures\winappsdk\x64\Microsoft.WindowsAppRuntime.Bootstrap.dll `
  -Codegen ..\..\..\target\release\dynwinrt-codegen.exe

.\run.ps1 -Python C:\path\to\python.exe -Major 2 -Minor 3
```

`Major` and `Minor` default to `2` and `3`. They must exactly match the Windows
App SDK product version represented by the metadata, bootstrap DLL, and
installed runtime.

Use `-Smoke` to create the window and controls, update the text once, print
`python-winui-hello-ok`, and exit automatically.
