# Handoff

This file is the canonical handoff note for this repo.
Update it whenever work switches between Codex, Claude, Gemini, Copilot, or a fresh session after a context reset.

## Handoff Rules

- Read this file, `git status --short`, and `git log --oneline -n 20` before making assumptions.
- Confirm the current branch with `git branch --show-current` before starting work.
- Codex must stay on the active Codex branch listed below unless the user explicitly changes that rule.
- Codex should use a dedicated worktree for the active Codex branch instead of sharing the user's main checkout.
- Keep Codex changes isolated to the active Codex branch; Claude may work on its own branch and merge reconciliation happens later through GitHub Copilot or the user's preferred flow.
- CSS boundary:
  - treat the Claude `claude/phase5-css` branch as the owner of the CSS parser/layout baseline
  - do not edit CSS-engine files or other Claude-owned CSS work by default
  - if a JS task genuinely needs CSS-facing integration, keep the change minimal and non-destructive, open/update a PR, request Copilot review before broadening the diff, and log the exact touched files plus the reason in `change.md`
  - read-only inspection of CSS files is fine; destructive or broad CSS edits are not
- Update the `Current Snapshot` section whenever the high-level state changes.
- Append a short entry to `Session Log` whenever meaningful work is handed off or resumed.
- Do not stage unrelated local helper artifacts unless the user explicitly asks for them.
  Current known local-only artifacts:
  `.claude/`, `.repomix/`, `copilot.md`, `gemini.md`, `repomix-output.xmlbrowser.xml`

## Current Snapshot

- Date: `2026-05-17`
- Repo / package name: `tobira`
- Active Codex branch: `codex/js-event-capture`
- Workflow:
  - keep the shared root checkout free for the user / Claude side
  - run Codex implementation from a separate `codex/js-event-capture` worktree
- Verification status:
  - `cargo test`: `126` passing tests on `2026-05-17`
  - `cargo build`: success on `2026-05-17`
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
  - CSS Phase 5 baseline is treated as complete on the Claude `claude/phase5-css` branch; Codex should not duplicate the parser/layout engine and should treat Phase 6 as the remaining CSS surface.
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
  - page and viewport state now stay in sync through JS-facing accessors for:
    - `window.innerWidth` / `window.innerHeight`
    - `window.scrollY` / `window.pageYOffset`
    - `document.activeElement`
    - `window.scrollTo(...)`, `window.scrollBy(...)`, and `scrollTop` setters on DOM nodes
  - page event listeners now support capture + bubbling, plus `once` listeners and capture-sensitive `removeEventListener(...)`
  - guarded JavaScript execution through `boa_engine`
  - lightweight mutable DOM bridge with:
    - `querySelector(...)`, `querySelectorAll(...)`, `getElementById(...)`
    - `createElement(...)`, `createTextNode(...)`
    - `appendChild(...)`, `insertBefore(...)`, `remove()`
    - dynamic `document.body`, `document.head`, and `document.documentElement`
    - `hasAttribute(...)`, `getAttributeNames(...)`
    - `matches(...)`, `closest(...)`, `contains(...)`
    - `firstElementChild`, `lastElementChild`, `previousElementSibling`, `nextElementSibling`
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
    - origin-scoped `localStorage`, `sessionStorage`, and `document.cookie`
  - browser chrome history controls for back/forward navigation across full document loads
  - same-document history back/forward now restores the stored scroll position for each entry
  - layout cache invalidates on viewport width or page revision changes
  - GUI-driven DOM attribute updates now push a fresh runtime snapshot back into the page, so mutation notifications can invalidate reflow immediately
  - local demo pages under `demo/` for CSS, JS, DOM mutation, form handling, event plumbing, keyboard event logging, storage/cookies, and scroll control
  - layout injects synthetic `data-tobira-node-id` attributes so page events can target ordinary rendered elements
  - inline `element.style` mutations now reflect through `cssText`, `setProperty(...)`, and common style accessors for text, size, and border properties
  - site-specific rendering paths for:
    - YouTube watch pages
    - YouTube home shell fallback
    - lightweight Google shell fallback
    - legacy frame/table-heavy pages such as the Abe Hiroshi site
  - generic `google.com` and `youtube.com` now try the real JS/HTML path before synthetic fallback
  - living JS roadmap tracked in `JS_ROADMAP.md`

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
- `src/site_state.rs`
  Shared origin-scoped storage and cookie registry used by HTTP and JS.
- `src/html.rs`
  Hand-rolled HTML parser with raw-text preservation for `script` / `style` / `title` / `textarea`.

