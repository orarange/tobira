# Engine Integration Plan — replacing `boa` with the self-built engine

Goal: make the self-built engine (`src/engine/`) the browser's actual JavaScript
runtime and remove `boa` entirely ("完全自作"), without regressing the browser's
existing features (DOM, shadow DOM, CSS, async UI).

This is the design for the real "boa removal" milestone. It supersedes the
risky direct merge of `codex/js-engine`'s old browser runtime into `master`:
land the engine as a library first, then integrate incrementally on `master`.

---

## 1. Where we are

**Engine (`src/engine/`, branch `codex/js-engine`)** — conformance-strong:
- Bytecode VM (`vm.rs`), custom lexer/parser/compiler, GC-able heap.
- Feature probes: tier-1 **131/131**, tier-2 **71/78** (closures/`this`, classes +
  private fields/methods, generators, RegExp, Symbol+iteration, Date, Proxy,
  Reflect, WeakMap, JSON reviver/replacer/toJSON, `arguments`, hoisting, …).
- A full **event loop** (`event_loop.rs`: macrotasks, microtasks, timer heap,
  RAF) driven by `Vm::event_loop_tick(now_ms, has_render_opportunity)`.
- A **`Host` trait** (`host.rs`) that was *explicitly designed to capture the
  current `src/js.rs` host footprint*: DOM read/mutate/event, console, timers,
  RAF, fetch, storage, observers, navigation, history, time, and a host-event
  pump. `Vm::with_host(Heap, Box<dyn Host>)` injects it.
- `vm.rs` already implements many JS-facing DOM/browser globals routed through
  the `Host` trait (document.querySelector/createElement/…, node mutation,
  classList, style, addEventListener, getBoundingClientRect, console,
  setTimeout/clearTimeout/setInterval, requestAnimationFrame, localStorage/
  sessionStorage, window scroll/getComputedStyle/matchMedia, crypto stubs).
- Tested in isolation via `Vm::with_host(Heap::new(), Box::new(TestDom))`
  (`tests/phase6_dom.rs`).

**Browser (`src/js.rs` + `browser.rs` + `gui.rs`, `master`)** — runs on `boa`:
- `src/js.rs` (~13–15k lines, ~1600 `boa` refs) builds the JS environment on a
  `boa_engine::Context`, exposing to the rest of the browser a **stable public
  API**:
  - `process_document_scripts(html, base_url) -> ProcessedScriptHtml`
  - `start_document_script_session(html, final_url) -> (ProcessedScriptHtml, JavaScriptSession)`
  - `struct JavaScriptSession` with event dispatch, pending-fetch pumping, and
    `ProcessedScriptHtml` snapshot production.
  - `DomEventRequest` / `DomEventDispatchResult`.
- `browser.rs` (`BrowserPage`) owns `Option<JavaScriptSession>` and drives it:
  `apply_script_snapshot`, `refresh_from_script_session`, event dispatch,
  `has_pending_fetches` / `fetch_result_queue`.
- `gui.rs` drives timers/RAF/animation and pumps the session.

**Key consequence:** the integration contract already exists (the `Host` trait),
and the browser ↔ scripting boundary is a *small, stable API* in `js.rs`. So the
migration is "swap the internals of `JavaScriptSession` from `boa` to the
engine" while keeping that public API — `browser.rs` / `gui.rs` barely change.

---

## 2. Strategy: keep the `js.rs` public API, swap the engine underneath

Do **not** rewrite `browser.rs` / `gui.rs`. Reimplement, behind the existing
`js.rs` surface:

| Public `js.rs` item | New implementation |
| --- | --- |
| `start_document_script_session` | `Parser::new(src).parse()` → `Compiler::compile()` → `Vm::with_host(Heap, BrowserHost)::execute()` |
| `JavaScriptSession` | wraps a `Vm` + `BrowserHost` instead of a `boa` `Context` |
| event dispatch | `vm` invokes registered listeners via the engine's call path |
| pending fetches | `BrowserHost::fetch` enqueues; results pumped via `HostEvent::NetworkResponse` + `event_loop_tick` |
| `ProcessedScriptHtml` snapshot | produced from the engine-side DOM mutations, same shape as today |
| async/timers | `gui.rs` calls `vm.event_loop_tick(now, render_opportunity)` instead of `boa` `context.run_jobs()` |

