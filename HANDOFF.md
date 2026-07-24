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

- Date: `2026-07-22`
- Repo / package name: `tobira`
- Working branch: `master`
- Workflow:
  - use the shared checkout the user pointed at unless a dedicated worktree is explicitly requested
  - keep the handoff notes current when switching between sessions or collaborating agents
- Verification status:
- `cargo test`: `703` passing tests on `2026-07-24` (Windows checkout; also green under `TOBIRA_VERIFY_BYTECODE=1`)
- `cargo build`: success on `2026-06-19` (release; use `RUSTFLAGS='-C debuginfo=0'` to dodge OneDrive PDB locks)
- North star / current goal:
  - Chromeと同程度の実用感を目指し、Google/YouTubeなどの複雑なサイトをsynthetic fallbackに頼らず閲覧・操作できるようにする
  - Scope caveat: "閲覧・操作" means rendering the page's DOM/CSS and running its JS — **not** video playback. The `softbuffer` CPU renderer has no GPU compositing or media decode, so smooth YouTube *video* is out of scope for the current rendering backend (a separate, much larger effort: codecs + GPU).
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
  - guarded JavaScript execution through a from-scratch bytecode engine (`src/engine/`): self-built compiler + VM + tracing GC heap. Only `boa_ast`/`boa_parser`/`boa_interner` remain, as the parser front-end; the boa runtime (`boa_engine`/`boa_gc`) was removed (2026-06-12). JS values and DOM nodes live in one unified GC heap, so there is no JS-GC ↔ DOM lifetime-sync problem.
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

