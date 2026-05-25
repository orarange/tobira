# Handoff

This file is the canonical handoff note for this repo.
Update it whenever work switches between Codex, Claude, Gemini, Copilot, or a fresh session after a context reset.

## Handoff Rules

- Read this file, `git status --short`, and the latest `git log --oneline -n 20` before making assumptions.
- Confirm the current branch with `git branch --show-current` before starting work.
- Work in the branch / checkout the user has currently designated; do not assume a separate Claude/Codex split unless the user explicitly asks for one.
- CSS files may be edited when the current task genuinely needs it. Keep the change minimal, call out any non-trivial CSS touch in `change.md`, and prefer review before broadening a CSS-heavy diff.
- Update the `Current Snapshot` section whenever the high-level state changes.
- Append a short entry to `Session Log` whenever meaningful work is handed off or resumed.
- Do not stage unrelated local helper artifacts unless the user explicitly asks for them.
  Current local artifacts that are present but not part of the tracked repo are:
  `.claude/`, `.repomix/`, `copilot.md`, `gemini.md`, `repomix-output.xmlbrowser.xml`
- **PR title** — When opening a pull request, always include the agent's name in the title.
  Example: `[Claude] fix CSS calc() precedence` / `[Codex] add image lazy-loading`

## Current Snapshot

- Date: `2026-05-25`
- Repo / package name: `tobira`
- Working branch: `master`
- Workflow:
  - use the shared checkout the user pointed at unless a dedicated worktree is explicitly requested
  - keep the handoff notes current when switching between sessions or collaborating agents
