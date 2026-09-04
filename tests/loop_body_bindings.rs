//! Per-iteration bindings for declarations inside a loop BODY.
//!
//! A `let`/`const` declared in a loop body is a fresh binding on every
//! iteration, so a closure made during one iteration must keep the value it
//! saw. The loop variable itself already worked (`FreshenLocal`, emitted from
//! the four loop compilers); a binding declared in the body did not, so every
//! closure shared one cell and saw the last value.
//!
//! This is the shape most event-handler code on the web is written in:
//!
//! ```js
//! for (let i = 0; i < items.length; i++) {
//!   const item = items[i];
//!   el.addEventListener('click', () => use(item));   // every listener saw the LAST item
//! }
//! ```
//!
//! Nothing threw and nothing was logged when this was wrong — the page rendered
//! and the handlers simply did something else. `var` is a different matter: it
//! is function-scoped, so sharing one binding is correct, and the tests below
//! pin that down too so a fix cannot "fix" it.

use tobira_engine::engine::{Compiler, Heap, Parser, Value, Vm};

/// Builds `f`, an array of three closures, and returns
/// `f[0]() * 100 + f[1]() * 10 + f[2]()`. Per-iteration bindings give 12
/// (0, 1, 2); one shared binding gives 222.
fn captured(body: &str) -> f64 {
    let source = format!("const f=[]; {body} globalThis.out = f[0]()*100 + f[1]()*10 + f[2]();");
    let program = Parser::new(&source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
    match vm.eval_source("globalThis.out") {
        Ok(Value::Number(value)) => value,
        other => panic!("expected a number, got {other:?}"),
    }
}

fn assert_per_iteration(label: &str, body: &str) {
    let got = captured(body);
    assert_eq!(
        got, 12.0,
        "{label}: closures should have captured 0, 1, 2 (12), got {got}"
    );
}

#[test]
fn the_loop_variable_itself() {
    assert_per_iteration("for(let i)", "for(let i=0;i<3;i++){ f.push(()=>i); }");
    assert_per_iteration("for-of", "for(const x of [0,1,2]){ f.push(()=>x); }");
    assert_per_iteration(
        "for-in",
        "const o={a:0,b:1,c:2}; for(const k in o){ f.push(()=>o[k]); }",
    );
}

#[test]
fn a_binding_declared_in_the_body() {
    assert_per_iteration(
        "const",
        "for(let i=0;i<3;i++){ const n=i; f.push(()=>n); }",
    );
    assert_per_iteration("let", "for(let i=0;i<3;i++){ let n=i; f.push(()=>n); }");
    assert_per_iteration(
        "initialised from an expression",
        "for(let i=0;i<3;i++){ const n=i*1; f.push(()=>n); }",
    );
    assert_per_iteration(
        "destructured",
        "for(let i=0;i<3;i++){ const {v}={v:i}; f.push(()=>v); }",
    );
    assert_per_iteration(
        "captured by a function declaration",
        "for(let i=0;i<3;i++){ const n=i; function g(){return n;} f.push(g); }",
    );
    assert_per_iteration(
        "captured by a nested arrow",
        "for(let i=0;i<3;i++){ const n=i; f.push(()=>(()=>n)()); }",
    );
}

#[test]
fn two_bindings_in_one_body() {
    // Was 432: the loop variable was freshened but a and b were not, so the
    // closures computed 2 + 2 - k. Both halves of that have to be right.
    assert_per_iteration(
        "const a=i, b=i",
        "for(let i=0;i<3;i++){ const a=i, b=i; f.push(()=>a+b-i); }",
    );
}

#[test]
fn every_loop_form() {
    assert_per_iteration(
        "while",
        "let i=0; while(i<3){ const n=i; f.push(()=>n); i++; }",
    );
    assert_per_iteration(
        "do-while",
        "let i=0; do { const n=i; f.push(()=>n); i++; } while(i<3);",
    );
    assert_per_iteration(
        "for-of",
        "for(const x of [0,1,2]){ const n=x; f.push(()=>n); }",
    );
    assert_per_iteration(
        "for-in",
        "const o={a:0,b:1,c:2}; for(const k in o){ const v=o[k]; f.push(()=>v); }",
    );
}

#[test]
fn nested_and_guarded_blocks() {
    assert_per_iteration(
        "extra block",
        "for(let i=0;i<3;i++){ { const n=i; f.push(()=>n); } }",
    );
    assert_per_iteration(
        "inner loop",
        "for(let a=0;a<1;a++){ for(let i=0;i<3;i++){ const n=i; f.push(()=>n); } }",
    );
    assert_per_iteration(
        "try block",
        "for(let i=0;i<3;i++){ try { const n=i; f.push(()=>n); } catch(e){} }",
    );
    assert_per_iteration(
        "switch case",
        "for(let i=0;i<3;i++){ switch(1){ case 1: { const n=i; f.push(()=>n); } } }",
    );
}

#[test]
fn the_shape_real_pages_are_written_in() {
    assert_per_iteration(
        "index loop + const item",
        "const items=[0,1,2]; for(let i=0;i<items.length;i++){ const item=items[i]; f.push(()=>item); }",
    );
    assert_per_iteration(
        "for-of over items",
        "const items=[0,1,2]; for(const item of items){ f.push(()=>item); }",
    );
}

// ---- what must NOT change -------------------------------------------------

#[test]
fn var_stays_function_scoped() {
    // `var` has one binding per function, not per iteration. All three closures
    // share it and see the last value: 2, 2, 2. A fix that gives every
    // body-declared binding a fresh cell would wrongly turn this into 12.
    assert_eq!(
        captured("for(let i=0;i<3;i++){ var n=i; f.push(()=>n); }"),
        222.0,
        "var is function-scoped: all three closures share one binding"
    );
    // Same for the loop variable when it is declared with `var` — after the
    // loop it is 3, and every closure sees that.
    assert_eq!(
        captured("for(var i=0;i<3;i++){ f.push(()=>i); }"),
        333.0,
        "for(var i) has one binding; after the loop it is 3"
    );
}

#[test]
fn a_binding_from_outside_the_loop_is_not_freshened() {
    // The closures capture something declared OUTSIDE the loop, so they must all
    // see the later assignment. Freshening every slot a body closure touches
    // would freeze them at 0 instead.
    let source = "const f=[]; let outer=0;
                  for(let i=0;i<3;i++){ f.push(()=>outer); }
                  outer=9;
                  globalThis.out = f[0]()*100 + f[1]()*10 + f[2]();";
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
    assert_eq!(
        vm.eval_source("globalThis.out"),
        Ok(Value::Number(999.0)),
        "a binding declared outside the loop keeps one cell"
    );
}

#[test]
fn a_body_binding_mutated_after_capture_is_still_shared_within_its_iteration() {
    // Per-iteration does not mean per-statement: within one iteration the
    // binding is one cell, so a later write is visible to a closure made
    // earlier in that same iteration.
    let source = "const f=[];
                  for(let i=0;i<3;i++){ let n=i; f.push(()=>n); n=n+10; }
                  globalThis.out = f[0]()*100 + f[1]()*10 + f[2]();";
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
    // 10, 11, 12 -> 10*100 + 11*10 + 12
    assert_eq!(
        vm.eval_source("globalThis.out"),
        Ok(Value::Number(1122.0)),
        "within one iteration the binding is still a single cell"
    );
}
