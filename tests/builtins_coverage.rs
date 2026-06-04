// Regression tests for the built-in library additions (array/string/number/
// object/math methods and global functions).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn array_splice_mutates_and_returns_removed() {
    run(r#"
        const a = [1, 2, 3, 4];
        const removed = a.splice(1, 2, 'x');
        assert(removed.length === 2);
        assert(removed[0] === 2);
        assert(removed[1] === 3);
        assert(a.length === 3);
        assert(a[0] === 1);
        assert(a[1] === 'x');
        assert(a[2] === 4);
    "#);
}

#[test]
fn array_flat_depth_and_flatmap() {
    run(r#"
        assert([1, [2, [3, [4]]]].flat(2).length === 4);
        assert([1, [2, [3]]].flat(Infinity).length === 3);
        assert([1, 2].flatMap(x => [x, x * 10]).join(',') === '1,10,2,20');
    "#);
}

#[test]
fn array_at_fill_copywithin() {
    run(r#"
        assert([1, 2, 3].at(-1) === 3);
        assert([1, 2, 3].at(0) === 1);
        assert([1, 2, 3].at(5) === undefined);
        assert([1, 2, 3, 4].fill(0, 1, 3).join('') === '1004');
    "#);
}

#[test]
fn array_iterators_and_reduce_right() {
    run(r#"
        assert([...['a', 'b'].keys()].join('') === '01');
        assert([...['a', 'b'].values()].join('') === 'ab');
        let s = 0;
        for (const [i, v] of [10, 20].entries()) s += i + v;
        assert(s === 31);
        assert(['a', 'b', 'c'].reduceRight((acc, x) => acc + x) === 'cba');
    "#);
}

#[test]
fn array_from_of_iterables() {
    run(r#"
        assert(Array.of(1, 2, 3).length === 3);
        assert(Array.from('abc').length === 3);
        assert(Array.from(new Set([1, 1, 2])).length === 2);
        assert(Array.from({ length: 3 }, (_, i) => i).join('') === '012');
        assert(Array.from(new Map([['a', 1]])).length === 1);
    "#);
}

#[test]
fn string_at_concat_fromcharcode() {
    run(r#"
        assert('abc'.at(-1) === 'c');
        assert('ab'.concat('cd', 'ef') === 'abcdef');
        assert(String.fromCharCode(97, 98, 99) === 'abc');
        assert(typeof 'x'.normalize() === 'string');
    "#);
}

#[test]
fn number_tofixed_tostring_radix() {
    run(r#"
        assert((3.14159).toFixed(2) === '3.14');
        assert((255).toString(16) === 'ff');
        assert((5).toString(2) === '101');
        assert((255).toString() === '255');
        assert((1024).toString(16) === '400');
    "#);
}

#[test]
fn number_and_string_callable() {
    run(r#"
        assert(Number('42') === 42);
        assert(Number('3.5') === 3.5);
        assert(String(42) === '42');
        assert(String(true) === 'true');
        assert(Boolean(0) === false);
        assert(Boolean('x') === true);
    "#);
}

#[test]
fn object_fromentries_hasown_names() {
    run(r#"
        const o = Object.fromEntries([['a', 1], ['b', 2]]);
        assert(o.a === 1 && o.b === 2);
        assert(Object.hasOwn(o, 'a'));
        assert(!Object.hasOwn(o, 'z'));
        assert(Object.getOwnPropertyNames(o).length === 2);
    "#);
}

#[test]
fn math_sign_hypot() {
    run(r#"
        assert(Math.sign(-5) === -1);
        assert(Math.sign(5) === 1);
        assert(Math.sign(0) === 0);
        assert(Math.hypot(3, 4) === 5);
        assert(Math.trunc(-2.7) === -2);
    "#);
}

#[test]
fn global_parse_and_uri_functions() {
    run(r#"
        assert(parseInt('42px', 10) === 42);
        assert(parseInt('0xff') === 255);
        assert(parseInt('  -17 ') === -17);
        assert(parseFloat('3.14xyz') === 3.14);
        assert(parseFloat('  1e3 ') === 1000);
        assert(isNaN(NaN));
        assert(isNaN('abc'));
        assert(!isFinite(Infinity));
        assert(isFinite(42));
        assert(encodeURIComponent('a b&c') === 'a%20b%26c');
        assert(decodeURIComponent('a%20b%26c') === 'a b&c');
    "#);
}