- Verification status:
- `cargo test`: `200` passing tests on `2026-05-25`
- `cargo build`: success on `2026-05-25`
- North star / current goal:
  - Chromeと同程度の実用感を目指し、Google/YouTubeなどの複雑なサイトをsynthetic fallbackに頼らず閲覧・操作できるようにする
  - priority order: WebComponents / shadow DOM details -> DOM mutation to reflow / hit-test sync -> fetch/XHR / history / storage browser-grade behavior -> real-site stability checks
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
  - CSS Phase 5 baseline is treated as complete on the Claude `claude/phase5-css` branch; Codex should not duplicate the parser/layout engine and should treat Phase 6 as the remaining CSS surface.
  - software-rendered GUI with custom title bar and address bar
  - page loading now runs on a dedicated background worker and content rendering runs on a separate worker, so the window chrome stays responsive while pages load
  - no loading-screen UI; the chrome remains interactive and the content area updates when the background work finishes
  - blank startup page and direct URL entry
  - address bar editing shortcuts including `Ctrl+A`, `Ctrl+C`, `Ctrl+X`, and `Ctrl+V`
  - clickable links in the rendered page
  - first-class GUI page controls for:
    - text inputs
    - buttons
    - checkbox / radio toggles
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
  - Node introspection and mutation helpers are now much closer to browser DOM behavior:
    - `nodeType`, `nodeName`, `nodeValue`, `firstChild`, `lastChild`, `previousSibling`, `nextSibling`, `isConnected`
    - `cloneNode(...)`, `replaceChild(...)`, `removeChild(...)`
    - `append(...)`, `prepend(...)`, `before(...)`, `after(...)`, `replaceWith(...)`, `replaceChildren(...)`
    - `document.createDocumentFragment(...)` with fragment flattening on insertion
  - page event listeners now support capture + bubbling, plus `once` listeners and capture-sensitive `removeEventListener(...)`
  - shadow DOM / WebComponents now have `customElements`, `attachShadow(...)`, `slot.assignedNodes(...)` / `slot.assignedElements(...)` with `flatten`, `assignedSlot`, `slotchange`, and shadow-boundary event retargeting with `Event.composedPath()`
  - guarded JavaScript execution through `boa_engine`
  - lightweight mutable DOM bridge with:
    - `querySelector(...)`, `querySelectorAll(...)`, `getElementById(...)`
    - `createElement(...)`, `createTextNode(...)`
    - `appendChild(...)`, `insertBefore(...)`, `remove()`
    - dynamic `document.body`, `document.head`, and `document.documentElement`
    - `hasAttribute(...)`, `hasAttributes(...)`, `getAttributeNames(...)`, `toggleAttribute(...)`
    - `matches(...)`, `closest(...)`, `contains(...)`
    - `firstElementChild`, `lastElementChild`, `previousElementSibling`, `nextElementSibling`
    - `innerHTML`, `textContent`, `classList`, `id`, `className`
    - `classList.value`, `classList.length`, `classList.item(...)`, `classList.toString()`, `classList.replace(...)`
    - `element.attributes` as a live NamedNodeMap-style collection with `length`, `item(...)`, `getNamedItem(...)`, and array-like iteration
    - `document.write(...)` with recursive script expansion
    - DOM mutations serialized back into the HTML pipeline after JS runs
    - reflected `value`, `src`, `href`, `rel`, `type`, `name`, `content`
  - JS execution / runtime support for:
    - dedicated larger-stack worker thread
    - queued host-task plumbing for `queueMicrotask(...)`, `setTimeout(...)`, `setInterval(...)`, and `requestAnimationFrame(...)`
    - Promise job flushing (drained after top-level script eval via `context.run_jobs()`)
    - lightweight `fetch(...)` with response headers iteration
    - lightweight `XMLHttpRequest` with `getResponseHeader(...)` / `getAllResponseHeaders()`
    - loop-iteration runtime budget for runaway scripts
    - same-origin request and redirect guards
    - script-driven `location.href` follow-up navigation
    - origin-scoped `localStorage`, `sessionStorage`, and `document.cookie`
  - browser chrome history controls for back/forward navigation across full document loads
  - `location.reload()` and `history.go(0)` now request a reload of the current effective URL
  - `history.scrollRestoration` supports `auto` and `manual` to control back/forward scroll replay
  - browser-level history entries now remember scroll positions and restore them on back/forward
  - same-document history entries now expose `history.state` and dispatch `popstate` / `hashchange`
  - same-document history back/forward now restores the stored scroll position for each entry
  - browser chrome no longer blocks on page loading; navigation and rendering completion are delivered back to the UI thread through user events
  - layout cache invalidates on viewport width or page revision changes
  - GUI-driven DOM attribute updates now push a fresh runtime snapshot back into the page, so mutation notifications can invalidate reflow immediately
  - local demo pages under `demo/` for CSS, JS, DOM mutation, form handling, event plumbing, keyboard event logging, storage/cookies, and scroll control
  - layout injects synthetic `data-tobira-node-id` attributes so page events can target ordinary rendered elements
  - inline `element.style` mutations now reflect through `cssText`, `setProperty(...)`, and common style accessors for text, size, and border properties
  - `getComputedStyle(...)` snapshots now expose common layout-sensitive values for DOM-driven callers
  - site-specific rendering paths for:
    - YouTube watch pages
    - YouTube home shell / cards / nudge UI
    - lightweight Google shell
    - legacy frame/table-heavy pages such as the Abe Hiroshi site
  - generic YouTube home / non-watch pages now take a synthetic fast path before the heavy JS session so the app does not spin on the full app shell
  - generic `google.com` and `youtube.com` now try the real JS/HTML path before synthetic fallback
  - living JS roadmap tracked in `JS_ROADMAP.md`

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
- `src/site_state.rs`
  Shared origin-scoped storage and cookie registry used by HTTP and JS.

## Recent Commit Landmarks

- `1616499` mutation notifications and history scroll restoration implementation complete (Codex JS/Event capture)
- `e2558bf` docs: update HANDOFF + CSS_ROADMAP for Phase 5 completion (Claude Phase 5 CSS)
- `0e81ade` feat: Phase 5 Batch 6 — filter, ::placeholder/::selection, @supports/@layer, no-op props — PR #49
- `737409a` feat: Phase 5 Batch 5 — min/max-content, fit-content(), sticky, cursor, pointer-events
- `dccc1d1` feat: Phase 5 Batch 4 — CSS Grid layout (fr/repeat/auto-placement)
- `b14996d` feat: Phase 5 Batch 3 — inline-flex, align-content, flex-flow, :checked/:disabled
- `7ce1272` feat: Phase 5 Batch 2 — :hover/:focus/:active + element hitboxes + GUI re-layout
- `de7dbb5` feat: Phase 5 Batch 1 — clamp/min/max, aspect-ratio, object-fit, content:attr()
- `7af71f3` dom traversal api implementation complete (Codex JS/Event capture)
- `0cf8113` viewport sync and active element support complete (Codex JS/Event)
- `f51ddca` [Claude] fix: restore lost types, Copilot review fixes (form-context, clipping, offscreen, box-shadow) — PR #47 merged
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
  - GUI-to-page event delivery now covers capture + bubbling `click`, `input`, `change`, `submit`, `keydown`, and `keyup`, plus target-only `focus` and `blur`; passive listener semantics and `{ signal }` cancellation are in place, and `location.hash` plus `history.pushState(...)` / `replaceState(...)` now support soft navigation without a reload, while the rest of the option matrix and back/forward stack still need depth.
