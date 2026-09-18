# test262 subset

From https://github.com/tc39/test262 at commit
`35d566604512cba908054eec49f85e64a59f3091` (2026-09-19). `LICENSE` is theirs.

Vendored: all of `harness/`, and under `test/built-ins/`: `Math`, `Number`,
`Symbol`, `Array/prototype/join`, `String/prototype/startsWith`,
`Object/assign`, `Promise/all` (plus the top-level files of `Array`, `Object`,
`Promise`, `String`, which a cone-mode sparse checkout brings along).

To add a directory:

```
git clone --depth 1 --filter=blob:none --sparse https://github.com/tc39/test262.git
cd test262 && git sparse-checkout set harness test/built-ins/<Dir> ...
```

copy it in, run with `TOBIRA_T262_BLESS=1`, and commit `baseline.txt` with it.

The runner is `src/engine/test262.rs` (lib crate):

```
cargo test --release --lib test262_subset -- --nocapture
TOBIRA_T262_DIR=built-ins/Math     print every failure under a directory
TOBIRA_T262_OUT=fail.tsv           every failure to a file
TOBIRA_T262_TRACE=1                name each test before it runs (for a crash no catch sees)
TOBIRA_T262_BLESS=1                rewrite baseline.txt
```

`baseline.txt` is the list of tests that pass. One that stops passing fails
the run; newly passing ones are printed until blessed. Read its diff in
review: that is what a change moved.

Outcomes are pass / fail / **absent**. Absent is a failure in a test whose
`features:` names something in the runner's `ABSENT` table -- left out on
purpose, with the reason beside it. Anything else missing is a fail.
`module` and `async` tests are counted and not yet run.
