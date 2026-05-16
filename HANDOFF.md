# Handoff

This file is the canonical handoff note for this repo.
Update it whenever work switches between Codex, Claude, Gemini, Copilot, or a fresh session after a context reset.

## Handoff Rules

- Read this file, `git status --short`, and the latest `git log --oneline -n 20` before making assumptions.
- Confirm the current branch with `git branch --show-current` before starting work.
- Codex must stay on the active Codex branch listed below unless the user explicitly changes that rule.
- Codex should use a dedicated worktree for the active Codex branch instead of sharing the user's main checkout.
- Keep Codex changes isolated to the active Codex branch; Claude may work on its own branch and merge reconciliation happens later through GitHub Copilot or the user's preferred flow.
- Update the `Current Snapshot` section whenever the high-level state changes.
- Append a short entry to `Session Log` whenever meaningful work is handed off or resumed.
- Do not stage unrelated local helper artifacts unless the user explicitly asks for them.
  Current local artifacts that are present but not part of the tracked repo are:
  `.claude/`, `.repomix/`, `copilot.md`, `gemini.md`, `repomix-output.xmlbrowser.xml`
- **PR title** — When opening a pull request, always include the agent's name in the title.
  Example: `[Claude] fix CSS calc() precedence` / `[Codex] add image lazy-loading`

## Current Snapshot

- Date: `2026-05-16`
- Repo / package name: `tobira`
- Active Codex branch: `codex/js-event-capture`
- Active Claude branch: `claude/phase2-css`
- Workflow:
  - keep the shared root checkout free for the user / Claude side
  - run Codex implementation from a separate `codex/js-event-capture` worktree
- Verification status:
  - `cargo test`: `111` passing tests on `2026-05-16`
  - `cargo build`: success on `2026-05-16`
- Current implementation highlights:
  - hand-rolled `http://` and `https://` client with redirects and compressed response decoding
  - custom HTML parser and DOM-like tree
  - CSS engine with broader selector and expression support than the original README says
    - descendant / child selectors
    - attribute selectors
    - `:first-child`, `:last-child`, `:nth-child(...)`, `:not(...)`
    - `@media` handling
    - `calc(...)`
    - `rgba(...)` blending
  - software-rendered GUI with custom title bar and address bar
  - blank startup page and direct URL entry
  - address bar editing shortcuts including `Ctrl+A`, `Ctrl+C`, `Ctrl+X`, and `Ctrl+V`
  - clickable links in the rendered page
  - first-class GUI page controls for:
    - text inputs
    - buttons
    - caret / selection / clipboard shortcuts
    - IME cursor placement
    - basic `GET` form submission with relative action resolution and query encoding
    - focused-input keyboard event delivery for `keydown` / `keyup`
    - live GUI typing synchronized into DOM-backed `value`
  - page keyboard events:
    - focused page inputs receive bubbling `keydown` / `keyup`
    - key metadata includes `key`, `code`, modifier flags, and `repeat`
  - page event listeners now support capture + bubbling, plus `once` listeners and capture-sensitive `removeEventListener(...)`
  - guarded JavaScript execution through `boa_engine`
  - lightweight mutable DOM bridge with:
    - `querySelector(...)`, `querySelectorAll(...)`, `getElementById(...)`
    - `createElement(...)`, `createTextNode(...)`
    - `appendChild(...)`, `insertBefore(...)`, `remove()`
    - `innerHTML`, `textContent`, `classList`, `id`, `className`
    - `document.write(...)` with recursive script expansion
    - DOM mutations serialized back into the HTML pipeline after JS runs
  - JS execution now runs inside a dedicated larger-stack worker thread
  - Promise jobs are drained after top-level script eval via `context.run_jobs()`
  - initial network-facing JS support:
    - Promise-backed `fetch(...)`
    - minimal `XMLHttpRequest` constructor / `open()` / `send()` / `onload`
    - JS-triggered `location.href` / `assign()` / `replace()` can request a follow-up page load
    - common DOM property reflection for `src`, `href`, `rel`, `type`, `name`, `value`, `content`
  - local test pages for CSS, basic JS, and DOM mutation coverage under `demo/`
  - site-specific rendering paths for:
    - YouTube watch pages
    - YouTube home shell / cards / nudge UI
    - lightweight Google shell
    - legacy frame/table-heavy pages such as the Abe Hiroshi site

## Important Modules

- `src/browser.rs`
  Main page-loading pipeline, site-specific rewrites, legacy page handling, YouTube/Google synthetic documents.
