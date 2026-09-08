# JavaScript-defined standalone WinRT interfaces

This sample implements IBackgroundTask, IStringable, and IClosable on one
standalone native object. A second implemented object supplies a controlled
IBackgroundTaskInstance. Calling the generated `task.run(instance)` enters the
native WinRT vtable, invokes the JavaScript handler, and makes nested native
property calls on the task instance.

There is no WinUI base, package identity, OS task registration, COM server, or
background activation. Unused task-instance fixture methods throw rather than
pretending that an OS registration or deferral exists.

From the repository root, build the matching runtime and generator:

```powershell
cargo build -p dynwinrt-codegen
Push-Location .\bindings\js
npm ci
npm run build
Pop-Location

Push-Location .\samples\js\interface-implementation
npm install --ignore-scripts --omit=optional
.\generate.ps1 -Codegen ..\..\..\target\debug\dynwinrt-codegen.exe
npm start
Pop-Location
```

The local npm dependency uses the runtime built in this checkout. Generation
auto-detects the Windows SDK; pass `-Winmd` to select another Windows.winmd.

The owner and typed views have independent references. `owner.release()`
releases only the creator's reference, so the retained views continue to work.
`owner.dispose()` disconnects all future callbacks; the sample confirms a
later native call fails and prints its contextual diagnostic. An already
entered callback is allowed to finish. IClosable.Close is just another
handler-defined method, not a hidden alias for owner disposal. The generated
package's `releaseProjected(view)` independently releases each typed view.

See [the public guide](../../../docs/guides/windows/winrt-interface-implementations.md)
for the supported contracts and owner-thread/lifetime restrictions.
