// Regression tests for JSON toJSON/replacer/reviver and generator object/class
// methods.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn json_tojson() {
    run(r#"
        const o = { toJSON() { return 'custom'; } };
        assert(JSON.stringify(o) === '"custom"');
        const wrapper = { nested: { toJSON() { return 42; } } };
        assert(JSON.stringify(wrapper) === '{"nested":42}');
    "#);
}

#[test]
fn json_replacer_function() {
    run(r#"
        const out = JSON.stringify({ a: 1, b: 2 }, (k, v) => (k === 'b' ? undefined : v));
        assert(out === '{"a":1}');
        const doubled = JSON.stringify({ a: 1 }, (k, v) => (typeof v === 'number' ? v * 2 : v));
        assert(doubled === '{"a":2}');
    "#);
}

#[test]
fn json_reviver() {
    run(r#"
        const o = JSON.parse('{"a":1,"b":2}', (k, v) => (typeof v === 'number' ? v * 10 : v));
        assert(o.a === 10 && o.b === 20);
        const filtered = JSON.parse('{"a":1,"b":2}', (k, v) => (k === 'b' ? undefined : v));
        assert(filtered.a === 1);
        assert(!('b' in filtered));
    "#);
}

#[test]
fn json_reviver_nested() {
    run(r#"
        const o = JSON.parse('{"x":{"y":[1,2]}}', (k, v) => v);
        assert(o.x.y[0] === 1 && o.x.y[1] === 2);
    "#);
}

#[test]
fn generator_object_method() {
    run(r#"
        const o = {
            *range() { yield 1; yield 2; yield 3; },
        };
        assert([...o.range()].length === 3);
        let sum = 0;
        for (const x of o.range()) sum += x;
        assert(sum === 6);
    "#);
}

#[test]
fn generator_class_method() {
    run(r#"
        class Collection {
            constructor() { this.items = [10, 20, 30]; }
            *[Symbol.iterator]() {
                for (const item of this.items) yield item;
            }
        }
        const c = new Collection();
        assert([...c].join(',') === '10,20,30');
    "#);
}

#[test]
fn generator_method_uses_this() {
    run(r#"
        const counter = {
            value: 5,
            *count() { yield this.value; yield this.value + 1; },
        };
        const out = [...counter.count()];
        assert(out[0] === 5 && out[1] === 6);
    "#);
}
