// Regression tests for class static initialization blocks and the class's
// inner name binding (the name is available inside the class body while it is
// being defined).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn static_block_basic() {
    run(r#"
        class A { static x; static { A.x = 5; } }
        assert(A.x === 5);
    "#);
}

#[test]
fn static_block_runs_in_source_order_with_fields() {
    run(r#"
        class A {
            static a = 1;
            static { A.b = A.a + 1; }   // sees a == 1
            static c = 10;
            static { A.d = A.c + A.b; } // sees c == 10, b == 2
        }
        assert(A.a === 1);
        assert(A.b === 2);
        assert(A.c === 10);
        assert(A.d === 12);
    "#);
}

#[test]
fn multiple_static_blocks_accumulate() {
    run(r#"
        class Counter {
            static total = 0;
            static { Counter.total += 1; }
            static { Counter.total += 2; }
            static { Counter.total += 3; }
        }
        assert(Counter.total === 6);
    "#);
}

#[test]
fn static_block_has_block_scope() {
    run(r#"
        class A {
            static out;
            static {
                let local = 7;
                const k = 3;
                A.out = local * k;
            }
        }
        assert(A.out === 21);
        // The block-scoped names must not leak to the outer scope.
        assert(typeof local === 'undefined');
        assert(typeof k === 'undefined');
    "#);
}

#[test]
fn named_class_expression_inner_binding() {
    run(r#"
        // `A` is only bound inside the class expression; the static block must
        // still resolve it.
        const C = class A {
            static v;
            static { A.v = 42; }
        };
        assert(C.v === 42);
        assert(typeof A === 'undefined');
    "#);
}

#[test]
fn static_block_can_use_control_flow() {
    run(r#"
        class A {
            static items = [];
            static {
                for (let i = 0; i < 3; i++) {
                    A.items.push(i * i);
                }
            }
        }
        assert(A.items.length === 3);
        assert(A.items[0] === 0 && A.items[1] === 1 && A.items[2] === 4);
    "#);
}

#[test]
fn inner_name_binding_does_not_break_methods() {
    // Regression: introducing the class-body inner name binding must not break
    // methods that reference the class by name (resolved when called later).
    run(r#"
        class A {
            static make() { return new A(); }
            kind() { return 'a'; }
        }
        const inst = A.make();
        assert(inst.kind() === 'a');
        assert(inst instanceof A);
    "#);
}

#[test]
fn static_block_with_inheritance() {
    run(r#"
        class Base { static tag = 'base'; }
        class Derived extends Base {
            static info;
            static { Derived.info = Derived.tag + '/derived'; }
        }
        // Static fields are inherited, so Derived.tag resolves via Base.
        assert(Derived.info === 'base/derived');
    "#);
}
