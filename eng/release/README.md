# Release notes

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