- Native page input typing now syncs `value` into the JS DOM.
- Text inputs and textareas now support browser-like selection APIs (`selectionStart`, `selectionEnd`, `selectionDirection`, `setSelectionRange(...)`, `select()`), and the focused GUI editor mirrors selection state back into the JS DOM.
- DOM traversal APIs now include `matches(...)`, `closest(...)`, `contains(...)`, and element sibling / child accessors for event delegation and framework-style code paths.
- The richer `attributes` / `dataset` surface still needs deeper parity, even though `element.attributes` is now a live collection and `hasAttributes(...)` / `toggleAttribute(...)` now exist.
- `MutationObserver` now fires for `attributes`, `childList`, and `characterData`, and the JS layer also exposes browser-style event constructors (`Event`, `CustomEvent`, `KeyboardEvent`, `InputEvent`, `MouseEvent`, `FocusEvent`, `SubmitEvent`) plus `AbortController` / `AbortSignal`.
- text nodes now expose browser-like `CharacterData` helpers including `data`, `length`, `nodeValue`, and `splitText(...)`.
- Framework-facing browser APIs still need a lot more depth.
- History / back-forward replay still needs depth beyond the current scroll restoration work.
- Script-driven scrolling now has basic window / DOM setter support, and full-document / same-document history scroll restoration is in place.
- Modern app-shell sites still need more DOM APIs, richer history replay, and CSS Phase 6 visual effects / advanced rendering.
- Incremental reflow still needs deeper invalidation for more DOM/style mutations.
- The inline style bridge still needs broader CSS property coverage and more computed-style parity to be browser-grade, but the core CSS parser/layout baseline is already part of the shared codebase.
- Form support is still limited to simple text-like fields and `GET` submission; `POST`, selects, and file inputs are not wired yet.
- The `XMLHttpRequest` shim is enough for lightweight callers, but prototype / `instanceof` semantics are still incomplete.
- Actual media playback and a true YouTube watch experience are still incomplete.
- CSS Phase 5 baseline is already part of the shared codebase; remaining CSS work is mostly the Phase 6 visual-effects / advanced-rendering surface.
- CSS Phase 6 items remain: `transform: scale/rotate` rendering, `animation`/`@keyframes`, `transition`, `filter: blur()` rendering, `grid-template-areas`, RTL text.
- JS support still needs storage/cookies, richer history/back-forward, and more DOM depth for app-shell sites.
- text node `characterData` mutation notifications and `splitText(...)` are now in place for common DOM edit flows.

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

### 2026-05-26 - Codex (checkbox / radio form controls)

- Added first-class checkbox and radio controls to the native page form pipeline so they now render, hit-test, toggle, and serialize through `GET` submissions.
- Added a browser-like `checked` accessor on DOM nodes that reflects the underlying attribute state, plus JS setter support so scripts can toggle checkable inputs.
- Expanded the local form demo to cover checkbox and radio interactions, and verified the update with `cargo test` (`208` passing tests) and `cargo build`.

### 2026-05-25 - Codex (shadow DOM / composed path)

- Added `customElements` lifecycle scaffolding plus `attachShadow(...)` support, slot assignment helpers, and `ShadowRoot` / `slot` accessors.
- Implemented shadow-boundary event retargeting and `Event.composedPath()` for composed events so WebComponents listeners see browser-like targets.
- Added regression coverage for custom element upgrade callbacks, attribute change callbacks, and shadow DOM host / slot behavior.
- Verified the updated state with `cargo test` (`200` passing tests) and `cargo build`.

