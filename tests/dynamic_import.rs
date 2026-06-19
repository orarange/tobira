use tobira_engine::engine::ast::SourceType;
use tobira_engine::engine::compiler::ModuleContext;
use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn exec_script(vm: &mut Vm, source: &str) {
    let program = Parser::new(source).parse().expect("script should parse");
    let chunk = Compiler::new(&program)
        .compile()
        .expect("script should compile");
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn import_literal_resolves_to_namespace() {
    let dep_key = "\u{0}module:dep".to_string();
    let self_key = "\u{0}module:self".to_string();
    let program = Parser::new(
        r#"
        globalThis.__p = import("./dep");
        import("./dep").then(ns => { globalThis.__x = ns.value; });
        "#,
    )
    .with_source_type(SourceType::Module)
    .parse()
    .expect("module should parse");
    let chunk = Compiler::new(&program)
        .with_module_context(ModuleContext {
            self_key: self_key.clone(),
            imports: Default::default(),
            dynamic_imports: std::iter::once(("./dep".to_string(), dep_key.clone())).collect(),
        })
        .compile()
        .expect("module should compile");

    let mut vm = Vm::new(Heap::new());
    vm.set_global_object(self_key);
    exec_script(
        &mut vm,
        &format!("globalThis[{dep_key:?}] = {{ value: 42 }};"),
    );
    vm.execute_module(&chunk).expect("module should execute");
    exec_script(
        &mut vm,
        r#"
        if (!(globalThis.__p instanceof Promise)) throw new Error("expected promise");
        if (globalThis.__x !== 42) throw new Error("expected resolved value");
        "#,
    );
}

#[test]
fn import_unknown_specifier_rejects() {
    let self_key = "\u{0}module:self".to_string();
    let program = Parser::new(
        r#"
        import((() => { globalThis.__side = 1; return "./missing"; })())
            .then(
                () => { globalThis.__ok = 1; },
                () => { globalThis.__rej = 1; }
            );
        "#,
    )
    .with_source_type(SourceType::Module)
    .parse()
    .expect("module should parse");
    let chunk = Compiler::new(&program)
        .with_module_context(ModuleContext {
            self_key: self_key.clone(),
            imports: Default::default(),
            dynamic_imports: Default::default(),
        })
        .compile()
        .expect("module should compile");

    let mut vm = Vm::new(Heap::new());
    vm.set_global_object(self_key);
    vm.execute_module(&chunk).expect("module should execute");
    exec_script(
        &mut vm,
        r#"
        if (globalThis.__side !== 1) throw new Error("expected side effect");
        if (globalThis.__rej !== 1) throw new Error("expected rejection handler");
        if (typeof globalThis.__ok !== "undefined") throw new Error("unexpected fulfill");
        "#,
    );
}
