# Handoff

This file is the canonical handoff note for this repo.
Update it whenever work switches between Codex, Claude, Gemini, Copilot, or a fresh session after a context reset.

## Handoff Rules

- Read this file, `git status --short`, and the latest `git log --oneline -n 20` before making assumptions.
- Confirm the current branch with `git branch --show-current` before starting work.
- Codex must stay on the dedicated branch `codex/codex` unless the user explicitly changes that rule.
- Update the `Current Snapshot` section when the high-level state changes.
- Append a short entry to `Session Log` whenever you hand off or resume meaningful work.
- Claude work may happen on its own dedicated branch. Keep Codex changes isolated to `codex/codex` and let merge reconciliation happen later through GitHub Copilot / the user's chosen merge flow.
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
  - `cargo test` passes: `79` tests green on `2026-05-14`
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

### 2026-05-15 - Codex (fixed branch rule)

- Locked the documented Codex workflow to the dedicated branch `codex/codex`.
- Claude is expected to stay on its own separate branch so both agents can work in parallel without branch drift.
