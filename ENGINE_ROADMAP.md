# JS Engine Roadmap (Self-Hosted Engine)

This document is the roadmap for **replacing boa with a custom, from-scratch JavaScript engine** built specifically for Tobira.

It is separate from `JS_ROADMAP.md`, which tracks JS *feature* coverage on top of the current boa runtime. This file tracks the *engine replacement* effort.

## Why Build Our Own Engine

Tobira already self-hosts every other layer of the stack:

- HTML parser — self-built
- CSS parser / selector / cascade — self-built
- Layout engine — self-built
- Renderer — self-built

The JS runtime (boa) is the only borrowed core, and it is now the main bottleneck:

- **boa is slow.** YouTube's 10MB main bundle would take tens of seconds, if it ran at all.
- **The integration is awkward.** `#[unsafe_ignore_trace]`, GC-rooting workarounds, and the `settle_pending_state` convergence loop all exist to work around boa's model.
- **No control over the roadmap.** We cannot tune the engine for Tobira's actual workload.

A custom engine designed for Tobira's workload can make different, cheaper tradeoffs (see Design Principles).

## Locked Decisions

| Decision | Choice | Notes |
| --- | --- | --- |
| Interpreter | **Bytecode VM** from the start | AST → bytecode → stack-machine VM. Needed for YouTube-grade throughput. |
| Parser | **Custom (recommended)** | See "Parser Decision" below — leaning custom, finalize in Phase 0. |
| boa removal | **After core is complete** | Develop on a new branch; keep boa compiling until the new engine's core (language + async + DOM integration) is proven, then rip boa out in one pass. |

### Parser Decision (finalize in Phase 0)

Recommendation: **build the parser ourselves.**

Rationale:
- Consistent with the project's self-hosting philosophy (we already wrote the HTML and CSS parsers).
- The JS grammar is well-specified and has abundant reference implementations, so it is one of the *more* tractable parts to build with LLM assistance.
- The genuinely hard parts of an engine are the *semantics* (object model, async ordering, GC) — not parsing. Owning the parser does not add meaningful risk to those.
- Owning the AST lets us shape it for our bytecode compiler instead of adapting to OXC/SWC's AST.

Cost: a JS lexer + parser is roughly 3–6 weeks of focused work. If that proves too slow, the fallback is OXC (fastest Rust JS parser) — but we start custom.

## Target Staging

The **ultimate goal is a Chrome replacement.** That is years out and not the near-term target.

**Near-term target: practical pages render and operate at Chrome-comparable quality.** News sites, docs, SPAs, search, forms — they look right and are interactive.

The rule for every design decision: **prioritize what practical pages need now, but never bake in an assumption that blocks the eventual Chrome-replacement goal.** In particular, do not hardcode the current "settle once and stop" model — design for a live runtime from day one, even if the heavier continuous features are deferred.

Deferred (not blocked) for later: 60fps compositor loop, `<video>`/`<audio>` decode & playback, WebSocket / long-polling, WebGL/WASM, service workers.

## Design Principles (Tobira-Specific Shortcuts)

A general-purpose engine is enormous. We are NOT building V8 in one go. We exploit Tobira's constraints where they don't conflict with the Chrome-replacement goal:

- **Live runtime, per-page heap.** The runtime must keep running while a page is open (real timers, event loop, rAF), NOT settle-and-stop. But the JS heap is still scoped to one page and dropped wholesale on navigation — same as Chrome discarding a tab's heap.
- **No JIT (for now).** A well-built bytecode interpreter is enough to reach the near-term target; JIT is the last performance step and is explicitly deferred, not designed out.
- **No security sandbox (for now).** We are not isolating untrusted multi-tenant code yet, so we skip that hardening. Revisit before any "real Chrome replacement" claim.
- **Rust-native DOM.** `document.getElementById(...)` returns a handle backed directly by our Rust DOM, instead of marshaling through a generic object model.
- **Target ES2020+.** No legacy quirks-mode JS compatibility burden.

