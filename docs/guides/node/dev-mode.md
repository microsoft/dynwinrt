# Plain Node.js dev mode

This guide walks through running the dynamic WinRT projection ([`@microsoft/dynwinrt`](https://www.npmjs.com/package/@microsoft/dynwinrt)) from a **plain Node.js process** — no Electron, no MSIX packaging step — while still getting the Windows features that require **package identity** (native notifications, Phi Silica, Store-only APIs).

The [WinApp CLI](https://github.com/microsoft/winappCli) handles all the packaging plumbing for you. This guide covers the flavors of the dev-mode flow, from the simplest one-shot command to a persistent alias you can invoke from any terminal.

- [When you need this](#when-you-need-this)
- [Prerequisites](#prerequisites)
- [Path A — execution alias (recommended for iteration)](#path-a--execution-alias-recommended-for-iteration)
- [Path B — one-shot `winapp run` (recommended for scripts and CI)](#path-b--one-shot-winapp-run-recommended-for-scripts-and-ci)
- [Path C — bare `winapp run` without a copied Node](#path-c--bare-winapp-run-without-a-copied-node)
- [Why copy `node.exe` into the project?](#why-copy-nodeexe-into-the-project)
- [Cleaning up: `winapp unregister`](#cleaning-up-winapp-unregister)
- [From dev mode to distribution](#from-dev-mode-to-distribution)
- [Troubleshooting](#troubleshooting)

## When you need this

- You want to call a WinRT API from Node.js on your own machine, without shipping an MSIX
- Some Windows features you want to use require **package identity** (notifications, Phi Silica, share target, background tasks, most Store-integrated APIs)
- Running `node app.js` from a normal terminal fails with errors like `E_APPMODEL_ERROR_NO_PACKAGE` or `hasPackageIdentity() === false`

Running `node.exe` from a normal terminal has no package identity by default. Windows attaches package identity only to processes that were **launched through a registered package**. This guide sets up a small loose-layout package around your Node.js script so Windows treats it as a first-class app during development.

## Prerequisites

- Windows 11 with **Developer Mode** enabled (Settings → System → For developers)
- [Node.js 20+](https://nodejs.org/) on your `PATH`
- A fresh empty folder for the experiment

Install the CLI and initialize the project:

```powershell
mkdir my-winrt-experiment; cd my-winrt-experiment
npm init -y
npm install --save-dev @microsoft/winappcli
npx winapp init . --use-defaults --add-js-bindings
```

`winapp init` writes a `Package.appxmanifest`, adds `@microsoft/dynwinrt` + `@microsoft/dynwinrt-codegen` to `package.json`, and generates typed bindings under `.winapp/bindings/`.

If you plan to call a **restricted-capability** API (for example Phi Silica), edit `Package.appxmanifest` and add the corresponding capability:

```xml
<Capabilities>
  <rescap:Capability Name="systemAIModels" />
</Capabilities>
```

Now pick one of the three paths below depending on your workflow.

## Path A — execution alias (recommended for iteration)

Best when you want to iterate quickly on the same experiment: register **once**, then invoke your script many times from **any terminal** through a personal command name like `mynode.exe`.

### 1. Copy Node into the project

```powershell
mkdir .local-node
copy (Get-Command node).Source .\.local-node\node.exe
```

See [Why copy `node.exe` into the project?](#why-copy-nodeexe-into-the-project) below for the rationale.

### 2. Add an execution alias

```powershell
npx winapp manifest add-alias --name mynode.exe --manifest .\Package.appxmanifest
```

This inserts an [`<uap3:Extension Category="windows.appExecutionAlias">`](https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-uap3-appexecutionalias) into the manifest, pointing at `.local-node\node.exe`.

### 3. Register the loose-layout package

```powershell
npx winapp run . --exe .local-node\node.exe --no-launch
```

`--no-launch` registers the package (so the alias becomes resolvable) without starting anything yet. After this step, `mynode.exe` is available on your `PATH` and inherits the package identity from your app.

### 4. Run any script

From this terminal or any new one:

```powershell
mynode.exe app.js
mynode.exe some-other-script.js
mynode.exe --version
```

Every invocation runs under your registered package's identity — notifications get an AUMID, Phi Silica sees the capability, etc.

## Path B — one-shot `winapp run` (recommended for scripts and CI)

Best when you want a **single command** that does everything in one go — no persistent alias, no cleanup required. Ideal for automation, CI scripts, or throwaway experiments.

Skip the alias step and let `winapp run` do register-launch-unregister in one go:

```powershell
mkdir .local-node
copy (Get-Command node).Source .\.local-node\node.exe

npx winapp run . --exe .local-node\node.exe --args "app.js" --unregister-on-exit
```

What this does:

1. Registers your project as a loose-layout dev package
2. Launches `.local-node\node.exe app.js` with package identity
3. Waits for the process to exit
4. Unregisters the package (`--unregister-on-exit`)

Every run pays a couple of seconds for register + unregister, so it's slower than Path A for tight loops, but it leaves nothing behind afterwards.

## Path C — bare `winapp run` without a copied Node

If you don't need a stable executable path (e.g. you just want to run `node --version` under identity to check something), you can point `--exe` at the system `node.exe` directly:

```powershell
npx winapp run . --exe (Get-Command node).Source --args "app.js" --unregister-on-exit
```

This works, but it's not recommended for anything you'll iterate on:

- **A Node upgrade or `nvm use` changes where `node.exe` lives.** Your package will silently point at whatever version happens to be first on `PATH` at register time, not the one you meant.
- **Windows caches the executable path in the registered manifest.** If `node.exe` moves, the package can end up pointing at a stale file and fail to launch.

Copy Node into the project (Paths A / B) for anything you'll come back to more than once.

## Why copy `node.exe` into the project?

Windows loose-layout packages resolve executables **relative to the package root**. Your app's registered manifest needs to point at an `.exe` inside the package layout — the system `node.exe` at `C:\Program Files\nodejs\node.exe` (or wherever nvm dropped it) isn't part of your package.

Copying `node.exe` into `.local-node\node.exe` gives you:

- **A stable, in-project path.** The manifest points at `.local-node\node.exe` and never breaks when your global Node changes.
- **A pinned Node version.** The copy is frozen at the moment you set up the experiment — global upgrades don't affect it. Handy when comparing behavior across Node versions.
- **Ability to add an execution alias.** Execution aliases can only alias `.exe` files inside the package. A system-wide `node.exe` isn't eligible.

The copy is a plain byte-for-byte copy of `node.exe` — no wrapper, no launcher script. Everything about how it runs your `app.js` is identical to `node app.js`, except that it has package identity.

## Cleaning up: `winapp unregister`

Path A registers the package until you tear it down. When you're done experimenting:

```powershell
npx winapp unregister
```

This removes the loose-layout registration and the execution alias. Your project files are untouched.

Path B does this for you every run via `--unregister-on-exit`.

You can list what's currently registered with `Get-AppxPackage | Where-Object PublisherId -eq (…)`, but the easiest sanity check is `mynode.exe --version` — if it prints a Node banner, the package is still registered; if it fails with "command not found", it's already gone.

## From dev mode to distribution

Once your experiment works, packaging for distribution is the same layout with two extra CLI calls:

```powershell
npx winapp pack   # produces a signed .msix
npx winapp sign   # signs with your (or a test) certificate
```

The generated bindings under `.winapp/bindings/`, your `Package.appxmanifest`, and your JavaScript code all carry forward unchanged — dev mode is a subset of the same layout, not a separate structure.

For end users installing the MSIX, no dev mode / no `winapp run` — Windows registers the package the normal way at install time.

## Troubleshooting

### `mynode.exe : The term 'mynode.exe' is not recognized...`

The package isn't registered yet (or was already unregistered). Re-run:

```powershell
npx winapp run . --exe .local-node\node.exe --no-launch
```

Open a new terminal after registering; the old one may have a stale `PATH`.

### `hasPackageIdentity()` returns `false`

You're running `node.exe` directly instead of the aliased `mynode.exe` (or bypassing `winapp run`). Only invocations that go through the registered package (alias, or `winapp run`) get package identity.

### `E_APPMODEL_ERROR_NO_PACKAGE`, `0x80073D54`, `AppNotificationManager` errors on `.show()`

Same root cause — the process has no package identity. Use the alias flow (Path A) or `winapp run --args` (Path B).

### `roInitialize` complains about apartment mismatch

Some WinRT APIs are STA-only (`roInitialize(0)`), some are MTA-only (`roInitialize(1)`), and some don't care. As a starting point:

- Clipboard, most UI-linked types → STA
- Phi Silica, most background tasks → MTA

If you see `RPC_E_CHANGED_MODE (0x80010106)`, you called `roInitialize` twice with different apartment values in the same process.

### Phi Silica: `ensureReadyAsync` fails with capability error

Restart your terminal after adding `systemAIModels` to `Package.appxmanifest` and re-registering the package. Windows caches capability grants per registration.
