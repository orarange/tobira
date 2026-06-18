use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn match_media_returns_noop_listener_object() {
    run(r#"
        const m = matchMedia('(prefers-color-scheme: dark)');
        assert(typeof m.addEventListener === 'function');
        assert(typeof m.removeEventListener === 'function');
        assert(m.media === '(prefers-color-scheme: dark)');
        assert(m.matches === false);
        let fired = false;
        m.addEventListener('change', function(){ fired = true; });
        assert(fired === false);
        m.removeEventListener('change', function(){});
        assert(typeof m.addListener === 'function');
    "#);
}
