# Python samples

All Python samples use generated bindings and the local `dynwinrt` runtime.
Run each sample's `generate.ps1` before `run.ps1`.

| Sample | Demonstrates |
| --- | --- |
| [`async-file-io`](async-file-io/) | Temporary files, storage interfaces, and native `asyncio` |
| [`ocr-image`](ocr-image/) | Self-contained image creation, bitmap decoding, and Windows OCR |
| [`app-lifecycle-single-instance`](app-lifecycle-single-instance/) | Multi-process activation redirection with Windows App SDK AppLifecycle |
| [`cryptography`](cryptography/) | SHA-256, `IBuffer`, text encoding, and hex conversion |
| [`device-watcher`](device-watcher/) | Device enumeration and thread-safe typed event subscriptions |
| [`text-to-speech`](text-to-speech/) | `asyncio`, speech synthesis, media playback, and typed events |
| [`app-notification`](app-notification/) | Windows App SDK bootstrap, notification builder, display, activation, and cleanup |
| [`winui-hello-world`](winui-hello-world/) | Minimal WinUI 3 XAML, controls, events, STA, and projected lifetime management |
| [`custom-winmd-codegen`](custom-winmd-codegen/) | Generate, type, and import a Python package from an external WinMD |
| [`winui-tic-tac-toe`](winui-tic-tac-toe/) | XAML, a Python-defined WinUI control, native layout overrides, and Fluent resources |
| [`winui-tic-tac-toe-code-only`](winui-tic-tac-toe-code-only/) | A larger programmatic WinUI application without XAML |

The WinUI and AppNotification samples require matching Windows App SDK
metadata, reference WinMDs, runtime packages, and bootstrap DLL. The stock
Windows samples auto-detect the newest installed Windows SDK `Windows.winmd`.
