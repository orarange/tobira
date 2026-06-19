use tobira_engine::engine::{
    Compiler, Opcode, Parser, SourceType, compute_stack_depths, verify_stack_balance,
};

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
        Opcode::Pop
        | Opcode::SetLocal(_)
        | Opcode::SetUpvalue(_)
        | Opcode::SetGlobal(_) => (1, 0),
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

#[test]
fn compute_stack_depths_matches_linear_transitions() {
    let source = "function f(a, b, c) { var g = () => this.x + a + b + c; var h = () => a * b; return g() + h(); }";
    let program = Parser::new(source).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    assert_linear_depths(&chunk.top_level);
}
