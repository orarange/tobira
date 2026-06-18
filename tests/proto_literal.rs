use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn proto_null_literal_sets_prototype() {
    run(r#"
        const o = { __proto__: null, a: 1 };
        assert(Object.getPrototypeOf(o) === null);
        assert(!Object.prototype.hasOwnProperty.call(o, '__proto__'));
        const k = [];
        for (const n in o) k.push(n);
        assert(k.length === 1 && k[0] === 'a');
        assert(Object.getOwnPropertyNames(o).length === 1);
    "#);
}

#[test]
fn proto_object_literal_sets_prototype() {
    run(r#"
        const p = { greet() { return 1; } };
        const o = { __proto__: p, x: 2 };
        assert(Object.getPrototypeOf(o) === p);
        assert(o.greet() === 1);
        assert(!Object.prototype.hasOwnProperty.call(o, '__proto__'));
    "#);
}

#[test]
fn proto_string_key_also_sets_prototype() {
    run(r#"
        const o = { "__proto__": null };
        assert(Object.getPrototypeOf(o) === null);
        assert(!Object.prototype.hasOwnProperty.call(o, '__proto__'));
    "#);
}

#[test]
fn proto_primitive_value_ignored() {
    run(r#"
        const o = { __proto__: 5, a: 1 };
        assert(Object.getPrototypeOf(o) === Object.prototype);
        assert(!Object.prototype.hasOwnProperty.call(o, '__proto__'));
        assert(o.a === 1);
    "#);
}

#[test]
fn proto_computed_key_is_own_property() {
    run(r#"
        const o = { ["__proto__"]: 42 };
        assert(Object.prototype.hasOwnProperty.call(o, '__proto__'));
        assert(o['__proto__'] === 42);
        assert(Object.getOwnPropertyNames(o).indexOf('__proto__') !== -1);
    "#);
}

#[test]
fn proto_shorthand_is_own_property() {
    run(r#"
        const __proto__ = 'val';
        const o = { __proto__ };
        assert(Object.prototype.hasOwnProperty.call(o, '__proto__'));
        assert(o['__proto__'] === 'val');
    "#);
}