Keep `ProcessedScriptHtml`, `DomEventRequest`, `DomEventDispatchResult`
byte-compatible so callers are unaffected.

---

## 3. Staged migration (each stage keeps the build + browser working)

### Stage 0 — Land the engine on `master` as a library
Merge only `src/engine/`, `tests/`, `src/lib.rs` (`pub mod engine;`), and the
Cargo deps onto `master`; keep `master`'s `boa` browser untouched. `master`
builds; the engine ships fully tested but unused by the browser yet. (This is
the low-risk "engine-as-lib" merge — the safe foundation for everything below.)

### Stage 1 — `BrowserHost`: implement the `Host` trait for the real browser
**Status: ✅ landed (PR #53).** `src/engine_host.rs` implements `Host` over an
arena DOM built from `html::parse_document`, with full `read_dom`/`mutate_dom`
coverage, a right-to-left selector matcher (lists, compound, descendant/child
combinators), console capture, location, and HTML (re)serialization. `fetch_sync`
and `MutationObserver` recording are now implemented; remaining observers
(`Resize`/`Intersection`) are stubbed (Stage 3). Note a deviation from the
original sketch: the host builds its **own** arena from the document HTML rather
than sharing the browser's live `html::Node` tree; reconciling the two (so
interactive mutations reflect back) is part of Stage 2/3.

Create `src/engine_host.rs` (a production `impl Host`) bridging each trait
method to the browser subsystem the `boa` path already uses:

| `Host` method | Bridges to |
| --- | --- |
| `read_dom` / `mutate_dom` | the browser's live DOM tree (same backend `js.rs`'s boa bindings mutate) |
| `dispatch_dom_event` | the browser's capture/bubble event dispatch |
| `console` | console-log collection surfaced in `ProcessedScriptHtml` |
| `schedule_timer` / `cancel_timer` / `request_animation_frame` | `gui.rs` timer/RAF queues |
| `fetch` / `abort_fetch` | `http.rs` (same client `js.rs` uses) |
| `storage` | origin-scoped local/session storage + cookies |
| `observer` | Mutation/Resize/Intersection observers |
| `navigate` / `history` | the browser navigation + history stack |
| `now` / `wait_for_host_events` | monotonic/wall clock + the GUI event pump |

Reuse the existing boa-side DOM/network/storage backends; only the *binding
layer* changes (trait calls instead of boa `NativeFunction`s).

### Stage 2 — Reimplement `JavaScriptSession` on the engine
**Status: 🚧 in progress.** The flag-gated **initial-render path** is wired:
`TOBIRA_ENGINE=1` makes `start_document_script_session` parse/compile/execute the
document's inline scripts on the `Vm` + `BrowserHost` (via
`engine_host::run_document_scripts`) and return the resulting
`ProcessedScriptHtml`. `boa` stays the default when the flag is unset.

The initial path now also **settles async deferred work** before snapshotting:
after the inline scripts run, `Vm::run_due_jobs` drains Promise microtasks and
fires zero-delay timers (`setTimeout(fn, 0)`) to quiescence *without advancing
virtual time*, so deferred first-render patterns reflect in the snapshot.
Genuinely delayed timers stay pending (they belong to the persistent session).

The **persistent interactive session** is now wired too: `start_document_script_session`'s
engine branch spawns a worker thread that owns an `engine_host::EngineSession`
(`Vm` + `BrowserHost`) and services the same `JavaScriptSessionCommand` protocol
the boa worker does — `DispatchEvent`, `DispatchGlobalEvent`, `Snapshot`,
`SetAttribute` — returning a real `JavaScriptSession`, so `browser.rs`/`gui.rs`
need **no changes**. **Node-identity is reconciled**: `BrowserHost::handle_for_node_id`
reproduces `browser.rs::annotate_node_ids` numbering exactly (verified by tests
over the real serialize→parse→annotate pipeline), so a click dispatched by
`target_node_id` lands on the right engine node, runs its listeners, mutates the
DOM, and the next snapshot reflects it.

