// Regression tests for lexical scoping: transitive upvalue capture across more
// than one function boundary, and lexical `this` in arrow functions.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

// --- Transitive upvalues -----------------------------------------------------

#[test]
fn curry_three_levels() {
    // The innermost arrow captures `a` from the grandparent and `b` from the
    // parent — neither is a local of the immediate enclosing function.
    run("const add = a => b => c => a + b + c; assert(add(1)(2)(3) === 6);");
}

#[test]
fn deeply_nested_capture() {
    run(r#"
        function outer() {
            const x = 10;
            function mid() {
                function inner() { return x; }
                return inner();
            }
            return mid();
        }
        assert(outer() === 10);
    "#);
}

#[test]
fn skip_level_capture_with_intermediate_locals() {
    // `inner` references `a` (depth 2) while the intermediate `mid` only has its
    // own local `m`. The compiler must thread an upvalue through `mid`.
    run(r#"
        function outer(a) {
            return function mid() {
                const m = 100;
                return function inner() { return a + m; };
            };
        }
        assert(outer(5)()() === 105);
    "#);
}

#[test]
fn captured_variable_is_mutable_and_shared() {
    run(r#"
        function makeCounter() {
            let n = 0;
            const inc = () => () => { n += 1; return n; };
            return inc();
        }
        const step = makeCounter();
        assert(step() === 1);
        assert(step() === 2);
        assert(step() === 3);
    "#);
}

#[test]
fn multiple_upvalues_from_distinct_depths() {
    run(r#"
        const f = a => b => c => d => a * 1000 + b * 100 + c * 10 + d;
        assert(f(1)(2)(3)(4) === 1234);
    "#);
}

// --- Lexical `this` in arrows ------------------------------------------------

#[test]
fn arrow_captures_method_this() {
    run(r#"
        const o = { v: 5, get() { return (() => this.v)(); } };
        assert(o.get() === 5);
    "#);
}

#[test]
fn arrow_this_through_nested_arrows() {
    run(r#"
        const o = { v: 7, get() { return (() => (() => this.v)())(); } };
        assert(o.get() === 7);
    "#);
}

#[test]
fn arrow_this_in_array_callback() {
    run(r#"
        const o = {
            factor: 10,
            scale(arr) { return arr.map(x => x * this.factor); },
        };
        const out = o.scale([1, 2, 3]);
        assert(out[0] === 10);
        assert(out[1] === 20);
        assert(out[2] === 30);
    "#);
}

#[test]
fn regular_function_has_own_this() {
    // A non-arrow nested function must NOT capture the outer `this`; called
    // bare, its `this` is undefined, so reading a property throws.
    run(r#"
        const o = {
            v: 1,
            get() {
                function inner() { return typeof this; }
                return inner();
            },
        };
        assert(o.get() === 'undefined');
    "#);
}

#[test]
fn class_method_arrow_this() {
    run(r#"
        class Box {
            constructor(v) { this.v = v; }
            read() { return [1].map(() => this.v)[0]; }
        }
        assert(new Box(42).read() === 42);
    "#);
}
