//! The mark-sweep collector: what it reclaims, and what it must not.
//!
//! Until 2026-09-04 this file documented the opposite — the engine had no
//! in-session collector, so the heap was monotonic and the second test here
//! asserted that garbage was *never* reclaimed. That is now inverted.
//!
//! The expensive half of these tests is the second half. Reclaiming too little
//! costs memory; reclaiming too much frees an object that is still in use, and
//! because a stale `GcRef` reads back as `None` rather than faulting, the
//! damage surfaces far from its cause. Run the suite with `TOBIRA_GC_VERIFY=1`
//! to have the collector flag garbage instead of freeing it and report any
//! reference it turns out something could still reach.

use tobira_engine::engine::{Compiler, Heap, Parser, Value, Vm};

fn run(source: &str) -> Vm {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
    vm
}

fn string_bytes(vm: &Vm) -> usize {
    vm.heap()
        .strings()
        .pages()
        .iter()
        .flat_map(|page| page.iter_cells())
        .map(|cell| cell.value().text.len())
        .sum()
}

/// `TOBIRA_GC_VERIFY` deliberately flags garbage instead of reclaiming it, so
/// the two tests that measure reclamation have nothing to measure there. The
/// tests that matter under verify mode -- the ones checking that reachable data
/// survives -- run in both.
fn reclaims(test: &str) -> bool {
    if std::env::var_os("TOBIRA_GC_VERIFY").is_some() {
        eprintln!("{test}: skipped, TOBIRA_GC_VERIFY condemns instead of reclaiming");
        return false;
    }
    true
}

fn number(vm: &mut Vm, source: &str) -> f64 {
    match vm.eval_source(source) {
        Ok(Value::Number(value)) => value,
        other => panic!("expected a number from {source}, got {other:?}"),
    }
}

#[test]
fn fresh_vm_starts_small() {
    let vm = run("");
    let live = vm.heap().objects().len();
    eprintln!("fresh_vm live objects: {live}");
    // Baseline is the builtin/prototype objects installed at startup, and it
    // grows as standard surface is added. The bound has generous headroom — its
    // job is to catch a fresh VM ballooning into the thousands, not to pin the
    // exact count. Bump it when new builtins legitimately push it up, and say
    // what moved.
    //
    // 2026-08-24: 556 -> 619, from the per-tag HTML element interfaces plus the
    // event and structural DOM interfaces.
    //
    // 2026-08-30: 619 -> 726. The DOM interface prototypes now carry the
    // methods a page can borrow off them (`Element.prototype.matches.call`).
    assert!(
        live < 800,
        "fresh VM should start small, got {live} live objects"
    );
}

#[test]
fn throwaway_objects_are_reclaimed() {
    if !reclaims("throwaway_objects_are_reclaimed") {
        return;
    }
    let vm = run("for (let i = 0; i < 20000; i++) { let g = { a: i, b: [i, i + 1], c: 'x' }; } 1;");
    let live = vm.heap().objects().len();
    let (collections, objects_freed, _) = vm.gc_stats();
    eprintln!("live={live} collections={collections} freed={objects_freed}");
    assert!(collections > 0, "the collector should have run");
    // Three objects per iteration, none of them reachable after it. Before the
    // collector this was 20,000+ live; the bound is the collector's threshold,
    // not the loop count.
    assert!(
        live < 8_000,
        "garbage from 20,000 iterations should not all be live, got {live}"
    );
}

#[test]
fn a_string_builder_no_longer_grows_quadratically() {
    if !reclaims("a_string_builder_no_longer_grows_quadratically") {
        return;
    }
    // `s += 'x'` leaves every intermediate behind. Retained, that is n²/2
    // bytes: half a gigabyte at n = 32,000. Collected, it is bounded by the
    // collector's threshold and barely moves with n.
    let small = string_bytes(&run("let s=''; for(let i=0;i<2000;i++){ s += 'x'; } s.length;"));
    let large = string_bytes(&run("let s=''; for(let i=0;i<32000;i++){ s += 'x'; } s.length;"));
    eprintln!("string bytes: n=2000 -> {small}, n=32000 -> {large}");
    // n grows 16x. Quadratic retention would be 256x the bytes.
    assert!(
        large < 8 * 1024 * 1024,
        "retained string bytes should stay bounded, got {large} at n=32000"
    );
    assert!(
        large < small.saturating_mul(16),
        "retention should not scale with n²: {small} at n=2000 vs {large} at n=32000"
    );
}

#[test]
fn reachable_data_survives_collection() {
    let mut vm = run(
        "globalThis.keep = [];
         for (let i = 0; i < 5000; i++) { globalThis.keep.push({ n: i, s: 'v' + i }); }
         for (let i = 0; i < 20000; i++) { const junk = { x: i, y: 'j' + i }; }",
    );
    assert!(vm.gc_stats().0 > 0, "the collector should have run");
    assert_eq!(number(&mut vm, "globalThis.keep.length"), 5000.0);
    assert_eq!(number(&mut vm, "globalThis.keep[4999].n"), 4999.0);
    // The string on a retained object has to survive with it.
    assert_eq!(
        vm.eval_source("globalThis.keep[4999].s === 'v4999'"),
        Ok(Value::Bool(true))
    );
}

#[test]
fn closure_captures_survive_collection() {
    // A closure's captured variables hang off the function object through the
    // `callables` table, which is weak by key — traced only once the function
    // object is known live. Get that wrong and the capture is freed while the
    // closure is still callable.
    //
    // Captures the loop variable directly. Capturing a `const` declared inside
    // the loop body would be the more natural way to write this, but that is
    // broken independently of the collector: every closure ends up sharing one
    // binding. See the note in HANDOFF.
    let mut vm = run(
        "globalThis.fns = [];
         for (let i = 0; i < 2000; i++) { globalThis.fns.push(() => i * 2); }
         for (let j = 0; j < 20000; j++) { const junk = { x: j, y: 'j' + j }; }",
    );
    assert!(vm.gc_stats().0 > 0, "the collector should have run");
    assert_eq!(number(&mut vm, "globalThis.fns[1999]()"), 3998.0);
    assert_eq!(number(&mut vm, "globalThis.fns[0]()"), 0.0);
    assert_eq!(number(&mut vm, "globalThis.fns[1000]()"), 2000.0);
}

#[test]
fn pending_timers_and_promises_survive_collection() {
    // Callbacks waiting in the event loop are reachable from nothing else.
    let mut vm = run(
        "globalThis.log = 0;
         setTimeout(function () { globalThis.log += 1; }, 0);
         Promise.resolve(41).then(function (v) { globalThis.log += v; });
         for (let i = 0; i < 20000; i++) { const junk = { x: i, y: 'j' + i }; }",
    );
    assert!(vm.gc_stats().0 > 0, "the collector should have run");
    vm.run_due_jobs(100);
    assert_eq!(number(&mut vm, "globalThis.log"), 42.0);
}

#[test]
fn an_explicit_collection_reclaims_and_keeps_the_reachable() {
    let mut vm = run(
        "globalThis.keep = { tag: 'kept' };
         globalThis.junk = { tag: 'junk' };",
    );
    let before = vm.heap().objects().len();
    vm.eval_source("globalThis.junk = null;").expect("drop");
    vm.collect_garbage_now();
    let after = vm.heap().objects().len();
    eprintln!("objects {before} -> {after}");
    assert!(vm.gc_stats().0 > 0, "an explicit collection should run");
    assert_eq!(
        vm.eval_source("globalThis.keep.tag === 'kept'"),
        Ok(Value::Bool(true)),
        "the reachable object must survive an explicit collection"
    );
}
