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
- native GUI typing stays in sync with live DOM `input.value`
- basic DOM event plumbing for capture + bubbling `click`, `input`, `change`, and `submit`, plus target-only `focus` and `blur`
- `Promise` job flushing
- guarded `fetch(...)` and `XMLHttpRequest`
- same-origin navigation checks
- loop-iteration runtime budget for runaway scripts
- native GUI form controls for `GET` submissions
- passive listener semantics
- `location.hash`, `history.pushState(...)`, `replaceState(...)`, `back()`, and `forward()` for same-document navigation
- same-document history back/forward now restores stored scroll positions
- browser-level back/forward navigation across document loads
- layout cache invalidation keyed by viewport width and page revision
- JS-visible viewport and focus state are wired up through `window.innerWidth` / `window.innerHeight`, `window.scrollY` / `window.pageYOffset`, and `document.activeElement`
- basic script-driven scrolling APIs now exist through `window.scrollTo(...)`, `window.scrollBy(...)`, and `scrollTop` setters on DOM nodes
- inline style mutations now reflect back into the DOM snapshot
- the inline style bridge now exposes more text, size, and border-related properties
- GUI-driven DOM attribute mutations now refresh the live page snapshot so reflow invalidation can happen immediately after mutation notifications

CSS baseline note:

- the broad CSS parser / selector / cascade / computed-style foundation is treated as complete on the Claude `claude/phase5-css` branch
- Codex's Phase 5 work is therefore about JS-driven reflow and rendering feedback on top of that baseline, not reimplementing the CSS engine
- if a JS task genuinely needs CSS-facing integration, keep the diff minimal, request Copilot review, and log the touched files in `change.md`

Still missing or shallow:

- richer networking semantics
- session-history replay polish across full document loads
- async browser APIs that modern frameworks expect
- rendering invalidation and layout reflow after DOM mutation still need deeper incremental invalidation
- the style bridge still needs the rest of the CSS property matrix and computed-style parity
- remaining CSS work is mostly Phase 6 visual effects / advanced rendering, not the core parser/layout baseline

## Execution Order (Simple -> Hard)

If we want to keep momentum and avoid getting stuck on the biggest browser gaps too early, the practical implementation order is:

1. attribute / DOM introspection helpers like `hasAttribute(...)`, `getAttributeNames(...)`, and broader property reflection
2. event-delegation helpers like `matches(...)`, `closest(...)`, `contains(...)`, and element traversal accessors
3. basic listener-option edge cases and default-action sequencing
4. `document.body` / `document.head` / `document.documentElement` consistency and `innerHTML` edge cases
5. mutation notifications plus incremental reflow invalidation for DOM and style changes
6. same-document and full-document history replay polish, including scroll restoration
7. fetch / XHR semantics and safer cross-origin handling
8. Google / YouTube / app-shell compatibility smoke tests
9. media and advanced APIs

The roadmap below still keeps the big browser areas grouped by phase, but the list above is the preferred order when we need the next easiest high-impact task.

## Phase 1: Real Event Plumbing

Goal: make page interaction feel like a browser, not a custom app.

Tasks:

- `addEventListener(...)` and basic listener registration are in place
- basic capture + bubbling exists for `click`, `input`, `change`, `submit`, `keydown`, and `keyup`; `focus` and `blur` are target-only
- page controls now dispatch DOM events before default actions
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
- improve `classList`, `dataset`, `attributes`, and property reflection
- add `querySelector(...)` coverage for more selectors if needed
- support `document.body`, `document.head`, `document.documentElement` consistently
- add mutation notifications for DOM changes when they affect layout or event targets
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
- same-document scroll restoration is now in place; finish replay polish for full-document loads and richer history syncing
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
- GUI scroll changes now sync back into the JS runtime so scroll listeners can react to the current offset
- script-driven scroll APIs now feed back into the GUI viewport state as well
- DOM mutation notifications now refresh the live snapshot after GUI-driven attribute changes; deeper incremental invalidation for other mutation paths is still to do
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
