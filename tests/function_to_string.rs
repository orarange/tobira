//! `Function.prototype.toString` did not exist, so calling `.toString()` on a
//! function fell through to `Object.prototype.toString` and answered
//! `[object Function]`.
//!
//! Two idioms depend on getting this right: comparing against
//! `Object.prototype.constructor.toString()` to recognise the real `Object`,
//! and testing for `[native code]` to decide whether a native implementation
//! exists or a polyfill is needed.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn engine_provided_functions_report_the_native_form() {
    run(r#"
        assert(Object.toString() === 'function Object() { [native code] }');
        assert(Array.toString() === 'function Array() { [native code] }');
        assert(Function.prototype.toString.call(Object) === Object.toString());
        assert(Object.prototype.constructor.toString() === 'function Object() { [native code] }');
    "#);
}

/// The `[native code]` probe has to answer honestly in both directions: a
/// library that believes a JS-level function is native will skip the polyfill
/// it actually needs.
#[test]
fn native_detection_distinguishes_builtins_from_user_functions() {
    run(r#"
        const isNative = (fn) => /\[native code\]/.test(Function.prototype.toString.call(fn));
        assert(isNative(Object) === true);
        assert(isNative(Array.prototype.map) === true);

        function userFn() {}
        const userArrow = () => 1;
        class UserClass {}
        assert(isNative(userFn) === false);
        assert(isNative(userArrow) === false);
        assert(isNative(UserClass) === false);
    "#);
}

/// A JS-level function still produces something function-shaped and carries its
/// own name, even though this engine does not retain source text.
#[test]
fn user_functions_report_their_name() {
    run(r#"
        function Named() {}
        assert(Named.toString().indexOf('Named') !== -1);
        assert(Named.toString().indexOf('function') === 0);
    "#);
}

/// The check this was found through, in full: the class-instance branch only
/// runs when the `Ctor === Object` shortcut misses, so it exercises the
/// comparison against the native string.
#[test]
fn immers_plain_object_check_rejects_class_instances() {
    run(r#"
        const objectCtorString = Object.prototype.constructor.toString();
        function isPlainObject(value) {
            if (!value || typeof value !== 'object') return false;
            const proto = Object.getPrototypeOf(value);
            if (proto === null) return true;
            const Ctor = Object.hasOwnProperty.call(proto, 'constructor') && proto.constructor;
            if (Ctor === Object) return true;
            return typeof Ctor == 'function'
                && Function.prototype.toString.call(Ctor) === objectCtorString;
        }
        class K {}
        assert(isPlainObject({}) === true);
        assert(isPlainObject(new K()) === false, 'a class instance is not a plain object');
    "#);
}

#[test]
fn calling_it_on_a_non_function_throws() {
    run(r#"
        let threw = false;
        try { Function.prototype.toString.call({}); } catch (e) { threw = e instanceof TypeError; }
        assert(threw);
    "#);
}
