# Package a dynwinrt Node.js application as MSIX

This guide shows how to package a Node.js application that uses
`@microsoft/dynwinrt` into an MSIX with
[WinApp CLI](https://github.com/microsoft/WinAppCli) 0.6.2.

The process first creates an application-specific Node
[Single Executable Application (SEA)](https://nodejs.org/api/single-executable-applications.html).
The executable, application JavaScript, generated WinRT bindings, and dynwinrt
native runtime are then staged together and packaged as a signed MSIX.

The installed application appears in the Start menu and launches with package
identity.

## 1. Initialize the Node.js project

Create a Node.js project:

```powershell
mkdir my-dynwinrt-app
cd my-dynwinrt-app
npm init -y
npm install --save-dev @microsoft/winappcli@0.6.2
npx winapp init . --use-defaults --add-js-bindings
```

`winapp init` creates the manifest and assets, installs matching dynwinrt
runtime/codegen packages, and generates bindings under `.winapp\bindings`.

For setup and restore details, see the
[WinApp CLI Electron setup guide](https://github.com/microsoft/WinAppCli/blob/main/docs/guides/electron/setup.md).
Its initialization steps also apply to a plain Node.js project; the
Electron-specific debug-identity steps are not needed here.

## 2. Build the application and Node SEA executable

Build the application JavaScript first. The examples below assume the entry
module is emitted as `dist\main.js`.

Install postject, the tool used by Node to inject the SEA blob:

```powershell
npm install --save-dev postject@1.0.0-alpha.6
```

Create `sea-bootstrap.cjs`. The bootstrap is embedded in the executable, then
loads the external application files from the installed package:

```javascript
const { createRequire } = require('node:module')
const path = require('node:path')

const packageRoot = path.dirname(process.execPath)
process.chdir(packageRoot)

const externalRequire = createRequire(path.join(packageRoot, 'main.js'))
externalRequire('./main.js')
```

Create `sea-config.json`:

```json
{
  "main": "sea-bootstrap.cjs",
  "output": "build/sea-prep.blob",
  "disableExperimentalSEAWarning": true,
  "useSnapshot": false,
  "useCodeCache": false
}
```

Download a pinned `node.exe` directly from the
[official Node.js distribution](https://nodejs.org/dist/) and save it as the
application executable. This example uses Node.js 24.19.0 x64:

Node.js does not publish a separate SEA executable. The regular official
`node.exe` already contains SEA support; postject turns a copy of it into the
application executable by injecting the blob and enabling its SEA fuse.

```powershell
New-Item .\build -ItemType Directory -Force

$nodeVersion = "24.19.0"
$nodeUrl = "https://nodejs.org/dist/v$nodeVersion/win-x64/node.exe"
$nodeSha256 = "3602f2bb1a10f2cbab4c36886218a33c1ab3db87290e73b033c46c77147d0237"
$nodeExe = ".\build\MyApp.exe"

Invoke-WebRequest $nodeUrl -OutFile $nodeExe

$actualSha256 = (
  Get-FileHash $nodeExe -Algorithm SHA256
).Hash.ToLowerInvariant()
if ($actualSha256 -ne $nodeSha256) {
  throw "Node.js executable SHA256 mismatch"
}
```

Before modifying it, use the downloaded executable to generate the SEA
preparation blob:

```powershell
& $nodeExe --experimental-sea-config .\sea-config.json
```

The official Node executable is Authenticode-signed. Injection changes the
file, so remove that signature before running postject:

```powershell
npx winapp tool signtool remove `
  /s .\build\MyApp.exe
```

Inject the blob and enable Node's SEA fuse:

```powershell
npx postject `
  .\build\MyApp.exe `
  NODE_SEA_BLOB `
  .\build\sea-prep.blob `
  --sentinel-fuse NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2
```

Postject updates `build\MyApp.exe` in place. The resulting executable starts
the packaged `main.js`.

Before continuing, make sure these files exist:

```text
build\MyApp.exe
dist\main.js
```

For x64 and ARM64, build separate executables and use the matching
`dynwinrt.node`.

## 3. Configure the manifest and assets

`winapp init` already generated `Package.appxmanifest` and the image assets.
Update the app name, publisher, version, and any required capabilities.

To replace the generated artwork:

```powershell
npx winapp manifest update-assets `
  .\packaging\logo.svg `
  --manifest .\Package.appxmanifest
```

## 4. Stage the runtime files

Create the package layout and copy the files needed at runtime:

```powershell
New-Item @(
  ".\layout\.winapp",
  ".\layout\node_modules\@microsoft",
  ".\artifacts"
) -ItemType Directory -Force

Copy-Item .\build\MyApp.exe .\layout
Copy-Item .\dist\* .\layout -Recurse
Copy-Item .\Package.appxmanifest .\layout
Copy-Item .\Assets .\layout\Assets -Recurse
Copy-Item .\package.json .\layout
Copy-Item .\.winapp\bindings .\layout\.winapp\bindings -Recurse
Copy-Item `
  .\node_modules\@microsoft\dynwinrt `
  .\layout\node_modules\@microsoft\dynwinrt `
  -Recurse
```

Adjust `build` and `dist` if the project uses different output directories.

## 5. Create a development certificate

For local testing:

```powershell
npx winapp cert generate `
  --manifest .\layout\Package.appxmanifest `
  --output .\artifacts\devcert.pfx `
  --export-cer `
  --password password
```

Trust it once from an elevated terminal:

```powershell
npx winapp cert install .\artifacts\devcert.pfx `
  --password password
```

The generated certificate is for development only. Never commit the PFX or its
password.

Sign the final SEA executable after injection:

```powershell
npx winapp sign `
  .\layout\MyApp.exe `
  .\artifacts\devcert.pfx `
  --password password
```

## 6. Build the MSIX

```powershell
npx winapp pack `
  .\layout `
  --manifest .\layout\Package.appxmanifest `
  --executable MyApp.exe `
  --output .\artifacts\Contoso.DynWinRTApp_1.0.0.0_x64.msix `
  --cert .\artifacts\devcert.pfx `
  --cert-password password
```

WinApp CLI detects the executable architecture, resolves the manifest token,
generates PRI resources, creates the MSIX, and signs it.

Inspect the result when diagnosing packaging issues:

```powershell
npx winapp tool makeappx unpack `
  /p .\artifacts\Contoso.DynWinRTApp_1.0.0.0_x64.msix `
  /d .\artifacts\unpacked `
  /o

Get-AuthenticodeSignature `
  .\artifacts\Contoso.DynWinRTApp_1.0.0.0_x64.msix
```

Here `/p` is the input package, `/d` is the unpack destination, and `/o`
allows overwriting an existing destination.

## 7. Install and launch

```powershell
Add-AppxPackage `
  .\artifacts\Contoso.DynWinRTApp_1.0.0.0_x64.msix
```

Launch through the registered application identity:

```powershell
$package = Get-AppxPackage -Name Contoso.DynWinRTApp
$app = Get-StartApps |
  Where-Object AppID -Like "$($package.PackageFamilyName)!*" |
  Select-Object -First 1

Start-Process explorer.exe `
  -ArgumentList "shell:AppsFolder\$($app.AppID)"
```

Verify the identity from application code:

```javascript
const { hasPackageIdentity } = require('@microsoft/dynwinrt')

if (!hasPackageIdentity()) {
  throw new Error('The process was not launched with package identity')
}
```

Uninstall the development package:

```powershell
Get-AppxPackage -Name Contoso.DynWinRTApp | Remove-AppxPackage
```

## Windows App Runtime applications

A Node.js application that only calls stock `Windows.*` APIs does not need a
Windows App Runtime dependency.

A Node.js application that calls `Microsoft.Windows.*` or WinUI APIs can
either:

- declare the matching Windows App Runtime framework package; or
- use `npx winapp pack --self-contained`.

A framework-dependent packaged process must not call the unpackaged Windows
App SDK bootstrap path. Package activation resolves the declared framework.
Keep metadata, runtime files, generated projections, and native extensions on
the same Windows App SDK version.

## Optional: test the layout before packaging

Use `winapp run` when the application needs package identity during
development. See [Node.js development mode](../node/dev-mode.md) for the
registration, launch, and cleanup workflows.

## Writable application data

The installed package directory is immutable. The Node.js application must
store logs, databases, caches, and user state elsewhere.

Packaged desktop applications can virtualize `%LOCALAPPDATA%` paths under:

```text
%LOCALAPPDATA%\Packages\<package-family-name>\LocalCache\Local\
```

Use `Windows.Storage.ApplicationData` for package-scoped state. Use an
explicit external location if data must survive uninstall. Test upgrade,
uninstall, and reinstall behavior for the selected location.

## x64 and ARM64

Build and test one layout per native architecture:

```text
publish\
  x64\
    MyApp.exe
    node_modules\@microsoft\dynwinrt\...
  arm64\
    MyApp.exe
    node_modules\@microsoft\dynwinrt\...
```

For ARM64, repeat the SEA build with the same Node version's
[`win-arm64/node.exe`](https://nodejs.org/dist/v24.19.0/win-arm64/node.exe)
and the ARM64 dynwinrt runtime.

WinApp CLI 0.6.2 can create a multi-architecture bundle directly:

```powershell
npx winapp pack `
  .\publish\x64 `
  .\publish\arm64 `
  --manifest .\layout\Package.appxmanifest `
  --executable MyApp.exe `
  --output .\artifacts\Contoso.DynWinRTApp_1.0.0.0_x64_arm64.msixbundle `
  --cert .\artifacts\devcert.pfx `
  --cert-password password
```

This produces one `.msixbundle` at `--output`. The bundle contains separate
x64 and ARM64 MSIX packages, and Windows installs the matching architecture.

WinApp CLI validates that package identity, capabilities, and dependencies are
consistent across slices and stamps each package with the detected PE
architecture.

## Production release

Use Microsoft Store signing or an organization signing service when possible.
If CI uses a protected PFX, keep building and signing in separate jobs and
timestamp the signatures:

```powershell
$password = $env:MSIX_CERT_PASSWORD

npx winapp sign .\layout\MyApp.exe .\secure\release.pfx `
  --password $password `
  --timestamp https://timestamp.digicert.com

npx winapp pack .\layout `
  --manifest .\layout\Package.appxmanifest `
  --executable MyApp.exe `
  --output .\artifacts\Contoso.DynWinRTApp_1.0.0.0_x64.msix

npx winapp sign .\artifacts\Contoso.DynWinRTApp_1.0.0.0_x64.msix `
  .\secure\release.pfx `
  --password $password `
  --timestamp https://timestamp.digicert.com
```

Before publishing, test the exact signed artifact on a clean x64 and ARM64
machine:

- install and uninstall;
- Start menu/AUMID activation;
- package identity and capabilities;
- framework and VC runtime dependencies;
- upgrade with preserved state; and
- absence of source files, build tools, and certificates in the package.
