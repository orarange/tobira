# JS Engine Roadmap (Self-Hosted Engine)

This document tracks the plan to replace `boa` with a Tobira-specific JavaScript engine.

It is separate from `JS_ROADMAP.md`:

- `JS_ROADMAP.md` tracks JavaScript and DOM feature coverage on the current `boa` runtime.
- `ENGINE_ROADMAP.md` tracks the engine replacement itself.

## Why Build Our Own Engine

Tobira already self-hosts the rest of the browser stack:

- HTML parser
- CSS parser and cascade
- layout engine
- renderer

`boa` is now the limiting borrowed core:

- YouTube's main bundle is far larger than the current `MAX_SCRIPT_SOURCE_BYTES` guard.
- Raising that guard does not solve the throughput problem; a ~10 MB modern bundle is still not realistic on `boa`.
- The current integration has accumulated `boa`-specific workarounds such as `#[unsafe_ignore_trace]`, explicit job draining, and `settle_pending_state`.

The replacement engine can make cheaper, Tobira-specific tradeoffs while preserving a path to the long-term Chrome-replacement goal.

## Locked Decisions

| Decision | Choice | Notes |
| --- | --- | --- |
| Interpreter | Bytecode VM from the start | AST -> bytecode -> stack VM. Tree-walk is intentionally skipped. |
| Parser | Custom parser | Finalized in Phase 0. See `ENGINE_PARSER_DECISION.md`. |
| Runtime model | Live event loop | The settle-and-stop model is retired. |
| Heap baseline | Page-scoped arena, no mid-session GC at first | Heap layout must admit later mark-sweep. |
| `boa` removal | After core is complete | Keep `boa` compiling until the new engine can replace it in one pass. |

## Target Staging

The long-term goal is a Chrome replacement.

The near-term goal is more practical: real pages should render and operate at Chrome-comparable quality for common browsing flows such as docs, search, forms, news pages, and SPA shells.

Design rule: optimize for real pages now without hardcoding assumptions that would block the eventual Chrome-replacement target.

## Design Principles

- Live runtime, per-page heap: a page keeps running while open, but its heap is still dropped wholesale on navigation.
- No JIT for now: a good bytecode interpreter is the first target.
- No security sandbox for now: defer process isolation and hostile-content hardening.
- Rust-native DOM: host bindings operate on native Tobira structures rather than a generic browser engine embedding API.
- ES2020+ target: do not optimize for legacy quirks-mode compatibility first.

## Architecture

```text
JS source
  -> lexer
  -> parser
  -> AST
  -> bytecode compiler
  -> bytecode chunk / function prototypes
  -> VM
       -> host boundary (DOM, window, timers, network, observers, storage, history)
       -> heap / GC
```

### Value Representation

Phase 0 keeps the representation explicit and readable:

```text
Value =
    Undefined
  | Null
  | Bool(bool)
  | Number(f64)
  | String(GcRef<JsString>)
  | Object(GcRef<JsObject>)
  | Symbol(SymbolId)
```

Revisit NaN-boxing only if profiling proves the tagged enum is a bottleneck.

### Heap / GC Strategy

1. Phase 2 baseline: arena allocation, no in-session collection, page heap dropped on navigation.
2. Later requirement: add a real mid-session collector once long-lived page memory proves it necessary.

Phase 0 therefore reserves:

- object headers with mark metadata
- stable `GcRef<T>` handles
- explicit root tracking
- page-oriented arenas that can later be walked by a collector

## Phases

### Phase 0 - Foundation & Scaffolding (complete)

Goal: land the unused engine skeleton alongside `boa`.

Completed in this phase:

- added `src/engine/` as a dead-code module with:
  - `value.rs`
  - `heap.rs`
  - `host.rs`
  - `mod.rs`
- defined a tagged `Value`
- defined arena/page scaffolding plus `GcRef<T>` and root tracking placeholders
- defined the first-pass Host boundary contract for DOM/window/console/timers/network/storage/history/observers
- finalized the parser decision in `ENGINE_PARSER_DECISION.md`
- drafted the async/event-loop design in `ENGINE_ASYNC_DESIGN.md` for PM review before implementation

Exit condition:

- `cargo build` passes with `boa` still active
- the engine module compiles but is not wired into runtime behavior

### Phase 1 - Lexer & Parser (complete)

Goal: JavaScript source -> AST.

- lexer: ES2020 token set, template literals, regex disambiguation, ASI
- parser: expressions, statements, functions, classes, destructuring, scripts/modules as needed
- validate with parse corpora and round-trip/snapshot tests

Completed in this phase:

