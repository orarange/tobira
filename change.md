# Change Log

This file records any exception where Codex touches CSS-facing code or other files that overlap with the Claude CSS branch.

## CSS Touch Policy

- Default: do not edit the CSS parser/layout baseline that is owned by the Claude `claude/phase5-css` branch.
- Allowed exception: if a JS feature genuinely needs CSS-facing integration, keep the change minimal, non-destructive, and narrowly scoped.
- Required for that exception: create or update a PR, request Copilot review before broadening the diff, and log the exact touched files plus the reason in this file.
- Read-only inspection of CSS files is fine; destructive or broad CSS edits are not.

## Current Entries

- 2026-05-17: Defined the CSS touch boundary policy and documented the exception workflow.
  - Touched files: `HANDOFF.md`, `JS_ROADMAP.md`, `change.md`
  - CSS engine files modified: none
