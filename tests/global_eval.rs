use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn global_eval_function_exists() {
    run(r#"
        assert(typeof eval === "function");
        assert(window.eval === eval);
        assert(globalThis.eval === eval);
    "#);
}

#[test]
fn global_eval_returns_completion_value() {
    run(r#"
        assert(eval("1 + 1") === 2);
        assert(eval('"a" + "b"') === "ab");
        assert(eval("var x = 5; x * 2") === 10);
        assert(eval("") === undefined);
    "#);
}

#[test]
fn global_eval_returns_non_strings_unchanged() {
    run(r#"
        assert(eval(42) === 42);
        var o = {};
        assert(eval(o) === o);
        assert(eval(null) === null);
        assert(eval(true) === true);
    "#);
}

#[test]
fn global_eval_reads_and_writes_globals() {
    run(r#"
        globalThis.g = 7;
        assert(eval("g") === 7);
        eval("globalThis.h = 9");
        assert(globalThis.h === 9);
    "#);
}

#[test]
fn global_eval_indirect_forms_work() {
    run(r#"
        var e = eval;
        assert(e("2+3") === 5);
        assert((0, eval)("4+5") === 9);
    "#);
}

#[test]
fn global_eval_can_nest() {
    run(r#"
        assert(eval('eval("6+7")') === 13);
    "#);
}

#[test]
fn global_eval_preserves_outer_frame_and_stack() {
    run(r#"
        function f() {
            var before = 20;
            eval("1");
            var after = before + 22;
            return after;
        }
        assert(f() === 42);
    "#);
}

#[test]
fn global_eval_runtime_throw_propagates() {
    run(r#"
        var caught = false;
        try {
            eval("throw new Error('boom')");
        } catch (e) {
            caught = true;
            assert(e.message === "boom");
        }
        assert(caught);
    "#);
}

#[test]
fn global_eval_parse_error_is_syntax_error() {
    run(r#"
        var t = false;
        try {
            eval("==");
        } catch (e) {
            t = e instanceof SyntaxError;
        }
        assert(t);
    "#);
}
