# Python-defined standalone WinRT interfaces

This sample implements IBackgroundTask, IStringable, and IClosable on one
standalone native object. Another implemented object supplies a complete
IBackgroundTaskInstance fixture. The generated `task.run(instance)` call
crosses a real native WinRT vtable before the Python handler runs; its progress
property reads/writes make additional native calls into the fixture.

No composable WinUI class, package identity, OS task registration, COM server,
or trigger is needed. Fixture methods that would need OS registration,
cancellation, or deferrals explicitly raise NotImplementedError.

Build/install the matching Python runtime from `bindings\py` and build the
generator, then run from the repository root:

```powershell
cargo build -p dynwinrt-codegen
.\samples\python\interface-implementation\generate.ps1 `
  -Codegen .\target\debug\dynwinrt-codegen.exe
.\samples\python\interface-implementation\run.ps1 `
  -Python .\bindings\py\.venv\Scripts\python.exe
```

The selected interpreter must have this checkout's runtime installed.
Generation auto-detects the Windows SDK; `-Winmd` selects an explicit metadata
file. The generated package is imported from the sample directory and does
not need to be installed separately.

Releasing the implementation owner's reference leaves independently retained
typed views usable. Explicit disposal in `finally` disconnects every view's
future callbacks. Already-entered callbacks may finish. The projected lifetime
scope releases typed wrappers before the apartment exits. The implementation
owner's own context-manager exit, when used, performs **dispose**, not release.
The public `dynwinrt.release_projected(view)` helper independently releases a
typed view without disconnecting its controller or other views.

Do not use `async def` for these synchronous handlers. Native calls from
another thread fail before entering Python. See
[the public guide](../../../docs/guides/windows/winrt-interface-implementations.md)
for the capability matrix, callback errors, native retention, and cycle/shutdown
semantics.
