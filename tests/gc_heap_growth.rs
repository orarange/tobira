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
    assert!(
        live < 400,
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
