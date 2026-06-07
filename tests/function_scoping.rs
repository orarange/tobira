use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) -> Result<(), String> {
    let program = Parser::new(src).parse().map_err(|e| format!("PARSE: {e:?}"))?;
    let chunk = Compiler::new(&program)
        .compile()
        .map_err(|e| format!("COMPILE: {e:?}"))?;
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).map(|_| ()).map_err(|e| format!("EXEC: {e:?}"))
}

#[test]
fn function_scoping_patterns() {
    let cases: &[(&str, &str)] = &[
        ("fn decl after return in block", r#"
            function f(){ return g(); return; function g(){ return 1; } }
            assert(f() === 1);
        "#),
        ("fn decl inside switch case", r#"
            function f(n){ switch(n){ case 1: return g(); function g(){return 2;} default: return 0; } }
            assert(f(1) === 2);
        "#),
        ("fn decl referenced from other switch case", r#"
            function f(n){ switch(n){ case 1: function g(){return 3;} case 2: return g(); default: return 0; } }
            assert(f(2) === 3);
        "#),
        // NOTE: Annex B "labeled function declaration" and "braceless-if function
        // declaration" are not parsed yet (separate parser limitation; Terser emits
        // braces so they don't appear in minified bundles). Tracked separately.
        ("nested fn decl referenced in sibling fn", r#"
            function f(){ function a(){ return b(); } function b(){ return 6; } return a(); }
            assert(f() === 6);
        "#),
        ("var assigned then read as implicit (strict off) seq", r#"
            function f(){ var I; return (I = function(){return 7;}), I(); }
            assert(f() === 7);
        "#),
        ("comma-declared vars deep", r#"
            function f(){ var a=1,b=2,c=3,d=4,e=5,I=8,g=9; return I; }
            assert(f() === 8);
        "#),
        ("fn expr name in own scope (named fn expr recursion)", r#"
            var fac = function I(n){ return n<=1?1:n*I(n-1); };
            assert(fac(5) === 120);
        "#),
        ("fn decl in try block used after", r#"
            function f(){ try { function g(){ return 10; } } catch(e){} return g(); }
            assert(f() === 10);
        "#),
        ("fn decl in for body", r#"
            function f(){ for(var i=0;i<1;i++){ function g(){return 11;} } return g(); }
            assert(f() === 11);
        "#),
        ("conditional fn decl both branches", r#"
            function f(c){ if(c){ function g(){return 12;} } else { function g(){return 13;} } return g(); }
            f(true);
        "#),
        ("fn decl shadowing outer in block", r#"
            function g(){ return 'outer'; }
            function f(){ { function g(){ return 'inner'; } return g(); } }
            assert(f() === 'inner');
        "#),
        ("IIFE bang with named fn decl inside", r#"
            var out;
            !function(){ function I(){ return 14; } out = I(); }();
            assert(out === 14);
        "#),
        ("do-block fn decl", r#"
            function f(){ do { function g(){return 15;} } while(false); return g(); }
            assert(f() === 15);
        "#),
        ("named FE name used in nested closure", r#"
            var f = function I(n){ return (function(){ return n<=1?1:n*I(n-1); })(); };
            assert(f(5) === 120);
        "#),
        ("named FE name used in nested arrow", r#"
            var f = function I(n){ var g = () => (n<=1?1:n*I(n-1)); return g(); };
            assert(f(4) === 24);
        "#),
        ("named FE referenced as property/value not call", r#"
            var f = function I(){ return typeof I === 'function' ? I : null; };
            assert(f() === f);
        "#),
        ("var fn-expr deep recursion (minified style)", r#"
            var m = {}; m.walk = function I(o,d){ return d<=0?o:I(o+1,d-1); };
            assert(m.walk(0,10) === 10);
        "#),
    ];

    let mut failures = Vec::new();
    for (name, src) in cases {
        let full = format!("function assert(c){{ if(!c) throw new Error('assert failed: '); }}\n{src}");
        if let Err(e) = run(&full) {
            failures.push(format!("{name} -> {e}"));
        }
    }
    assert!(
        failures.is_empty(),
        "function-scoping/hoisting regressions:\n{}",
        failures.join("\n")
    );
}
