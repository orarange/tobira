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
fn probe_hoisting() {
    let cases: &[(&str, &str)] = &[
        ("var hoists from block to function scope", r#"
            function f(){ { var x = 1; } return x; }
            assert(f() === 1);
        "#),
        ("var in if-block visible after", r#"
            function f(){ if (true) { var x = 5; } return x; }
            assert(f() === 5);
        "#),
        ("var in for-block", r#"
            function f(){ for (var i=0;i<1;i++){ var y = 9; } return y; }
            assert(f() === 9);
        "#),
        ("var in try", r#"
            function f(){ try { var z = 3; } catch(e){} return z; }
            assert(f() === 3);
        "#),
        ("var in switch case", r#"
            function f(n){ switch(n){ case 1: var w = 7; break; default: } return w; }
            assert(f(1) === 7);
        "#),
        ("function decl hoisted (call before)", r#"
            function f(){ return g(); function g(){ return 42; } }
            assert(f() === 42);
        "#),
        ("function decl in block hoists (Annex B)", r#"
            function f(){ { function g(){ return 11; } } return g(); }
            assert(f() === 11);
        "#),
        ("function decl in if-block used after", r#"
            function f(cond){ if(cond){ function g(){return 1;} } return typeof g; }
            f(true);
        "#),
        ("var hoist self-reference undefined then assigned", r#"
            function f(){ assert(typeof x === 'undefined'); var x = 2; assert(x===2); }
            f();
        "#),
        ("IIFE returns named fn expr", r#"
            var m = (function(){ function I(){ return 5; } return { I: I }; })();
            assert(m.I() === 5);
        "#),
        ("conditional function expr assigned to var in block", r#"
            function f(){ var I; { I = function(){ return 8; }; } return I(); }
            assert(f() === 8);
        "#),
        ("nested block var across siblings", r#"
            function f(){ { var a = 1; } { var b = a + 1; } return b; }
            assert(f() === 2);
        "#),
        ("hoist var declared after return-ish use in closure", r#"
            function f(){ function g(){ return I; } var I = 99; return g(); }
            assert(f() === 99);
        "#),
        ("labeled block with var", r#"
            function f(){ L: { var q = 4; break L; } return q; }
            assert(f() === 4);
        "#),
        ("do-while var", r#"
            function f(){ do { var d = 6; } while(false); return d; }
            assert(f() === 6);
        "#),
        ("var in catch param shadow", r#"
            function f(){ try { throw 1; } catch(e){ var r = e + 1; } return r; }
            assert(f() === 2);
        "#),
    ];

    let mut failures = Vec::new();
    for (name, src) in cases {
        let full = format!(
            "function assert(c){{ if(!c) throw new Error('assert failed'); }}\n{src}"
        );
        if let Err(e) = run(&full) {
            failures.push(format!("{name} -> {e}"));
        }
    }
    assert!(
        failures.is_empty(),
        "hoisting/scoping regressions:\n{}",
        failures.join("\n")
    );
}