**Verified in the GUI** under `TOBIRA_ENGINE=1`: clicks, scrolling, and typing
all work. Scroll no longer rebuilds the page (listener-free `scroll`/`resize`
events are skipped) and re-paints at the new offset; the engine tracks the
scroll/viewport so `window.scrollY`/`innerWidth` are correct.

**Keyboard/pointer/input events** now carry their details: host events build an
Event object with `target`, `key`, `code`, `data`, `inputType`, modifier flags,
and pointer coords, plus working `preventDefault`/`stopPropagation`/`stopImmediatePropagation`.
`dispatch_event` surfaces `default_prevented` so the browser can suppress the
default action (e.g. a key handler cancelling text insertion). The typed value
reaches the engine via the existing `set_attribute("value", …)` sync, so
`input.value` / `event.target.value` are correct inside listeners.

Continuous timers / animation (done): `gui.rs`'s `about_to_wait` drives the
active page's event loop while it has pending work, via `Vm::pump_event_loop`
(all due timers + one rAF pass per frame) behind a `JavaScriptSession::tick`.
A virtual clock advances only while animating (real-time-sized, clamped) so a
timer created in a click handler measures its delay from "now" rather than page
load, and `ControlFlow::WaitUntil(~16ms)` paces it to ~60fps (idle pages stay on
`ControlFlow::Wait`). No-op frames skip the DOM serialize. This makes
`setInterval`, `setTimeout(fn, delay)`, and `requestAnimationFrame` animation
loops fire over time. **Stage 2 is now feature-complete.**

Behind the unchanged public API: parse/compile/execute document scripts on the
`Vm` + `BrowserHost`, drive the event loop from `gui.rs` via `event_loop_tick`,
and produce the same `ProcessedScriptHtml` snapshots. Gate behind a
`TOBIRA_ENGINE=1` env flag at first so `boa` stays the default until parity holds.

### Stage 3 — Feature-parity pass (the real work)
Port the JS-facing APIs `boa`'s `js.rs` exposes but the engine lacks. Confirmed
gaps from a scan of `vm.rs` vs `js.rs`:

- **Shadow DOM / WebComponents**: `customElements.define/get/whenDefined/upgrade`,
  `attachShadow`, `slot.assignedNodes/assignedElements`, `assignedSlot`,
  `slotchange`, `Event.composedPath()`, shadow-boundary retargeting. (Host enum
  already has `AttachShadow`, `ShadowRoot`, `AssignedNodes`; the *JS bindings*
  are missing in `vm.rs`.)
- **Observers**: `MutationObserver` ✅ **done** — JS class with
  `observe(target, init)` / `disconnect()` / `takeRecords()`, `childList` +
  `attributes` (with `attributeOldValue`) + `subtree`, delivering
  `MutationRecord`s (`type`/`target`/`addedNodes`/`removedNodes`/`attributeName`
  /`oldValue`) at the microtask checkpoint. `BrowserHost` records mutations from
  `mutate_dom`; the VM owns the callbacks and delivers them. Still missing:
  `ResizeObserver`, `IntersectionObserver` (need layout geometry), and
  `characterData` records.
- **Networking JS**: `fetch(...)` global + `Response`/`Headers` ✅ **done**
  (synchronous via `Host::fetch_sync`; returns a resolved `Promise<Response>`
  with `ok`/`status`/`statusText`/`url`/`headers.get()`/`text()`/`json()`;
  `BrowserHost` performs the real HTTP via `http.rs`, resolving relative URLs
  against the document URL). Still missing: streaming/abort, `Request` class,
  and **`XMLHttpRequest`**. Note the current fetch blocks the worker for the
  request (no async host-event loop yet).
