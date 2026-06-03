// Regression tests for Symbol (values, well-known Symbol.iterator, custom
// iteration protocol) and Date (construction, accessors, ISO formatting).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn symbol_basic_and_typeof() {
    run(r#"
        const s = Symbol('desc');
        assert(typeof s === 'symbol');
        const o = { [s]: 1 };
        assert(o[s] === 1);
    "#);
}

#[test]
fn symbol_uniqueness() {
    run(r#"
        assert(Symbol('x') !== Symbol('x'));
        const s = Symbol('y');
        assert(s === s);
    "#);
}

#[test]
fn symbol_iterator_custom_iterable() {
    run(r#"
        const range = {
            [Symbol.iterator]() {
                let i = 0;
                return {
                    next() {
                        return i < 3 ? { value: i++, done: false } : { value: undefined, done: true };
                    },
                };
            },
        };
        const out = [...range];
        assert(out.length === 3);
        assert(out[0] === 0 && out[2] === 2);
        let sum = 0;
        for (const x of range) sum += x;
        assert(sum === 3);
    "#);
}

#[test]
fn date_now_is_number() {
    run(r#"
        assert(typeof Date.now() === 'number');
        assert(Date.now() > 0);
    "#);
}

#[test]
fn date_construct_components() {
    run(r#"
        const d = new Date(2020, 0, 15, 10, 30, 45);
        assert(d.getFullYear() === 2020);
        assert(d.getMonth() === 0);
        assert(d.getDate() === 15);
        assert(d.getHours() === 10);
        assert(d.getMinutes() === 30);
        assert(d.getSeconds() === 45);
    "#);
}

#[test]
fn date_from_epoch_and_gettime() {
    run(r#"
        const d = new Date(0);
        assert(d.getTime() === 0);
        assert(d.getUTCFullYear === undefined || d.getFullYear() === 1970);
        const d2 = new Date(1000);
        assert(d2.getTime() === 1000);
    "#);
}

#[test]
fn date_iso_string() {
    run(r#"
        const d = new Date(Date.UTC ? 0 : 0);
        const iso = new Date(0).toISOString();
        assert(iso === '1970-01-01T00:00:00.000Z');
    "#);
}

#[test]
fn date_weekday() {
    run(r#"
        // 1970-01-01 was a Thursday (day 4).
        assert(new Date(0).getDay() === 4);
    "#);
}
