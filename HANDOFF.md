# Handoff

This file is the canonical handoff note for this repo.
Update it whenever work switches between Codex, Claude, Gemini, Copilot, or a fresh session after a context reset.

## Handoff Rules

- Read this file, `git status --short`, and `git log --oneline -n 20` before making assumptions.
- Confirm the current branch with `git branch --show-current` before starting work.
- Codex must stay on branch `codex/codex` unless the user explicitly changes that rule.
- Codex should use a dedicated worktree for `codex/codex` instead of sharing the user's main checkout.
- Keep Codex changes isolated to `codex/codex`; Claude may work on its own branch and merge reconciliation happens later through GitHub Copilot or the user's preferred flow.
- Update the `Current Snapshot` section whenever the high-level state changes.
- Append a short entry to `Session Log` whenever meaningful work is handed off or resumed.
- Do not stage unrelated local helper artifacts unless the user explicitly asks for them.
  Current known local-only artifacts:
  `.claude/`, `.repomix/`, `copilot.md`, `gemini.md`, `repomix-output.xmlbrowser.xml`

## Current Snapshot

- Date: `2026-05-16`
- Repo / package name: `tobira`
- Active Codex branch: `codex/codex`
- Workflow:
  - keep the shared root checkout free for the user / Claude side
  - run Codex implementation from a separate `codex/codex` worktree
- Verification status:
  - `cargo test`: `102` passing tests on `2026-05-16`
  - `cargo build`: success on `2026-05-16`
- Current implementation highlights:
  - hand-rolled `http://` and `https://` client with redirects and compressed response decoding
  - custom HTML parser and DOM-like tree
  - CSS engine with:
    - descendant / child selectors
    - attribute selectors
    - `:first-child`, `:last-child`, `:nth-child(...)`, `:not(...)`
    - `@media`
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
  - guarded JavaScript execution through `boa_engine`
  - lightweight mutable DOM bridge with:
    - `querySelector(...)`, `querySelectorAll(...)`, `getElementById(...)`
    - `createElement(...)`, `createTextNode(...)`
    - `appendChild(...)`, `insertBefore(...)`, `remove()`
    - `innerHTML`, `textContent`, `classList`, `id`, `className`
    - reflected `value`, `src`, `href`, `rel`, `type`, `name`, `content`
    - recursive `document.write(...)`
  - JS runtime support for:
    - Promise job flushing
    - lightweight `fetch(...)`
    - lightweight `XMLHttpRequest`
    - loop-iteration runtime budget for runaway scripts
    - same-origin request and redirect guards
    - script-driven `location.href` follow-up navigation
  - local demo pages under `demo/` for CSS, JS, DOM mutation, and form handling
  - site-specific rendering paths for:
    - YouTube watch pages
    - YouTube home shell fallback
    - lightweight Google shell fallback
    - legacy frame/table-heavy pages such as the Abe Hiroshi site
  - generic `google.com` and `youtube.com` now try the real JS/HTML path before synthetic fallback

## Important Modules

- `src/browser.rs`
  Main page-loading pipeline, fallback heuristics, legacy page handling, site-specific rewrites.
- `src/css.rs`
  CSS parser, selector matching, cascade, `@media`, `calc(...)`, color parsing.
- `src/layout.rs`
  Layout pipeline, text flow, tables, images, inline controls, hitbox generation.
- `src/gui.rs`
  Custom chrome, address bar state, page control input handling, painting, hover/click navigation.
- `src/js.rs`
  JS runtime policy, DOM bridge, fetch/XHR shims, navigation handling.
- `src/http.rs`
  HTTP/TLS fetch layer and browser-like request headers.
- `src/html.rs`
  Hand-rolled HTML parser with raw-text preservation for `script` / `style` / `title` / `textarea`.

## Recent Commit Landmarks

- `6981cea` dedicated codex worktree setup documentation complete
- `5952827` page form controls feature implementation complete
- `c5266c1` copilot review round two fixes complete
- `8cb6455` copilot followup cleanup fixes complete
- `51b60ed` copilot review security fixes complete
- `fd5c362` real js first host pipeline update complete
- `d159cf0` dom backed javascript support implementation complete
- `8751537` address bar clipboard support implementation complete

## Known Gaps / Likely Next Work

