# Release notes

## preview.24 recovery branch (do not merge)

On `lei9444-release-preview24-recovery`, `.pipelines/release.yml` is a
**manual-only, single-release recovery pipeline**, not the normal tag pipeline
described below. Do not merge it into `main` or use it for another release.
It neither builds native packages nor creates/updates a GitHub release.

The original ADO run
[162292158](https://dev.azure.com/microsoft/71503134-3b3d-4e75-aa3f-25519f1b7d3f/_build/results?buildId=162292158)
failed during `Collect_Python`: the downloaded Python 3.12 installer's internal
pip upgrade tried the public index through a proxy that returned HTTP 403.
The recovery sets `PIP_INDEX_URL` on **UsePythonVersion itself**, as well as
retaining the approved index for the later `packaging` installation.

The recovery inputs are fixed in the checked-in pipeline and validator; they
are not queue-time version/build overrides:

| Input | Fixed identity |
| --- | --- |
| Package tag | `v0.1.0-preview.24` |
| Package source commit | `1ab0b31b6e59ef2a44215a5f394d90eed8fdcd4b` |
| npm source | ADO definition `191586`, run `162292158`, `npm-packages` artifact `356126797` |
| Python source | Existing [GitHub release](https://github.com/microsoft/dynwinrt/releases/tag/v0.1.0-preview.24), release ID `429044377`, 10 wheels |
| Python test evidence | [Run 41055989308](https://github.com/microsoft/dynwinrt/actions/runs/41055989308), all x64/ARM64 consumer jobs plus `assemble-release` and attachment job succeeded |
| npm / Python versions | `0.1.0-preview.24` / `0.1.0rc24` |

Before preparing this branch, the two npm tarballs were downloaded from the
original ADO artifact and compared with the release attachments. The 10 release
wheels were compared with tested assembly artifact `12977716730`
(`python-release-0.1.0-preview.24`). The validator pins every attachment ID and
SHA-256 digest, the original npm artifact's dedup identity, and the original
run/tag/release identities. It refuses changed/missing/duplicate packages,
failed/skipped Python tests, moved tags, changed release assets, or a changed
original publishing-stage result.

`Build.SourceVersion` identifies the **recovery orchestration commit**, not the
package source. The validator checks that commit and its ancestry, rejects
non-recovery source changes, runs the unchanged `verify_python_release.py`
complete-release checks against the authoritative source/stubs, and only then
emits `SetMeta.version`. The two original npm tarballs are copied without
repacking into a fresh artifact directory. Normal 1ES artifact/SBOM generation
and production release-job validation remain enabled for this recovery run;
the logs separately record the original package source and recovery commit.
No original artifact, tag, release, or attachment is modified.

### Manual run

1. Open the existing ADO pipeline
   [dynwinrt-release (191586)](https://dev.azure.com/microsoft/71503134-3b3d-4e75-aa3f-25519f1b7d3f/_build?definitionId=191586)
   and select **Run pipeline** (not rerun/retry on the failed tag run).
2. Select branch **`lei9444-release-preview24-recovery`**. Confirm the branch
   head is the reviewed recovery commit. Keep all stages selected.
3. For collection/validation only, leave **`DoEsrp=false`** (the default).
   `PublishPyPI=true` does not publish anything when `DoEsrp=false`.
4. To complete the release, explicitly set **`DoEsrp=true`** and
   **`PublishPyPI=true`**. Leave **`signingIdentity` at its existing default**
   and do not add/override variables. The order is collection/validation,
   npm ESRP, Python runtime ESRP, Python codegen ESRP. Existing service
   connections, signing variable group, production controls, owners and
   approvers are unchanged.

The pipeline's build identity must be able to read the original run/artifact
using `System.AccessToken`; normal GitHub download and ESRP authorizations are
still required. Missing access, artifact retention expiry, or a provenance
mismatch is a blocker, not permission to rebuild or replace release assets.
If any package has already been published by the time this is run, inspect
that result before proceeding; do not blindly retry a partially published run.
Never rerun the original tag pipeline: its `GitHubRelease@1 action:create`
fails on the already-existing release.

Local recovery regression checks (use the existing CI Python dependencies):

```powershell
python -m unittest discover -s eng\release -p test_preview24_recovery.py -v
```

The historical release-note wiring validators below apply to the normal
tag/create-release pipeline, not this intentionally non-mergeable recovery
variant. Recovery checks instead compare every ESRP job with the original
tagged pipeline and test the manual-only wiring and fail-closed guards.
Local checks and ADO `previewRun: true` expansion do not execute a release or
prove that hosted service authorization, Python setup, or ESRP publishing ran.

## Normal tagged releases

`RELEASE_NOTES.md` is the checked-in description staged from the tagged commit
for a public release. Keep published version notes intact; draft later changes
in `UNRELEASED.md`, which is not staged by the release pipeline. Before the
next release, promote applicable draft entries into `RELEASE_NOTES.md` under
the new literal release heading: file-based notes do not expand Azure DevOps
variables.

Before each public release:

1. Update `RELEASE_NOTES.md` for that release, including the heading, promoted
   draft entries and any version-specific installation commands. Remove
   promoted entries from `UNRELEASED.md`.
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
The pipeline stages only `RELEASE_NOTES.md`, not `UNRELEASED.md`; rerunning a
published tag uses the notes at that tag. Check the version heading against
the tag during release preparation: the validator checks the artifact wiring
and nonempty notes, not the heading's version.

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
