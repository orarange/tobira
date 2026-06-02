// Regression tests for compiler-level features: default parameters, object
// literal getters/setters, and labeled break/continue.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn default_parameters_basic() {
    run(r#"
        function f(a, b = 10) { return a + b; }
        assert(f(1) === 11);
        assert(f(1, 2) === 3);
        assert(f(1, undefined) === 11);
    "#);
}

#[test]
fn default_parameters_reference_earlier() {
    run(r#"
        function f(a, b = a * 2) { return a + b; }
        assert(f(3) === 9);
        assert(f(3, 1) === 4);
    "#);
}

#[test]
fn default_parameters_expression() {
    run(r#"
        function greet(name = 'world') { return 'hi ' + name; }
        assert(greet() === 'hi world');
        assert(greet('bob') === 'hi bob');
        function count(arr = []) { arr.push(1); return arr.length; }
        assert(count() === 1);
        assert(count([1, 2]) === 3);
    "#);
}

#[test]
fn default_parameters_arrow() {
    run(r#"
        const add = (a = 1, b = 2) => a + b;
        assert(add() === 3);
        assert(add(10) === 12);
        assert(add(10, 20) === 30);
    "#);
}

#[test]
fn object_literal_getter() {
    run(r#"
        const o = {
            _v: 3,
            get v() { return this._v; },
        };
        assert(o.v === 3);
        o._v = 9;
        assert(o.v === 9);
    "#);
}

#[test]
fn object_literal_getter_setter() {
    run(r#"
        const o = {
            _v: 0,
            get v() { return this._v; },
            set v(x) { this._v = x * 2; },
        };
        o.v = 5;
        assert(o.v === 10);
    "#);
}

#[test]
fn object_literal_computed_getter() {
    run(r#"
        const key = 'dynamic';
        const o = {
            get [key]() { return 42; },
        };
        assert(o.dynamic === 42);
    "#);
}

#[test]
fn labeled_continue_outer() {
    run(r#"
        let count = 0;
        outer: for (let i = 0; i < 3; i++) {
            for (let j = 0; j < 3; j++) {
                if (j === 1) continue outer;
                count++;
            }
        }
        assert(count === 3);
    "#);
}

#[test]
fn labeled_break_outer() {
    run(r#"
        let count = 0;
        outer: for (let i = 0; i < 3; i++) {
            for (let j = 0; j < 3; j++) {
                if (i === 1 && j === 1) break outer;
                count++;
            }
        }
        assert(count === 4);
    "#);
}

#[test]
fn labeled_block_break() {
    run(r#"
        let hit = false;
        block: {
            hit = true;
            break block;
            hit = false;
        }
        assert(hit === true);
    "#);
}
