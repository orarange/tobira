# Change Log

This file records coordination notes when Codex and Claude overlap on CSS-facing work or when CSS / JS changes need extra review.

## Coordination Policy

- Default: Codex may work directly on both CSS and JS on the active Codex branch when that is the shortest path to the goal.
- If a change overlaps with active Claude CSS work, keep the diff narrow, use the `claude` command when a targeted follow-up is useful, and log the overlap in this file.
- If a CSS-facing change is substantial, create or update a PR, request Copilot review before broadening the diff, and log the exact touched files plus the reason in this file.
- Read-only inspection of CSS files is always fine; broad or destructive churn still deserves a coordination pass.

## Current Entries

- 2026-05-21: Updated the operating policy so Codex can own both CSS and JS implementation work on the active branch.
  - Touched files: `HANDOFF.md`, `JS_ROADMAP.md`, `change.md`
  - CSS engine files modified: none
