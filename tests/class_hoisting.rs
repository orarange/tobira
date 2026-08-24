//! A class declaration binds its name for the whole enclosing scope, so code
//! written above it can still refer to it once everything has run.
//!
//! Bundlers emit that shape constantly — a helper function defined before the
//! class it constructs. Predeclaration only happened at a module's top level,
//! so inside a function body (where a webpack module lives) the name fell
//! through to a global lookup and threw at run time. This is what took down
//! Yahoo! JAPAN, with a literal `zb is not defined` from a minified `class zb`.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn a_function_above_a_class_can_still_construct_it() {
    run(r#"
        function inFunctionScope() {
            function make() { return new C(); }
            class C { constructor() { this.v = 1 } }
            return make().v;
        }
        assert(inFunctionScope() === 1);
    "#);
}

/// The exact shape from the minified bundle that surfaced this.
#[test]
fn the_bundler_shape_works() {
    run(r#"
        function webpackModule() {
            function factory() { return new Enum({ values: [1, 2] }); }
            class Enum { constructor(def) { this.def = def } }
            Enum.create = factory;
            return Enum.create().def.values.length;
        }
        assert(webpackModule() === 2);
    "#);
}

#[test]
fn arrows_above_a_class_capture_it_too() {
    run(r#"
        function scope() {
            const isOne = (v) => v instanceof K;
            class K {}
            return isOne(new K());
        }
        assert(scope() === true);
    "#);
}

/// At a script's top level a lexical declaration is a global, which is
/// late-bound, so the forward reference resolves there as well. Predeclaration
/// has to agree with that rather than quietly creating a second local binding.
#[test]
fn top_level_classes_still_work() {
    run(r#"
        class Animal {
            constructor(name) { this.name = name }
            speak() { return this.name + ' makes a noise.' }
        }
        class Dog extends Animal {
            constructor(name) { super(name) }
            speak() { return super.speak() + ' Woof!' }
        }
        const d = new Dog('Rex');
        assert(d.speak() === 'Rex makes a noise. Woof!');
        assert(d instanceof Dog);
        assert(d instanceof Animal);
    "#);
}

/// `extends` is evaluated when the class is defined, so a superclass declared
/// afterwards is genuinely unusable and must fail rather than silently work.
#[test]
fn extending_a_later_class_is_still_an_error() {
    run(r#"
        function scope() {
            let threw = false;
            try {
                class A extends B {}
                class B {}
                new A();
            } catch (e) { threw = true }
            return threw;
        }
        assert(scope() === true);
    "#);
}
