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

- 2026-09-26 (Claude): 行の高さの作り替えで `src/css.rs` を触った。
  - Touched files: `src/css.rs`, `src/layout.rs`, `src/gui.rs`, `src/browser.rs`（テストの初期化子のみ）
  - CSS engine files modified: `src/css.rs` — `line-height` の値の持ち方だけ。
    長さ（`px` など）は `ComputedStyle::line_height_fixed_mpx` に長さのまま持って
    長さのまま継承し、`em` / `%` は自分の font-size で長さに直す（仕様どおり）。
    単位の無い数は今までどおり比率で継承。`parse_line_height` は
    `set_line_height` + `finish_line_height` に置き換えた。
    理由: `line-height:40px` が比率で継承されて、32px の子の行が 80px になっとった
    （Chrome 40）。`font-size:32px; line-height:40px` も親の大きさで割っとった。
