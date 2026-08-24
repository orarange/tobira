//! `String(x)` used the engine's non-coercing stringifier and answered
//! `[object Object]` for every object — even `String([1, 2])`. Template
//! literals and `+` already ran the real ToString, so the three disagreed.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn string_runs_the_objects_own_to_string() {
    run(r#"
        assert(String({ toString() { return 'X' } }) === 'X');
        class C { toString() { return 'C!' } }
        assert(String(new C()) === 'C!');
        assert(String({ [Symbol.toPrimitive]() { return 'P' } }) === 'P');
    "#);
}

#[test]
fn string_matches_template_literals_and_concatenation() {
    run(r#"
        const cases = [[1, 2], /re/, { toString() { return 'X' } }, 5, true, null, undefined];
        for (const v of cases) {
            assert(String(v) === `${v}`, 'String() disagreed with a template literal');
            assert(String(v) === '' + v, 'String() disagreed with concatenation');
        }
    "#);
}

#[test]
fn builtin_objects_stringify_as_themselves() {
    run(r#"
        assert(String([1, 2]) === '1,2');
        assert(String([]) === '');
        assert(String(/ab+/g) === '/ab+/g');
        assert(String(new URL('https://a.example/b?c=1')) === 'https://a.example/b?c=1');
    "#);
}

/// A plain object still reports the generic tag, and `valueOf` alone does not
/// win under a string hint — `toString` is tried first and succeeds.
#[test]
fn the_generic_cases_are_unchanged() {
    run(r#"
        assert(String({}) === '[object Object]');
        assert(String({ valueOf() { return 7 } }) === '[object Object]');
        assert(String() === '');
        assert(String(null) === 'null');
        assert(String(undefined) === 'undefined');
    "#);
}

/// `String(sym)` describes the symbol rather than throwing, unlike ToString.
#[test]
fn symbols_are_described_not_thrown() {
    run(r#"
        assert(typeof String(Symbol('tag')) === 'string');
    "#);
}
