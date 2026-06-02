// Regression tests for generator functions: yield, yield*, two-way next(),
// iteration via for-of / spread, early return, and closures over generators.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn generator_basic_next() {
    run(r#"
        function* g() { yield 1; yield 2; }
        const it = g();
        const a = it.next();
        assert(a.value === 1 && a.done === false);
        const b = it.next();
        assert(b.value === 2 && b.done === false);
        const c = it.next();
        assert(c.value === undefined && c.done === true);
    "#);
}

#[test]
fn generator_return_value() {
    run(r#"
        function* g() { yield 1; return 99; }
        const it = g();
        assert(it.next().value === 1);
        const last = it.next();
        assert(last.value === 99 && last.done === true);
    "#);
}

#[test]
fn generator_spread_and_for_of() {
    run(r#"
        function* g() { yield 1; yield 2; yield 3; }
        assert([...g()].length === 3);
        let sum = 0;
        for (const x of g()) sum += x;
        assert(sum === 6);
    "#);
}

#[test]
fn generator_yield_delegate() {
    run(r#"
        function* inner() { yield 1; yield 2; }
        function* outer() { yield 0; yield* inner(); yield 3; }
        assert([...outer()].join(',') === '0,1,2,3');
    "#);
}

#[test]
fn generator_yield_star_array() {
    run(r#"
        function* g() { yield* [1, 2, 3]; }
        assert([...g()].length === 3);
    "#);
}

#[test]
fn generator_two_way_next() {
    run(r#"
        function* echo() {
            const a = yield 'first';
            const b = yield a;
            return b;
        }
        const it = echo();
        assert(it.next().value === 'first');
        assert(it.next(10).value === 10);
        assert(it.next(20).value === 20);
        assert(it.next().done === true);
    "#);
}

#[test]
fn generator_captures_closure_state() {
    run(r#"
        function* counter(start) {
            let n = start;
            while (true) {
                yield n;
                n++;
            }
        }
        const it = counter(5);
        assert(it.next().value === 5);
        assert(it.next().value === 6);
        assert(it.next().value === 7);
    "#);
}

#[test]
fn generator_with_arguments_and_locals() {
    run(r#"
        function* range(start, end) {
            for (let i = start; i < end; i++) {
                yield i;
            }
        }
        assert([...range(2, 5)].join(',') === '2,3,4');
    "#);
}

#[test]
fn generator_expression() {
    run(r#"
        const g = function* () { yield 'a'; yield 'b'; };
        assert([...g()].join('') === 'ab');
    "#);
}

#[test]
fn generator_early_return_method() {
    run(r#"
        function* g() { yield 1; yield 2; yield 3; }
        const it = g();
        assert(it.next().value === 1);
        const r = it.return(42);
        assert(r.value === 42 && r.done === true);
        assert(it.next().done === true);
    "#);
}
