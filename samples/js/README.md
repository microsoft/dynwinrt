# JavaScript samples

The JavaScript samples use generated bindings and the
`@microsoft/dynwinrt` runtime.

| Sample                                                        | Demonstrates                                                 |
| ------------------------------------------------------------- | ------------------------------------------------------------ |
| [`winui-tic-tac-toe`](winui-tic-tac-toe/)                     | WinUI 3 XAML loading, `projectAs()`, events, and Mica        |
| [`winui-tic-tac-toe-code-only`](winui-tic-tac-toe-code-only/) | Programmatic WinUI 3 controls, collections, events, and Mica |
| [`windows-hello`](windows-hello/)                             | Electron, Windows Hello, and HWND-bound Classic COM interop  |
| [`electron-share-ui`](electron-share-ui/)                     | Electron Share UI with WinRT and Classic COM                 |
| [`electron-smtc`](electron-smtc/)                             | Electron system media controls and GSMTC loopback            |

The WinUI samples use WinApp CLI to restore matching Windows App SDK metadata,
runtime packages, bootstrap binaries, and generated npm bindings.
Their `winapp.yaml` and `winapp.jsBindings` configuration is already checked in,
so run `npm run restore` rather than initializing the sample again.
Run `prepare-local.ps1` first to build the repository's JavaScript runtime and
codegen executable used by both samples.
