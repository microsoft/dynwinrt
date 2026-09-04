# Release notes

`RELEASE_NOTES.md` is the checked-in, curated description for the next public
release. Version-specific installation commands are intentional so each GitHub
Release remains a useful snapshot.

Before each public release:

1. Update only `RELEASE_NOTES.md` for the release-note content, including every
   version and installation command.
2. Merge the release preparation to `main`.
3. Tag that exact `main` commit.

Do not edit `.pipelines/release.yml` for each release. The Azure DevOps
`GitHubRelease@1` task reads the notes from the tagged checkout, so the contents
are snapshotted from the tag commit. Its automatic changelog remains enabled
and is appended after the curated notes.