## Architecture

```
JS source text
  → Lexer (custom)            → tokens
  → Parser (custom)           → AST
  → Bytecode compiler         → Chunk { code, constants, functions }
  → VM (stack machine)        → execution
        ↕ Host bindings (DOM, window, fetch, timers, observers)
        ↕ Heap / GC (arena + optional mark-sweep)
```

### Value representation

Start with a tagged enum for clarity; revisit NaN-boxing only if profiling demands it.

```
Value = Undefined | Null | Bool(bool) | Number(f64)
      | String(GcRef<JsString>) | Object(GcRef<JsObject>) | Symbol(SymbolId)
```

### Heap / GC strategy

1. **Phase 2 baseline:** bump/arena allocation, *no* in-session collection. The entire heap is dropped on navigation. Simplest possible; correct for short page sessions and enough to reach the near-term target.
2. **Required before the Chrome-replacement goal:** a real mid-session collector (mark-sweep). A long-lived tab running an SPA for hours WILL generate garbage that must be reclaimed without navigating away. The drop-on-navigation shortcut is necessary but not sufficient. Build this once memory profiling on a long-running page shows it's needed — but treat it as on the critical path, not optional.

Starting without in-session GC sidesteps cycle detection for the initial milestones, but the heap layout must be designed so a collector can be added later (object headers, root tracking).

---

## Phases

### Phase 0 — Foundation & Scaffolding

Goal: a new branch with the engine crate skeleton compiling alongside boa.

- create the new branch; add the engine as an internal module/crate that builds but is unused
- define `Value`, the heap/arena allocator skeleton, and `GcRef<T>`
- define the **Host boundary trait** — the single interface between the VM and Tobira (DOM access, console, timers, network). This is the contract the existing `js.rs` bindings will be ported onto.
- finalize the parser decision (custom vs OXC)

Exit: `cargo build` passes with boa still active; engine module compiles as dead code.

### Phase 1 — Lexer & Parser

Goal: JS text → AST.

- lexer: full ES2020 token set, template literals, regex literal disambiguation, ASI (automatic semicolon insertion)
- parser: expressions, statements, functions, classes, destructuring patterns, modules-as-scripts
- validate with a round-trip corpus (parse → pretty-print → re-parse equivalence) and a snapshot test set

Exit: the YouTube main bundle parses without error into an AST.

### Phase 2 — Bytecode Compiler & VM Core

Goal: run real (non-async, non-OOP) programs.

- bytecode format (`Chunk`: instructions, constant pool, nested function protos)
- compiler: expressions, operators (incl. correct coercion rules), control flow, loops
- VM: stack machine, call frames, lexical scopes, closures (upvalue capture), `this` binding
- the `JS_LOOP_ITERATION_LIMIT`-style runaway guard, but per-call-budget rather than global

Exit: arithmetic, string, control-flow, and closure-heavy programs run correctly against a unit corpus.

### Phase 3 — Object Model

Goal: the prototype-based object system.

- objects, property descriptors (value/get/set/enumerable/configurable/writable)
- prototype chain lookup and `[[Prototype]]` operations
- built-in prototypes: `Object`, `Function`, `Array`, `String`, `Number`, `Boolean`, `Math`, `JSON`
- property access fast path (prep for inline caches in Phase 7)

Exit: idiomatic OOP JS (constructors, prototype methods, `Object.defineProperty`) runs correctly.

### Phase 4 — Language Completeness

Goal: the rest of ES2020 surface that real bundles use.

- `class` (fields, methods, static, inheritance, `super`)
- destructuring, spread/rest, default params, template literals
- `RegExp` (decide: port a regex engine vs. wrap an existing Rust regex crate with JS-regex semantics)
- `Map`, `Set`, `WeakMap`, `WeakSet`, `Symbol` (incl. well-known symbols)
- error types, `try`/`catch`/`finally`, `throw`, stack capture

