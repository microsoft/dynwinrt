# Release notes

`RELEASE_NOTES.md` is the checked-in description for the next public release.
Generic package and installation text or release-specific highlights belong
in that file, without requiring pipeline changes. Use a literal release
heading: file-based notes do not expand Azure DevOps variables.

Before each public release:

1. Update only `RELEASE_NOTES.md` for the release-note content, including the
   release heading and any version-specific installation commands.
2. Merge the release preparation to `main`.
3. Tag that exact `main` commit.

Do not edit `.pipelines/release.yml` for each release. The Azure DevOps Build
job checks out the tagged commit and copies `RELEASE_NOTES.md` into
`$(Build.ArtifactStagingDirectory)/release-notes`. It publishes that entire
directory as the `release-notes` pipeline artifact, not the Markdown file
alone, so the 1ES-generated `_manifest` SBOM is included with the notes.
The GitHub release job declares this artifact
in `templateContext.inputs` and reads
`$(Pipeline.Workspace)/release-notes/RELEASE_NOTES.md`, without checking out
source. This preserves the tagged notes while complying with the 1ES
restriction on checkout in release jobs. The automatic changelog remains
enabled and is appended after the checked-in notes.

Build CI and the release pipeline both run `validate_release_notes.ps1` and
`test_validate_release_notes.ps1` to check the notes file, release task inputs,
the Build staging step and artifact producer, the release job's artifact input, and the absence
of release-job source checkout. These checks do not create a tag or publish a
release.

Both scripts derive their default repository root from their own location,
independent of the working directory or an agent's temporary wrapper script.
The regression suite exercises the dot-sourced invocation used by
`PowerShell@2`, including Windows PowerShell 5.1 on Windows. An explicitly
supplied `-RepositoryRoot` is still honored.

The release job's 1ES SBOM validation remains enabled. Local configuration
tests check the staging and publication paths; hosted 1ES execution is needed
to verify SBOM generation and validation.

## Python release orchestration

`Wait_Python` invokes `wait_python_release.ps1` to find a push run of
`python-release.yml` for the exact `v<version>` tag and source commit SHA.
Duplicate matches warn and select the newest `created_at`, then numeric run ID.
After that run's `assemble-release` job succeeds, the `WaitPython` task exports
`pythonRunId` as an Azure output variable; later waits cannot select another run.

`Release_GitHub` then creates the release. `Collect_Python` depends directly
on `Wait_Python` and waits for that exact run's `github-release` job to succeed
after its authenticated `gh release upload`. Each poll revalidates the run's
identity and refreshes its status. Failed jobs, completed runs without the
required successful job, API retry exhaustion, and timeouts fail explicitly.
Discovery allows 10 attempts and job polling 120 attempts, 30 seconds apart;
Azure jobs allow 90 minutes for polling and API overhead.

The script still checks the release's exact tag and `target_commitish` after
upload succeeds, but never reads embedded `release.assets`, which can be empty
even when wheels are uploaded. The existing `DownloadGitHubRelease@0` service
connection downloads the wheels; `verify_python_release.py release-set` still
requires the exact version's 8 runtime and 2 codegen wheels and their metadata
before freezing `python-packages`. Automatic npm/PyPI publication and
`DoEsrp`/`PublishPyPI` conditions remain unchanged, with no new approval gate.

Run `eng\release\test_wait_python_release.ps1` with either PowerShell 7 (`pwsh`)
or Windows PowerShell 5.1 (`powershell.exe`): fake API responses and a sleep
recorder avoid network requests and real sleeps. Build CI runs both engines;
the Azure release Build job also runs the suite.

Already-running Azure releases retain their original YAML. Use a clean rerun
from a tag on the fixed `main` commit; merging this change does not repair the
in-flight `v0.1.0-preview.22` run.
