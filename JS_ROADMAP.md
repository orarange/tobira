# JS Roadmap

This document is the living roadmap for making Tobira's JavaScript support "browser-grade" instead of "lightweight and useful".

## What "Perfect" Means Here

For this project, "perfect JS" does not mean implementing every single Web Platform API.
It means:

- common modern sites stop crashing or hanging
- forms, buttons, search boxes, and navigation work naturally
- Google, YouTube, and similar app-shell sites can be browsed and operated
- scripts can mutate the DOM, trigger events, fetch data, and update UI without needing special-case rewrites

## Current Baseline

Already working:

- inline and external scripts
- recursive `document.write(...)`
- lightweight DOM mutation helpers
- live `element.attributes` collection with `length`, `item(...)`, `getNamedItem(...)`, and array-like iteration
- browser-grade Node accessors and mutation helpers such as `nodeType`, `nodeName`, `nodeValue`, sibling traversal, `cloneNode(...)`, `replaceChild(...)`, `removeChild(...)`, `append(...)`, `prepend(...)`, `before(...)`, `after(...)`, `replaceWith(...)`, `replaceChildren(...)`
- `document.createDocumentFragment(...)` with fragment flattening during insertion
- native GUI typing stays in sync with live DOM `input.value`
- basic DOM event plumbing for capture + bubbling `click`, `input`, `change`, and `submit`, plus target-only `focus` and `blur`
- `Promise` job flushing
- guarded `XMLHttpRequest`
- **async `fetch(...)`**: returns a pending promise immediately, runs HTTP off-thread, and resolves through a completion queue drained during settle; event-handler-triggered fetches re-enter the session via a GUI `JsSettleRequired` watcher instead of blocking the window
- `MutationObserver` (attributes / childList / characterData), `ResizeObserver`, and `IntersectionObserver` firing from layout / viewport / scroll updates
- layout hitboxes fed back into the loaded JS session after reflow so observer geometry stays current
- same-origin request method/body support plus plain-object request headers for `fetch(...)` and `XMLHttpRequest`
- response header iteration plus XHR `getResponseHeader(...)` / `getAllResponseHeaders()`
- same-origin navigation checks
- loop-iteration runtime budget for runaway scripts
- native GUI form controls for `GET` submissions
- GUI hover transitions now dispatch `mouseover` / `mouseout` / `mouseenter` / `mouseleave` with `relatedTarget`
- passive listener semantics
- `location.hash`, `history.pushState(...)`, `replaceState(...)`, `back()`, and `forward()` for same-document navigation
- `history.state`, `popstate`, and `hashchange` for same-document session history changes
- same-document history back/forward now restores stored scroll positions
- browser-level back/forward navigation across document loads
- browser-level history entries now also remember scroll positions across document loads
- generic YouTube / Google / app-shell pages stay on the regular HTML/JS path instead of a synthetic fast path
- layout cache invalidation keyed by viewport width and page revision
- scroll events that do not change the DOM now keep the current layout cache instead of forcing a rebuild
- JS-visible viewport and focus state are wired up through `window.innerWidth` / `window.innerHeight`, `window.scrollY` / `window.pageYOffset`, and `document.activeElement`
- basic script-driven scrolling APIs now exist through `window.scrollTo(...)`, `window.scrollBy(...)`, and `scrollTop` setters on DOM nodes
- inline style mutations now reflect back into the DOM snapshot
- the inline style bridge now exposes more text, size, and border-related properties
- `getComputedStyle(...)` snapshots now expose common layout-sensitive values
- `toggleAttribute(...)` and richer `classList` helpers (`value`, `length`, `item(...)`, `toString()`, `replace(...)`) are in place
- GUI-driven DOM attribute mutations now refresh the live page snapshot so reflow invalidation can happen immediately after mutation notifications

CSS baseline note:

- the broad CSS parser / selector / cascade / computed-style foundation is treated as complete on the Claude `claude/phase5-css` branch and has already been merged into master
- Codex may now touch CSS when it helps the current goal, but should keep any overlap with active Claude work small, use the `claude` command when a targeted follow-up is useful, and log meaningful overlap in `change.md`
- the remaining CSS work is mostly Phase 6 visual effects / advanced rendering rather than parser/layout duplication