## Recent Commit Landmarks

- `7af71f3` dom traversal api implementation complete
- `6981cea` dedicated codex worktree setup documentation complete
- `c64f16a` event listener capture groundwork complete
- `d864ed6` codex branch switch handoff update complete
- `5952827` page form controls feature implementation complete
- `c5266c1` copilot review round two fixes complete
- `8cb6455` copilot followup cleanup fixes complete
- `51b60ed` copilot review security fixes complete
- `fd5c362` real js first host pipeline update complete
- `d159cf0` dom backed javascript support implementation complete
- `8751537` address bar clipboard support implementation complete
- `18f2be6` copilot review runtime limit and fragment fixes complete
- `1df11f6` live input value sync implementation complete
- `0cf8113` viewport sync and active element support complete

## Known Gaps / Likely Next Work

- JS support is still far from a full browser DOM / framework runtime.
- GUI-to-page event delivery now covers capture + bubbling `click`, `input`, `change`, `submit`, `keydown`, and `keyup`, plus target-only `focus` and `blur`; passive listener semantics are in place, and `location.hash` plus `history.pushState(...)` / `replaceState(...)` now support soft navigation without a reload, while the rest of the option matrix and back/forward stack still need depth.
- Native page input typing now syncs `value` into the JS DOM.
- DOM traversal APIs now include `matches(...)`, `closest(...)`, `contains(...)`, and element sibling / child accessors for event delegation and framework-style code paths.
- The richer `attributes` / `dataset` surface still needs deeper parity.
- Framework-facing browser APIs still need a lot more depth.
- History / back-forward replay and scroll restoration still need depth.
- Script-driven scrolling now has basic window / DOM setter support, but full scroll restoration across history still needs depth.
- Modern app-shell sites still need more DOM APIs, richer history replay, and CSS Phase 6 visual effects / advanced rendering.
- Incremental reflow still needs deeper invalidation for more DOM/style mutations.
- The inline style bridge still needs broader CSS property coverage and computed-style parity to be browser-grade, but the core CSS parser/layout baseline is already considered done on the Claude branch.
- Form support is still limited to simple text-like fields and `GET` submission; `POST`, checkboxes, radios, and file inputs are not wired yet.
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
cargo run -- http://127.0.0.1:8765/demo/events-demo.html
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

### 2026-05-16 - Codex (JS roadmap)

- Added `JS_ROADMAP.md` as the living plan for taking JavaScript support from lightweight and useful to browser-grade.
- Linked the roadmap from `README.md` so future sessions can find the priority order quickly.

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

### 2026-05-16 - Codex (viewport, focus, and scroll sync)

- Wired GUI viewport size changes into the JS runtime so `window.innerWidth` / `window.innerHeight` stay current and `resize` listeners fire on actual browser resizes.
- Added JS-visible focus state through `document.activeElement` and `document.hasFocus()`-style behavior for the currently focused page control.
- Exposed `window.scrollY`, `window.pageYOffset`, and `scrollTop`-style DOM accessors, plus `scroll` events when the user scrolls the GUI.
- Added regression coverage for viewport resize, focus / blur, and scroll event handling.

### 2026-05-16 - Codex (branch switch after merge)

- Moved Codex work from `codex/codex` to a fresh branch, `codex/js-event-capture`, so the next JS/event slice can continue cleanly after the previous merge.
- Keep future Codex implementation work on this branch unless the user explicitly asks to switch again.

### 2026-05-16 - Codex (layout reflow cache)

- Added a lightweight layout cache keyed by viewport width and page revision.
- Invalidated cached layout when JS-driven DOM snapshots change the page content.
- Updated the README, roadmap, and handoff notes to reflect the incremental reflow work.

### 2026-05-16 - Codex (inline style bridge)

- Added a native `element.style` bridge that reflects inline CSS through `cssText`, `setProperty(...)`, `getPropertyValue(...)`, and common style accessors.
- Added a regression test that checks inline style mutations serialize back into the DOM snapshot.

### 2026-05-16 - Codex (style property matrix expansion)

- Expanded the inline style bridge to cover more text, size, and border-related properties that the current layout engine already understands.
- Added regression coverage for the expanded style accessors and the browser-facing serialization path.

### 2026-05-16 - Codex (CSS boundary clarification)

- Confirmed on the Claude `claude/phase5-css` branch that the broad CSS parser/layout foundation should be treated as complete for this repo.
- Reframed the remaining CSS work for Codex as Phase 6 visual effects / advanced rendering and JS-driven reflow integration, not parser/layout duplication.

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
- The next likely follow-up after merge is richer history/back-forward behavior and replay across document loads.

