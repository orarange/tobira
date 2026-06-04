// Regression tests for URLSearchParams.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn from_query_string() {
    run(r#"
        const p = new URLSearchParams('a=1&b=2&a=3');
        assert(p.get('a') === '1');
        assert(p.getAll('a').length === 2);
        assert(p.get('b') === '2');
        assert(p.has('a') && !p.has('z'));
        assert(p.get('z') === null);
    "#);
}

#[test]
fn leading_question_mark_and_decoding() {
    run(r#"
        const p = new URLSearchParams('?name=John+Doe&city=New%20York');
        assert(p.get('name') === 'John Doe');
        assert(p.get('city') === 'New York');
    "#);
}

#[test]
fn from_object_and_array() {
    run(r#"
        const a = new URLSearchParams({ x: 1, y: 'two' });
        assert(a.get('x') === '1' && a.get('y') === 'two');
        const b = new URLSearchParams([['k', 'v'], ['k', 'w']]);
        assert(b.getAll('k').join(',') === 'v,w');
    "#);
}

#[test]
fn append_set_delete() {
    run(r#"
        const p = new URLSearchParams();
        p.append('a', '1');
        p.append('a', '2');
        assert(p.getAll('a').length === 2);
        p.set('a', '9');
        assert(p.getAll('a').length === 1 && p.get('a') === '9');
        p.delete('a');
        assert(!p.has('a'));
    "#);
}

#[test]
fn to_string_encoding() {
    run(r#"
        const p = new URLSearchParams();
        p.append('q', 'a b');
        p.append('r', 'x&y');
        assert(p.toString() === 'q=a+b&r=x%26y');
    "#);
}

#[test]
fn iteration() {
    run(r#"
        const p = new URLSearchParams('a=1&b=2');
        const out = [];
        for (const [k, v] of p) out.push(k + '=' + v);
        assert(out.join('&') === 'a=1&b=2');
        assert([...p.keys()].join(',') === 'a,b');
        assert([...p.values()].join(',') === '1,2');
        let collected = '';
        p.forEach((v, k) => { collected += k + v; });
        assert(collected === 'a1b2');
    "#);
}

#[test]
fn sort() {
    run(r#"
        const p = new URLSearchParams('c=3&a=1&b=2');
        p.sort();
        assert([...p.keys()].join('') === 'abc');
    "#);
}

#[test]
fn copy_constructor() {
    run(r#"
        const a = new URLSearchParams('x=1');
        const b = new URLSearchParams(a);
        b.append('y', '2');
        assert(a.toString() === 'x=1');
        assert(b.toString() === 'x=1&y=2');
    "#);
}
