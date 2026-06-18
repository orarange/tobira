use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn property_is_enumerable_matches_own_enumerable_state() {
    run(r#"
        const o = { a: 1 };
        assert(o.propertyIsEnumerable('a') === true);
        assert(o.propertyIsEnumerable('b') === false);
        Object.defineProperty(o, 'hidden', { value: 2, enumerable: false });
        assert(o.propertyIsEnumerable('hidden') === false);
        assert(o.hidden === 2);
        assert(({}).propertyIsEnumerable('toString') === false);
    "#);
}
