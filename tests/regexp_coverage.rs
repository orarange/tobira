// Regression tests for the RegExp engine: test/exec, named groups, and the
// regex-aware String methods (match/matchAll/replace/split/search).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn regex_test() {
    run(r#"
        assert(/\d+/.test('abc123'));
        assert(!/^\d+$/.test('abc'));
        assert(/foo/i.test('FOOBAR'));
    "#);
}

#[test]
fn regex_exec_groups() {
    run(r#"
        const m = /(\d+)-(\d+)/.exec('12-34');
        assert(m[0] === '12-34');
        assert(m[1] === '12');
        assert(m[2] === '34');
        assert(m.index === 0);
        assert(m.input === '12-34');
    "#);
}

#[test]
fn regex_named_groups() {
    run(r#"
        const m = /(?<year>\d{4})-(?<month>\d{2})/.exec('2024-06');
        assert(m.groups.year === '2024');
        assert(m.groups.month === '06');
    "#);
}

#[test]
fn regex_constructor_and_tostring() {
    run(r#"
        const re = new RegExp('\\d+', 'g');
        assert(re.test('a1'));
        assert(re.source === '\\d+');
        assert(re.global === true);
        assert(/ab/gi.toString() === '/ab/gi');
    "#);
}

#[test]
fn regex_exec_global_lastindex() {
    run(r#"
        const re = /\d/g;
        const a = re.exec('a1b2');
        assert(a[0] === '1');
        const b = re.exec('a1b2');
        assert(b[0] === '2');
        const c = re.exec('a1b2');
        assert(c === null);
    "#);
}

#[test]
fn string_match_global_and_non_global() {
    run(r#"
        const all = 'a1b2c3'.match(/\d/g);
        assert(all.length === 3);
        assert(all[0] === '1' && all[2] === '3');
        const one = 'a1b2'.match(/(\d)/);
        assert(one[1] === '1');
        assert('xyz'.match(/\d/) === null);
    "#);
}

#[test]
fn string_match_all() {
    run(r#"
        const matches = [...'a1b2'.matchAll(/(\d)/g)];
        assert(matches.length === 2);
        assert(matches[0][1] === '1');
        assert(matches[1][1] === '2');
    "#);
}

#[test]
fn string_replace_regex() {
    run(r#"
        assert('aaa'.replace(/a/g, 'b') === 'bbb');
        assert('a1b2'.replace(/\d/g, d => '[' + d + ']') === 'a[1]b[2]');
        assert('2024-06'.replace(/(\d+)-(\d+)/, '$2/$1') === '06/2024');
        assert('hello'.replace(/l/, 'L') === 'heLlo');
    "#);
}

#[test]
fn string_replace_named_group() {
    run(r#"
        assert('2024-06'.replace(/(?<y>\d+)-(?<m>\d+)/, '$<m>/$<y>') === '06/2024');
    "#);
}

#[test]
fn string_split_and_search_regex() {
    run(r#"
        assert('a1b2c'.split(/\d/).length === 3);
        assert('a1b2c'.split(/\d/)[1] === 'b');
        assert('abc123'.search(/\d/) === 3);
        assert('abc'.search(/\d/) === -1);
    "#);
}

#[test]
fn string_replace_function_with_offset() {
    run(r#"
        const out = 'abc'.replace(/./g, (m, offset) => offset);
        assert(out === '012');
    "#);
}

#[test]
fn regex_lookahead_lookbehind() {
    run(r#"
        assert('$123'.replace(/\$(?=\d)/, 'USD ') === 'USD 123');
        assert('1.99'.replace(/(?<=\.)\d+/, '00') === '1.00');
        assert(/foo(?=bar)/.test('foobar'));
        assert(!/foo(?=bar)/.test('foobaz'));
        assert(/(?<!a)b/.test('cb'));
        assert(!/(?<!a)b/.test('ab'));
    "#);
}

#[test]
fn regex_backreference() {
    run(r#"
        assert(/(\w)\1/.test('hello'));
        assert(!/(\w)\1/.test('abc'));
        const m = /(\w+) \1/.exec('hi hi');
        assert(m !== null);
        assert(m[1] === 'hi');
    "#);
}

#[test]
fn regex_gtag_url_pattern() {
    run(r#"
        const re = /^(?:([^:\/?#.]+):)?(?:\/\/(?:([^\\/?#]*)@)?([^\\/?#]*?)(?::([0-9]+))?(?=[\\/?#]|$))?([^?#]+)?(?:\?([^#]*))?(?:#([\s\S]*))?$/;
        assert(re.test('https://example.com/path?q=1#h'));
    "#);
}