- **Event constructors** ✅ **done**: `Event` / `CustomEvent` (#58); keyboard/
  pointer/input event detail on dispatched events (#60). Still missing:
  `KeyboardEvent`/`MouseEvent` *constructors*, `AbortController`/`AbortSignal`.
- **Misc**: `URLSearchParams` ✅ (#52), `MutationObserver` ✅, richer `crypto`,
  `history.state`/`popstate`/`hashchange`.
- Audit the full `js.rs` DOM/node binding list against `vm.rs`'s `BuiltinId`
  DOM set and close any remaining method gaps.

#### Framework-readiness pass ✅ (heavy-JS / React-class apps, #65–#69)

Empirically driven by two diagnostic harnesses — `tests/heavy_js_probe.rs`
(82 language/stdlib cases) and `dom_heavy_probe_report` in `engine_host.rs`
(29 DOM cases via the real `BrowserHost`) — to find and close the gaps that
broke framework-style code. All probes pass; an end-to-end
`react_like_app_full_interactive_loop` test runs a small React-style app
(element factory, state, re-render, event delegation) through the real dispatch
path.

- **ToPrimitive object coercion** (#65): `Symbol.toPrimitive`/`valueOf`/`toString`
  routed through `+`, arithmetic, unary `+`/`-`, relational, and template
  literals; `'' + [1,2,3]` → `"1,2,3"` (Array.prototype.toString). Fixes the
  pervasive `"[object Object]"` breakage.
- **`window`/`globalThis` expando writes** (#65): assigning unknown globals
  creates global bindings (UMD globals, feature flags).
- **Object/array semantics** (#66): nested object-spread stack bug fixed
  (`{a, p:{...x}}`), object-rest excludes destructured keys, `Array.sort` is
  stable (insertion), `instanceof` honors `Symbol.hasInstance`, `lastIndexOf`.
- **DOM node identity** (#67): node wrappers interned by handle, so
  `el.parentNode === parent` and expandos persist. Plus `el.style.<camelCase>`
  reads + `cssText`, `el.dataset`, real `prepend()`, `hasChildNodes()`,
  `createElementNS`, and a `replaceChild` panic fix.
- **Events** (#68, #69): `dispatchEvent` AND host-fired (real user) events
  bubble target→ancestors with `currentTarget`/`target`,
  `stopPropagation`/`stopImmediatePropagation`; `removeEventListener` works;
  node expando properties persist. Event delegation (one root listener) now
  works for real clicks — the model React/most frameworks use.

Still missing for full React parity: `ResizeObserver`/`IntersectionObserver`,
`XMLHttpRequest`, `customElements`/shadow DOM, `characterData` mutation records,
and attribute reflection for some IDL props (`checked` etc.).

Track this as a parity checklist; each item is engine-side work mirroring an
existing `boa` binding (the host backend already exists).

### Stage 4 — Cutover and remove `boa`
- Flip the default to the engine; run the existing JS suite on it.
- Real-site smoke tests in `--release` GUI (Google → Wikipedia → a docs SPA →
  YouTube shell), watching for DOM/async/render regressions.
- Delete `boa`-backed code paths in `js.rs`; remove `boa_engine`/`boa_gc` from
  `Cargo.toml` (the engine already uses `boa_ast`/`boa_parser`/`boa_interner`
  only for its own AST — decide separately whether to also drop those by
  finishing the custom parser).

---

## 4. Verification ladder

1. Engine unit + probe tests (already green: 131/131, 71/78).
2. Existing JS suite running on the engine instead of boa.
3. DOM/shadow-DOM/observer parity tests ported from the boa path.
4. Real-site smoke tests in the `--release` GUI.
5. `boa_engine`/`boa_gc` removed; full `cargo build` + `cargo test` green.

## 5. Risks & mitigations

| Risk | Mitigation |
| --- | --- |
| DOM/API coverage gaps vs boa | Stage 3 parity checklist; flag-gated cutover keeps boa as fallback until parity holds |
| Microtask / event-loop timing differences | Diff against the existing async-ordering tests (`tests/phase5_async.rs`) before cutover |
| GUI render/animation integration (same class of bugs hit earlier this session) | Verify in the real `--release` app, not just unit tests |
| Snapshot / reflow mechanism mismatch | Keep `ProcessedScriptHtml` shape identical; compare boa vs engine snapshots on the same pages |
| Big-bang regression | Strict staging + `TOBIRA_ENGINE` flag; never remove boa until the engine passes the ladder |

## 6. Effort shape

- Stage 0: small (mechanical merge).
- Stage 1–2: the structural core (`BrowserHost` + `JavaScriptSession` reimpl).
- Stage 3: the bulk — iterative parity work, mostly mirroring existing boa
  bindings onto the engine + Host backend.
- Stage 4: mechanical once parity holds.

The engine's conformance is already strong; the remaining work is *integration
breadth* (DOM/host API parity), not language correctness.
