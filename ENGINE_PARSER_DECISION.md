# Engine Parser Decision

## Decision

Build the parser ourselves.

OXC remains the fallback if the custom parser misses Phase 1 exit criteria badly enough to block the engine schedule, but the planned path is a custom lexer + parser.

## Context

The engine is intentionally Tobira-specific:

- bytecode VM from the start
- Rust-native DOM bindings
- no `boa` compatibility layer
- eventual ownership of async semantics, GC, and host integration

Because the AST is an internal compiler input rather than a public API, parser choice should be judged by how well it serves the bytecode compiler and the long-term maintenance model.

## Options

### Option A: Custom parser

Pros:

- AST shape can match the bytecode compiler directly instead of mirroring another project's tree.
- Avoids a persistent AST translation layer between parser output and compiler input.
- Keeps parser invariants, node kinds, and source-span strategy aligned with the bytecode compiler.
- Fits the repo's self-hosting direction; HTML/CSS parsing are already native.
- Keeps the hard semantics work local instead of mixing "our VM" with "someone else's parser model".
- Easier to make parser/debug output and invariants match the exact needs of this engine.

Cons:

- Upfront schedule cost is real.
- JavaScript parsing has sharp edges: ASI, template literals, regex-literal disambiguation, and complex patterns.
- We must build and maintain our own parser regression corpus.

### Option B: Reuse OXC

Pros:

- Mature grammar coverage on day one.
- Very fast parsing of large minified bundles.
- Existing ecosystem familiarity and battle-tested syntax support.

Cons:

- Requires an AST translation layer or forces the compiler to target OXC's tree shape.
- Raises the maintenance surface: dependency upgrades, AST churn, feature flags, and internal representation mismatch.
- Pulls the project away from the current self-hosted direction while only solving the easiest part of the engine problem.
- Does not reduce the hard implementation work: runtime semantics, object model, async ordering, host bindings, and GC still dominate risk.

## Comparison Matrix

| Criterion | Custom parser | OXC |
| --- | --- | --- |
| Time to first parse | Slower | Faster |
| Fit for bytecode compiler | Best | Adapter required |
| Long-term ownership | Best | External dependency |
| Syntax correctness risk | Higher initially | Lower initially |
| Large-bundle parse throughput | Good enough if well-built | Excellent |
| Self-hosting consistency | Best | Weak |
| Total engine complexity removed | Low | Low |

## Why OXC Is Not the Default

OXC makes Phase 1 easier, but it does not materially simplify the engine's hardest phases:

- closures and scope capture
- property semantics and prototypes
- Promise/microtask ordering
- timers, rAF, observers, network completion
- host bindings onto Tobira's DOM/layout/browser state
- memory management

In other words, OXC improves the least risky major component while introducing an adapter boundary that will stay with us forever.

That does not mean compiler work such as scope analysis, temporary-slot assignment, closure lowering, or destructuring normalization disappears on the custom path. Those passes still exist either way; the benefit is narrower and more honest: a custom parser lets those passes consume an AST designed for this compiler instead of an imported one.

## Why Custom Parsing Is Acceptable Here

The engine is not trying to be a drop-in embeddable JavaScript runtime with every browser quirk on day one.

Scope assumptions help:

- target ES2020+ syntax first
- scripts before full module loader complexity
- syntax support can be validated directly on the real bundle corpus we care about
- parser internals can be optimized around the compiler instead of around a reusable public AST

That makes the parser challenging but tractable.

## Mitigations for the Custom Path

- Keep the AST intentionally compiler-oriented instead of "spec pretty".
- Build the lexer and parser with snapshot-heavy tests from the start.
- Add focused corpora for:
  - minified bundles
  - destructuring and parameter lists
  - template literals
  - regex/division ambiguity
  - ASI edge cases
- Parse the YouTube main bundle as the Phase 1 reality check, not just toy files.
- Treat that bundle milestone as a realism gate, not as a correctness proof; correctness still comes from targeted syntax suites plus continued real-bundle coverage.

## Fallback Trigger

Switch temporarily to OXC only if one of these becomes true:

1. The custom parser cannot reach "parse the target large bundle" within the planned Phase 1 window.
2. Syntax-correctness bugs remain the main blocker after the lexer and parser structure are already in place.
3. The team decides the bytecode compiler must start before parser correctness is under control.

If that happens, treat OXC as a schedule escape hatch, not the preferred end state.

## Recommendation

Proceed with a custom parser.

Why this still wins despite the higher early syntax-risk:

- the AST/compiler fit matters for the whole lifetime of the engine, not just Phase 1
- the fallback to OXC remains available if schedule risk turns into a real blocker
- the hardest engine risk is still semantics and host/runtime behavior, so the parser choice should not permanently contort the compiler unless it clearly buys down schedule risk enough to justify it

That makes the custom parser the default recommendation, with OXC kept as an explicit escape hatch instead of the baseline architecture.
