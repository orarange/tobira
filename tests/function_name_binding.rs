use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) -> Result<(), String> {
    let p = Parser::new(src).parse().map_err(|e| format!("PARSE: {e:?}"))?;
    let c = Compiler::new(&p).compile().map_err(|e| format!("COMPILE: {e:?}"))?;
    Vm::new(Heap::new())
        .execute(&c)
        .map(|_| ())
        .map_err(|e| format!("EXEC: {e:?}"))
}

/// Only a name written in the source goes into scope inside the function.
///
/// The parser works a name out for `{ n: function () {} }` so the function has
/// one to report, and binding that shadowed whatever the surrounding code
/// called `n`. Minified code names things exactly that way: babel's `for...of`
/// helper keeps its iterator in an outer `var n` and reads it back from inside a
/// property also called `n`, so the read found the method instead of the
/// iterator. Every `for...of` over a NodeList threw "next is not a function",
/// which is how firefox.com's header navigation came out as an empty strip.
#[test]
fn an_inferred_function_name_is_not_a_binding() {
    let cases: &[(&str, &str)] = &[
        (
            "object literal property",
            r#"
            var n = { tag: "outer" };
            var o = { n: function () { return n.tag; } };
            assert(o.n() === "outer");
        "#,
        ),
        (
            "object literal method shorthand",
            r#"
            var n = { tag: "outer" };
            var o = { n() { return n.tag; } };
            assert(o.n() === "outer");
        "#,
        ),
        (
            "the babel for-of helper shape",
            r#"
            function helper(source) {
                var n = source[Symbol.iterator];
                return {
                    s: function () { n = n.call(source); },
                    n: function () { return n.next(); },
                };
            }
            var it = helper([7, 8]);
            it.s();
            var first = it.n();
            assert(first.done === false);
            assert(first.value === 7);
        "#,
        ),
        (
            "a written name still binds",
            r#"
            var n = { tag: "outer" };
            var o = { n: function n() { return typeof n; } };
            assert(o.n() === "function");
        "#,
        ),
        (
            "and still binds outside an object literal",
            r#"
            var f = function fact(n) { return n <= 1 ? 1 : n * fact(n - 1); };
            assert(f(5) === 120);
        "#,
        ),
    ];
    for (label, source) in cases {
        if let Err(error) = run(source) {
            panic!("{label}: {error}");
        }
    }
}