- added `src/engine/ast.rs` as the compiler-facing AST surface, keeping `SourceType`, strict-mode state, and statement-level wrappers aligned with the future bytecode compiler's needs
- added `src/engine/lexer.rs` as a custom lexer with ES2020-oriented token coverage, source locations, regex-vs-division goal switching, template literal chunking, and ASI-relevant line-terminator tracking
- added `src/engine/parser.rs` as the parser entry point, using a single-pass parse flow and converting the result into the Phase 2-facing AST surface
- covered the parser with regression tests for expressions, calls/member access, destructured params, classes with `extends` and private fields, async/generator syntax, destructuring forms, template literals, optional chaining, nullish coalescing, and module syntax
- added a multi-kilobyte synthetic bundle-style regression plus an ignored manual test path that reads a full bundle from `TOBIRA_YOUTUBE_KEVLAR_BASE_PATH`
- manually verified that a full YouTube `kevlar_base` bundle (about `9.7` MB) parses end-to-end successfully

Not done in this phase:

- the bytecode compiler and VM remain Phase 2 work
- the new parser is intentionally not wired into `src/js.rs` or the existing `boa` runtime yet
- no AST lowering or execution pipeline exists beyond parsing

Exit: complete. The YouTube main bundle parses successfully via the manual bundle-path test flow.

Note: the ignored full-bundle parser test currently benefits from running with a larger stack, for example `RUST_MIN_STACK=67108864`, when invoked manually.

### Phase 2 - Bytecode Compiler & VM Core

Goal: run real synchronous programs.

- bytecode format
- compiler for expressions, control flow, loops, closures, scopes, `this`
- VM stack, frames, locals, upvalues
- runaway-script guard based on execution budget

Exit: arithmetic, strings, control flow, closures, and other core language tests pass.

### Phase 3 - Object Model

Goal: prototype-based JavaScript objects.

- property descriptors
- prototype lookup and mutation
- built-in prototypes such as `Object`, `Function`, `Array`, `String`, `Number`, `Boolean`, `Math`, `JSON`
- property access fast path groundwork

Exit: idiomatic prototype-based code executes correctly.

### Phase 4 - Language Completeness

Goal: the ES2020 surface modern bundles actually use.

- `class`, inheritance, `super`
- destructuring, spread/rest, default params, template literals
- `RegExp`
- `Map`, `Set`, `WeakMap`, `WeakSet`, `Symbol`
- errors, `try`/`catch`/`finally`, `throw`, stack capture

Exit: a representative modern minified bundle runs through synchronous startup without feature holes.

### Phase 5 - Async Core & Live Event Loop

Goal: persistent, spec-shaped asynchronous execution.

- real task queues
- microtask checkpoints
- wall-clock timers
- frame clock and `requestAnimationFrame`
- Promise jobs
- `async`/`await`
- observer delivery ordering

Implementation for this phase must follow the written draft in `ENGINE_ASYNC_DESIGN.md` after PM review.

Completed in this phase:

- added `src/engine/event_loop.rs` with `EventLoop`, single-queue `VecDeque<TaskEntry>` with `TaskSource` tags, `BinaryHeap<Reverse<TimerEntry>>` for timer scheduling, `IndexMap<u32, RafEntry>` for rAF registry, `VecDeque<MicrotaskJob>` for microtask queue
- added `ObjectKind::Promise(Box<PromiseState>)` and `ObjectKind::AsyncResumer(Box<AsyncContext>)` to `value.rs`
- added `PromiseState`, `PromiseReaction`, `AsyncContext` types
- added `Await` and `AsyncReturn` opcodes to `chunk.rs`; added `is_async` / `is_generator` flags to `FunctionProto`
- implemented `Promise` built-in with all static methods (`resolve`, `reject`, `all`, `race`, `allSettled`, `any`) and instance methods (`.then`, `.catch`, `.finally`)
- implemented `suspend_current_async_frame`: pops frame into `AsyncContext`, attaches `AsyncResumer` reactions to the awaited Promise
- implemented `resume_async_context`: restores frame with adjusted `stack_base`, runs until frame depth
- key ordering fix: `PromiseReaction` handlers that are `AsyncResumer` objects are directly resumed inside `run_microtask_job` without enqueuing an extra `AsyncResume` microtask — this makes `async/await` resume order match browser behavior
- implemented `event_loop_tick(now_ms, has_render_opportunity)`: ingest due timers, run one macrotask, drain microtasks, rAF stage with per-callback microtask checkpoints
- timer semantics: negative delays clamped to 0, nesting level > 5 clamps interval to ≥ 4 ms, intervals reschedule from `due_ms + interval_ms`, cancelled timers removed before execution
- registered `queueMicrotask`, `setTimeout`, `setInterval`, `clearTimeout`, `clearInterval`, `requestAnimationFrame`, `cancelAnimationFrame` as global built-ins
- all types exported from `src/engine/mod.rs`
- 18 Phase 5 corpus tests covering Promise static methods, microtask ordering, async/await basics, timer ordering, and async-vs-then ordering regression

Not done in this phase:

