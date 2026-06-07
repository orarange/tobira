use tobira_engine::engine::{Compiler, Heap, Parser, Vm};
fn run(src: &str) -> Result<(), String> {
    let p = Parser::new(src).parse().map_err(|e| format!("PARSE: {e:?}"))?;
    let c = Compiler::new(&p).compile().map_err(|e| format!("COMPILE: {e:?}"))?;
    Vm::new(Heap::new()).execute(&c).map(|_|()).map_err(|e| format!("EXEC: {e:?}"))
}
#[test]
fn var_hoisting_from_nested_blocks() {
    let cases: &[(&str,&str)] = &[
        ("hoisted fn refs var in later nested block", r#"
            function f(){ function g(){ return I; } { var I = 5; } return g(); }
            assert(f() === 5);
        "#),
        ("hoisted fn refs var in later if-block", r#"
            function f(){ function g(){ return I; } if(true){ var I = 7; } return g(); }
            assert(f() === 7);
        "#),
        ("hoisted fn refs var declared after (top level of fn)", r#"
            function f(){ function g(){ return I; } var I = 9; return g(); }
            assert(f() === 9);
        "#),
        ("fn expr in var refs var in later block", r#"
            function f(){ var g = function(){ return I; }; { var I = 3; } return g(); }
            assert(f() === 3);
        "#),
        ("hoisted fn refs var in later for-init", r#"
            function f(){ function g(){ return I; } for(var I=4;false;){} return g(); }
            assert(f() === 4);
        "#),
        ("hoisted fn refs var deep in nested blocks", r#"
            function f(){ function g(){ return I; } { { { var I = 8; } } } return g(); }
            assert(f() === 8);
        "#),
        ("hoisted fn refs var in switch case", r#"
            function f(n){ function g(){ return I; } switch(n){ case 1: var I = 6; } return g(); }
            assert(f(1) === 6);
        "#),
    ];
    let mut fails = Vec::new();
    for (name, src) in cases {
        let full = format!("function assert(c){{ if(!c) throw new Error('x'); }}\n{src}");
        if let Err(e) = run(&full) {
            fails.push(format!("{name} -> {e}"));
        }
    }
    assert!(fails.is_empty(), "var-hoisting regressions:\n{}", fails.join("\n"));
}
