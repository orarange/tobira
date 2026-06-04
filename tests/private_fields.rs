// Regression tests for private class fields and class getters/setters.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn private_field_with_accessors() {
    run(r#"
        class A {
            #v = 0;
            get v() { return this.#v; }
            set v(x) { this.#v = x; }
        }
        const a = new A();
        assert(a.v === 0);
        a.v = 5;
        assert(a.v === 5);
    "#);
}

#[test]
fn private_field_initializer() {
    run(r#"
        class Counter {
            #count = 10;
            increment() { this.#count++; return this.#count; }
            get value() { return this.#count; }
        }
        const c = new Counter();
        assert(c.value === 10);
        assert(c.increment() === 11);
        assert(c.value === 11);
    "#);
}

#[test]
fn private_field_in_constructor() {
    run(r#"
        class Point {
            #x;
            #y;
            constructor(x, y) { this.#x = x; this.#y = y; }
            dist2() { return this.#x * this.#x + this.#y * this.#y; }
        }
        assert(new Point(3, 4).dist2() === 25);
    "#);
}

#[test]
fn class_getter_setter_public() {
    run(r#"
        class Temp {
            constructor(c) { this._c = c; }
            get celsius() { return this._c; }
            set celsius(v) { this._c = v; }
            get fahrenheit() { return this._c * 9 / 5 + 32; }
        }
        const t = new Temp(100);
        assert(t.celsius === 100);
        assert(t.fahrenheit === 212);
        t.celsius = 0;
        assert(t.fahrenheit === 32);
    "#);
}

#[test]
fn static_getter() {
    run(r#"
        class Config {
            static get version() { return '1.0'; }
        }
        assert(Config.version === '1.0');
    "#);
}

#[test]
fn private_field_inheritance() {
    run(r#"
        class Base {
            #secret = 42;
            reveal() { return this.#secret; }
        }
        class Derived extends Base {
            constructor() { super(); }
        }
        assert(new Derived().reveal() === 42);
    "#);
}
