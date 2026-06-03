// Regression tests for Proxy (get/set/has traps + forwarding to the target).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn proxy_get_trap() {
    run(r#"
        const p = new Proxy({}, { get: (t, k) => (k === 'x' ? 42 : undefined) });
        assert(p.x === 42);
        assert(p.y === undefined);
    "#);
}

#[test]
fn proxy_set_trap() {
    run(r#"
        let calls = 0;
        const p = new Proxy({}, {
            set(t, k, v) { calls++; t[k] = v; return true; },
        });
        p.a = 1;
        p.b = 2;
        assert(calls === 2);
        assert(p.a === 1 && p.b === 2);
    "#);
}

#[test]
fn proxy_forwards_without_traps() {
    run(r#"
        const target = { a: 1 };
        const p = new Proxy(target, {});
        assert(p.a === 1);
        p.b = 2;
        assert(target.b === 2);
        assert(p.b === 2);
    "#);
}

#[test]
fn proxy_has_trap() {
    run(r#"
        const p = new Proxy({}, { has: (t, k) => k.startsWith('_') });
        assert('_secret' in p);
        assert(!('public' in p));
    "#);
}

#[test]
fn proxy_default_get_forwards() {
    run(r#"
        const p = new Proxy({ greet() { return 'hi'; } }, {});
        assert(p.greet() === 'hi');
    "#);
}

#[test]
fn proxy_computed_reactivity_pattern() {
    run(r#"
        // Mimics a minimal reactive store.
        const log = [];
        const state = new Proxy({ count: 0 }, {
            set(t, k, v) { log.push(k + '=' + v); t[k] = v; return true; },
        });
        state.count = 1;
        state.count = 2;
        assert(state.count === 2);
        assert(log.join(',') === 'count=1,count=2');
    "#);
}