Exit: a representative modern minified bundle executes its synchronous top level without unimplemented-feature errors.

### Phase 5 — Async Core & Live Event Loop ⚠️ (highest risk)

Goal: a **persistent event loop** with spec-accurate asynchronous execution — not a batch "settle and stop" pass. **Requires an explicit written design before coding.**

This is the part most prone to subtle bugs (the current boa branch already hit `setTimeout` ordering bugs) and is also where the live-runtime vs. settle-model distinction lives. Design the event loop + microtask/task model on paper first, citing the spec, then implement.

- **a real event loop** that stays alive while the page is open: macrotask queue, microtask checkpoint after each macrotask, and integration with the GUI thread's event source
- **wall-clock timers:** `setTimeout`/`setInterval` fire on real elapsed time, driven by the host loop — not flushed during a settle pass
- **frame clock hook:** `requestAnimationFrame` callbacks driven by a render-tick the host can pace (start at a fixed interval; a true display-refresh compositor loop is deferred)
- microtask queue (PromiseJobs) with spec-accurate draining order
- `Promise` (full Promises/A+ semantics, `then`/`catch`/`finally`, `Promise.all/race/allSettled/any`)
- `async`/`await` (built on the microtask queue)
- generators & iterators (state-machine lowering in the bytecode compiler)
- `queueMicrotask`

Design note: the current `settle_pending_state` convergence loop is a workaround for not having a real event loop. The new design replaces it — the host (GUI) drives the loop and JS runs in response to timers/events/frames, instead of Rust spinning until things stop changing.

Exit: a regression suite of nested-timeout, promise-chain, and async/await ordering cases matches real-browser output exactly; a `setInterval`-driven animation keeps ticking while the page is idle.

### Phase 5.5 — ContentThread Separation (browser-codex prerequisite)

Goal: move all page logic off the winit main thread so JS execution cannot freeze the browser chrome.

This is a **browser-codex** structural change, not an engine phase. It must land before Phase 6 so that, when the new engine is wired in, the DOM and JS run on the same ContentThread and the main thread stays responsive.

**Current problem:** `dispatch_dom_event`, `settle_pending_state`, and all JS entry points run synchronously on the winit event-loop thread. Heavy JS blocks address-bar redraws, scroll, and resize.

**Target structure:**

```
Main thread (winit)
  ├── Chrome UI only: address bar, nav buttons, title bar
  ├── Receives input events → forwards to ContentThread as ContentCommand
  ├── Receives RenderedFrame + metadata from ContentThread via BrowserUserEvent
  └── Blits rendered pixels into the softbuffer Surface and calls present()

ContentThread (one per page)
  ├── Owns DocumentView / BrowserPage (including JavaScriptSession)
  ├── Owns FontContext for content layout
  ├── Processes ContentCommand messages: Navigate, Input, Scroll, Resize, etc.
  ├── Runs JS, does layout, paints content pixels into a Vec<u32>
  └── Sends ContentEvent back via EventLoopProxy: RenderedFrame, TitleChanged, UrlChanged, etc.
```

Key properties:
- `softbuffer::Surface` is not `Send` — stays on the main thread for blit + present
- DOM and JS are both on ContentThread — no cross-thread DOM ownership problem
- History management moves to ContentThread
- JS never touches the winit thread

Exit: opening a page that runs heavy JS does not prevent the address bar from repainting or the window from resizing.

### Phase 6 — Host Integration (port from boa)

Goal: re-wire all of Tobira's browser APIs onto the new Host boundary trait.

The existing `js.rs` (~500KB) is the **specification** for this phase — every binding already exists, it just targets boa's API. Port each onto the new engine:

