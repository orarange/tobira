// Regression tests for semantic correctness fixes: nullish coalescing stack
// balance, the `delete` operator, and JSON.stringify number/indent formatting.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn nullish_coalescing_value_and_stack_balance() {
    run(r#"
        assert((null ?? 5) === 5);
        assert((undefined ?? 7) === 7);
        assert((0 ?? 9) === 0);
        assert(('' ?? 9) === '');
        assert((false ?? 9) === false);
        // Used as a call argument (regression: extra stack value broke the call).
        function id(x) { return x; }
        assert(id(null ?? 42) === 42);
        assert(id(1 ?? 42) === 1);
    "#);
}

#[test]
fn nullish_chained() {
    run(r#"
        const a = null;
        const b = undefined;
        const c = 'value';
        assert((a ?? b ?? c) === 'value');
    "#);
}

#[test]
fn delete_property_named_and_computed() {
    run(r#"
        const o = { a: 1, b: 2 };
        assert('a' in o);
        assert(delete o.a === true);
        assert(!('a' in o));
        assert(o.a === undefined);
        const key = 'b';
        assert(delete o[key] === true);
        assert(!('b' in o));
    "#);
}

#[test]
fn delete_from_array() {
    run(r#"
        const arr = [1, 2, 3];
        delete arr[1];
        assert(arr[1] === undefined);
        assert(arr.length === 3);
    "#);
}

#[test]
fn delete_nonexistent_returns_true() {
    run(r#"
        const o = {};
        assert(delete o.missing === true);
    "#);
}

#[test]
fn json_stringify_integers_not_floats() {
    run(r#"
        assert(JSON.stringify({ a: 1, b: [2, 3] }) === '{"a":1,"b":[2,3]}');
        assert(JSON.stringify(42) === '42');
        assert(JSON.stringify([1, 2, 3]) === '[1,2,3]');
        assert(JSON.stringify(3.5) === '3.5');
        assert(JSON.stringify(NaN) === 'null');
        assert(JSON.stringify(Infinity) === 'null');
    "#);
}

#[test]
fn json_stringify_preserves_key_order() {
    run(r#"
        assert(JSON.stringify({ z: 1, a: 2, m: 3 }) === '{"z":1,"a":2,"m":3}');
    "#);
}

#[test]
fn json_stringify_with_indent() {
    run(r#"
        assert(JSON.stringify({ a: 1 }, null, 2) === '{\n  "a": 1\n}');
        assert(JSON.stringify([1, 2], null, 2) === '[\n  1,\n  2\n]');
    "#);
}

#[test]
fn json_roundtrip() {
    run(r#"
        const o = { a: { b: { c: [1, 2, { d: true }] } } };
        const parsed = JSON.parse(JSON.stringify(o));
        assert(parsed.a.b.c[2].d === true);
        assert(parsed.a.b.c[0] === 1);
    "#);
}
