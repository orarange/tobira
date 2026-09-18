use tobira_engine::engine::{
    Compiler, Opcode, Parser, SourceType, compute_stack_depths, verify_stack_balance,
};

fn verify_script(source: &str) -> Option<String> {
    let program = match Parser::new(source).parse() {
        Ok(program) => program,
        Err(e) => return Some(format!("{source}
    -> parse: {e:?}")),
    };
    let chunk = match Compiler::new(&program).compile() {
        Ok(chunk) => chunk,
        Err(e) => return Some(format!("{source}
    -> compile: {e:?}")),
    };
    verify_stack_balance(&chunk.top_level)
        .err()
        .map(|e| format!("{source}
    -> {e:?}"))
}

fn verify_module(source: &str) -> Option<String> {
    let program = Parser::new(source)
        .with_source_type(SourceType::Module)
        .parse()
        .expect("module should parse");
    let chunk = Compiler::new(&program)
        .compile()
        .expect("module should compile");
    verify_stack_balance(&chunk.top_level)
        .err()
        .map(|e| format!("{source}
    -> {e:?}"))
}

fn assert_linear_depths(proto: &tobira_engine::engine::FunctionProto) {
    let depths = compute_stack_depths(proto);
    for nested in &proto.nested_functions {
        assert_linear_depths(nested);
    }

    for ip in 0..proto.code.len().saturating_sub(1) {
        let Some(depth) = depths[ip] else {
            continue;
        };
        let Some(next_depth) = depths[ip + 1] else {
            continue;
        };

        let opcode = &proto.code[ip];
        let Some((pops, pushes)) = linear_effect(opcode) else {
            continue;
        };
        if matches!(
            opcode,
            Opcode::Jump(_)
                | Opcode::JumpIfTrue(_)
                | Opcode::JumpIfFalse(_)
                | Opcode::JumpIfTruePop(_)
                | Opcode::JumpIfFalsePop(_)
                | Opcode::JumpIfNullish(_)
                | Opcode::Return
                | Opcode::AsyncReturn
                | Opcode::Throw
                | Opcode::Spread
                | Opcode::GetSuperCtor
        ) {
            continue;
        }
        let expected = depth - pops + pushes;
        assert_eq!(
            next_depth, expected,
            "depth mismatch at ip {ip} for {:?}: {depth} -> {next_depth}, expected {expected}",
            opcode
        );
    }
}

fn linear_effect(opcode: &Opcode) -> Option<(i64, i64)> {
    Some(match opcode {
        Opcode::LoadConst(_)
        | Opcode::LoadUndefined
        | Opcode::LoadNull
        | Opcode::LoadTrue
        | Opcode::LoadFalse
        | Opcode::LoadThis
        | Opcode::LoadNewTarget
        | Opcode::GetLocal(_)
        | Opcode::GetUpvalue(_)
        | Opcode::GetGlobal(_)
        | Opcode::GetGlobalOptional(_)
        | Opcode::DynamicImport
        | Opcode::LoadArguments
        | Opcode::MakeClosure(_)
        | Opcode::MakeObject
        | Opcode::MakeRegExp(_)
        | Opcode::GetProp
        | Opcode::GetIndex
        | Opcode::GetForInKeys
        | Opcode::GetForOfIterator
        | Opcode::GetForAwaitIterator
        | Opcode::GetProto => {
            let pops = match opcode {
                Opcode::GetProp | Opcode::GetIndex => 2,
                Opcode::DynamicImport => 1,
                Opcode::GetForInKeys
                | Opcode::GetForOfIterator
                | Opcode::GetForAwaitIterator
                | Opcode::GetProto => 1,
                _ => 0,
            };
            (pops, 1)
        }
        Opcode::GetPropForCall(_) => (1, 2),
        Opcode::GetIndexForCall => (2, 2),
        Opcode::Pop | Opcode::SetLocal(_) | Opcode::SetUpvalue(_) | Opcode::SetGlobal(_) => (1, 0),
        Opcode::Neg
        | Opcode::Not
        | Opcode::BitNot
        | Opcode::Typeof
        | Opcode::ToNumber
        | Opcode::Delete
        | Opcode::Void
        | Opcode::Await
        | Opcode::Yield => (1, 1),
        Opcode::Dup => (0, 1),
        Opcode::FreshenLocal(_) => (0, 0),
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::Div
        | Opcode::Rem
        | Opcode::Exp
        | Opcode::Eq
        | Opcode::StrictEq
        | Opcode::Ne
        | Opcode::StrictNe
        | Opcode::Lt
        | Opcode::Le
        | Opcode::Gt
        | Opcode::Ge
        | Opcode::BitAnd
        | Opcode::BitOr
        | Opcode::BitXor
        | Opcode::Shl
        | Opcode::Shr
        | Opcode::UShr
        | Opcode::In
        | Opcode::Instanceof => (2, 1),
        Opcode::DeleteProp => (2, 1),
        Opcode::DefineGetter | Opcode::DefineSetter => (3, 0),
        Opcode::Call(argc) | Opcode::CallSpread(argc) => (i64::from(*argc) + 2, 1),
        Opcode::MakeArray(count) => (i64::from(*count), 1),
        Opcode::SetProp | Opcode::SetIndex => (3, 0),
        Opcode::CopyDataProperties => (2, 1),
        Opcode::New(argc) => (i64::from(*argc) + 1, 1),
        Opcode::ForOfNext => (1, 2),
        Opcode::SetProtoOf => (2, 1),
        Opcode::SetObjectLiteralProto => (2, 0),
        // Pushes this module's `import.meta`; takes nothing.
        Opcode::ImportMeta(_) => (0, 1),
        Opcode::EnterTry(_) | Opcode::LeaveTry | Opcode::EndFinally | Opcode::Nop => (0, 0),
        Opcode::Jump(_)
        | Opcode::JumpIfTrue(_)
        | Opcode::JumpIfFalse(_)
        | Opcode::JumpIfTruePop(_)
        | Opcode::JumpIfFalsePop(_)
        | Opcode::JumpIfNullish(_)
        | Opcode::Return
        | Opcode::AsyncReturn
        | Opcode::Throw
        | Opcode::Spread
        | Opcode::GetSuperCtor => return None,
    })
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
        // A value-producing expression in every place an expression can go.
        // The class expression as a call argument left two values on the
        // stack (an extra `Dup`), which underflow and merge checks never see;
        // the return-depth check does. Each shape below is tried with a
        // class, a function, an arrow, an object and an array, so the same
        // slip in any other `compile_*_value` shows up too.
        "f(class {});",
        "f(1, class {}, 2);",
        "o.m(class {});",
        "o['m'](class {});",
        "new F(class {});",
        "f(...[class {}]);",
        "var a1 = [class {}, 1];",
        "var o1 = { k: class {}, n: 1 };",
        "var o2 = { [class {}]: 1 };",
        "function r() { return class {}; }",
        "var d = (x = class {}) => x;",
        "var t = cond ? class {} : 1;",
        "var c = (0, class {});",
        "var s = `${class {}}`;",
        "var b = typeof class {} + '';",
        "var n = new (class {})();",
        "var e = class extends Base {};",
        "var e2 = class X { static s = 1; #p = 2; static { init(); } m() {} get g() { return 1; } static sm() {} };",
        "f(class { constructor(a) { this.a = a; } });",
        "f(function () {}, () => 1, { k: 1 }, [1]);",
        "var q = f(f(class {}));",
        "function g1(it) { for (const x of it) { if (x) return x; } }",
        "function g2(it) { for (const x of it) { if (x) break; } return 1; }",
        "function g3() { try { return f(); } finally { h(); } }",
        "function g4() { try { throw e; } catch (x) { return x; } }",
        "function g5(o) { for (const k in o) { return k; } }",
        "function g6(a) { switch (a) { case 1: return 2; default: return 3; } }",
        "function g7() { label: for (;;) { for (;;) { break label; } } return 1; }",
        "async function g8(it) { for await (const x of it) { return x; } }",
        "function* g9() { const x = yield 1; return x; }",
        "function g10(o) { const { a = f(class {}) } = o; return a; }",
        "function g11(a) { return a?.b?.(class {}); }",
        "function g12() { return tag`x${class {}}y`; }",
        "function g13(a) { a ||= class {}; a &&= 1; a ??= 2; return a; }",
        "function g14() { return [...f(class {})]; }",
        "function g15() { return { ...f(class {}) }; }",
        "function g16() { return new.target; }",
        "function g17() { return delete o[f(class {})]; }",
        "function g18(a) { return a instanceof class {} && 'x' in class {}; }",
        "function g19() { return (class {}).name; }",
        "function g20(x) { return x = class {}; }",
        "function g21(o) { return o.p = class {}; }",
        "function g22(o) { return o[f()] = class {}; }",
        "function g23() { return [class {}][0]; }",
        "function g24() { return (function () { return class {}; })(); }",
        "function g25() { return f(class {}) + f(class {}); }",
    ];

    // Every failure, not the first: a table slip usually shows in several.
    let mut failures = Vec::new();
    for source in scripts {
        failures.extend(verify_script(source));
    }

    let modules = [
        "export const x = 1; export default function f() { return x; }",
        "export const ns = 1;",
        "export async function load() { return await Promise.resolve(1); }",
    ];

    for source in modules {
        failures.extend(verify_module(source));
    }
    assert!(
        failures.is_empty(),
        "{} of the corpus failed to verify:
{}",
        failures.len(),
        failures.join("
")
    );
}

#[test]
fn compute_stack_depths_matches_linear_transitions() {
    let source = "function f(a, b, c) { var g = () => this.x + a + b + c; var h = () => a * b; return g() + h(); }";
    let program = Parser::new(source).parse().expect("script should parse");
    let chunk = Compiler::new(&program)
        .compile()
        .expect("script should compile");
    assert_linear_depths(&chunk.top_level);
}
