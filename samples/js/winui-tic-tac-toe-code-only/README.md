# JavaScript WinUI Tic-Tac-Toe (code-only)

A playable 3x3 Tic-Tac-Toe sample built entirely in JavaScript with generated
WinUI 3 projections. It constructs every control programmatically, uses the
default Fluent resources, and creates `MicaBackdrop` through its generated
composable constructor.

## Prerequisites

- Windows 11 with Node.js 20 or newer.
- WinApp CLI 0.6.2 or newer.

Restore the pinned Windows App SDK, generate the npm bindings, and run:

```powershell
cd samples\js\winui-tic-tac-toe-code-only
..\prepare-local.ps1
npm install
npm run restore
npm start
```

`prepare-local.ps1` builds the JavaScript runtime and Rust code generator, then
places the codegen executable in its local npm package. The sample consumes both
packages through `file:` dependencies. WinApp CLI can warn that the local
runtime's `file:` specifier cannot be compared with the codegen package version;
this is expected for a source-checkout build.

`npm run restore` writes SDK artifacts under `.winapp\`, copies the
architecture-specific bootstrap DLL to `.winapp\bin`, and generates bindings
under `.winapp\bindings`. `app.mjs` selects the bootstrap DLL for the current
Node architecture before calling `initWinappsdk(2, 3)`.

Closing the window exits the WinUI application and releases the projected
objects and event subscriptions.
