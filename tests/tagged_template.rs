// Regression tests for tagged template literals and String.raw.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn tagged_template_basic() {
    run(r#"
        function tag(strings, ...values) {
            return strings[0] + values[0] + strings[1];
        }
        assert(tag`x${9}y` === 'x9y');
    "#);
}

#[test]
fn tagged_template_strings_and_values() {
    run(r#"
        function tag(strings, ...values) {
            assert(strings.length === 3);
            assert(values.length === 2);
            assert(strings[0] === 'a');
            assert(strings[1] === 'b');
            assert(strings[2] === 'c');
            assert(values[0] === 1);
            assert(values[1] === 2);
            return 'ok';
        }
        assert(tag`a${1}b${2}c` === 'ok');
    "#);
}

#[test]
fn tagged_template_raw_property() {
    run(r#"
        function tag(strings) {
            return strings.raw[0];
        }
        // The cooked value has a real newline; raw keeps the backslash-n.
        assert(tag`line\n` === 'line\\n');
    "#);
}

#[test]
fn string_raw() {
    run(r#"
        assert(String.raw`a\nb` === 'a\\nb');
        assert(String.raw`${1}+${2}` === '1+2');
        assert(String.raw`no subs` === 'no subs');
    "#);
}

#[test]
fn tagged_template_no_substitutions() {
    run(r#"
        function tag(strings, ...values) {
            return strings[0] + ':' + values.length;
        }
        assert(tag`hello` === 'hello:0');
    "#);
}
