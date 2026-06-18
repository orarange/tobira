use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run_with_current_script_src(source: &str, current_script_src: Option<&str>) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.set_current_script_src(current_script_src.map(str::to_owned));
    vm.execute(&chunk).expect("execute");
}

#[test]
fn current_script_stub_exposes_attribute_helpers() {
    run_with_current_script_src(
        r#"
        assert(typeof document.currentScript.getAttribute === 'function');
        assert(typeof document.currentScript.hasAttribute === 'function');
        assert(document.currentScript.getAttribute('data-x') === null);
        assert(document.currentScript.getAttribute('src') === 'https://x/y.js');
        assert(document.currentScript.hasAttribute('src') === true);
        assert(document.currentScript.hasAttribute('data-x') === false);
    "#,
        Some("https://x/y.js"),
    );
}

#[test]
fn current_script_stub_is_null_without_src() {
    run_with_current_script_src(
        r#"
        assert(document.currentScript === null);
    "#,
        None,
    );
}
