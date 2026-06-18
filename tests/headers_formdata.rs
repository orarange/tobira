// Regression tests for Headers and FormData.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn headers_basic_behaviour() {
    run(r#"
        const h = new Headers({ 'Content-Type': 'text/html' });
        assert(h.get('content-type') === 'text/html');
        assert(h.has('Content-Type') === true);
        h.append('X-A', '1');
        h.append('X-A', '2');
        assert(h.get('x-a') === '1, 2');
        h.set('x-a', '9');
        assert(h.get('x-a') === '9');
        h.delete('x-a');
        assert(h.has('x-a') === false);
        assert(h.get('nope') === null);
    "#);
}

#[test]
fn form_data_basic_behaviour() {
    run(r#"
        const f = new FormData();
        f.append('a', '1');
        f.append('a', '2');
        f.append('b', '3');
        assert(f.get('a') === '1');
        const all = f.getAll('a');
        assert(all.length === 2 && all[0] === '1' && all[1] === '2');
        assert(f.has('b') === true);
        f.set('a', '9');
        assert(f.getAll('a').length === 1 && f.get('a') === '9');
        f.delete('b');
        assert(f.has('b') === false);
    "#);
}

#[test]
fn headers_iteration() {
    run(r#"
        const h = new Headers();
        h.append('k', 'v');
        let seen = '';
        h.forEach(function(val, key) { seen += key + '=' + val; });
        assert(seen === 'k=v');
    "#);
}
