//! Standard members that shipped bundles reach for. Each was found by loading a
//! real page and reading the uncaught error; Yahoo! JAPAN alone was stopped by
//! `MessageEvent`, `location.toString` and `String.prototype.substr` in turn.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

/// `substr` takes a *length*, not an end index, and a negative start counts
/// back from the end — the two easy things to get wrong.
#[test]
fn string_substr() {
    run(r#"
        assert('abcdef'.substr(1, 3) === 'bcd');
        assert('abcdef'.substr(2) === 'cdef');
        assert('abcdef'.substr(-2) === 'ef');
        assert('abcdef'.substr(-2, 1) === 'e');
        assert('abcdef'.substr(0, 0) === '');
        assert('abcdef'.substr(10) === '');
        assert('abcdef'.substr(1, 100) === 'bcdef');
        assert('abcdef'.substr(1, -1) === '');

        // It differs from substring, which takes an end index.
        assert('abcdef'.substring(1, 3) === 'bc');
    "#);
}

#[test]
fn locale_variants_fall_back_to_the_plain_behaviour() {
    run(r#"
        assert('ÀBc'.toLocaleLowerCase() === 'ÀBc'.toLowerCase());
        assert('aBc'.toLocaleUpperCase() === 'aBc'.toUpperCase());
        assert([1, 2].toLocaleString() === [1, 2].toString());
        assert(({}).toLocaleString() === ({}).toString());
        assert(typeof new Date(0).toLocaleString() === 'string');
    "#);
}

#[test]
fn number_to_exponential() {
    run(r#"
        assert(typeof (12345).toExponential === 'function');
        assert((12345).toExponential(2).indexOf('e+') !== -1, 'exponent needs an explicit sign');
        assert((0.00012).toExponential(1).indexOf('e-') !== -1);
        assert(typeof (5).toExponential() === 'string');
    "#);
}

/// `location` stringifies to its href and carries the navigation methods.
#[test]
fn location_surface_exists() {
    run(r#"
        assert(typeof location === 'object');
        assert(typeof location.toString === 'function');
        assert(typeof location.assign === 'function');
        assert(typeof location.replace === 'function');
        assert(typeof location.reload === 'function');
    "#);
}
