// Regression tests for new.target.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn new_target_defined_under_new() {
    run(r#"
        function F() { return new.target !== undefined; }
        assert(new F() instanceof F);
        assert(F() === false);
    "#);
}

#[test]
fn new_target_is_the_constructor() {
    run(r#"
        let captured;
        function F() { captured = new.target; }
        new F();
        assert(captured === F);
    "#);
}

#[test]
fn new_target_guard_pattern() {
    run(r#"
        function MustUseNew() {
            if (!new.target) throw new Error('use new');
            this.ok = true;
        }
        assert(new MustUseNew().ok === true);
        let threw = false;
        try { MustUseNew(); } catch (e) { threw = true; }
        assert(threw === true);
    "#);
}

#[test]
fn new_target_in_class_constructor() {
    run(r#"
        class Base {
            constructor() { this.created = new.target === Base; }
        }
        assert(new Base().created === true);
    "#);
}
