use std::collections::HashMap;

use tobira_engine::engine::ast::SourceType;
use tobira_engine::engine::compiler::ModuleContext;
use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn compile_module(source: &str, self_key: String, imports: HashMap<String, String>) -> tobira_engine::engine::Chunk {
    let program = Parser::new(source)
        .with_source_type(SourceType::Module)
        .parse()
        .expect("module should parse");
    Compiler::new(&program)
        .with_module_context(ModuleContext {
            meta_url: String::new(),
            self_key,
            imports,
            dynamic_imports: Default::default(),
        })
        .compile()
        .expect("module should compile")
}

fn exec_script(vm: &mut Vm, source: &str) {
    let program = Parser::new(source).parse().expect("script should parse");
    let chunk = Compiler::new(&program)
        .compile()
        .expect("script should compile");
    vm.execute(&chunk).expect("script should execute");
}

fn exec_module_pair(module_a: &str, module_b: &str) -> Vm {
    let a_key = "\u{0}module:a".to_string();
    let b_key = "\u{0}module:b".to_string();
    let a_chunk = compile_module(module_a, a_key.clone(), Default::default());
    let b_chunk = compile_module(
        module_b,
        b_key.clone(),
        std::iter::once(("./a.js".to_string(), a_key.clone())).collect(),
    );

    let mut vm = Vm::new(Heap::new());
    vm.set_global_object(a_key);
    vm.set_global_object(b_key);
    vm.execute_module(&a_chunk).expect("module A should execute");
    vm.execute_module(&b_chunk).expect("module B should execute");
    vm
}

#[test]
fn module_top_level_const_does_not_collide_across_modules() {
    let mut vm = exec_module_pair(
        r#"
        const inner = () => "real";
        export const outer = () => inner;
        "#,
        r#"
        import { outer } from "./a.js";
        const inner = ["items"];
        globalThis.__r = outer()();
        "#,
    );

    exec_script(
        &mut vm,
        r#"if (globalThis.__r !== "real") throw new Error("expected module A binding");"#,
    );
}

#[test]
fn module_top_level_functions_are_hoisted_for_mutual_recursion() {
    let mut vm = exec_module_pair(
        r#"
        export function a(n){ return n <= 0 ? "done" : b(n - 1); }
        function b(n){ return a(n - 1); }
        "#,
        r#"
        import { a } from "./a.js";
        globalThis.__r = a(3);
        "#,
    );

    exec_script(
        &mut vm,
        r#"if (globalThis.__r !== "done") throw new Error("expected mutual recursion");"#,
    );
}

#[test]
fn module_top_level_forward_reference_resolves_to_local() {
    let mut vm = exec_module_pair(
        r#"
        export const outer = () => later;
        const later = 42;
        "#,
        r#"
        import { outer } from "./a.js";
        globalThis.__r = outer();
        "#,
    );

    exec_script(
        &mut vm,
        r#"if (globalThis.__r !== 42) throw new Error("expected forward reference");"#,
    );
}

#[test]
fn module_top_level_destructured_const_is_captured_locally() {
    let mut vm = exec_module_pair(
        r#"
        const { x } = { x: 7 };
        export const get = () => x;
        "#,
        r#"
        import { get } from "./a.js";
        const x = 999;
        globalThis.__r = get();
        "#,
    );

    exec_script(
        &mut vm,
        r#"if (globalThis.__r !== 7) throw new Error("expected destructured binding");"#,
    );
}

#[test]
fn module_top_level_closure_captures_block_var() {
    let mut vm = exec_module_pair(
        r#"
        export const get = () => v;
        { var v = "block"; }
        "#,
        r#"
        import { get } from "./a.js";
        globalThis.__r = get();
        "#,
    );

    exec_script(
        &mut vm,
        r#"if (globalThis.__r !== "block") throw new Error("expected block var capture");"#,
    );
}

#[test]
fn module_top_level_declarations_do_not_leak_to_global_this() {
    let key = "\u{0}module:leak".to_string();
    let chunk = compile_module(
        r#"
        var leak = 1;
        const leak2 = 2;
        "#,
        key.clone(),
        Default::default(),
    );
    let mut vm = Vm::new(Heap::new());
    vm.set_global_object(key);
    vm.execute_module(&chunk).expect("module should execute");

    exec_script(
        &mut vm,
        r#"
        if (typeof globalThis.leak !== "undefined") throw new Error("var leaked");
        if (typeof globalThis.leak2 !== "undefined") throw new Error("const leaked");
        "#,
    );
}
