use tobira_engine::engine::{Compiler, Parser, SourceType, verify_stack_balance};

fn verify_script(source: &str) {
    let program = Parser::new(source).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    verify_stack_balance(&chunk.top_level).expect(source);
}

fn verify_module(source: &str) {
    let program = Parser::new(source)
        .with_source_type(SourceType::Module)
        .parse()
        .expect("module should parse");
    let chunk = Compiler::new(&program).compile().expect("module should compile");
    verify_stack_balance(&chunk.top_level).expect(source);
}

#[test]
fn corpus_verifies_stack_balance() {
    let scripts = [
        "let x = 1 + 2 * 3; x;",
        "if (a) { b(); } else { c(); }",
        "while (i < 10) { i++; }",
        "do { i--; } while (i > 0);",
        "for (let i = 0; i < 10; i++) { sum += i; }",
        "for (const key in obj) { keys.push(key); }",
        "for (const value of values) { total += value; }",
        "switch (x) { case 1: y(); break; default: z(); }",
        "try { f(); } catch (e) { g(e); }",
        "try { f(); } finally { h(); }",
        "try { f(); } catch (e) { g(e); } finally { h(); }",
        // Unary operators transform the top of stack (pop 1, push 1). A closure
        // reading a captured var through `!` in a condition (the runtime prelude's
        // shape) caught a verifier table bug that grouped these with Pop (1,0).
        "if (!flag) { run(); }",
        "var n = -value; var t = typeof obj; var b = ~bits; var v = void 0; use(n, t, b, v);",
        "function outer(other) { return function () { if (!other) return; use(other); }; }",
        "const x = (a && b) || c;",
        "const z = a ?? b;",
        "const y = cond ? left() : right();",
        "function f(a, b) { return a + b; }",
        "const f = (a, b) => a * b;",
        "class A { constructor(x) { this.x = x; } m() { return this.x; } }",
        "const { a, b: c } = obj;",
        "const [x, ...rest] = arr;",
        "const o = { a: 1, ['b' + 2]: 3, ...src };",
        "const arr2 = [1, 2, ...arr, 4];",
        "tag`hello ${name} world`;",
        "async function af() { await g(); return 1; }",
        "function* gen() { yield 1; return 2; }",
        "const m = import('./mod.js');",
    ];

    for source in scripts {
        verify_script(source);
    }

    let modules = [
        "export const x = 1; export default function f() { return x; }",
        "export const ns = 1;",
        "export async function load() { return await Promise.resolve(1); }",
    ];

    for source in modules {
        verify_module(source);
    }
}
