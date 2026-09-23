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
job checks out the tagged commit and publishes `RELEASE_NOTES.md` as the
`release-notes` pipeline artifact. The GitHub release job declares this artifact
in `templateContext.inputs` and reads
`$(Pipeline.Workspace)/release-notes/RELEASE_NOTES.md`, without checking out
source. This preserves the tagged notes while complying with the 1ES
restriction on checkout in release jobs. The automatic changelog remains
enabled and is appended after the checked-in notes.

Build CI and the release pipeline both run `validate_release_notes.ps1` and
`test_validate_release_notes.ps1` to check the notes file, release task inputs,
the Build artifact producer, the release job's artifact input, and the absence
of release-job source checkout. These checks do not create a tag or publish a
release.
