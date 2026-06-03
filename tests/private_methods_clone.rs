// Regression tests for private class methods and structuredClone.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn private_method() {
    run(r#"
        class A {
            #secret() { return 7; }
            reveal() { return this.#secret(); }
        }
        assert(new A().reveal() === 7);
    "#);
}

#[test]
fn private_method_with_field() {
    run(r#"
        class Counter {
            #count = 0;
            #step() { this.#count++; }
            tick() { this.#step(); return this.#count; }
        }
        const c = new Counter();
        assert(c.tick() === 1);
        assert(c.tick() === 2);
    "#);
}

#[test]
fn structured_clone_deep() {
    run(r#"
        const o = { a: [1, 2], b: { c: 3 } };
        const clone = structuredClone(o);
        clone.a[0] = 99;
        clone.b.c = 99;
        assert(o.a[0] === 1);
        assert(o.b.c === 3);
        assert(clone.a[0] === 99);
    "#);
}

#[test]
fn structured_clone_collections() {
    run(r#"
        const m = new Map([['a', 1]]);
        const cm = structuredClone(m);
        cm.set('a', 2);
        assert(m.get('a') === 1);
        assert(cm.get('a') === 2);
        const s = new Set([1, 2, 3]);
        const cs = structuredClone(s);
        cs.add(4);
        assert(s.size === 3 && cs.size === 4);
    "#);
}

#[test]
fn structured_clone_primitives() {
    run(r#"
        assert(structuredClone(42) === 42);
        assert(structuredClone('x') === 'x');
        assert(structuredClone(null) === null);
        assert(structuredClone(true) === true);
    "#);
}
