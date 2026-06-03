// Regression tests for `typeof` on undeclared names, the `arguments` object,
// and function/var hoisting.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn typeof_undeclared_global() {
    run(r#"
        assert(typeof neverDeclared === 'undefined');
        assert(typeof window === 'undefined' || typeof window === 'object');
    "#);
}

#[test]
fn arguments_object() {
    run(r#"
        function f() { return arguments.length; }
        assert(f() === 0);
        assert(f(1, 2, 3) === 3);
        function sum() {
            let s = 0;
            for (let i = 0; i < arguments.length; i++) s += arguments[i];
            return s;
        }
        assert(sum(1, 2, 3, 4) === 10);
    "#);
}

#[test]
fn arguments_beyond_params() {
    run(r#"
        function f(a) { return arguments[1] + arguments[2]; }
        assert(f(1, 2, 3) === 5);
    "#);
}

#[test]
fn function_hoisting_top_level() {
    run(r#"
        assert(hoisted() === 42);
        function hoisted() { return 42; }
    "#);
}

#[test]
fn function_hoisting_in_function() {
    run(r#"
        function outer() {
            return helper();
            function helper() { return 'ok'; }
        }
        assert(outer() === 'ok');
    "#);
}

#[test]
fn mutual_recursion_hoisting() {
    run(r#"
        function isEven(n) { return n === 0 ? true : isOdd(n - 1); }
        function isOdd(n) { return n === 0 ? false : isEven(n - 1); }
        assert(isEven(10) === true);
        assert(isOdd(7) === true);
    "#);
}

#[test]
fn nested_function_still_captures_locals() {
    // Regression: hoisting must not break closure capture of later-declared
    // const/let in the same scope.
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
fn var_hoisting_typeof() {
    run(r#"
        function f() {
            assert(typeof v === 'undefined');
            var v = 3;
            return v;
        }
        assert(f() === 3);
    "#);
}
