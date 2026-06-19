use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn new_vm() -> Vm {
    Vm::new(Heap::new())
}

fn execute_script(vm: &mut Vm, source: &str) {
    let program = Parser::new(source).parse().expect("script should parse");
    let chunk = Compiler::new(&program)
        .compile()
        .expect("script should compile");
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn url_absolute_split_and_search_params() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        const u = new URL('https://user:pass@example.com:8080/p/q?a=1&b=2#frag');
        assert(u.protocol === 'https:');
        assert(u.username === 'user');
        assert(u.password === 'pass');
        assert(u.hostname === 'example.com');
        assert(u.port === '8080');
        assert(u.host === 'example.com:8080');
        assert(u.pathname === '/p/q');
        assert(u.search === '?a=1&b=2');
        assert(u.hash === '#frag');
        assert(u.origin === 'https://example.com:8080');
        assert(u.searchParams.get('a') === '1');
        assert(u.searchParams.get('b') === '2');
        "#,
    );
}

#[test]
fn url_no_port() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        const u = new URL('https://example.com/');
        assert(u.port === '');
        assert(u.host === 'example.com');
        assert(u.origin === 'https://example.com');
        assert(u.pathname === '/');
        "#,
    );
}

#[test]
fn url_relative_resolution() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        const a = new URL('/x/y', 'https://h.com/a/b');
        const b = new URL('z', 'https://h.com/a/b');
        const c = new URL('?q=1', 'https://h.com/a/b');
        assert(a.href === 'https://h.com/x/y');
        assert(b.pathname === '/a/z');
        assert(c.pathname === '/a/b');
        assert(c.search === '?q=1');
        // "." / ".." directory segments keep the trailing slash.
        assert(new URL('.', 'https://h.com/a/b').href === 'https://h.com/a/');
        assert(new URL('..', 'https://h.com/a/b/c').href === 'https://h.com/a/');
        assert(new URL('./d', 'https://h.com/a/b').href === 'https://h.com/a/d');
        "#,
    );
}

#[test]
fn url_to_string_matches_href() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        const u = new URL('https://h.com/p?x#y');
        assert(u.toString() === 'https://h.com/p?x#y');
        "#,
    );
}

#[test]
fn url_invalid_throws_type_error() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let threw = false;
        try {
            new URL('not a url');
        } catch (e) {
            threw = (e instanceof TypeError);
        }
        assert(threw);
        "#,
    );
}
