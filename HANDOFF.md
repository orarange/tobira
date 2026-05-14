# Handoff

This file is the canonical handoff note for this repo.
Update it whenever work switches between Codex, Claude, Gemini, Copilot, or a fresh session after a context reset.

## Handoff Rules

- Read this file, `git status --short`, and the latest `git log --oneline -n 20` before making assumptions.
- Confirm the current branch with `git branch --show-current` before starting work.
- Update the `Current Snapshot` section when the high-level state changes.
- Append a short entry to `Session Log` whenever you hand off or resume meaningful work.
- Parallel work with Claude may happen on a different branch. Keep Codex changes isolated to the current branch and let merge reconciliation happen later through GitHub Copilot / the user's chosen merge flow.
- Do not stage unrelated local helper artifacts unless the user explicitly asks for them.
  Current local artifacts that are present but not part of the tracked repo are:
  `.claude/`, `.repomix/`, `copilot.md`, `gemini.md`, `repomix-output.xmlbrowser.xml`

## Current Snapshot

- Date: `2026-05-15`
- Repo / package name: `tobira`
- Active branch seen by Codex: `codex/codex`
- App identity in code:
  - Cargo package: `tobira`
  - window title prefix: `Tobira`
  - README was previously under the old `Scratch Browser` name
- Verification status:
  - `cargo test` passes: `80` tests green on `2026-05-15`
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
  - clickable links in the rendered page with hit-testing in the GUI
  - image loading / rendering for supported formats
  - guarded JavaScript execution through `boa_engine` with a growing set of stubs
  - lightweight mutable DOM bridge for script execution
    - real `document.querySelector(...)` / `querySelectorAll(...)`
    - `getElementById(...)`
    - `createElement(...)`
    - `appendChild(...)` / `insertBefore(...)` / `remove()`
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

- `91cc671` Merge branch `claude/modest-pascal-9bf652`
- `7fda6c9` fix `@media` brace parsing, `calc()` precedence, `rgba` blending, add 15 tests
- `9c2e24b` comprehensive CSS support implementation complete
- `373dd0c` step 1: relax JS filter to block-list, add crypto/cookie/URLSearchParams stubs
- `ba0df47` tobira rename and youtube UI improvements complete
- `9e69220` link click navigation and youtube card interactivity implementation complete
- `4be7625` youtube home ui rendering implementation complete

## Known Gaps / Likely Next Work

- README capability list is partially stale; prefer this file for the latest snapshot.
- JS support is still far from a full browser DOM / framework runtime.
- Event dispatch, `addEventListener` depth, and async network-backed browser APIs are still mostly stubbed.
- `fetch` / `XMLHttpRequest` are still partial and currently practical mainly for simple GET flows.
- History / back-forward behavior is not yet called out as complete.
- Modern app-shell sites may still need more DOM APIs, events, storage behavior, and CSS coverage.
- Actual media playback and a true YouTube watch experience are still incomplete.

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

- Reviewed and verified Codex's JS runtime improvements (browser.rs + js.rs working-tree changes).
- `cargo test` confirmed all 80 tests green before merging.
- Committed changes as `3930a90` into master: script navigation depth limiting, 32 MB worker thread stack, larger script budgets, Promise job flush, `createTextNode`, DOM property accessors (`value/src/href/rel/type/name/content/parentElement/ownerDocument`).
- Created `scripts/merge-loop.ps1` — a PowerShell loop that runs every N seconds, finds unmerged `codex/*` + `claude/*` branches, runs `cargo test`, and merges passing ones into master.
  - Usage: `.\scripts\merge-loop.ps1 -IntervalSeconds 300` (default 5 min)
  - Flags: `-Once` (single cycle), `-DryRun` (no actual commit/push)
- Created `.github/workflows/ai-branch-merge-loop.yml` — GitHub Actions version that triggers on push to AI branches and on a 10-minute cron schedule.
- Next recommended work: event dispatch (`dispatchEvent`, bubbling), `localStorage`/`sessionStorage` stubs, history API (`pushState`/`replaceState`), `ResizeObserver` stub.