Still missing or shallow:

- richer networking semantics, especially `Headers` / `Request` / `Response` parity and more request-body shapes
- session-history replay polish across full document loads
- async browser APIs that modern frameworks expect
- rendering invalidation and layout reflow after DOM mutation still need deeper incremental invalidation
- the style bridge still needs the rest of the CSS property matrix and more computed-style parity
- remaining CSS work is mostly Phase 6 visual effects / advanced rendering, not the core parser/layout baseline

## Execution Order (Simple -> Hard)

If we want to keep momentum and avoid getting stuck on the biggest browser gaps too early, the practical implementation order is:

1. attribute / DOM introspection helpers like `hasAttribute(...)`, `hasAttributes(...)`, `getAttributeNames(...)`, `toggleAttribute(...)`, live `element.attributes`, and broader property reflection
2. event-delegation helpers like `matches(...)`, `closest(...)`, `contains(...)`, and element traversal accessors
3. basic listener-option edge cases and default-action sequencing
4. `document.body` / `document.head` / `document.documentElement` consistency and `innerHTML` edge cases
5. mutation notifications plus incremental reflow invalidation for DOM and style changes
6. same-document and full-document history replay polish, including scroll restoration
7. fetch / XHR semantics and safer cross-origin handling, including method / body / header handling
8. Google / YouTube / app-shell compatibility smoke tests
9. media and advanced APIs

The roadmap below still keeps the big browser areas grouped by phase, but the list above is the preferred order when we need the next easiest high-impact task.

## Risk Matrix

If we are choosing what is most likely to block "modern browser-like JS" first, this is the practical risk order:

| Rank | Bottleneck | Why it is risky | Typical symptom | Preferred countermeasure |
| --- | --- | --- | --- | --- |
| 1 | Event loop / reentrancy | Promise jobs, timers, DOM events, and callbacks can recurse into each other in surprising ways | Pages freeze, handlers run out of order, or one callback starves the rest | Keep the microtask / task / event sequencing explicit and small enough to reason about |
| 2 | DOM mutation -> reflow / repaint / hit-test sync | DOM changes only matter if layout, focus, and clicks are recomputed afterward | The page looks updated but clicks land in the wrong place, or the UI visually drifts | Make invalidation cheap and deterministic, and reflow only the affected subtree or page revision slice |
| 3 | Network semantics | `fetch` / `XHR` behavior is very site-dependent and easy to get subtly wrong | App shells stop loading, retry loops appear, or requests are silently rejected | Keep same-origin / redirect / abort / header handling explicit and add smoke tests for real sites |
| 4 | Input / form / selection details | Modern sites lean on precise typing, selection, and submission behavior | Search boxes accept text but do not submit correctly, or caret / selection jumps oddly | Treat `value`, selection, and default actions as a single pipeline |
| 5 | Framework-facing DOM parity | React / YouTube / Google-like code expects browser quirks, not just a basic DOM | The site renders but the client app never becomes usable | Add the smallest browser-facing DOM gaps first, then test against real app-shell paths |
| 6 | Performance / memory growth | More caching and observer machinery can accidentally make the browser heavy | Memory climbs until the process stalls or recovers in bursts | Measure frequently and keep caches / snapshots bounded |
| 7 | Cross-branch integration risk | CSS baseline is owned by Claude, so accidental overlap can create merge churn | Repeated conflicts or duplicated engine work | Avoid CSS engine files unless integration is truly needed, and use PR + Copilot review when it is |

## Phase 1: Real Event Plumbing

Goal: make page interaction feel like a browser, not a custom app.

Tasks:

- `addEventListener(...)` and basic listener registration are in place
- basic capture + bubbling exists for `click`, `input`, `change`, `submit`, `keydown`, and `keyup`; `focus` and `blur` are target-only
- page controls now dispatch DOM events before default actions
- queued host-task plumbing now defers `queueMicrotask(...)`, `setTimeout(...)`, `setInterval(...)`, and `requestAnimationFrame(...)` callbacks instead of running them synchronously
- submit and link clicks can be canceled with `preventDefault()`
- browser chrome back/forward navigation is now in place

