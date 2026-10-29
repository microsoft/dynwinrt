# Generate Python bindings from a custom WinMD

This sample is a reusable template for metadata shipped outside the Windows
SDK, including a private component, a vendor SDK, or an individual Windows App
SDK package.

It demonstrates:

- passing an explicit WinMD and reference set to `dynwinrt-codegen`;
- generating a typed Python package with `.pyi` files and `py.typed`;
- importing every generated module; and
- inspecting public generated types and documentation.

## Generate

```powershell
.\generate.ps1 `
  -Winmd C:\path\to\Contoso.Component.winmd `
  -Namespace Contoso.Component `
  -RefList C:\path\to\references.txt `
  -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
```

Limit generation to selected runtime classes when the namespace is large:

```powershell
.\generate.ps1 `
  -Winmd C:\path\to\Contoso.Component.winmd `
  -Namespace Contoso.Component `
  -ClassName "Widget,WidgetOptions" `
  -RefList C:\path\to\references.txt
```

Each non-empty line in `references.txt` is an additional WinMD used for type
resolution but not emitted.

## Inspect

```powershell
.\run.ps1 -Python C:\path\to\python.exe
.\run.ps1 -Python C:\path\to\python.exe -Symbol Widget
```

The default command imports every generated module and prints the first public
symbols. Activation and method calls require the component that implements the
WinMD to be installed, registered, or bootstrapped according to that
component's deployment model.

After generation, install the generated project into the same interpreter used
to run the sample: `python -m pip install -e .\generated` (use that interpreter's
full path when needed). The import remains `generated`; sources now live in
`generated\src\generated`, not directly in `generated`. When upgrading from
preview.23 or earlier, regenerate the complete output and reinstall it.
