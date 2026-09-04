# JavaScript WinUI Tic-Tac-Toe

A playable Mica/Fluent 3x3 Tic-Tac-Toe sample using generated WinUI 3
projections. It loads the visual tree with `XamlReader`, converts raw XAML
objects with `projectAs()`, and uses generated events, window APIs, and the
composable `MicaBackdrop` constructor.

## Prerequisites

- Windows 11 with Node.js 20 or newer.
- WinApp CLI 1.0 or newer.

Restore the pinned Windows App SDK, generate the npm bindings, and run:

```powershell
cd samples\js\winui-tic-tac-toe
npm install
npm run restore
npm start
```

`npm run restore` honors the standard NuGet configuration, writes SDK artifacts
under `.winapp\`, copies the architecture-specific bootstrap DLL to
`.winapp\bin`, and generates bindings under `.winapp\bindings`. `app.mjs`
selects the bootstrap DLL for the current Node architecture before calling
`initWinappsdk(2, 3)`.

Closing the window exits the WinUI application and releases the projected
objects and event subscriptions.
