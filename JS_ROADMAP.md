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
- basic DOM event plumbing for `click`, `input`, `change`, `submit`, `focus`, and `blur`
- `Promise` job flushing
- guarded `fetch(...)` and `XMLHttpRequest`
- same-origin navigation checks
- loop-iteration runtime budget for runaway scripts
- native GUI form controls for `GET` submissions

Still missing or shallow:

- `removeEventListener(...)`
- keyboard events such as `keydown` and `keyup`
- capture phase / richer listener options
- tighter alignment between GUI text editing and live DOM `input.value`
- storage and cookies
- richer networking semantics
- full history/navigation behavior
- async browser APIs that modern frameworks expect
- rendering invalidation and layout reflow after DOM mutation

## Phase 1: Real Event Plumbing

Goal: make page interaction feel like a browser, not a custom app.

Tasks:

- `addEventListener(...)` and basic listener registration are in place
- basic bubbling exists for `click`, `input`, `change`, `submit`, `focus`, and `blur`
- page controls now dispatch DOM events before default actions
- submit and link clicks can be canceled with `preventDefault()`

Still to finish in this phase:

- `removeEventListener(...)`
- keyboard events such as `keydown` and `keyup`
- capture phase / richer listener options
- live GUI typing synchronized into script-visible DOM state in every edit path
- more complete default-action sequencing for edge cases

Exit criteria:

- simple JS-driven buttons and forms work without special-case browser code
- page scripts can observe user typing and clicks
- Google-style search boxes can react to input and submit handlers

## Phase 2: DOM Fidelity

Goal: support the DOM shape that frameworks and interactive sites rely on.

Tasks:

- expand node/element APIs that are commonly used
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

- add cookie store with origin scoping
- add `localStorage` and `sessionStorage`
- add `history.pushState(...)`, `replaceState(...)`, `back()`, `forward()`
- keep `location` updates and history state in sync
- support hash navigation and same-document scroll targets

Exit criteria:

- login-ish flows keep their session state
- back/forward works for document navigation and hash changes
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

- recompute layout after DOM mutations and script-driven style changes
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