- `04bfc2f` engine: switch JS regex backend from `regex` crate to `regress` (look-ahead/-behind/backrefs; fixes react.dev's gtag regex)
- `bac4893` engine: Annex B `{__proto__: value}` object-literal proto setting (fixes rust-lang.org highlight.js crash)
- `896bf94` engine: `Object.*` introspection coerces primitives (ToObject, ES2015+)
- `e4d1737` engine: ES module imports are live bindings (fixes circular deps) — part of the ES-module series `ce9ffbf`/`5059803`/`eee72e5`/`27923e9` (vuejs.org renders end-to-end)
- (Major arc since Phase 5 CSS: the JS backend was rewritten from the boa runtime to a from-scratch bytecode engine — `src/engine/` — with ES2015+ coverage. The campaign drives real pages by following each uncaught error and filling the missing API; CLEAN: example.com / Hacker News / Wikipedia / web.dev / vuejs.org / rust-lang.org.)
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
- GUI-to-page event delivery now covers capture + bubbling `click`, `input`, `change`, `submit`, `keydown`, and `keyup`, plus target-only `focus` and `blur`; passive listener semantics are in place, and `location.hash` plus `history.pushState(...)` / `replaceState(...)` now support soft navigation without a reload, while the rest of the option matrix and back/forward stack still need depth.
- Native page input typing now syncs `value` into the JS DOM.
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
- Form support is still limited to simple text-like fields and `GET` submission; `POST`, checkboxes, radios, and file inputs are not wired yet.
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

### 2026-07-24 - Claude PM / Codex (frameset loss — engine VOID_ELEMENTS + stray end tags)

- User report: abehiroshi.la.coocan.jp (a `<frameset cols=18,82>` page) rendered only the
  left menu frame; the right content frame vanished. Probe showed `expand_frameset` received
  a frameset with just ONE frame child.
- Two stacked defects, both fixed:
  1. The engine-side `VOID_ELEMENTS` list (engine_host.rs) was missing `"frame"` (html.rs's
     `is_void_element` has it). The load path round-trips HTML through the engine DOM
     (`serialize_node`), so `<frame>` re-serialized as `<frame ...></frame>`.
  2. html.rs `close_element` unwound the ENTIRE open stack when an end tag had no matching
     open element. On re-parse, the void `<frame>`'s stray `</frame>` closed `frameset`+`html`,
     dropping the second frame out of the frameset. Per HTML5, unmatched end tags are now
     ignored (pre-scan the stack; return if no match). This also protects legacy pages with
     stray `</td>` / `</font>` etc. from tree destruction.
- Tests: frameset with `</frame>` close tags keeps 2 sibling frames; stray `</b>` is ignored;
  engine snapshot serializes frame as void (no `</frame>`, `<frame ` x2). `cargo test` 703
  green. Real site verified: both frames render (menu + profile content).
- Flagged separately (task chip): `annotate_resource_urls` resolves `mailto:` hrefs against
  the base URL, producing `https://host/mailto:...` — scheme-qualified hrefs should skip
  relative resolution.

### 2026-07-24 - Claude PM / Codex (inline image rendering — InlineFragment::Image)

- User report: abehiroshi.la.coocan.jp/nonno/nonno.htm showed "[image]" text links where
  Chrome shows magazine covers. Diagnosis: fetch/decode/ImageStore were all fine (verified
  with a temporary probe — every cover JPEG decoded); the gap was purely in layout.
  `collect_inline_fragments()` unconditionally replaced inline-context `<img>` with its
  alt text / "[image]" — `InlineFragment` had no image variant, so any `<a><img></a>` or
  text-mixed image (this page is `<td><a><img></a><br>No.N</td>`) never drew. Only the
  block path (`layout_image_element`) could draw images.
- Fix (Codex, spec by Claude): new `InlineFragment::Image` + `InlineImageSpec` +
  `LineBuilder::push_image()`, modeled on the existing `Control` inline-box pattern.
  Store-hit images become sized fragments (`image_dimensions` against available width);
  store-miss keeps the alt/"[image]" fallback. All three white-space paths handle images
  (normal wraps like a word); emit is bottom-aligned in the line box, honors
  opacity/filter via LayerCommand, and emits a LinkCommand hitbox for `<a>`-wrapped
  images (gated on pointer-events). `ImageStore` + available width now thread through
  `flatten/collect_inline_fragments`.
- Tests: linked inline image emits ImageCommand + link hitbox and no "[image]" text;
  store-miss falls back to alt; image raises the line advance. `cargo test` 700 green.

### 2026-07-24 - Claude PM / Codex (HTML tokenizer char-boundary panic + non-HTML Content-Type)

- Fixed a crash reported from a GUI session: navigating to a JPEG URL panicked at
  `src/html.rs:219` ("byte index is not a char boundary"). Root cause: in `tokenize()`,
  when `<` is followed by a non-tag-name character, the recovery path advanced `index += 1`
  (one byte), landing inside a multi-byte UTF-8 char (U+FFFD from lossy-decoding binary).
  Fix: advance by the char's `len_utf8()`. Regression tests with `\u{FFFD}` and a
  JPEG-like lossy byte string.
- Added Content-Type awareness to the document load path (`browser.rs`):
  new `synthesize_non_html_document()` — `image/*` responses become a synthetic
  `<html><body><img src="{final_url}"></body></html>` viewer document (like real browsers),
  `text/plain` gets HTML-escaped and wrapped in `<pre>`. `text/html` / missing
  content-type unchanged. Unit tests for both paths + escaping.
- Verified: `cargo test` 697 green; `--cli https://httpbin.org/image/jpeg` no longer
  panics (renders the img document); rollupjs.org still CLEAN.

### 2026-07-23 - Claude PM / Codex (module top-level scope isolation — rollupjs.org CLEAN)

- Root-caused the rollupjs.org `object is not callable (kind Array, ["items"])` crash:
  module top-level `let`/`const`/`var`/`function` bindings were compiled as **shared flat
  globals keyed by name** (`resolve_declaration_binding` returned `Global` at top level even
  for modules), and closure references compiled to call-time `GetGlobal(name)`. With minified
  multi-chunk bundles, a later-executed module's same-named top-level binding clobbers the
  first module's value. Exact real-site chain: VPAlgoliaSearchBox chunk declares
  `var $i=["items"]` → overwrites framework's `$i` (Vue `withCtx`) → framework's exported
  `L = $u = e => $i` returns the array → theme's `vs=_o(); vs(renderFn)` throws.
  (The source-position backtrace `at <script> (2:40706)` from 33ab431 is what made this findable.)
- Minimal repro: module A `const inner=()=>"real"; export const outer=()=>inner;`, module B
  imports `outer`, declares its own `const inner=["items"]`, and `outer()` returned B's array.
- Fix (Codex, spec by Claude): when compiling a module (`module_context.is_some()`), top-level
  declarations become **frame locals** (`Rc<RefCell>` cells) like function bodies, so closures
  capture cells via the existing upvalue machinery instead of falling through to `GetGlobal`.
  Import live-bindings (per-use namespace `GetProp`) and export emission (`resolve_binding` →
  `SetProp`) work unchanged. Script (non-module) behavior untouched (window sharing).
  - `compiler/mod.rs`: `is_module_top_level()` helper.
  - `compiler/scope.rs`: Var/Let/Const storage arms gated with it.
  - `compiler/statements.rs`: function-decl hoist + switch-case hoist + block-nested `var`
    collection enabled at module top level; `predeclare_hoisted` extended to cover
    destructuring pattern names, class declarations, and export-wrapped declarations.
  - `compiler/patterns.rs`: `collect_binding_names` / `collect_pattern_names` helpers.
- New `tests/module_scope_isolation.rs` (6 tests): cross-module name collision, mutual
  recursion, forward reference, destructuring capture, block-`var` capture, and
  no-globalThis-leak. `cargo test` 691 green; `TOBIRA_VERIFY_BYTECODE=1` green.
- **rollupjs.org now renders CLEAN end-to-end** (nav, hero, feature cards, footer; no
  uncaught errors). This closes the module-scope arc that source-position backtraces opened.
- Note for later: `load_module_graph` eagerly executes dynamically-imported chunks (they land
  in `post_order` before their importer) and each `<script type="module">` tag builds its own
  registry (a shared dep imported by two tags would re-execute and its namespace object be
  recreated). Both are latent correctness leads, not urgent.

### 2026-06-19 - Claude PM / Codex (compiler split + GC evidence)

- Split the 4423-line `src/engine/compiler.rs` monolith into focused submodules under `src/engine/compiler/` (no logic change, 571 tests green at every step, each extraction its own commit): `mod.rs` 392 (core: structs/new/finish/emit/function compilation), `scope.rs` 267 (ScopeFrame/UpvalueState/OuterBindings + binding resolution), `modules.rs` 327 (import/export), `classes.rs` 465 (class/super), `patterns.rs` 363 (destructuring), `statements.rs` 1536 (control flow), `expressions.rs` 1124. Submodules are children of `compiler`, so they reach FunctionCompiler's private fields; moved methods are `pub(super)`.
- Added GC evidence (read-only, no collector landed): `Vm::heap()` accessor + `tests/gc_heap_growth.rs`. Measured — fresh VM ≈ 380 builtin objects; a 2000-iteration loop of unreachable `{…}`+`[…]` literals leaves ≈ 4380 live (grew ≈ 4000 = 2/iter, zero reclaimed). Confirms the heap is monotonic within a run (no in-session collection). A reclaiming mark-sweep collector is intentionally deferred — it needs review because the `callables` side-table and closure-upvalue `Rc<RefCell>` cells are roots outside the arena. When it lands, `heap_grows_without_in_session_collection` flips to assert reclamation.
- Context: these two items are debt paydown surfaced by an external code critique (compiler monolith + no in-session GC were its only landed points; its boa-GC and "200 tests" claims were stale).
- Real-page campaign: implemented the `URL` global (hand-rolled parser, no new dep) — nodejs.org and tailwindcss.com stopped on `URL is not defined`; both now advance (nodejs.org's next wall is `getAttribute is not a function`). New `tests/url_global.rs`.
- Call-stack correctness (`tests/call_stack_depth.rs`): StackOverflow is now a catchable `RangeError` ("Maximum call stack size exceeded", the web-standard message) instead of a fatal abort — `try { recurse() } catch {}` works; frame cap raised 1024 -> 10_000 (was far below real engines). This did NOT fix vitejs.dev: its bundle hits a genuine *uncaught* infinite recursion, a separate semantic gap.
- More campaign wins (each its own commit + test, all green): `crypto.getRandomValues`/`randomUUID` (was an empty stub); `typeof` of window-globals now reports the real type (`typeof crypto` was "undefined" — `GetGlobalOptional` skipped the window-global fallback, breaking `typeof crypto !== 'undefined'` feature-detection); `matchMedia()` returns a MediaQueryList with (no-op) `addEventListener`/`media` (Bootstrap color-modes); `document.currentScript` gained `getAttribute`/`hasAttribute` (fathom/beacon/framework `currentScript.getAttribute('data-…')`). Result: **expressjs.com is now CLEAN**; nodejs.org, tailwindcss.com, getbootstrap.com all advanced past their first walls. 587 tests green.
- Also added `Object.prototype.propertyIsEnumerable` (httpbin/swagger-ui). 588 tests green.
- **Top next target = dynamic `import()`**: `Unimplemented("import() calls")` is the single most common remaining wall — crates.io, svelte.dev, rollupjs.org, webpack.js.org (4+ sites). A mid-size ESM feature (async load + instantiation) with scope choices, best done interactively on top of the existing module graph (load_module_graph / ModuleContext). Other deeper targets: getbootstrap `bootstrap is not defined`; vitejs.dev genuine infinite recursion; webpack/next `modules[id].call` (react.dev, typescriptlang); babeljs.io/pnpm.io parse error on a "https:" script (looks like fetched content isn't JS — a networking/CDN issue, not a parser gap). CLEAN: example, HN, Wikipedia, web.dev, vuejs, rust-lang, docs.rs, lodash, expressjs, prettier, tc39.es.
- `document.currentScript` follow-up: the stub still lacks the real element's `data-*` attributes (getAttribute returns null for them); a full fix returns the actual DOM script node via host integration.
- Proactive standard-library audit (probe of ~120 prototype methods + ~80 globals): added the cleanly-missing ones — Math hyperbolics/expm1/fround, Number.isSafeInteger, Object.getOwnPropertySymbols, Date.UTC/parse/getTimezoneOffset, Object.prototype.propertyIsEnumerable, TextEncoder/TextDecoder, escape/unescape, WeakRef, Headers, FormData. Remaining gaps are large/design-gated or N/A (Request/Response/Blob/File/FileReader, WebSocket/Worker, BigInt, Intl, DOMParser, Audio/Notification). **597 tests green.** Note: each added builtin grows the fresh-VM object count, tracked by tests/gc_heap_growth.rs (fresh-VM bound now 600).

### 2026-06-19 - Claude PM / Codex (real-page campaign: rust-lang, react.dev + doc refresh)

- Fixed rust-lang.org crash: object literal `{__proto__: value}` now sets `[[Prototype]]` (Annex B.3.1) instead of creating an own `__proto__` property. Root cause was highlight.js's `Object.freeze({__proto__:null,...})` + `for...in` enumerating the bogus own property (`typeof null === "object"`) → `Object.getOwnPropertyNames(null)` threw. New opcode `SetObjectLiteralProto`; primitives ignored (no throw). Regression tests in `tests/proto_literal.rs`. (`bac4893`, `af64f1b`)
- Fixed react.dev regex abort: swapped the JS regex backend from the Rust `regex` crate to `regress` (JS-compatible: look-ahead/-behind/backreferences). New `src/engine/js_regex.rs` adapter keeps vm.rs call sites mostly unchanged; `translate_regex_named_groups` dropped (regress supports `(?<name>)` natively); `regex` dependency removed. Regression tests added to `tests/regexp_coverage.rs`. react.dev's next wall is webpack-internal `modules[id].call` (deeper; content already renders). (`04bfc2f`)
- Refreshed stale docs: README/HANDOFF said `boa_engine` (removed 2026-06-12) and `200` tests (now `571`); clarified the north-star scope (render YouTube's page DOM/CSS ≠ play its video on the CPU `softbuffer` renderer). These stale lines had invited an off-base external critique.
- Verified: `cargo test` `571` passing; release build green.

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

### 2026-06-19 - Claude PM / Codex (dynamic import())

- Implemented dynamic `import()` (preload model, user-chosen Option A) end-to-end. Step 1 (compiler+VM): `ModuleContext.dynamic_imports` map + `Opcode::DynamicImport` wraps a module namespace (or undefined→reject) in a Promise; literal specifiers resolve from the map, computed/unknown reject gracefully. Step 2 (host): `load_module_graph` walks the full AST (boa_ast Visitor) for `import("literal")` calls and preloads those module graphs (non-fatal on failure), populating `dynamic_imports`. crates.io/rollupjs/webpack.js.org/svelte.dev all clear the old `Unimplemented("import() calls")` compile wall; vuejs.org still renders (no ESM regression). 599 tests green (tests/dynamic_import.rs). Not yet: computed `import(var)` (rejects — needs runtime specifier resolution in the VM).
- New leads surfaced after the unblock: svelte.dev `Invalid URL` (our hand-rolled URL parser is too strict for some input — clean fix candidate); rollupjs `Object.create prototype must be an object or null`.

### 2026-07-22 - Claude (Object.defineProperty descriptor merge)

- Root-caused the rollupjs.org `Object.create prototype must be an object or null` lead: `Object.defineProperty` (and `Reflect.defineProperty` / `Object.defineProperties`) replaced the existing own property with the parsed descriptor wholesale, so Babel's per-class `Object.defineProperty(fn, "prototype", {writable:false})` clobbered `fn.prototype` to `undefined`, and the next `class B extends A` died inside `_inherits`.
- Implemented spec-style ValidateAndApplyPropertyDefinition merging (`value_to_property_descriptor_merged`): fields absent from the descriptor object inherit the existing property's attributes; accessor/data kind switches follow the spec; mixed accessor+value descriptors now throw TypeError. `Object.create` errors now report the received type, with an optional backtrace dump under `TOBIRA_DEBUG_CONSOLE`.
- Verified headless via `--cli`: rollupjs.org clears the Algolia module crash (next lead: `object is not callable` in `theme.DwJmkNlp.js`); svelte.dev now loads with meaningful content and no `Invalid URL` in the current checkout. `645` tests green including new `tests/define_property_merge.rs` (9 cases).

### 2026-07-22 - Claude PM / Codex (Symbol.iterator as a real property)

- `Symbol.iterator` existed only on `generator_prototype`, `URLSearchParams`, `Headers`, and `FormData`. `for..of` over arrays/strings/Map/Set worked solely through the VM's internal fast path, so reading the property returned `undefined`: `[][Symbol.iterator]`, `''[Symbol.iterator]`, `new Map()[Symbol.iterator]`, `arguments[Symbol.iterator]` were all missing. Transpiled bundles gate on exactly this (`_createForOfIteratorHelper` reads `o[Symbol.iterator]` and throws `Invalid attempt to iterate non-iterable instance` when it is absent), so every Babel/SWC-compiled `for..of` over a non-array broke on real sites.
- Wired it as a real data property (`writable: true, enumerable: false, configurable: true`) on `Array.prototype` (identical function object to `Array.prototype.values`), `Map.prototype` (=== `entries`), `Set.prototype` (=== `values`), `TypedArray.prototype` (new `TypedArrayProtoValues`), and `String.prototype` (new `StringProtoIterator`, iterating by Unicode code point so astral characters stay whole). `arguments` inherits from `Array.prototype` and needs nothing of its own.
- Added a shared `%IteratorPrototype%`-style `iterator_prototype` carrying `next` and a self-returning `Symbol.iterator`; `ForOfIterator` objects and `generator_prototype` both inherit from it. Without this, iterators themselves were not iterable, which left the main real-world case (`for (const x of arr.values())` under a transpiler) still broken. Both methods are non-enumerable, so iterators no longer leak `next` into `Object.keys` / `for..in`.
- Deliberate scope extension: `Map.prototype.entries/keys/values` and `Set.prototype.values` now return an iterator instead of an array, matching the spec. Verified no regression across `for..of`, spread, `Array.from`, and destructuring.
- `652` tests green, including new `tests/symbol_iterator_property.rs` (7 cases, covering the `_createForOfIteratorHelper` shape applied to iterator results, not just to containers).
- Still open from the earlier probe: arrow functions and shorthand/class methods still have a `.prototype` (spec says they must not).

### 2026-07-22 - Claude PM / Codex (DOM interface constructors are real functions)

- The `DOM_INTERFACE_NAMES` globals (`HTMLElement`, `EventTarget`, `Node`, `Element`, …) were built with `allocate_ordinary_object`, so `typeof HTMLElement` was `"object"`, `.name` was missing, and `.prototype.constructor` was undefined. Sites feature-detect with `typeof HTMLElement === 'function'` before installing web-component code, and transpiled `class X extends HTMLElement` reaches `Reflect.construct(HTMLElement, …)` which died in `require_callable`.
- They are now callable objects backed by a new `BuiltinId::DomInterfaceConstructor`, with a non-writable `name`, a `prototype.constructor` back-reference, and browser-matching descriptors. A plain call `HTMLElement()` throws `TypeError: Illegal constructor`; `new HTMLElement()` and `super()` from a subclass construct normally.
- Fixed a latent hole this exposed: `instanceof_value` short-circuited on DOM interface constructors, checking only `host_node_interfaces` and returning `false` for anything else. So `class X extends HTMLElement {}; new X() instanceof HTMLElement` was `false` even though the prototype chain was correct. The interface check is now a fallback — host nodes match by interface name, everything else falls through to the ordinary prototype-chain walk. Verified both directions (deep subclass chains match; plain objects, unrelated classes, and sibling interfaces still do not).
- `require_callable`'s bare `object is not callable` now names the offender (`name`, or `constructor.name`, plus the `ObjectKind`). Error-path only. This should make the remaining real-site walls much cheaper to diagnose.
- `661` tests green: new `tests/dom_interface_constructors.rs` plus a real-host `instanceof` case in `tests/phase6_dom.rs` (`document.body instanceof HTMLElement/Element/Node/EventTarget`).
- **Next lead — `Reflect.construct` ignores its newTarget argument, and `new.target` is not propagated.** This is pre-existing and NOT specific to DOM interfaces: it reproduces on ordinary functions (`Object.getPrototypeOf(Reflect.construct(Base, [], Derived)) === Derived.prototype` is `false`; `new.target` inside a constructor invoked via `Reflect.construct` is not the passed newTarget). Babel's `_createSuper` is built on exactly this, so every transpiled `class extends` currently gets the wrong instance prototype — likely the widest-reaching remaining JS gap and the best next target. (Done in the next entry.)

### 2026-07-22 - Claude PM / Codex (newTarget propagation)

- Threaded an explicit newTarget through the construct path: new `construct_value_with_new_target(_sync)`, with the old two-argument entry points delegating with `new_target = constructor` so every existing call site is unchanged. `construct_this_value` now derives the prototype from the newTarget (spec: OrdinaryCreateFromConstructor), the closure frame's `new_target` is the passed newTarget, and bound constructors forward the newTarget they received instead of substituting the bound target.
- `Reflect.construct` reads its third argument, defaults it to the target when absent/undefined, and throws `TypeError: Reflect.construct: The last argument is not a constructor` when it is present but not constructible (new `is_constructor_value` helper).
- Payoff: Babel's `_inherits` + `_createSuper` shape now works end to end — a transpiled subclass instance gets the subclass prototype, keeps the base in its chain, and both subclass and base methods resolve. Before this, every transpiled `class extends` produced an instance with the *base* prototype, silently losing all subclass methods.
- Scope extension found along the way: `Object.create(proto, descriptors)` silently ignored its second argument. The pre-existing `tests/define_property_merge.rs` used that form but only asserted the prototype link, so nothing caught it. Now implemented over own *enumerable* keys with spec descriptor defaults; verified accessor descriptors, `false` defaults, non-enumerable descriptor-map entries being skipped, and non-object descriptors throwing.
- **Known limitation, now covered by a test that asserts the current behaviour:** a native `super()` is lowered to `Opcode::Call` rather than routed through the construct path, so `new.target` inside a base constructor reached via `new Derived()` is `undefined` instead of `Derived`. Measured: standalone `new A()` correctly gives `A`; `Reflect.construct(A, [], B)` correctly gives `B`; only the native `super()` chain is wrong. The common abstract-class guard (`if (new.target === Abstract) throw`) still behaves correctly by accident. Fixing it means changing how the compiler lowers `super()`.
- `670` tests green. Note for whoever picks this up: Codex's first pass had *rewritten* `tests/new_target.rs` and dropped two pre-existing passing cases (the `new.target` guard pattern and the native-class-constructor case). They were restored from `HEAD`. Watch for this when handing an existing test file to an agent.

### 2026-07-22 - Real-site status after the three JS fixes

Headless sweep via `--cli` with `TOBIRA_DEBUG_CONSOLE=1`, after `Symbol.iterator` + DOM interface constructors + newTarget:

- **svelte.dev — CLEAN.** 6.8 KB of rendered content, zero uncaught JS errors.
- **webpack.js.org — advanced.** The old lead `property assignment requires an object (got undefined)` is **gone** (0 occurrences). The page now renders its nav and content (4.0 KB). New wall: `TransformStream is not defined` — a missing Web Streams global, a different class of gap (add the global, or stub it well enough for the bundle's feature detection).
- **rollupjs.org — renders fully** (2.4 KB: hero, feature cards, nav, all links). One error remains, unchanged in location: `object is not callable (kind Array)` in `assets/chunks/theme.DwJmkNlp.js` (the VitePress theme JS — search box, appearance toggle). Content is unaffected. The `(kind Array)` detail is new, courtesy of the improved `require_callable` diagnostic: something is calling an Array. A plausible shape is Babel's `_createForOfIteratorHelperLoose` doing `it = it.call(o)` where `o[Symbol.iterator]` resolved to an array rather than a function — worth checking which object that is before assuming.
- **crates.io — still title-only** (122 B: just `# crates.io: Rust Package Registry`, no errors reported). Unchanged by this work; it is an Ember SPA and the gap is elsewhere. Needs its own investigation.

Recommended next targets, in order: (1) `TransformStream` / Web Streams globals for webpack.js.org — smallest and well-defined; (2) the rollupjs `kind Array` callee — now much cheaper to chase with the improved diagnostic; (3) native `super()` `new.target` propagation (compiler lowering change); (4) crates.io's empty render.

### 2026-07-23 - (separate local session) strip leftover BOMs

- `src/engine/vm.rs` and `src/engine_host.rs` still carried a leading UTF-8 BOM from the old encoding accident. Removed both (commit `527c55c`); no code change. All tracked `*.rs` are now BOM-free.

### 2026-07-23 - Claude PM / Codex (global `eval`)

- Implemented the global `eval` function. It was entirely absent (`typeof eval === "undefined"`); Google Search's inline script #2 died on `eval is not defined` and collapsed the results page to a 407-byte fallback.
- **Indirect eval only.** The evaluated code runs at global scope, reusing the existing `eval_source` machinery (originally built for `document.write`'d `<script>`). Global reads/writes and `var`/`function` declarations leaking to the global object all work; the completion value is returned (`eval("1+1") === 2`). Non-string arguments are returned unchanged. Parse/compile failures throw a `SyntaxError` (the `document.write` path still uses its old `TypeError` mapping — the two now share `eval_source_with_errors` with an error-kind flag). New `BuiltinId::GlobalEval`; `window.eval`/`globalThis.eval` resolve to the same function.
- Completion value needed a compiler path: `compile_for_eval_completion` / `compile_statements_preserving_final_expression` leave the final expression-statement on the stack instead of `Pop`-ing it. Verified stack balance under `TOBIRA_VERIFY_BYTECODE=1` (full suite green with it on).
- **Explicitly out of scope — direct eval.** `function f(){ var local = 1; return eval("local"); }` cannot see `local` (throws ReferenceError); direct eval would need the compiler to special-case `eval(...)` call sites and keep local scopes reifiable. If a real site relies on direct eval, this will not help it.
- **Completion-value limitation:** only a final *expression statement* is returned. `eval("if (true) 5")` yields `undefined`, not `5` (spec would give `5`). Acceptable for now.
- Adjacent gap noticed while testing (NOT fixed): `new String("x")` returns a primitive string, not a wrapper object — `typeof new String("x") === "string"`. Unrelated to eval; flag for later.
- `679` tests green (new `tests/global_eval.rs`, 9 cases). Codex left the working tree with 28 line-ending-only dirtied files again (real changes were only the 3 source files + the new test); restored as before. This has happened on every Codex run this session — the agent writes LF into CRLF files. Not harmful (committed blobs are LF via `core.autocrlf`), but check `git diff --numstat` before staging.

### 2026-07-23 - Claude (real-site sweep + rollupjs deep-dive; diagnostics)

- After `eval`, re-swept Google/search engines. All render their top shell but die in the heavy dynamic JS: Google Search `eval` wall cleared but next line does `undefined()` in the same inline script; Google top `_DumpException is not a function`; Bing `_w is not defined`; DuckDuckGo non-function call; Google News `object is not callable (kind Ordinary)`. Google-class obfuscated loaders are many-layered — one fix just exposes the next wall. YouTube: `www.youtube.com` returns a Google **login/consent** page (bot-treated), not the app; only engine gap there is `<canvas>.getContext` unwired. Video playback is out of scope regardless (CPU renderer).
- webpack.js.org's `TransformStream` wall was investigated and is **not small**: `vendor.js` uses `new TransformStream({...}).pipeThrough(...)` — it's the Vercel AI SDK (in-page AI assistant) using the full Web Streams API (ReadableStream/WritableStream/pipeThrough/reader/backpressure, all async), not feature detection. The doc **body already renders** without it; Streams would only power the AI widget. Deferred as a large, low-ROI-for-this-page item. (User confirmed: treat webpack as body-OK, move on.)
- **rollupjs.org `object is not callable (kind Array)` — narrowed but not yet located.** Built two diagnostics to chase it (both kept, see below). Findings: the culprit value is the array **`["items"]`** (length 1), **called as a plain function** (`this=undefined`) with **a single function argument**, at **module top level** (`theme.DwJmkNlp.js`, backtrace shows only `<script>`). The source never literally calls an array (`]()` appears 0 times in the 106 KB bundle), so our engine is **mis-evaluating some expression to `["items"]`**. Ruled out simple array-method bugs: `Object.keys(obj).reduce/forEach/map/filter/find/some/every/sort/flatMap`, `Object.entries/values(...).x`, and direct `["items"].reduce/forEach` all work (15/15 probes pass). The two literal `["items"]` in source are Vue `createVNode(..., 8, ["items"])` **dynamicProps** (data, not callees), and they sit inside render closures — not the top-level `<script>` frame — so they are NOT the culprit. Leading remaining hypothesis: a **scope/binding-resolution bug** in heavily-minified single-letter-name code, where a helper reference (e.g. Vue's `withCtx`/`computed`, called as `X(fn)`) resolves to the wrong binding holding `["items"]`. Pinia store setup (`Sc=Oo()` at top level, `Object.keys(getters).reduce(...=>...A(()=>...))`) is the most likely neighborhood.
- **Blocker: the bytecode has no source-line info** (`FunctionProto { code: Vec<Opcode> }`; no spans). So backtraces are function-name-only (`capture_backtrace`) and cannot map to a minified column. Pinpointing this (and future minified-site bugs) efficiently needs **source-line/column tracking in the bytecode** so backtraces read `at <name> (url:line:col)`. That is the recommended foundational next step — it cracks this bug and makes every future real-site debug far cheaper. Medium task (compiler emits spans per opcode; `capture_backtrace` formats them).
- **Diagnostics kept** (proved their worth locating the above; my own code, error-path only): (1) `describe_non_callable_object` now previews an Array callee's length + first elements, so `object is not callable (kind Array, length 1, ["items"])`; (2) module-execution errors now append the backtrace (mirroring the classic-script path), so ESM failures surface `\n    at <script>` etc. — previously only classic `<script>`/inline errors did.

### 2026-07-23 - Claude PM / Codex (source-position backtraces + rollupjs pinpointed)

- **Source positions now flow to backtraces.** The parser front-end is still boa (`boa_ast` 0.21.1; "boa removal" was only the runtime); boa's `Call`/`New` nodes impl `Spanned` and expose column-level positions. The compiler now records `(line, column)` for every Call/New/tagged-template site into a new `FunctionProto.call_positions: Vec<CallSitePosition>` (empty when unused → no overhead), and `capture_backtrace` binary-searches it by the frame's `ip-1` to print `at <name> (line:column)`. This is the foundation the earlier blocker asked for — it makes minified-site debugging tractable (column matters; bundles are ~1 line). `685` tests green (new `tests/call_source_position.rs`, incl. a same-line-different-columns test).
- **rollupjs.org `["items"]` PINPOINTED with the new backtrace.** It reads `at <script> (2:40706)`. `theme.DwJmkNlp.js` line 2 col 40706 is `const vs=_o(); … const ys=vs((e,t,n,o,s,i)=>(l(),P("div",gs)));` — the failing call is `vs(arrowFn)`. So `vs = _o()` returned the array `["items"]` instead of a function. `_o` is the ESM import `L as _o` from `./framework.P2XOc7lE.js`.
- **Import-alias hypothesis DISPROVEN — bug is in executing the framework helper, not in binding.** I traced the whole binding chain against boa 0.21 semantics (`ImportSpecifier.binding`=local, `.export_name`=imported; `ExportSpecifier.private_name`=local, `.alias`=exported). `compile_import_declaration` (modules.rs) registers `_o → Named{framework, "L"}`; the use-site (`statements.rs` `ModuleImport` arm) emits `emit_module_import_name(framework, "L")` = fetch namespace["L"]; `compile_export_list` reads private_name/alias correctly. All correct. Also: the failure is at the `vs(` call, not the `_o(` call, so `_o` (framework export `L`) IS a callable function — but **`L()` returns the array `["items"]` instead of a wrapper function.** So the remaining bug is that our engine mis-executes the Vue runtime helper `L`'s body (in `framework.P2XOc7lE.js`) to yield `["items"]`. NEXT STEP: locate `L`'s definition in `framework.P2XOc7lE.js` (use the new backtrace / a probe on `_o`'s value right after `vs=_o()`), identify which JS construct in `L`'s body our engine evaluates to `["items"]`, and reproduce that construct in isolation. (`L` is likely Vue's `withScopeId`/`pushScopeId`-family helper: `fo("data-v-…")` then `vs=_o()` then `vs(renderFn)`.)
- **Process hazards hit this session (for the next agent):**
  - **Codex ran `cargo fmt`** at the end of the first source-position run, reformatting the ENTIRE codebase (38 files, thousands of whitespace-only lines). Unshippable. Recovered via `git checkout -- .` and re-ran Codex with an explicit "DO NOT run cargo fmt / minimal diff / list only the 4 target files" banner — the redo produced a clean 4-file diff. Always forbid `cargo fmt` in Codex prompts and check `git diff --numstat` before staging.
  - **Disk filled up** (`target/` reached 28.8 GB; C: hit 0.01 GB free) mid-session from repeated rebuilds, causing spurious `autocfg`/`os error 112` build panics that look like test failures but are not. `cargo clean` freed it (→ 22 GB). If builds panic in build scripts, check free space first.
