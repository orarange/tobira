// Regression tests for the JS call-frame depth limit. JS->JS calls are
// iterative (heap frames driven by the interpreter loop, no native Rust
// recursion per call), so the limit is an artificial runaway guard, not a
// native-stack limit. It was raised from 1024 to 10_000 to match real engines;
// these tests pin that deep-but-finite recursion succeeds and that unbounded
// recursion throws the web-standard "Maximum call stack size exceeded".
use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn deep_but_finite_recursion_succeeds_past_old_1024_limit() {
    // 5000 deep — comfortably past the old 1024 cap, well under the new 10_000.
    run(r#"
        function countdown(n) {
            if (n === 0) return 0;
            return countdown(n - 1);
        }
        assert(countdown(5000) === 0);
    "#);
}

#[test]
fn unbounded_recursion_throws_range_error_with_standard_message() {
    run(r#"
        function boom() { return boom(); }
        let caught = null;
        try {
            boom();
        } catch (e) {
            caught = e;
        }
        assert(caught !== null);
        assert(caught instanceof RangeError);
        assert(String(caught.message) === 'Maximum call stack size exceeded');
    "#);
}
