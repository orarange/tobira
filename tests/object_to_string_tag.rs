//! `Object.prototype.toString.call(x)` is the classic runtime type test, and
//! it used to answer `[object Object]` for absolutely everything — including
//! `null`, arrays and functions. Libraries branch on it constantly; core-js's
//! `RegExp.prototype.sticky` getter throws "Incompatible receiver, RegExp
//! required" when it disagrees, which took whole polyfill bundles down.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn reports_the_builtin_tag_for_each_kind_of_object() {
    run(r#"
        const S = Object.prototype.toString;
        const tag = (v) => S.call(v);

        assert(tag({}) === '[object Object]');
        assert(tag([]) === '[object Array]');
        assert(tag(/a/) === '[object RegExp]');
        assert(tag(new Date()) === '[object Date]');
        assert(tag(function () {}) === '[object Function]');
        assert(tag(() => {}) === '[object Function]');
        assert(tag(new Error('x')) === '[object Error]');
        assert(tag(new Map()) === '[object Map]');
        assert(tag(new Set()) === '[object Set]');
        assert(tag(Promise.resolve()) === '[object Promise]');
        assert(tag(new ArrayBuffer(1)) === '[object ArrayBuffer]');
        assert(tag(new Uint8Array(1)) === '[object Uint8Array]');
    "#);
}

/// `undefined` and `null` are answered before ToObject, so they have tags of
/// their own rather than throwing or collapsing to Object.
#[test]
fn null_and_undefined_have_their_own_tags() {
    run(r#"
        const S = Object.prototype.toString;
        assert(S.call(undefined) === '[object Undefined]');
        assert(S.call(null) === '[object Null]');
    "#);
}

/// Primitives report the tag of the wrapper they would box into.
#[test]
fn primitives_report_their_wrapper_tag() {
    run(r#"
        const S = Object.prototype.toString;
        assert(S.call(5) === '[object Number]');
        assert(S.call('s') === '[object String]');
        assert(S.call(true) === '[object Boolean]');
        assert(S.call(Symbol('x')) === '[object Symbol]');
    "#);
}

/// An explicit `Symbol.toStringTag` wins over the built-in tag; this is how
/// user classes and most modern built-ins name themselves.
#[test]
fn symbol_to_string_tag_overrides_the_builtin_tag() {
    run(r#"
        const S = Object.prototype.toString;

        const plain = {};
        plain[Symbol.toStringTag] = 'Custom';
        assert(S.call(plain) === '[object Custom]');

        class Named { get [Symbol.toStringTag]() { return 'Named' } }
        assert(S.call(new Named()) === '[object Named]');

        // A non-string tag is ignored and the built-in tag stands.
        const numeric = {};
        numeric[Symbol.toStringTag] = 42;
        assert(S.call(numeric) === '[object Object]');
    "#);
}

/// The exact shape core-js uses to decide whether a receiver is a real RegExp.
/// This is the check that used to throw.
#[test]
fn the_core_js_regexp_receiver_check_answers() {
    run(r#"
        const classof = (v) => Object.prototype.toString.call(v).slice(8, -1);
        assert(classof(/a/) === 'RegExp');
        assert(classof({}) !== 'RegExp');
        assert(/Version\/10(?:\.\d+){1,2}(?: [\w.\/]+)? Safari\//.test('nope') === false);
    "#);
}