- real wall-clock time (browser-codex provides `now_ms` via `event_loop_tick`)
- real network I/O wiring (stubs; Phase 6)
- ResizeObserver delivery loop (stub in place; Phase 6)
- generator syntax (`function*`, `yield`) — not needed for `async/await`
- `AbortController`, `AbortSignal`
- ContentThread threading model (Phase 5.5)

Exit: complete. 18 + 232 = 250 tests passed on commit `a29511f`.

### Phase 6 - Host Integration

Goal: port Tobira's browser APIs from `src/js.rs` onto the new Host boundary.

- DOM/document/window bindings
- timers and event dispatch
- `fetch` / `XMLHttpRequest`
- observers
- custom elements and shadow DOM
- storage, cookies, location, history, navigator

Exit: the existing JS regression suite passes on the new engine.

### Phase 7 - Performance

Goal: practical throughput on large bundles.

- inline caches / shapes
- string interning
- hot-op profiling
- value layout tuning if warranted
- mid-session GC if memory data says it is required

Exit: the large YouTube bundle becomes practical rather than pathological.

### Phase 8 - `boa` Removal & Validation

Goal: cut over fully.

- remove `boa` from `Cargo.toml`
- delete `boa`-backed runtime paths
- run a test262 subset
- run real-site smoke tests
- update docs to reflect the new runtime

Exit: `boa` is gone and target sites are usable in core flows.

### Phase 9 - Language Conformance Hardening (complete)

Goal: close the gap between "boa removed" and "idiomatic real-world JS runs",
driven by data rather than a feature checklist.

Method: `tests/feature_probe.rs` runs ~130 representative real-world snippets
(closures, classes, destructuring, iteration, builtins, regex, JSON, dates,
generators…) and reports which fail. Each round fixes the highest-impact gaps,
adds hard regression tests, and re-runs the probe. The probe rose from **78/131
to 131/131**.

Landed in this phase:

- Compiler: transitive upvalue capture + lexical `this` in arrows; default
  parameters; object-literal and class getters/setters; labeled break/continue;
  per-iteration `let` bindings in `for` loops; tagged template literals; private
  class fields (`#x`); generator functions (`yield`, `yield*`, two-way `next`).
- Runtime library: filled out Array/String/Number/Boolean/Object/Math, global
  `parseInt`/`isFinite`/`encodeURIComponent`/…, primitive Number/Boolean
  prototypes, `Symbol` + the iteration protocol, `Date`, and a `RegExp` engine
  (test/exec/match/replace/split, named groups) on top of the `regex` crate.
- Semantics: nullish-coalescing stack fix, a real `delete`, JSON integer/indent
  formatting with key-order preservation, sloppy vs strict frozen-write behavior.

Known not-yet-done (tracked for the next conformance round): async generators,
generator object/class methods, `Proxy`/`Reflect`, `BigInt`, typed arrays,
`structuredClone`, `String`/`Date` parsing edge cases, and full microtask-pumped
async ergonomics. RegExp lookbehind/backreferences are unsupported (the `regex`
crate lacks them) and surface as a thrown error.

Next: continue the data-driven loop with a tier-2 probe, then return to Phase 7
performance work on large real bundles.

## Core Complete Definition

Remove `boa` only after all of the following hold:

1. Phases 1 through 6 are complete.
2. The existing JS suite passes on the new engine.
3. At least one real JS-driven page loads and becomes interactive on the new runtime.

## Risk Callouts

| Risk | Where | Mitigation |
| --- | --- | --- |
| Async ordering bugs | Phase 5 | Write and review the design before coding; diff behavior against real browsers. |
| Parser schedule slip | Phase 1 | Keep OXC as an explicit fallback if custom parsing stalls. |
| GC correctness | Phase 2 / 7 | Start with page-scoped allocation, but keep collector hooks in the layout from day one. |
| `callables` side-table GC | Phase 7 | `Vm.callables: HashMap<RawGcRef, Callable>` is a root set outside the heap arena. When mark-sweep lands: (1) treat every entry as a GC root during marking, (2) remove entries whose `RawGcRef` is freed. Closure upvalue cells (`Rc<RefCell<Value>>`) are also outside the arena — cycles through upvalues will not be collected; assess whether a separate cycle-collector is needed. |
| Performance ceiling | Phase 7 | Profile large real bundles, not toy benchmarks. |
| Conformance long tail | Phase 8 | Prioritize failures that block real sites before test262 completeness. |

## Validation Ladder

1. parser corpus
2. language unit corpus
3. async ordering regressions against real-browser output
4. existing JS suite
5. test262 subset
6. real-site smoke tests

## Working Rule

Whenever a phase lands or a blocker appears:

- update this file
- update `HANDOFF.md`
- keep the `boa` build green until core completion
- record the state in the session log

## Division of Labor

- PM / architecture review: async model review, phase ordering, spec-correctness checks
- Engineer / implementation: parser, VM, runtime, bindings, tests, validation