### 2026-05-18 - Codex (Node / fragment DOM APIs)

- Added browser-grade Node accessors to the JS DOM bridge, including `nodeType`, `nodeName`, `nodeValue`, sibling accessors, and `isConnected` on document and element nodes.
- Added structural mutation helpers: `cloneNode(...)`, `replaceChild(...)`, `removeChild(...)`, `append(...)`, `prepend(...)`, `before(...)`, `after(...)`, `replaceWith(...)`, and `replaceChildren(...)`.
- Added `document.createDocumentFragment(...)` and fragment flattening during insertion so DOM batches behave more like a real browser.
- Verified the updated state with `cargo test` (`188` passing tests) and `cargo build`.

### 2026-05-19 - Codex (event loop / timer queue)

- Replaced the immediate timer / animation / microtask fallback path with queued host-task plumbing so callbacks do not reenter the current JS turn immediately.
- Added queued support for `queueMicrotask(...)`, `setTimeout(...)`, `setInterval(...)`, and `requestAnimationFrame(...)`, plus `clearTimeout(...)`, `clearInterval(...)`, and `cancelAnimationFrame(...)` handle cleanup.
- Added a regression test that confirms nested timeouts defer to the next turn instead of recursively firing in the same turn.
- Updated the README and JS roadmap so the documented JS runtime status matches the queued task behavior.
- Verified the updated state with `cargo test` (`193` passing tests) and `cargo build`.

### 2026-05-19 - Codex (characterData / splitText)

- Added browser-like `CharacterData` support for text nodes, including `data`, `length`, `nodeValue`, and `splitText(...)`.
- Updated `textContent` / `nodeValue` setters so text-node edits now emit `characterData` mutation records instead of only child-list churn.
- Added a regression test that confirms `MutationObserver` receives `characterData` changes and that `splitText(...)` preserves text-node sibling relationships.
- Updated the README and roadmap notes to reflect the deeper text-node DOM surface.
- Verified the updated state with `cargo test` (`193` passing tests) and `cargo build`.

### 2026-05-24 - Codex (async UI / background render)

- Moved page navigation into a background worker so the title bar and address bar remain responsive while page loading is in flight.
- Added a separate background render worker that produces content frames off the UI thread, then hands completed frames back through user events.
- Removed any loading-screen style UI; the chrome stays interactive and the content area updates when the async work completes.
- Verified the updated state with `cargo test` (`196` passing tests) and `cargo build`.

### 2026-05-24 - Codex (policy update)

- Relaxed the CSS-editing guardrail because the user explicitly said CSS may be touched when needed.
- Dropped the Claude/Codex branch-split assumption from the shared handoff rules so future work can follow the current shared branch/worktree the user designates.

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
- The next likely follow-up after merge is storage/cookies and richer history/back-forward behavior.

### 2026-05-16 - Gemini (branch merge)

- Merged `codex/js-event-capture` into master, resolving conflicts in HANDOFF.md, README.md, and src/browser.rs.
- Also merging `claude/phase2-css` (position/z-index/flexbox) — in progress.

### 2026-05-16 - Claude (CSS phase2 merge fix-up + Copilot review pass)

- Fixed deep merge regressions introduced when `claude/phase2-css` was merged into master (`10e3399`):
  - Restored `FormControlCommand` and `FormControlKind` type definitions that were lost in the merge.
  - Re-unified `merge_fragment` (bad conflict resolution had split it into two fragments, leaving controls-extend outside any function).
  - Fixed `layout_preformatted_fragments` Control arm (referenced undeclared variables from a different function).
  - Fixed `LayoutContext` initialization (missing `..LayoutContext::default()`).
  - Fixed `layout_block_element` / `layout_mixed_children` call sites (missing `current_form: None` argument).
  - Fixed `browser.rs` test `ComputedStyle` literals (missing `effective_opacity` field).
