//! `for...of` used to snapshot its source before the loop body ran once.
//!
//! That is observable in three ways, all of which real code depends on: `break`
//! could not stop the producer, side effects did not interleave with the loop
//! body, and an endless producer never returned. Built-in collections are still
//! snapshotted — they cannot suspend, so it is not observable there and copying
//! once is cheaper than a call per element.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn break_stops_a_generator_from_producing_more() {
    run(r#"
        let produced = 0;
        function* counter() {
            for (let i = 0; i < 1000; i++) { produced++; yield i; }
        }
        let seen = 0;
        for (const value of counter()) { seen++; if (seen === 3) break; }
        assert(seen === 3);
        assert(produced === 3, 'the generator produced ' + produced + ' values, expected 3');
    "#);
}

#[test]
fn break_stops_a_custom_iterable_from_being_pulled() {
    run(r#"
        let pulled = 0;
        const iterable = {
            [Symbol.iterator]() {
                let n = 0;
                return { next: () => { pulled++; return n < 1000 ? { value: n++, done: false } : { done: true }; } };
            }
        };
        let seen = 0;
        for (const value of iterable) { seen++; if (seen === 3) break; }
        assert(pulled === 3, 'next() was called ' + pulled + ' times, expected 3');
    "#);
}

/// The producer must run between iterations, not all of it before any of them.
#[test]
fn producer_and_loop_body_interleave() {
    run(r#"
        const log = [];
        function* g() { log.push('in1'); yield 1; log.push('in2'); yield 2; }
        for (const v of g()) { log.push('out' + v); }
        assert(log.join(',') === 'in1,out1,in2,out2', 'got ' + log.join(','));
    "#);
}

/// An endless generator is only usable if iteration is lazy. Snapshotting one
/// would spin until the internal cap and then hand back the wrong thing.
#[test]
fn an_endless_generator_is_usable_with_break() {
    run(r#"
        function* naturals() { let i = 0; while (true) yield i++; }
        const first = [];
        for (const n of naturals()) { first.push(n); if (first.length === 5) break; }
        assert(first.join(',') === '0,1,2,3,4');
    "#);
}

/// Spread and destructuring genuinely need every element, so they still drain.
#[test]
fn eager_consumers_still_get_everything() {
    run(r#"
        function* g() { yield 1; yield 2; yield 3; }
        assert([...g()].join(',') === '1,2,3');
        assert(Array.from(g()).join(',') === '1,2,3');
        const [a, b] = g();
        assert(a === 1 && b === 2);
    "#);
}

/// Built-in collections keep the snapshot path; iterating them must still work.
#[test]
fn builtin_collections_still_iterate() {
    run(r#"
        assert([...[1, 2, 3]].join(',') === '1,2,3');
        assert([...'ab'].join(',') === 'a,b');
        assert([...new Set([1, 2])].join(',') === '1,2');
        assert([...new Map([['k', 'v']])][0].join(':') === 'k:v');
        assert([...[10, 20].entries()][1].join(':') === '1:20');

        let sum = 0;
        for (const n of [1, 2, 3]) { sum += n; if (n === 2) break; }
        assert(sum === 3);
    "#);
}

/// `throw()` resumes the generator by raising at the paused `yield`, so the
/// generator's own `try`/`catch` sees it. Transpiled async/await and saga
/// libraries drive generators this way.
#[test]
fn generator_throw_is_catchable_inside_the_generator() {
    run(r#"
        function* g() {
            try { yield 1; } catch (e) { yield 'caught:' + e; }
            yield 'after';
        }
        const it = g();
        assert(it.next().value === 1);
        assert(it.throw('boom').value === 'caught:boom');
        assert(it.next().value === 'after');
    "#);
}

/// With nothing to catch it, the value escapes to the caller and the generator
/// is finished.
#[test]
fn generator_throw_escapes_when_uncaught() {
    run(r#"
        function* g() { yield 1; yield 2; }
        const it = g();
        it.next();
        let caught = null;
        try { it.throw('boom'); } catch (e) { caught = e; }
        assert(caught === 'boom');
        assert(it.next().done === true, 'the generator should be finished');

        // Throwing into a generator that has not started cannot be caught by it.
        const fresh = g();
        let escaped = null;
        try { fresh.throw('early'); } catch (e) { escaped = e; }
        assert(escaped === 'early');
    "#);
}
