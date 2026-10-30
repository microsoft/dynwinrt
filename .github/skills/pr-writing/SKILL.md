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
2. Identify whether the change fixes a bug, adds a feature, changes documentation,
   or is internal-only. For mixed changes, cover each relevant behavior without
   repeating shared context.
3. Check the relevant code, documentation, issue, and validation evidence.
   Distinguish observed behavior from expectations and unverified claims.
   Never invent a root cause, reproduction, or successful check.
4. Write a concise title naming the change or outcome, rather than a work log.

## Fill the template

Keep all of the template's section headings in their original order. Replace
placeholders with meaningful content; adapt the subordinate labels to the change
type and explain non-applicable items rather than inventing an example.

- In the summary, explain what changes for users or contributors and why.
  For a bug, include the supported root cause and the key fix; if the cause
  remains uncertain, state that explicitly. For a feature, explain its purpose
  and implementation approach. Include only implementation details that aid
  review, not a file-by-file change list. Link a relevant issue if one exists.
- Make a bug reproducible: separate necessary prerequisites from exact steps or
  runnable code, then describe the behavior before and after the fix. Give a
  feature's minimal usage example and expected result. For documentation or
  internal-only changes without a runnable example, explain why it does not
  apply; do not invent one or silently omit the section.
- Report focused validation as what was exercised and what was observed. Refer
  back to the reproduction instead of copying it. Include relevant edge cases or
  manual checks when they add evidence, and explain any necessary check not run.
  Do not include CI status, workflow/job names, test counts, coverage statistics,
  full logs, or a chronological command list. A minimal reproduction command
  and its result are useful evidence, not an execution diary.
- Explain compatibility impact, upgrade actions, limitations, and unverified
  behavior only when they matter to this change. Write "None" when there are
  none. Do not list unrelated subsystems or internal session coordination.
  Mention dependencies only when they affect review or merge order; omit
  irrelevant base hashes and independence statements. Keep versions or revision
  identifiers when they are actually needed to reproduce the behavior.

## Before submitting or updating

Run the proportionate checks required by the repository; keeping CI reporting
out of the body does not remove the obligation to validate the change. Keep raw
logs and coordination evidence outside the PR description.

Reread the title and body against the diff and template. Confirm the sections
are complete and ordered, the reproduction or non-applicability explanation is
usable, claims match the evidence, and placeholders, repetition, and work-log
details are gone. When updating an existing PR, preserve still-relevant content
and make sure the description reflects the current change.