- JS support is still far from a full browser DOM / framework runtime.
- GUI-to-page event delivery is still shallow; forms are native-painted controls, not true DOM event targets yet.
- `addEventListener`, richer event types, and framework-facing browser APIs still need a lot more depth.
- History / back-forward behavior is not yet complete.
- Modern app-shell sites still need more DOM APIs, cookies/storage, and CSS coverage.
- CSS is still computed once up front instead of being rebuilt against the live window width.
- Form support is still limited to simple text-like fields and `GET` submission; `POST`, checkboxes, radios, and file inputs are not wired yet.
- JS `input.value` reflection still targets the backing attribute/default value, not the live focused editor state after GUI typing begins.
- The `XMLHttpRequest` shim is enough for lightweight callers, but prototype / `instanceof` semantics are still incomplete.
- Actual media playback and a true YouTube watch experience are still incomplete.
- `fetch(...)` / `XMLHttpRequest` remain intentionally conservative; cross-origin app shells are still blocked until a safer policy exists.

## Useful Commands

```powershell
cargo run
cargo run -- https://www.google.com/
cargo run -- https://www.youtube.com/
cargo run -- --cli https://www.google.com/
python -m http.server 8765
cargo run -- http://127.0.0.1:8765/demo/forms-demo.html
cargo test
cargo build
git status --short
git log --oneline -n 20
git worktree list
```

## Session Log

### 2026-05-14 - Codex

- Inspected the repo after a context gap and confirmed the project had already moved to the `tobira` name.
- Added the original handoff file and linked it from `README.md`.
- Reworked `src/js.rs` around a lightweight mutable DOM instead of mostly fake JS stubs.
- Added DOM-backed support for selectors, element creation, child insertion/removal, `innerHTML`, `textContent`, `classList`, and recursive `document.write(...)`.
- Added `demo/dom-demo.html` and `demo/dom-demo.js`.
- Added address-bar clipboard support backed by the OS clipboard.

### 2026-05-15 - Codex (real JS-first pipeline)

- Cherry-picked the larger-stack JS worker, Promise job flushing, `fetch`, `XMLHttpRequest`, `createTextNode`, DOM property reflection, and script-driven navigation handling onto `codex/codex`.
- Relaxed the browser pipeline so generic `google.com` and `youtube.com` try the real JS/HTML path first and only fall back when the post-script body is still effectively empty.
- Addressed Copilot review rounds around same-origin checks, redirect blocking, request getter errors, and XHR bootstrap/error behavior.
- Added `Url::shares_origin(...)`, limited HTTP fetches for JS, and regression coverage around same-origin request policy.

### 2026-05-15 - Codex (page form controls pass)

- Promoted page inputs and buttons to first-class layout commands so the GUI can hit-test and paint them separately from static text.
- Added native rendering for page text inputs and buttons, including focus border, caret, selection highlight, placeholder text, clipboard shortcuts, and IME placement.
- Added basic `GET` form submission with relative action resolution and query-string encoding.
- Added `demo/forms-demo.html` and regression tests for form-control emission and GET form URL building.

### 2026-05-15 - Codex (dedicated worktree setup)

- Moved the shared root checkout back to `master`.
- Created a dedicated Codex worktree on branch `codex/codex`.
- Future Codex implementation and review work should happen from that dedicated worktree so local file edits no longer collide with Claude's branch checkout or the user's main shell.

### 2026-05-15 - Codex (PR #25 follow-up fixes)

- Fixed the inline-control wrap branch in `src/layout.rs` and removed duplicate layout-time control rectangle painting so GUI controls render from a single source of truth.
- Reduced page-form submission overhead to a single layout pass, fixed empty button-submission values, and surfaced unsupported non-GET form methods in the GUI status line.
- Made `location.href` assignments resolve relative URLs against the immutable document URL for consistency with the same-origin security model.
- Added regression coverage for same-origin URLs with explicit default ports and repeated `location.href` updates resolved from the original document URL.

### 2026-05-16 - Codex (PR #29 Copilot follow-up)

- Replaced saturating form/control ID allocation with checked overflow guards so pathological layouts fail fast instead of silently reusing IDs.
- Fixed `Url::resolve(...)` for fragment-only and query-only targets when the current URL already carries a fragment.
- Added regression coverage for fragment-preserving GET form submissions and fragmented base-URL resolution.
- Added a `boa_engine` loop-iteration runtime budget so runaway `for` / `while` scripts bail out with a JS error instead of hanging the browser worker indefinitely.
