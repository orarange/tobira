use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn escape_and_unescape() {
    run(
        r#"
        assert(escape('a b+c') === 'a%20b+c');
        assert(unescape('a%20b') === 'a b');
        assert(unescape(escape('Hello World!')) === 'Hello World!');
    "#,
    );
}

#[test]
fn weak_ref() {
    run(
        r#"
        const o = {x:1};
        const r = new WeakRef(o);
        assert(r.deref() === o);
        assert(r.deref().x === 1);
        let threw = false;
        try { new WeakRef(5); } catch (e) { threw = e instanceof TypeError; }
        assert(threw);
    "#,
    );
}

#[test]
fn text_encoder() {
    run(
        r#"
        const enc = new TextEncoder();
        const bytes = enc.encode('AB');
        assert(bytes.length === 2);
        assert(bytes[0] === 65);
        assert(bytes[1] === 66);
        assert(enc.encode('').length === 0);
        assert(enc.encoding === 'utf-8');
    "#,
    );
}

#[test]
fn text_decoder() {
    run(
        r#"
        const dec = new TextDecoder();
        const u = new Uint8Array([72, 105]);
        assert(dec.decode(u) === 'Hi');
    "#,
    );
}

#[test]
fn round_trip() {
    run(
        r#"
        assert(new TextDecoder().decode(new TextEncoder().encode('Round!')) === 'Round!');
    "#,
    );
}
