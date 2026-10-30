---
name: pr-writing
description: Use when preparing, writing, or revising a pull request title or body in dynwinrt.
---

# Pull request writing

Use the repository's [PR template](../../PULL_REQUEST_TEMPLATE.md) as the
single source of truth for the body structure. Read it each time; do not
substitute a remembered template. This skill governs PR writing, not code
review, CI operation, merging, or release.

## Before writing

1. Read the actual diff against the intended PR base, including pending changes
   that will be part of the PR. For an existing PR, also read its current title
   and body. Do not summarize only commit subjects or session history.
2. Identify the actual behavior impact: a bug fix, feature, pure text change,
   refactor, build/dependency change, or a mixture. Do not classify only by file
   extension; documentation can change executable examples or user workflows.
3. Check the relevant code, documentation, issue, and validation evidence.
   Distinguish observed behavior from expectations and unverified claims.
   Never invent a root cause, reproduction, or successful check.
4. Write a concise title naming the change or outcome, rather than a work log.

## Select and fill sections

Use the template's heading text and relative order for the sections selected.
Only Summary is common to every PR. Delete inapplicable sections, including
their headings and placeholders; do not fill them with "None" or "N/A".

- Summary explains the change, its reason, and implementation details that aid
  review, not a file-by-file work log. A bug needs its supported root cause and
  key fix; state uncertainty instead of inventing a cause. A feature needs its
  purpose and approach. Link a relevant issue if one exists.
- Bug fixes require Reproduction: necessary prerequisites, exact runnable steps
  or code, the actual failure, and the result after the fix.
- Features require Usage: a minimal API call or command, necessary setup or
  configuration, and an observable result. Do not use bug-style before/after
  fields or combine the two purposes under a generic heading.
- Pure text, formatting, and contribution-guidance changes normally need only
  Summary. Do not invent usage, reproduction, validation, or risks to fill space.
- For refactors, build/dependency changes, documentation with executable
  examples, and mixed changes, choose only sections justified by actual behavior
  impact. Include both Reproduction and Usage only for distinct needs, not every
  section associated with every file type.

Validation is optional. Include only focused behavioral evidence or important
verification gaps that readers need and cannot already see in the reproduction
or CI panel. Describe what was exercised and observed, or why an important check
was not performed. Do not repeat earlier steps, routine proofreading, Markdown
rendering, or heading checks. Do not include CI status, workflow/job names, test
counts, coverage statistics, full logs, or a chronological command list.

Compatibility and risks is optional. Keep it only for real compatibility
impact, upgrade actions, important limitations, or unverified behavior. Do not
invent risks or list unrelated subsystems or internal session coordination.
Mention dependencies only when they affect review or merge order; omit
irrelevant base hashes and independence statements. Keep versions or revision
identifiers when they are actually needed to reproduce the behavior.

## Before submitting or updating

Run the proportionate checks required by the repository even when the PR needs
no Validation section. Doing a check does not make it useful PR content. Keep raw
logs and coordination evidence outside the description.

In the published PR body, keep each ordinary prose paragraph on one continuous
line and let the page wrap it. Preserve necessary line breaks for paragraphs,
headings, lists, and code blocks. Source Markdown may be wrapped; do not carry
those hard wraps into published prose.

Reread the title and body against the diff and template. Confirm the selected
sections serve the change, required reproductions or usage examples are usable,
and claims match the evidence. Remove inapplicable sections, placeholders,
repetition, and work-log details. When updating an existing PR, preserve
still-relevant content and make sure the description reflects the current change.