Still to finish in this phase:

- finish the rest of the richer listener option matrix
- more complete default-action sequencing for edge cases
- session-history restoration for same-document states is still shallow

Exit criteria:

- simple JS-driven buttons and forms work without special-case browser code
- page scripts can observe user typing and clicks
- Google-style search boxes can react to input, submit, and keyboard handlers
- capture-phase and once listeners behave like the browser for the common page-control cases

## Phase 2: DOM Fidelity

Goal: support the DOM shape that frameworks and interactive sites rely on.

Tasks:

- expand node/element APIs that are commonly used
- DOM traversal helpers like `matches(...)`, `closest(...)`, `contains(...)`, and element sibling accessors are now in place
- Node introspection and mutation helpers now cover the common insertion / replacement paths, plus `document.createDocumentFragment(...)`
- improve `classList`, `dataset`, `attributes`, and property reflection beyond the current helper surface; live `element.attributes` is now in place, but deeper parity is still open
- add `querySelector(...)` coverage for more selectors if needed
- support `document.body`, `document.head`, `document.documentElement` consistently
- add mutation notifications for DOM changes when they affect layout or event targets; `MutationObserver` now fires for `attributes`, `childList`, and `characterData`, browser-style event constructors plus `AbortController` / `AbortSignal` are available, and text nodes now expose `splitText(...)`, but deeper parity and more mutation types are still open
- improve `innerHTML` parsing and serialization edge cases

Exit criteria:

- DOM-heavy pages can build and rearrange UI without special-case rewrites
- watch pages and search pages remain stable after script mutations

## Phase 3: Storage, Cookies, and Navigation

Goal: keep session state and navigation behavior close to a normal browser.

Tasks:

- cookie store with origin scoping is now in place
- `localStorage` and `sessionStorage` are now in place
- browser history stack and back/forward UI are now in place for full document loads
- same-document scroll restoration is now in place, and browser-level history now restores scroll too; finish replay polish for richer history syncing
- keep `location` updates and history state in sync
- extend the current soft-navigation handling so it cooperates with browser history instead of only updating the current URL
- support hash navigation and same-document scroll targets

Exit criteria:

- login-ish flows keep their session state via cookies / storage
- back/forward works for same-document navigation, hash changes, and full document loads
- sites that rely on history state stop losing context

## Phase 4: Networking Semantics

Goal: let JS fetch and submit data like a browser without blowing open security boundaries.

Tasks:

- improve `fetch(...)` request/response coverage
- add request headers and response headers that app shells expect
- request method/body support is now in place for the common same-origin cases; keep expanding toward `Headers` / `Request` parity
- support abort signals and request cancellation
- improve `XMLHttpRequest` beyond the current lightweight shim
- decide a safer cross-origin policy for controlled use cases
- make redirects, same-origin checks, and body-size limits consistent across fetch paths

Exit criteria:

- API-driven sites can load their data without special rewrites
- cross-origin behavior is predictable and explicitly bounded

## Phase 5: Layout Reflow and Rendering Feedback

Goal: when JS changes the DOM, the page should reflow like a browser.

Tasks:

- viewport-width and page-revision based layout cache invalidation is in place
- a native `element.style` bridge now reflects inline CSS changes back into the DOM tree
- the bridge covers more text, size, and border-related properties that the current layout engine already understands
- scroll event handling now avoids rebuilding unchanged documents, so pure scrolling stays responsive
- GUI scroll changes now sync back into the JS runtime so scroll listeners can react to the current offset
- script-driven scroll APIs now feed back into the GUI viewport state as well
- DOM mutation notifications now refresh the live snapshot after GUI-driven attribute changes; deeper incremental invalidation for other mutation paths is still to do
- layout hitboxes now feed browser geometry reads like `getBoundingClientRect()`, `getClientRects()`, and `offset*` / `client*` / `scroll*`
- invalidate cached layout when width or content changes
- support more CSS properties that interactive pages depend on
- add better inline/block mixing and table/layout stability
- ensure dynamically inserted controls and links get hit-tested correctly

Exit criteria:

- interactive pages update visually after JS changes them
- forms, menus, and shell UIs do not need a reload to reflect script updates

## Phase 6: Framework Compatibility

