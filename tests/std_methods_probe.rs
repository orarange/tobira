use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn std_methods_probe() {
    run(r#"
        assert(Math.sinh(0) === 0);
        assert(Math.cosh(0) === 1);
        assert(Math.tanh(0) === 0);
        assert(Math.expm1(0) === 0);
        assert(Math.fround(1.5) === 1.5);
        assert(typeof Math.asinh === 'function' && typeof Math.acosh === 'function' && typeof Math.atanh === 'function');

        assert(Number.isSafeInteger(5) === true);
        assert(Number.isSafeInteger(2**53) === false);
        assert(Number.isSafeInteger(1.5) === false);
        assert(Number.isSafeInteger('5') === false);

        const s = Symbol('x');
        const o = {};
        o[s] = 1;
        const syms = Object.getOwnPropertySymbols(o);
        assert(Array.isArray(syms));
        assert(syms.length === 1);
        assert(syms[0] === s);
        assert(Object.getOwnPropertySymbols({a:1}).length === 0);

        assert(Date.UTC(1970, 0, 1) === 0);
        assert(Date.UTC(1970, 0, 2) === 86400000);
        assert(Date.parse('1970-01-01') === 0);
        assert(Date.parse('1970-01-02T00:00:00Z') === 86400000);
        assert(Number.isNaN(Date.parse('not a date')));
        assert((new Date(0)).getTimezoneOffset() === 0);
    "#);
}