### 2026-05-16 - Codex (storage and cookie support)

- Added origin-scoped `localStorage` and `sessionStorage` backed by shared site state.
- Added `document.cookie` getter/setter behavior and request/response cookie propagation in the HTTP layer.
- Added `demo/storage-demo.html` so storage and cookie state can be exercised manually.

### 2026-05-16 - Codex (browser history back/forward)

- Added browser-level history tracking for full document loads.
- Added back/forward chrome buttons and `Alt+Left` / `Alt+Right` shortcuts.
- Kept same-document soft navigation in sync with the browser history entry for the current page.

### 2026-05-17 - Codex (DOM traversal APIs)

- Added `matches(...)`, `closest(...)`, and `contains(...)` to the DOM bridge so selector-driven event delegation code can walk the tree without special cases.
- Added `firstElementChild`, `lastElementChild`, `previousElementSibling`, and `nextElementSibling` accessors for framework-style traversal.
- Added a regression test that exercises selector matching, ancestor lookup, containment, and sibling traversal together on a nested DOM tree.

### 2026-05-17 - Codex (attribute introspection)

- Added `hasAttribute(...)` and `getAttributeNames(...)` to the DOM bridge so scripts can inspect element attributes without special casing.
- Kept the new work CSS-neutral and stayed on the `codex/js-event-capture` branch.
- Updated the JS roadmap to put attribute/introspection helpers before harder browser gaps like history replay and reflow invalidation.

### 2026-05-17 - Codex (DOM traversal APIs)

- Added `matches(...)`, `closest(...)`, and `contains(...)` to the lightweight DOM bridge so event delegation code can inspect and climb the tree without special cases.
- Added `firstElementChild`, `lastElementChild`, `previousElementSibling`, and `nextElementSibling` accessors so framework-style traversal paths can read the surrounding element structure.
- Added a regression test that exercises selector matching, ancestor lookup, containment, and sibling traversal together on a small nested DOM tree.

### 2026-05-17 - Codex (script-driven scroll APIs)

- Added `window.scrollTo(...)`, `window.scrollBy(...)`, and `scrollTop` setter support so scripts can move the viewport directly.
- Wired JS scroll changes back into the GUI scroll state so the rendered page and `window.scrollY` stay aligned.
- Added regression coverage for scroll-position getters, setters, and scroll-driven event handling.

### 2026-05-17 - Codex (scroll demo page)

- Added `demo/scroll-demo.html` so the new scroll APIs can be exercised manually without digging through source code.
- The demo uses a tall DOM tree plus buttons for `scrollTo`, `scrollBy`, and `scrollTop` setter checks.

### 2026-05-17 - Codex (CSS boundary policy)

- Defined a clearer boundary for CSS work: treat the Claude `claude/phase5-css` branch as the CSS parser/layout owner and avoid broad or destructive CSS edits from Codex.
- Documented the exception workflow for JS tasks that genuinely need CSS-facing integration: keep the diff minimal, request Copilot review, and log touched files in `change.md`.
- Kept the current update CSS-neutral; this change only tightened coordination rules and documentation.

### 2026-05-17 - Codex (dynamic document root getters)

- Converted `document.body`, `document.head`, and `document.documentElement` to dynamic getters so they stay consistent if the DOM is extended after load.
- Added a regression test that creates body/head nodes after startup and verifies the getters track the live tree.
- Updated the roadmap and README to reflect the current DOM consistency surface.

### 2026-05-17 - Codex (mutation snapshot refresh)

- Made GUI-driven DOM attribute writes refresh the live page snapshot so mutation notifications can bump layout revision and invalidate cached reflow immediately.
- Added a regression test that mutates the root element, then verifies the refreshed page snapshot and layout revision update together.
- Recorded the new snapshot-refresh behavior in the README and roadmap notes.

### 2026-05-17 - Codex (same-document history scroll restore)

- Extended same-document history entries to store scroll positions, and restored them on `history.back()` / `history.forward()`.
- Added a regression test that walks a same-document history stack and verifies the stored scroll position comes back with each entry.
- Updated the README and roadmap notes to mention same-document scroll restoration.

### 2026-05-16 - Codex (browser history back/forward)

- Added browser-level history tracking for full document loads.
- Added back/forward chrome buttons and `Alt+Left` / `Alt+Right` shortcuts.
- Kept same-document soft navigation in sync with the browser history entry for the current page.