Goal: pass the minimum runtime expectations of the sites we actually care about.

Targets:

- Google search results pages
- YouTube home, watch, and search flows
- common news and docs-style app shells
- local demo pages for event handling, storage, and network APIs

Tasks:

- run site-specific smoke tests against real pages
- keep a small set of regression demos in `demo/`
- add compatibility notes whenever a site requires a new API

Exit criteria:

- a fresh run can open, search, click, and navigate on the target sites without falling back to synthetic pages

## Phase 7: Media and Advanced APIs

Goal: handle the higher-end browser features that keep showing up in modern sites.

Tasks:

- improve media element support
- add canvas or other rendering primitives if needed
- support richer input methods and composition flows
- add better `navigator` / user agent / feature detection coverage

Exit criteria:

- video-centric and app-shell-heavy pages stop failing on feature detection

## Validation Ladder

The roadmap should be validated in this order:

1. local unit tests
2. local demo pages
3. the event plumbing demo
4. Google top/search flows
5. YouTube home/watch/search flows
6. common real-world app shells that exercise events, storage, and network APIs

## Working Rule

Whenever a phase lands or a new blocker shows up:

- update this file
- update `HANDOFF.md`
- add or adjust a demo page if it helps prove the feature
- record the change in the session log

---

## 🔻 Strategic Pivot (added 2026-05-28): engine replacement

This roadmap covers JS *features* on top of **boa**. As of 2026-05-28 the decision is to **replace boa with a custom from-scratch JS engine**, because boa is the main bottleneck for real sites:

- YouTube's ~9.7MB `kevlar_base` main bundle is **silently skipped** at the current `MAX_SCRIPT_SOURCE_BYTES = 2MB` cap, so the app never boots (only the skeleton renders).
- Even with the cap raised, boa is too slow to run a 10MB bundle in acceptable time.

The engine-replacement effort is tracked in **`ENGINE_ROADMAP.md`** (bytecode VM, custom parser, live event loop). This `JS_ROADMAP.md` remains the reference for the feature surface the new engine must reach — every "Already working" item is a behavior the new engine has to reproduce during its Host-integration phase. boa stays compiling until the new engine reaches "Core Complete."

## ⚠️ Roadmap Assessment (updated 2026-05-28)

*Originally added 2026-05-16; progress table refreshed 2026-05-28.*

### Progress Summary

| Phase | Status |
|---|---|
| Phase 1: Event Plumbing | ✅ Largely complete (capture/bubble events, keyboard, hover mouse events) |
| Phase 2: DOM Fidelity | 🟡 Substantial — traversal, mutation observers, fragments, live attributes in; deeper parity open |
| Phase 3: Storage & Cookies | ✅ Largely complete (localStorage / sessionStorage / cookies, history + scroll restore) |
| Phase 4: Networking | 🟡 async fetch + XHR in; `Headers`/`Request`/`Response` parity and cross-origin policy still shallow |
| Phase 5: Layout Reflow | 🟡 Observers + hitbox feedback + layout cache invalidation in; full incremental reflow still partial |
| Phase 6: Framework Compat | 🔴 Blocked on engine — boa skips large bundles (see Strategic Pivot) |
| Phase 7: Media & APIs | ❌ Not started |



### Critical Gap (updated 2026-05-28): the engine, not reflow

As of 2026-05-28 the single biggest risk has shifted. Phase 5 reflow has progressed (observer firing, hitbox feedback, viewport/revision-keyed cache invalidation), so it is no longer the top blocker.

