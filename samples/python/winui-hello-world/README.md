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

Use `-Smoke` to create the window and controls, update the text once, print
`python-winui-hello-ok`, and exit automatically.
