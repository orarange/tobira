//! Two scoping bugs found by probing the engine after a real page threw
//! `ReferenceError` from a minified bundle.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program)
        .compile()
        .expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

/// An arrow has no `arguments` of its own; it resolves the name lexically to
/// the nearest enclosing non-arrow function. This used to throw
/// `ReferenceError: arguments is not defined`.
#[test]
fn arrows_see_the_enclosing_functions_arguments() {
    run(r#"
        function f() { const g = () => arguments[0]; return g(); }
        assert(f(5) === 5);

        function len() { const g = () => arguments.length; return g(); }
        assert(len(1, 2, 3) === 3);

        // Through two arrow levels.
        function deep() { const g = () => () => arguments[0]; return g()(); }
        assert(deep(8) === 8);

        // An arrow returned from the function still sees it after the call.
        function escaped() { return () => arguments[0]; }
        assert(escaped(7)() === 7);
    "#);
}

/// A nested *non-arrow* function has its own `arguments` and must not borrow
/// the outer one.
#[test]
fn nested_non_arrow_functions_keep_their_own_arguments() {
    run(r#"
        function outer() {
            function inner() { return arguments[0]; }
            return inner('inner');
        }
        assert(outer('outer') === 'inner');
    "#);
}

/// A binding actually named `arguments` shadows the implicit one.
#[test]
fn an_explicit_arguments_binding_wins() {
    run(r#"
        function withParam(args) { return (() => args)(); }
        assert(withParam(1) === 1);

        function shadowedByParam(arguments) { return (() => arguments)(); }
        assert(shadowedByParam('mine') === 'mine');

        function shadowedByLet() {
            let args = 'block';
            return (() => args)();
        }
        assert(shadowedByLet() === 'block');
    "#);
}

/// A default initializer runs before the body, so the implicit binding has to
/// exist by then.
#[test]
fn arrows_in_parameter_defaults_see_arguments() {
    run(r#"
        function f(a = (() => arguments[0])()) { return a; }
        assert(f(5) === 5);
    "#);
}

/// Materialising `arguments` once per call rather than per read gives it a
/// stable identity, and makes writes through it visible.
#[test]
fn arguments_is_one_object_per_call() {
    run(r#"
        function same() { return arguments === arguments; }
        assert(same(1) === true);

        function written() { arguments[0] = 9; return arguments[0]; }
        assert(written(1) === 9);
    "#);
}

/// `for (let/const x of …)` and `for (let/const k in …)` create a fresh binding
/// per iteration, so a closure made in the body keeps the value it saw. The
/// classic `for (let i = …)` loop already did this; these two did not.
#[test]
fn for_of_and_for_in_bind_per_iteration() {
    run(r#"
        const ofConst = [];
        for (const v of [1, 2, 3]) ofConst.push(() => v);
        assert(ofConst.map(f => f()).join() === '1,2,3');

        const ofLet = [];
        for (let v of [1, 2]) ofLet.push(() => v);
        assert(ofLet.map(f => f()).join() === '1,2');

        const inConst = [];
        for (const k in { x: 1, y: 2 }) inConst.push(() => k);
        assert(inConst.map(f => f()).join() === 'x,y');

        // The classic loop keeps working.
        const classic = [];
        for (let i = 0; i < 3; i++) classic.push(() => i);
        assert(classic.map(f => f()).join() === '0,1,2');
    "#);
}

/// `var` deliberately shares one binding across the whole loop, so it must not
/// be freshened.
#[test]
fn var_loop_variables_still_share_one_binding() {
    run(r#"
        const fns = [];
        for (var i = 0; i < 3; i++) fns.push(() => i);
        assert(fns.map(f => f()).join() === '3,3,3');

        const ofVar = [];
        for (var v of [1, 2]) ofVar.push(() => v);
        assert(ofVar.map(f => f()).join() === '2,2');
    "#);
}
