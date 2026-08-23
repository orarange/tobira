//! Standard built-ins that real pages depend on.
//!
//! Each gap here was found by loading a real site, reading the uncaught engine
//! error, and tracing it back to the missing primitive. They are cheap to break
//! again by accident, and expensive to debug from a blank page, so they get
//! explicit coverage.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

/// `Set.prototype.keys` is the same function as `values`, and `entries` yields
/// `[v, v]`. Their absence killed the Next.js polyfill bundle, taking every
/// page built on it down to a blank render.
#[test]
fn set_exposes_the_full_iterator_trio() {
    run(r#"
        const s = new Set(['a', 'b']);
        assert(typeof Set.prototype.keys === 'function');
        assert(typeof Set.prototype.entries === 'function');
        assert(Set.prototype.keys === Set.prototype.values, 'keys and values are one function');

        assert([...s.keys()].join(',') === 'a,b');
        assert([...s.values()].join(',') === 'a,b');

        const entries = [...s.entries()];
        assert(entries.length === 2);
        assert(Array.isArray(entries[0]));
        assert(entries[0][0] === 'a' && entries[0][1] === 'a', 'entries pair a value with itself');
        assert(entries[1][0] === 'b' && entries[1][1] === 'b');

        assert([...new Set().entries()].length === 0);
    "#);
}

/// `Symbol` cannot be called with `new`, but it still has a `.prototype`.
/// Polyfill bundles feature-detect symbols with
/// `Object.prototype.isPrototypeOf.call(Symbol.prototype, Object(v))`; with no
/// prototype that threw instead of answering, and the bundle died.
#[test]
fn symbol_has_a_prototype_even_though_it_is_not_constructable() {
    run(r#"
        assert(typeof Symbol.prototype === 'object', 'Symbol.prototype must exist');
        assert(Symbol.prototype !== null);
        assert(typeof Symbol.prototype.toString === 'function');

        // The feature-detection shape that used to throw must now answer.
        const answered = Object.prototype.isPrototypeOf.call(Symbol.prototype, {});
        assert(answered === false);

        // Symbol itself stays non-constructable.
        let threw = false;
        try { new Symbol(); } catch (e) { threw = true; }
        assert(threw, 'Symbol is callable but not constructable');
    "#);
}

/// `Math.imul` is a 32-bit wrapping multiply. Minified bundles use it for
/// hashing and asm.js-style arithmetic, where the wrap-around is the point.
#[test]
fn math_imul_wraps_like_a_32_bit_multiply() {
    run(r#"
        assert(typeof Math.imul === 'function');
        assert(Math.imul(3, 4) === 12);
        assert(Math.imul(-5, 12) === -60);
        assert(Math.imul(0xffffffff, 5) === -5, 'operands are read as int32');
        assert(Math.imul(2, 0x80000000) === 0, 'the product wraps rather than growing');
        assert(Math.imul(0x7fffffff, 2) === -2);
        assert(Math.imul(NaN, 3) === 0);
        assert(Math.imul(Infinity, 3) === 0);
    "#);
}

#[test]
fn math_log1p_is_accurate_near_zero() {
    run(r#"
        assert(typeof Math.log1p === 'function');
        assert(Math.log1p(0) === 0);
        assert(Math.log1p(-1) === -Infinity);
        assert(Math.abs(Math.log1p(Math.E - 1) - 1) < 1e-12);
        // The reason log1p exists: 1 + x loses the small value entirely.
        assert(Math.log1p(1e-16) > 0, 'must not collapse to log(1) === 0');
    "#);
}