- `src/css.rs`
  CSS parser, selector matching, computed styles, `@media`, `calc(...)`, color parsing.
- `src/layout.rs`
  Layout pipeline, text flow, tables, image placement, background drawing, link hitbox generation.
- `src/gui.rs`
  Custom chrome, address bar state, input handling, hover/click navigation, rendering integration.
- `src/js.rs`
  Sandboxed JS execution policy plus the mutable DOM bridge used during script execution.
- `src/html.rs`
  Hand-rolled HTML parser. Now preserves raw text for `script` / `style` / `title` / `textarea`, which matters for JS and CSS correctness.
- `src/http.rs`
  HTTP/TLS fetch layer and browser-like request headers.

## Recent Commit Landmarks

- `1df11f6` live input value sync implementation complete
- `c64f16a` event listener capture groundwork complete
- `48f7141` Merge branch 'codex/codex' into master (resolved conflicts)
- `4b2c68b` Claude/phase2 css (#41)
- `91cc671` Merge branch `claude/modest-pascal-9bf652`
- `5952827` page form controls feature implementation complete
- `d159cf0` dom backed javascript support implementation complete

## Known Gaps / Likely Next Work

- README capability list is partially stale; prefer this file for the latest snapshot.
- JS support is still far from a full browser DOM / framework runtime.
- GUI-to-page event delivery now covers capture + bubbling `click`, `input`, `change`, `submit`, `keydown`, and `keyup`, plus target-only `focus` and `blur`; passive listener semantics are in place, and `location.hash` plus `history.pushState(...)` / `replaceState(...)` now support soft navigation without a reload, while the rest of the option matrix and back/forward stack still need depth.
- Native page input typing now syncs `value` into the JS DOM.
- Framework-facing browser APIs still need a lot more depth.
- History / back-forward behavior is not yet complete.
- Modern app-shell sites still need more DOM APIs, cookies/storage, and CSS coverage.
- CSS is still computed once up front instead of being rebuilt against the live window width.
- Form support is still limited to simple text-like fields and `GET` submission; `POST`, checkboxes, radios, and file inputs are not wired yet.
- The `XMLHttpRequest` shim is enough for lightweight callers, but prototype / `instanceof` semantics are still incomplete.
- Actual media playback and a true YouTube watch experience are still incomplete.
- Claude's `claude/phase2-css` branch adds `position: relative/absolute/fixed`, `z-index`, and `display: flex` — not yet merged to master.

## Useful Commands

```powershell
cargo run
cargo run -- https://www.youtube.com/
cargo run -- --cli https://www.youtube.com/
cargo test
cargo build
git status --short
git log --oneline -n 20

# AI branch merge loop (runs every 5 min, merges codex/* and claude/* if tests pass)
.\scripts\merge-loop.ps1 -IntervalSeconds 300
# Single cycle (dry-run preview)
.\scripts\merge-loop.ps1 -Once -DryRun
```

## Session Log

### 2026-05-14 - Codex

- Inspected the repo after user said Claude had advanced implementation during a context gap.
- Confirmed the repo has moved to the `tobira` name and the current branch head is `91cc671`.
- Confirmed `cargo test` is green with `74` passing tests.
- Added this handoff file and linked it from `README.md`.
- Established the rule that this file should be updated on every handoff / resume.

### 2026-05-14 - Codex (DOM / JS pass)

- Reworked `src/js.rs` so script execution runs against a lightweight mutable DOM instead of mostly fake stubs.
- Added DOM-backed support for selectors, element creation, child insertion/removal, `innerHTML`, `textContent`, `classList`, and ID/class mutation.
- Changed `document.write(...)` handling to mutate the DOM and recursively execute script tags written by scripts.
- Fixed a parsing correctness bug by teaching `src/html.rs` to keep raw-text contents for `script`, `style`, `title`, and `textarea`.
- Verified the current state with `cargo test` (`77` passing tests) and `cargo build`.

### 2026-05-14 - Codex (DOM demo follow-up)

- Added `demo/dom-demo.html` and `demo/dom-demo.js` to exercise the new DOM-backed JS path locally.
- Updated `README.md` so the documented JS scope matches the current implementation better and includes the new DOM demo command.

### 2026-05-14 - Codex (clipboard fix)

- Added address-bar clipboard support backed by the OS clipboard via `arboard`.
- `Ctrl+C`, `Ctrl+X`, and `Ctrl+V` now work against the current address-bar selection / insertion point.
- Added focused tests for selected-text and cut-selection behavior in `src/gui.rs`.

### 2026-05-15 - Codex (parallel branch workflow)

- Confirmed the current Codex branch is `codex/codex`.
- Recorded the new workflow: Codex and Claude may implement in parallel on separate branches, with merge reconciliation handled later through GitHub Copilot / the user's preferred merge flow.
- Future handoffs should always note the active branch before assuming current repo state.

### 2026-05-15 - Codex (JS runtime foundation pass)

- Moved `process_document_scripts` onto a dedicated larger-stack worker thread to reduce the chance of crashing on large bundles.
- Raised script execution budgets and removed the old pattern-based prefilter that used to skip `fetch` / `XMLHttpRequest` scripts outright.
- Added Promise job draining after top-level eval, Promise-backed `fetch`, and a minimal `XMLHttpRequest` object.
- Added JS navigation propagation so `location.href` changes can trigger a follow-up page load during initial script processing.
- Added DOM property reflection and `document.createTextNode()` support to improve dynamic script insertion and general DOM compatibility.

### 2026-05-15 - Copilot (merge-loop setup)

- Added `JS_ROADMAP.md` as the living plan for taking JavaScript support from lightweight and useful to browser-grade.
- Linked the roadmap from `README.md` so future sessions can find the priority order quickly.
- Created `scripts/merge-loop.ps1` — a PowerShell loop that runs every N seconds, finds unmerged `codex/*` + `claude/*` branches, runs `cargo test`, and merges passing ones into master.
  - Usage: `.\scripts\merge-loop.ps1 -IntervalSeconds 300` (default 5 min)
  - Flags: `-Once` (single cycle), `-DryRun` (no actual commit/push)
- Created `.github/workflows/ai-branch-merge-loop.yml` — GitHub Actions version that triggers on push to AI branches and on a 10-minute cron schedule.

### 2026-05-16 - Codex (event plumbing demo)

- Added a dedicated `demo/events-demo.html` / `demo/events-demo.js` page for verifying native page event plumbing.
- Updated the docs to reflect that bubbling DOM event dispatch covers `click`, `input`, `change`, `submit`, `keydown`, and `keyup`, while `focus` and `blur` remain target-only.
- Kept the roadmap and handoff notes in sync with the remaining capture-phase, richer listener option, and live-value reflection gaps.

### 2026-05-16 - Codex (keyboard event plumbing)

- Added page keyboard event dispatch for focused inputs so scripts can observe `keydown` and `keyup` before browser default actions run.
- Included keyboard metadata in the event payload (`key`, `code`, modifier flags, and `repeat`) and added demo logging for manual inspection.
- The next event-system gap is richer listener options and capture-phase dispatch, not basic key delivery.

### 2026-05-16 - Codex (keyboard roadmap step)

- Tightened the GUI event loop so focused page inputs receive `keydown` before default handling and `keyup` after the edit path finishes.
- Added a regression test that checks keyboard event metadata reaches JS listeners on the document.
- Updated the living roadmap and demo copy to treat keyboard delivery as a completed milestone and the next phase as richer listener options / capture phase.

### 2026-05-16 - Codex (branch switch after merge)

- Moved Codex work from `codex/codex` to a fresh branch, `codex/js-event-capture`, so the next JS/event slice can continue cleanly after the previous merge.
- Keep future Codex implementation work on this branch unless the user explicitly asks to switch again.

### 2026-05-16 - Codex (capture listener groundwork)

- Added capture-phase dispatch and `once` listener support to the DOM event bridge for ordinary page controls.
- Added regression tests for capture order, once-listener removal, and capture-sensitive `removeEventListener(...)`.
- Updated the roadmap, README, and event demo copy so the next session starts from the current event semantics instead of the pre-capture baseline.

### 2026-05-16 - Codex (live input sync)

- Removed the stale page-control value cache so rendered inputs now trust the DOM-backed `value` as the source of truth when they are not focused.
- Kept focused native editors authoritative during typing, while syncing their live text back into the DOM attribute on each edit path.
- Added a small regression test to lock in the focused-editor-vs-DOM value precedence.

### 2026-05-16 - Codex (merge prep checkpoint)

- Current branch `codex/js-event-capture` is clean and pushed with the latest live input sync work.
- PR #40 is the active merge target for the current JS/event progress checkpoint.
- The next likely follow-up after merge is storage/cookies and richer history/back-forward behavior.

### 2026-05-16 - Gemini (branch merge)

- Merged `codex/js-event-capture` into master, resolving conflicts in HANDOFF.md, README.md, and src/browser.rs.
- Also merging `claude/phase2-css` (position/z-index/flexbox) — in progress.
