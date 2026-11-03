# Issue tracker: GitHub

Issues and specs for this repository live in `pennel-app/pennel`.
Use the supported GitHub connector or the `gh` CLI.

## Required workflow

Follow the [Pennel roadmap agent workflow](https://github.com/pennel-app/pennel/blob/main/docs/desktop-roadmap.md#agent-workflow).

- Search for the owning issue before creating one.
- Track meaningful implementation and documentation/configuration changes
  on Pennel Project #1. Planning-only work needs no tracker writes.
- Default new work to Next / P2 with the appropriate Area and no invented
  assignee or deadline. Preserve existing metadata.
- Use In progress during implementation and In review while review,
  merge or required acceptance remains.
- Close only after all completion criteria are verified.
- Read back mutations and follow the canonical access-failure rules.

App-release and shared agent-guidance coordination belongs in
`pennel-app/pennel`. Cloud implementation belongs in the private
`pennel-app/pennel-cloud` tracker. Independently actionable tap packaging
work belongs in `pennel-app/homebrew-pennel`.

Keep private Cloud details and source references in its private tracker.

## Operations

Before creating an issue, verify the target with
`gh repo view --json nameWithOwner`. Never file Pennel work in the
skills' upstream repository.

- Create: `gh issue create --title "..." --body-file <file>`.
- Read: `gh issue view <number> --json number,title,body,labels,comments`.
- List: `gh issue list --state open --json number,title,body,labels,comments`.
- Edit: `gh issue edit <number> --body-file <file>`.
- Comment: `gh issue comment <number> --body-file <file>`.
- Labels: `gh issue edit <number> --add-label "..." --remove-label "..."`.
- Parent: `gh issue create --parent <parent> ...`, or
  `gh issue edit <parent> --add-sub-issue <child>`.
- Dependencies: `gh issue edit <child> --add-blocked-by <blocker>`.
- Close: post the explanation first, then `gh issue close <number>`.
  Verify both issue state and project status.

Use native issue relationships. If unavailable, maintain parent task-list
links and `Part of #...` / `Blocked by: #...` lines in child bodies.
A ticket is startable only when all blockers are closed.

Use files containing actual newlines for multiline bodies.

## Pull requests as a triage surface

**PRs as a request surface: no.**

GitHub shares issue and PR numbers. Resolve an ambiguous number with
`gh pr view <number>`, falling back to `gh issue view <number>`.

## Skill terminology

"Publish to the issue tracker" means create a GitHub issue.
"Fetch the relevant ticket" means read its body, comments and labels.

## Wayfinding

A map is one issue labelled `wayfinder:map`, containing Notes,
Decisions-so-far and Fog. Link child issues using native sub-issues,
with task-list links as the fallback.

Child labels are `wayfinder:research`, `wayfinder:prototype`,
`wayfinder:grilling` or `wayfinder:task`.

Choose the first open, unassigned child in map order whose blockers
are closed. Assign only when authorized by the user. Record answers
and evidence in the child and link them from the map. Apply the
required completion and acceptance workflow before closing.