- `document` / DOM node bindings (Rust-native handles)
- `window`, `console`, `location`, `history`, `navigator`
- timers (`setTimeout`/`setInterval`/`requestAnimationFrame`)
- `fetch` / `XMLHttpRequest` (reuse the async design from the current branch: pending promise + background thread + settle drain)
- `MutationObserver` / `ResizeObserver` / `IntersectionObserver`
- custom elements & shadow DOM
- `localStorage` / `sessionStorage` / cookies

Exit: the existing JS test suite (currently 207 passing on boa) passes on the new engine.

### Phase 7 — Performance

Goal: YouTube-grade throughput.

- inline caches for property access (shape/hidden-class based)
- string interning
- profile the YouTube 10MB bundle end-to-end; optimize the hottest VM ops
- consider NaN-boxing the `Value` representation if it pays off
- add mid-session mark-sweep GC only if memory profiling requires it

Exit: the YouTube main bundle loads and hydrates in a few seconds, not tens of seconds.

### Phase 8 — boa Removal & Validation (complete)

Goal: cut boa out entirely and prove the replacement.

Completed:
- removed `boa_engine = "0.21.1"` and `boa_gc = "0.21.1"` from `Cargo.toml`
- rewrote `src/js.rs` from 15,527 lines (boa host bindings) to ~210 lines (new engine session)
- `JavaScriptSession` now wraps a worker thread that owns a `tobira_engine::Vm`
- `start_document_script_session` → `start_new_engine_session` (worker thread + channels)
- All browser events (click, scroll, resize) dispatch through `Vm::fire_dom_event`
- DOMContentLoaded and load fire after inline scripts complete
- `ProcessedScriptHtml` extracted to `js_common.rs` (broke the circular import)
- `src/js_host.rs` exposes `start_new_engine_vm`, `snapshot_from_vm`, `dispatch_event_on_vm`
- `cargo build` passes with boa entirely removed

Remaining validation (post-Phase 8):
- test262 subset pass rate
- real-site smoke tests: Google → Wikipedia → YouTube → SPA docs

Exit: **complete** — boa is removed; tobira-engine is the sole JS runtime.

---

## Definition of "Core Complete" (boa removal trigger)

boa is removed only after **all** of these hold on the new branch:

1. Phases 1–6 are complete (language + object model + async + host integration)
2. the existing JS test suite passes on the new engine
3. a real page (start with a small JS-driven demo, then a news/docs SPA) loads and becomes interactive

Phase 7 (performance) and Phase 8 (validation) continue after removal.

## Risk Callouts

| Risk | Where | Mitigation |
| --- | --- | --- |
| Async ordering bugs | Phase 5 | Write the microtask/task design doc *before* coding; build a browser-diff regression suite. **PM (Claude) reviews this design before Codex implements.** |
| GC correctness | Phase 2/7 | Start with no in-session collection (drop-on-navigation); add mark-sweep only when measured. |
| test262 long tail | Phase 8 | Track pass rate as a metric, not a gate; fix failures that block real sites first. |
| Performance ceiling | Phase 7 | Profile against the real YouTube bundle, not microbenchmarks. |
| Regex semantics | Phase 4 | Decide early: JS-semantics wrapper over a Rust regex crate vs. a custom matcher. |

## Validation Ladder

1. parser round-trip corpus (Phase 1)
2. language unit corpus (Phases 2–4)
3. async ordering regression suite vs. real browser (Phase 5)
4. existing 207-test JS suite ported to the new engine (Phase 6)
5. test262 subset (Phase 8)
6. real-site smoke tests: Google → Wikipedia → YouTube → SPA docs (Phase 8)

## Working Rule

Whenever a phase lands or a blocker appears:

- update this file
- update `HANDOFF.md`
- keep the boa build green until the "Core Complete" trigger
- record the change in the session log

## Division of Labor

- **Claude (PM):** owns the Phase 5 async design doc, reviews engine architecture decisions, checks Codex's work for spec-correctness, sets phase order.
- **Codex (Engineer):** implements lexer/parser/VM/bindings, writes tests, runs the validation ladder.
