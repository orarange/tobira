// Regression tests for the tier-2 builtin additions: ES2023 array copy methods,
// function length/name, Array(n), Object descriptors/is, Reflect, WeakMap,
// Symbol.for/keyFor/description.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn array_copy_methods() {
    run(r#"
        const a = [3, 1, 2];
        assert(a.toSorted((x, y) => x - y).join('') === '123');
        assert(a.join('') === '312');               // original unchanged
        assert([1, 2, 3].toReversed().join('') === '321');
        assert([1, 2, 3].with(1, 9).join('') === '193');
    "#);
}

#[test]
fn array_constructor_length() {
    run(r#"
        assert(new Array(3).length === 3);
        assert(new Array(3).fill(0).join('') === '000');
        assert(Array(2, 3).length === 2);
        assert(Array(5).length === 5);
    "#);
}

#[test]
fn function_length_and_name() {
    run(r#"
        function f(a, b, c) {}
        assert(f.length === 3);
        assert(f.name === 'f');
        const g = (x) => x;
        assert(g.length === 1);
    "#);
}

#[test]
fn object_is_and_descriptors() {
    run(r#"
        assert(Object.is(NaN, NaN));
        assert(!Object.is(0, -0));
        assert(Object.is(-0, -0));
        const d = Object.getOwnPropertyDescriptors({ a: 1 });
        assert(d.a.value === 1 && d.a.writable === true);
        const o = {};
        Object.defineProperties(o, { x: { value: 5, enumerable: true } });
        assert(o.x === 5);
    "#);
}

#[test]
fn reflect_operations() {
    run(r#"
        const o = { a: 1 };
        assert(Reflect.get(o, 'a') === 1);
        Reflect.set(o, 'b', 2);
        assert(o.b === 2);
        assert(Reflect.has(o, 'a'));
        assert(Reflect.ownKeys(o).length === 2);
        Reflect.deleteProperty(o, 'a');
        assert(!('a' in o));
        function Add(a, b) { return a + b; }
        assert(Reflect.apply(Add, null, [2, 3]) === 5);
    "#);
}

#[test]
fn weakmap_weakset() {
    run(r#"
        const wm = new WeakMap();
        const k = {};
        wm.set(k, 42);
        assert(wm.get(k) === 42);
        assert(wm.has(k));
        const ws = new WeakSet();
        ws.add(k);
        assert(ws.has(k));
    "#);
}

#[test]
fn symbol_for_keyfor_description() {
    run(r#"
        const s = Symbol.for('shared');
        assert(Symbol.for('shared') === s);
        assert(Symbol.keyFor(s) === 'shared');
        assert(Symbol('local').description === 'local');
        assert(Symbol.keyFor(Symbol('local')) === undefined);
        assert(typeof Symbol.toStringTag === 'symbol');
    "#);
}

#[test]
fn string_locale_compare() {
    run(r#"
        assert('a'.localeCompare('b') < 0);
        assert('b'.localeCompare('a') > 0);
        assert('a'.localeCompare('a') === 0);
    "#);
}