The top blocker is now the **JS engine itself**: boa silently skips the large app bundles that modern sites ship (YouTube's ~9.7MB main bundle exceeds the 2MB script cap), so the client app never boots regardless of how good reflow is. This is why the project is pivoting to a custom engine — see the Strategic Pivot section above and `ENGINE_ROADMAP.md`.

Reflow still has remaining work (full incremental invalidation across more mutation paths), but it should be completed *on the new engine* rather than further hardened against boa. The earlier concern below is kept for context:

> The original architecture computed layout once at page load and never again. Fixing this fully requires either a reactive DOM→layout dependency graph, a full re-layout pass per mutation, or a hybrid subtree re-layout. The current code uses cache invalidation + observer-driven refresh as a partial solution.



---

## Revised Roadmap Proposal

The goal remains the same: **Google and YouTube browsable without synthetic pages.**

The phases below replace or refine the original ones to address the gaps above.

### Revised Phase 2: DOM Fidelity (continue)

- `classList`, `dataset`, attribute reflection, `parentElement`, `children`, `nextSibling`, `previousSibling`
- consistent `document.body`, `document.head`, `document.documentElement`
- `innerHTML` round-trip correctness
- dynamically inserted `<script>` tags execute correctly

Exit: DOM-heavy single-page init scripts can build their UI tree without crashing.

### Revised Phase 3: Storage & Cookies

- `localStorage` / `sessionStorage` (in-memory, per-origin)
- cookie read/write with basic `Set-Cookie` header support
- `document.cookie` read and write
- history state kept on full page loads (not just soft navigation)

Exit: login-ish flows and session-dependent pages retain state across navigations.

### Revised Phase 4: Networking

- `fetch(...)` request/response headers, JSON body, abort signal
- `XMLHttpRequest` prototype / `instanceof` and `onreadystatechange`
- cross-origin policy: block by default, user-configurable allow-list
- consistent redirect handling between fetch and page navigation

Exit: API-driven pages can load data without special-case rewrites.

### **NEW Phase 5: Incremental Reflow (architecture decision required)**

This phase requires an **explicit design decision** before implementation begins. The recommended approach:

**Strategy: "Dirty-subtree re-layout" (Option 2 simplified)**

1. Add a `dirty` flag to each DOM node. JS mutations set the flag on the affected node and its ancestors up to `<body>`.
2. After each JS execution session ends, if any node is dirty, re-run `layout_styled_document` for the full document. Cache the previous layout and diff at the `LayoutDocument` level to minimize redraws.
3. On the next render frame, use the new layout.

This is not perfect (full re-layout is O(n) per mutation), but it is **correct**, **implementable in 1-2 weeks**, and **enough to unblock Phase 6**. True subtree-only invalidation can come later.

Implementation steps:
1. Add `dirty: bool` to `StyledNode` (or track a global dirty flag per JS session)
2. After `dispatch_dom_event` or `process_document_scripts` returns, check the dirty flag
3. If dirty: invalidate `DocumentView`'s cached layout and request redraw
4. Ensure `layout_styled_document` is called fresh (it already is; just remove any caching that prevents this)

Exit: JS DOM mutations are reflected on screen without a page reload.

### Revised Phase 6: Framework Compatibility

Run against real targets in this order:

1. Google search (type a query, get results, click a result)
2. Wikipedia (navigate, follow links, search)
3. YouTube home (load page, scroll, click a card)
4. YouTube watch (load title/description; video playback is Phase 7)
5. A React or Vue-based docs site (e.g. Vue.js docs)

For each target: identify the top 3 JS or CSS features that break the experience and feed them back into earlier phases.

Exit: all five targets are browsable in their core flows without falling back to synthetic pages.

### Revised Phase 7: Media & Advanced APIs

- `<video>` and `<audio>` element stubs (show poster image, handle `play()`/`pause()` events)
- `<canvas>` 2D context (minimal: `fillRect`, `drawImage`, `fillText`)
- `ResizeObserver` / `IntersectionObserver` support, enough for layout-driven and viewport-driven lazy UI updates
- `navigator.userAgent`, `navigator.language`, feature-detection shims
- `requestAnimationFrame` loop (queued, but still not a full browser frame clock)

Exit: video-centric pages stop crashing on feature detection; canvas-based UI elements render.

---

## Revised Priority Order

If time is limited, tackle in this order:

1. **Phase 5 (reflow)** — unblocks everything else; do this before Phase 3/4 if possible
2. **Phase 3 (storage)** — relatively self-contained, high user-visible impact
3. **Phase 2 (DOM fidelity)** — fill gaps as they appear in real-site testing
4. **Phase 4 (networking)** — improve as real sites expose gaps
5. **Phase 6 (real-site testing)** — ongoing validation, not a one-time phase
6. **Phase 7 (media)** — lower priority unless YouTube video is a specific goal
