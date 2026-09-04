//! `Object.defineProperty` must merge a partial descriptor with the existing
//! own property (ValidateAndApplyPropertyDefinition) instead of resetting the
//! absent fields. Babel's class transform performs
//! `Object.defineProperty(fn, "prototype", {writable:false})` on every class;
//! clobbering the prototype value to `undefined` broke every subsequent
//! `class B extends A` on real sites (rollupjs.org's Algolia search box).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn babel_class_prototype_freeze_keeps_value() {
    run(r#"
        function A() {}
        Object.defineProperty(A, "prototype", { writable: false });
        assert(typeof A.prototype === "object");
        assert(A.prototype !== null);

        // The follow-up Babel _inherits pattern must keep working.
        function B() {}
        B.prototype = Object.create(A && A.prototype, {
            constructor: { value: B, writable: true, configurable: true },
        });
        Object.defineProperty(B, "prototype", { writable: false });
        assert(typeof B.prototype === "object");
        assert(Object.getPrototypeOf(B.prototype) === A.prototype);
    "#);
}

#[test]
fn partial_data_descriptor_inherits_existing_fields() {
    run(r#"
        const o = {};
        Object.defineProperty(o, "x", { value: 1, writable: true, enumerable: true, configurable: true });
        Object.defineProperty(o, "x", { writable: false });
        assert(o.x === 1);
        assert(Object.keys(o).length === 1); // enumerable survived
        const d = Object.getOwnPropertyDescriptor(o, "x");
        assert(d.writable === false);
        assert(d.enumerable === true);
        assert(d.configurable === true);
    "#);
}

#[test]
fn fresh_property_defaults_to_non_enumerable_undefined() {
    run(r#"
        const o = {};
        Object.defineProperty(o, "y", {});
        assert("y" in o);
        assert(o.y === undefined);
        assert(Object.keys(o).length === 0);
        const d = Object.getOwnPropertyDescriptor(o, "y");
        assert(d.writable === false && d.enumerable === false && d.configurable === false);
    "#);
}

#[test]
fn attribute_only_update_keeps_accessor() {
    run(r#"
        const o = {};
        Object.defineProperty(o, "v", {
            get() { return 42; },
            configurable: true,
        });
        Object.defineProperty(o, "v", { enumerable: true });
        assert(o.v === 42); // getter survived the attribute-only update
        assert(Object.keys(o).indexOf("v") !== -1);
    "#);
}

#[test]
fn partial_accessor_update_keeps_other_side() {
    run(r#"
        let backing = 0;
        const o = {};
        Object.defineProperty(o, "v", {
            get() { return backing; },
            set(next) { backing = next; },
            configurable: true,
        });
        Object.defineProperty(o, "v", { get() { return backing + 100; } });
        o.v = 7;          // setter must survive a get-only redefine
        assert(backing === 7);
        assert(o.v === 107);
    "#);
}

#[test]
fn mixed_accessor_and_data_descriptor_throws() {
    run(r#"
        let threw = false;
        try {
            Object.defineProperty({}, "x", { get() { return 1; }, value: 2 });
        } catch (e) {
            threw = e instanceof TypeError;
        }
        assert(threw);
    "#);
}

#[test]
fn reflect_define_property_merges_too() {
    run(r#"
        function A() {}
        Reflect.defineProperty(A, "prototype", { writable: false });
        assert(typeof A.prototype === "object");
    "#);
}

#[test]
fn define_properties_merges_each_key() {
    run(r#"
        const o = {};
        Object.defineProperty(o, "a", { value: 1, enumerable: true, configurable: true });
        Object.defineProperties(o, { a: { writable: false } });
        assert(o.a === 1);
        assert(Object.keys(o).length === 1);
    "#);
}

#[test]
fn object_create_second_argument_error_names_type() {
    // Companion diagnostic: Object.create with a bad prototype should say
    // what it actually received.
    let program = Parser::new("Object.create(undefined);")
        .parse()
        .expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    let error = vm.execute(&chunk).expect_err("must throw");
    let text = format!("{error:?}");
    assert!(text.contains("got undefined"), "unexpected error: {text}");
}
