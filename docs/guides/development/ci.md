# Build CI

The `Build` workflow in `.github/workflows/build.yml` runs independent Rust
validation and native package builds in parallel. It does not change the
shipping Cargo release profile, coverage instrumentation, or the separate
Python release matrix.

## Scheduling and artifacts

After change classification and lightweight checks, full validation starts
these lanes without waiting for the Rust tests:

| Job                   | Responsibility                                                                                             |
| --------------------- | ---------------------------------------------------------------------------------------------------------- |
| `test-rust`           | Existing Rust unit, ABI, codegen, census, determinism, cleanup and live i686 checks                        |
| `build-codegen-x64`   | Release x64 generator                                                                                      |
| `build-bindings-x64`  | Production release x64 JS distribution and CPython 3.12 x64 development-profile wheel                      |
| `build-js-test-hooks` | Separate release x64 JS distribution with native test fixtures                                             |
| `build-arm64`         | Release ARM64 generator/runtime and the ARM64 C-oracle compile check                                       |
| `e2e-runtime`         | Production JS/Python tests, generated WinRT/COM/Win32/implementation E2E, long paths, and dispatcher smoke |
| `e2e-native`          | TSFN, generated unsafe COM, native completion, borrowed copies, and Win32 cleanup fixtures                 |

The two E2E consumers depend only on their x64 producers, not the ARM64 lane or
Rust tests. The original `test` and `e2e` check names aggregate their respective
validation lanes; the `e2e` aggregate retains its Rust-test prerequisite without
delaying either E2E consumer. `dynwinrt` and `dynwinrt-codegen` assemble the original
dual-architecture final artifacts only after Rust, E2E and the relevant builds
succeed. `ci-success` checks every job result, including final artifact
verification. Failure, cancellation, a missing result or an unexpected skip
fails these gates.

Intermediate `ci-*` artifacts are inputs, **not release-ready packages**.
Downloads use the current workflow run. Each input includes a file-hash manifest
with the commit, run, package version, architecture, Cargo profile, feature set
and, for wheels, CPython ABI. Consumers verify it before using the payload.
The full JS distribution is transferred, including declarations and COM/Win32
sidecars. Assembly requires identical architecture-independent files and keeps
both native binaries; it never overlays test-hook files onto production.

Rust test executables still need their own `cargo test` builds. In particular,
the runtime-backed `implementation_naming_test` is not replaced by a generator
executable. Coverage binaries and the separate Python release workflow's
packaging configurations are not shared with production.

## Documentation-only pull requests

Only pull requests proven to change explanatory documents in the exact allowlist
in `eng/ci/classify_changes.py` take the lightweight path. The list includes the
root README, reviewed usage/architecture guides, and existing sample READMEs.
It does not include packaged READMEs, `docs/status`, generated reports, schemas,
tests, scripts, workflows, sample code/configuration, or unknown paths.

Classification compares the PR merge base to its head using complete Git
history, not just the latest commit. Both old and new rename paths must qualify;
allowlisted deletions are documentation changes too. An empty diff uses full
validation. Invalid or unavailable history fails classification rather than
guessing that the change is documentation-only.

The lightweight path runs the scheduling, artifact and prebuilt-selection
regressions without compiling native code. Stable check names still run and
validate the explicitly expected heavy-job skips; no package artifact is
created. Pushes to `main` and version tags always use full validation. The
workflow is not hidden behind a top-level path filter.

## Using a prebuilt generator locally

Ordinary developer commands are unchanged: omitting a generator override uses
Cargo with the selected `-CargoProfile` and `-CargoTarget`. `-SkipBuild` alone
skips initial native builds, but still permits the existing Cargo generation
path. To prohibit generator rebuilds, supply a real executable explicitly:

```powershell
.\tests\e2e\e2e_test.ps1 -SkipBuild `
    -Codegen C:\artifacts\dynwinrt-codegen.exe

.\tests\e2e\e2e_test.ps1 -SkipBuild -Suite implementations `
    -Codegen C:\artifacts\dynwinrt-codegen.exe -Lang py,ts
```

`DYNWINRT_CODEGEN` provides the same default for both PowerShell entrypoints and
the JavaScript generated-unsafe, native-completion and borrowed-copy tests.
An explicit `-Codegen` takes precedence. Missing/invalid explicit files fail;
they never silently fall back to Cargo or a stale local binary.

Binding artifacts must already be installed when using `-SkipBuild`: the full
matching JS distribution in `bindings\js\dist`, and a compatible wheel in the
selected Python environment. The JS fixture npm scripts retain their local
`build:test-hooks` behavior. CI instead runs their AVA tests directly against a
verified, separately downloaded test-hooks distribution.

Run the lightweight regressions with:

```powershell
python -m pip install -r eng\ci\requirements.txt
python -m unittest discover -s eng\ci -p "test_*.py" -v
node --test bindings\js\scripts\run-codegen.test.mjs
.\tests\e2e\e2e_preparation.tests.ps1
.\tests\e2e\prebuilt_codegen.tests.ps1
```

Parallelism removes the old Rust-test barrier and duplicate release builds;
actual hosted run times still depend on runner capacity and artifact transfer.
Measure completed runs before treating a wall-clock improvement as established.
