// Regression tests for per-iteration `let` loop bindings and sloppy-mode
// Object.freeze write semantics.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn for_let_per_iteration_closure() {
    run(r#"
        const fns = [];
        for (let i = 0; i < 3; i++) {
            fns.push(() => i);
        }
        assert(fns[0]() === 0);
        assert(fns[1]() === 1);
        assert(fns[2]() === 2);
    "#);
}

#[test]
fn for_var_shares_single_binding() {
    run(r#"
        const fns = [];
        for (var i = 0; i < 3; i++) {
            fns.push(() => i);
        }
        // `var` has a single function-scoped binding, so all closures see 3.
        assert(fns[0]() === 3);
        assert(fns[2]() === 3);
    "#);
}

#[test]
fn for_let_continue_still_per_iteration() {
    run(r#"
        const fns = [];
        for (let i = 0; i < 4; i++) {
            if (i === 2) continue;
            fns.push(() => i);
        }
        assert(fns.length === 3);
        assert(fns[0]() === 0);
        assert(fns[1]() === 1);
        assert(fns[2]() === 3);
    "#);
}

#[test]
fn object_freeze_sloppy_write_is_silent() {
    run(r#"
        const o = Object.freeze({ a: 1 });
        o.a = 2;          // ignored in sloppy mode (no throw)
        assert(o.a === 1);
        o.b = 3;          // adding to a frozen object is ignored too
        assert(o.b === undefined);
        assert(Object.isFrozen(o));
    "#);
}

#[test]
fn object_freeze_strict_write_throws() {
    // In strict mode, writing a frozen property throws — caught here.
    run(r#"
        'use strict';
        const o = Object.freeze({ a: 1 });
        let threw = false;
        try { o.a = 2; } catch (e) { threw = true; }
        assert(threw === true);
        assert(o.a === 1);
    "#);
}
