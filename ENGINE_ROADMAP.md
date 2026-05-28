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

### Phase 1 - Lexer & Parser

Goal: JavaScript source -> AST.

- lexer: ES2020 token set, template literals, regex disambiguation, ASI
- parser: expressions, statements, functions, classes, destructuring, scripts/modules as needed
- validate with parse corpora and round-trip/snapshot tests

Exit: the YouTube main bundle parses successfully.

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

Exit: async ordering matches browsers on nested timeout / Promise / observer / rAF regressions.

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
