// Regression tests for two engine fixes:
//  1. ECMAScript `Number::toString` (base 10) exponential formatting.
//  2. Array destructuring following the iterator protocol (any iterable).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

// ---- Number formatting --------------------------------------------------

#[test]
fn number_large_exponential_threshold() {
    run(r#"
        // >= 1e21 switches to exponential with a '+' sign.
        assert((1e21).toString() === '1e+21');
        assert((1e22).toString() === '1e+22');
        // Just below the threshold stays positional.
        assert((1e20).toString() === '100000000000000000000');
    "#);
}

#[test]
fn number_small_exponential_threshold() {
    run(r#"
        // <= 1e-7 switches to exponential with a '-' sign.
        assert((0.0000001).toString() === '1e-7');
        assert((1e-7).toString() === '1e-7');
        // Just above the threshold stays positional.
        assert((0.000001).toString() === '0.000001');
        assert((1e-6).toString() === '0.000001');
    "#);
}

#[test]
fn number_exponential_with_mantissa_digits() {
    run(r#"
        assert((1.23e-7).toString() === '1.23e-7');
        assert((1.5e21).toString() === '1.5e+21');
        assert((0.0000123).toString() === '0.0000123');
    "#);
}

#[test]
fn number_ordinary_formatting_unchanged() {
    run(r#"
        assert((0).toString() === '0');
        assert((42).toString() === '42');
        assert((-42).toString() === '-42');
        assert((3.14).toString() === '3.14');
        assert((-0.5).toString() === '-0.5');
        assert((1000000).toString() === '1000000');
        assert((123.456).toString() === '123.456');
        assert(String(1e21) === '1e+21');
        assert(`${1e-7}` === '1e-7');
        assert((NaN).toString() === 'NaN');
        assert((Infinity).toString() === 'Infinity');
        assert((-Infinity).toString() === '-Infinity');
    "#);
}

#[test]
fn number_negative_exponential() {
    run(r#"
        assert((-1e21).toString() === '-1e+21');
        assert((-1e-7).toString() === '-1e-7');
    "#);
}

// ---- Iterable destructuring ---------------------------------------------

#[test]
fn destructure_from_set() {
    run(r#"
        const [x, y] = new Set([1, 2]);
        assert(x === 1 && y === 2);
    "#);
}

#[test]
fn destructure_from_set_with_rest() {
    run(r#"
        const [first, ...rest] = new Set([10, 20, 30]);
        assert(first === 10);
        assert(Array.isArray(rest));
        assert(rest.length === 2);
        assert(rest[0] === 20 && rest[1] === 30);
    "#);
}

#[test]
fn destructure_from_map_entries() {
    run(r#"
        const m = new Map([['a', 1], ['b', 2]]);
        const [[k0, v0], [k1, v1]] = m;
        assert(k0 === 'a' && v0 === 1);
        assert(k1 === 'b' && v1 === 2);
    "#);
}

#[test]
fn destructure_from_string() {
    run(r#"
        const [a, b, c] = 'xyz';
        assert(a === 'x' && b === 'y' && c === 'z');
    "#);
}

#[test]
fn destructure_from_generator() {
    run(r#"
        function* gen() { yield 1; yield 2; yield 3; }
        const [a, b] = gen();
        assert(a === 1 && b === 2);
    "#);
}

#[test]
fn destructure_with_defaults_and_holes() {
    run(r#"
        const [, second = 99, third = 7] = new Set([1, 2]);
        assert(second === 2);   // present, default ignored
        assert(third === 7);    // missing, default applied
    "#);
}

#[test]
fn destructure_array_still_works() {
    run(r#"
        const [a, b, ...rest] = [1, 2, 3, 4];
        assert(a === 1 && b === 2);
        assert(rest.length === 2 && rest[0] === 3 && rest[1] === 4);
        // Missing elements are undefined.
        const [p, q, r] = [1];
        assert(p === 1 && q === undefined && r === undefined);
    "#);
}