- Addressed 3 remaining Copilot issues flagged before the rate limit:
  - `BoxShadow.color`: changed `u32` → `Option<u32>` (None = inherit `currentColor`).
  - `TextCommand.line_height_px`: new field; `clip_commands_to_box` now clips on line height, not font size.
  - `MAX_OFFSCREEN_PIXELS` in `gui.rs`: reduced from 8192×8192 (268 MB) to 4096×4096 (64 MB).
- Ran 5 Copilot review rounds (PRs #42 → #43 → #44 → #46 → #47); each round fixed all flagged comments.
  - Final PR #47 merged with zero Copilot comments.
- `cargo test`: 134 passing, 0 failed.
- `CSS_ROADMAP.md` was missing from master (was on `claude/phase2-css` only); PR #48 (`claude/add-css-roadmap`) adds it.

### 2026-05-16 - Claude (Phase 5 CSS roadmap — full implementation)

Implemented all Phase 5 CSS roadmap items across 6 batches on `claude/phase5-css` (PR #49).

- **Batch 1** — CSS math + images:
  - `clamp()`, `min()`, `max()` in all length contexts, including nested inside `calc()`
  - `aspect-ratio` (milliratio u32 to keep `Eq`), applied in image layout
  - `object-fit` / `object-position` with 5 rendering modes in `draw_scaled_image`
  - `content: attr(name)` resolved from element attributes in `::before`/`::after`

- **Batch 2** — Interactive pseudo-classes + element hitboxes:
  - `:hover`, `:focus`, `:active` as real pseudo-classes threaded through the entire cascade
  - `InteractiveState` struct passed into `build_styled_tree` + selector matching
  - `ElementHitbox` emitted per block element → GUI hit-tests to find hovered node
  - `BrowserPage.relayout()` + GUI re-renders only when hovered node changes

- **Batch 3** — Flex extensions + form pseudo-classes:
  - `display: inline-flex`, `align-content`, `flex-flow` shorthand
  - `:checked`, `:disabled`, `:enabled` pseudo-classes

- **Batch 4** — CSS Grid layout:
  - Full `display: grid` / `display: inline-grid` with auto-placement engine
  - `grid-template-columns/rows`, `fr` units (two-pass), `repeat()`, `span N`
  - `grid-auto-rows/columns`, explicit line-number placement

- **Batch 5** — Intrinsic sizing + sticky + cursor:
  - `min-content`, `max-content`, `fit-content()` as `LengthValue` variants
  - `position: sticky` lays out as relative (scroll-offset tracking deferred)
  - `CursorKind` enum (14 variants), `pointer-events: none` gates hitboxes

- **Batch 6** — Filter + pseudo-elements + parser stubs:
  - `filter: blur(px)`, `brightness(f)`, `opacity(f)` parsed into dedicated fields
  - `::placeholder`, `::selection` parsed; `compute_placeholder_style()` API
  - `@supports` (always-true), `@layer` (name ignored), ~20 no-op properties

- `cargo test`: 157 passing (was 134 at start of session), 0 failed.
- CSS_ROADMAP.md updated: Phase 5 → ✅, Phase 6 future work documented.

### 2026-05-16 - Codex (storage and cookie support)

- Added origin-scoped `localStorage` and `sessionStorage` backed by shared site state.
- Added `document.cookie` getter/setter behavior and request/response cookie propagation in the HTTP layer.
- Added `demo/storage-demo.html` so storage and cookie state can be exercised manually.

### 2026-05-16 - Codex (browser history back/forward)

- Added browser-level history tracking for full document loads.
- Added back/forward chrome buttons and `Alt+Left` / `Alt+Right` shortcuts.
- Kept same-document soft navigation in sync with the browser history entry for the current page.

### 2026-05-25 - Codex (goal lock)

- Locked the north star in the roadmap and handoff notes: Chrome-level practicality so Google / YouTube / other complex sites can be browsed and operated without synthetic fallback pages.
- Reaffirmed the working order as WebComponents / shadow DOM details, DOM mutation to reflow / hit-test synchronization, fetch / XHR / history / storage browser-grade behavior, and real-site stability checks.

### 2026-05-25 - Codex (slotchange and assignedSlot)

- Added `assignedSlot` on nodes and synchronous `slotchange` dispatch when slot distribution changes.
- Kept the implementation local to the existing shadow DOM bridge so it stays aligned with the current WebComponents work.

### 2026-05-25 - Codex (flattened slot assignment helpers)

- Extended `slot.assignedNodes(...)` and `slot.assignedElements(...)` with a `flatten` option so nested slot trees can be traversed more like a real browser.
- Updated the roadmap and README to reflect the broader WebComponents surface.

### 2026-05-17 - Codex (DOM traversal & manipulation APIs)

- Added `matches(...)`, `closest(...)`, and `contains(...)` to the DOM bridge so selector-driven event delegation code can walk the tree without special cases.
- Added `firstElementChild`, `lastElementChild`, `previousElementSibling`, and `nextElementSibling` accessors for framework-style traversal.
- Added dynamic `document.body`, `document.head`, and `document.documentElement` getters to stay consistent as the DOM grows.
- Extended `classList` with live helpers (`value`, `length`, `item(...)`, `toString()`, `replace(...)`, `toggle(...)`).
- Added live NamedNodeMap-style `element.attributes` collection with `length`, `item(...)`, `getNamedItem(...)`, and array-like iteration.
- Added `hasAttribute(...)`, `hasAttributes(...)`, `getAttributeNames(...)`, and `toggleAttribute(...)` to elements.
- Added regression coverage for DOM traversal, sibling lookup, attributes collection, token list, and dynamic getters.

### 2026-05-17 - Codex (script-driven scroll & history scroll restore)

- Added `window.scrollTo(...)`, `window.scrollBy(...)`, and node `scrollTop` setter support, wired back into GUI viewport scroll state.
- Extended same-document and browser-level history entries to store and restore scroll positions on `history.back()` / `history.forward()`.
- Added `demo/scroll-demo.html` so the new scroll APIs can be exercised manually.

### 2026-05-17 - Codex (computed style, header and state APIs)

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

### 2026-05-17 - Codex (full-document history scroll restore)

- Extended browser-level history entries to store scroll positions, and restored them when navigating back and forward across document loads.
- Updated the browser history load path so scroll state is reapplied after a full document load when history demands it.
- Recorded the browser-level scroll restoration behavior in the README and roadmap notes.

### 2026-05-17 - Codex (computed style and DOM token list helpers)

- Added `getComputedStyle(...)` snapshots for common layout-sensitive values, including inherited color / font / spacing properties and shorthand box values.
- Extended `classList` with `value`, `length`, `item(...)`, `toString()`, `replace(...)`, and force-aware `toggle(...)`.
- Added `hasAttributes(...)` and `toggleAttribute(...)` on elements so scripts can introspect and flip attributes without manual DOM plumbing.
- Updated the README and roadmap notes to reflect the broader DOM / computed-style surface.

### 2026-05-17 - Codex (attribute collection live bridge)

- Added a live `element.attributes` collection with `length`, `item(...)`, `getNamedItem(...)`, named lookup, and array-like iteration support.
- Added regression coverage for attributes collection indexing, named lookup, and iteration order.
- Updated the README and roadmap notes to reflect that live attribute collection support is now available.

### 2026-05-17 - Codex (fetch/XHR response headers)

- Added response header iteration helpers to the lightweight fetch response surface.
- Added XHR `getResponseHeader(...)` and `getAllResponseHeaders()` support backed by the stored response header map.
- Added regression coverage for response header iteration plus XHR header access.

### 2026-05-17 - Codex (history state and hashchange/popstate)

- Added `history.state` support for same-document session history entries.
- Dispatched `popstate` on history back/forward and `hashchange` on same-document fragment changes.
- Added regression coverage for `hashchange` and `popstate` dispatch behavior.

### 2026-05-17 - Codex (YouTube synthetic fast path)

- Short-circuited generic YouTube home / non-watch loads to a synthetic shell before starting the heavy JS session.
- Kept the watch-page summary path intact while avoiding the runaway memory growth seen on the full YouTube app shell.
- Verified the new path with a process-memory smoke test that stabilized instead of growing without bound.

### 2026-05-16 - Codex (browser history back/forward)

- Added browser-level history tracking for full document loads.
- Added back/forward chrome buttons and `Alt+Left` / `Alt+Right` shortcuts.
- Kept same-document soft navigation in sync with the browser history entry for the current page.
