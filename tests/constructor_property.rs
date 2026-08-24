//! `x.constructor` was `undefined` for everything: no prototype in the engine
//! carried a back-reference to the function that owns it.
//!
//! Libraries lean on it constantly — plain-object checks (`x.constructor ===
//! Object`), cloning (`new x.constructor()`), and diagnostics
//! (`x.constructor.name`). Immer's `isPlainObject` threw outright on it, which
//! is how this was found.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn builtin_prototypes_point_back_at_their_constructor() {
    run(r#"
        const pairs = [
            [Object, Object.prototype], [Array, Array.prototype], [Function, Function.prototype],
            [String, String.prototype], [Number, Number.prototype], [Boolean, Boolean.prototype],
            [Error, Error.prototype], [RegExp, RegExp.prototype], [Date, Date.prototype],
            [Map, Map.prototype], [Set, Set.prototype], [Promise, Promise.prototype],
            [Symbol, Symbol.prototype],
        ];
        for (const [ctor, proto] of pairs) {
            assert(proto.constructor === ctor, 'wrong constructor on a prototype');
        }
    "#);
}

#[test]
fn instances_report_their_constructor() {
    run(r#"
        assert(({}).constructor === Object);
        assert([].constructor === Array);
        assert(new Map().constructor === Map);
        assert(new Set().constructor === Set);
        assert((/a/).constructor === RegExp);
    "#);
}

/// User functions and classes get the same back-reference, so `new C()` can be
/// identified and cloned through its own constructor.
#[test]
fn user_functions_and_classes_link_their_prototype() {
    run(r#"
        function F() {}
        assert(F.prototype.constructor === F);
        assert(new F().constructor === F);

        class C { constructor() { this.v = 1 } }
        assert(C.prototype.constructor === C);
        assert(new C().constructor === C);
        assert(new C().constructor.name === 'C');

        // Cloning through the instance's own constructor.
        const made = new (new C().constructor)();
        assert(made.v === 1);
    "#);
}

/// `constructor` must not show up in enumeration.
#[test]
fn constructor_is_not_enumerable() {
    run(r#"
        const keys = [];
        for (const k in {}) keys.push(k);
        assert(keys.length === 0, 'plain object enumeration picked up ' + keys.join());
        assert(Object.keys(Object.prototype).indexOf('constructor') === -1);

        function F() {}
        const own = [];
        for (const k in new F()) own.push(k);
        assert(own.length === 0);
    "#);
}

/// Global built-in functions report the name they are registered under, so
/// `x.constructor.name` works for them too.
#[test]
fn builtin_constructors_have_a_name() {
    run(r#"
        assert(Object.name === 'Object');
        assert(Array.name === 'Array');
        assert(Map.name === 'Map');
        assert(({}).constructor.name === 'Object');
        assert([].constructor.name === 'Array');

        // A user function's own name still wins.
        function Named() {}
        assert(Named.name === 'Named');
    "#);
}

/// The check that found all of this: Immer's `isPlainObject`. It used to throw
/// because `Object.prototype.constructor` was undefined.
#[test]
fn immers_plain_object_check_works() {
    run(r#"
        const objectCtorString = Object.prototype.constructor.toString();
        function isPlainObject(value) {
            if (!value || typeof value !== 'object') return false;
            const proto = Object.getPrototypeOf(value);
            if (proto === null) return true;
            const Ctor = Object.hasOwnProperty.call(proto, 'constructor') && proto.constructor;
            if (Ctor === Object) return true;
            return typeof Ctor == 'function' && Function.toString.call(Ctor) === objectCtorString;
        }
        assert(isPlainObject({}) === true);
        assert(isPlainObject({ a: 1 }) === true);
        assert(isPlainObject(Object.create(null)) === true);
        assert(isPlainObject(5) === false);
        assert(isPlainObject(null) === false);
    "#);
}
