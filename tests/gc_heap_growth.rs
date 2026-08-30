// Documents current GC behavior: the engine has no in-session collector yet, so
// the heap is monotonic within a single run (freed slots are not reclaimed until
// navigation drops the whole heap). When a mark-sweep collector lands, the first
// test below should flip to assert reclamation instead.
use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run_and_keep_vm(source: &str) -> Vm {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
    vm
}

#[test]
fn fresh_vm_starts_small() {
    let vm = run_and_keep_vm("");
    let live = vm.heap().objects().len();
    eprintln!("fresh_vm live objects: {live}");
    // Baseline is the builtin/prototype objects installed at startup, and it
    // grows as standard surface is added. The bound has generous headroom — its
    // job is to catch a fresh VM ballooning into the thousands, not to pin the
    // exact count. Bump it when new builtins legitimately push it up, and say
    // what moved.
    //
    // 2026-08-24: 556 -> 619, from the per-tag HTML element interfaces plus the
    // event and structural DOM interfaces. Each is a constructor and a
    // prototype, so roughly 60 objects and well under 50 KB against a ~51 MB
    // resident baseline; without them real pages die on a bare ReferenceError.
    //
    // 2026-08-30: 619 -> 726. The DOM interface prototypes now carry the
    // methods a page can borrow off them (`Element.prototype.matches.call`),
    // which allocates each builtin at startup rather than on first use; plus
    // twelve more interface names, RegExp's ten flag getters, and a prototype
    // for AbortSignal. The methods are shared with the instance path, so this
    // is a one-off cost, not one that grows with the page.
    assert!(
        live < 800,
        "fresh VM should start small, got {live} live objects"
    );
}

#[test]
fn heap_grows_without_in_session_collection() {
    let src = r#"
        for (let i = 0; i < 2000; i = i + 1) {
            let garbage = { a: i, b: [i, i + 1], c: "x" };
        }
    "#;
    let vm = run_and_keep_vm(src);
    let live = vm.heap().objects().len();
    eprintln!("heap_grows live objects: {live}");
    assert!(
        live >= 2000,
        "no in-session reclamation: expected >=2000 live objects, got {live}"
    );
}
