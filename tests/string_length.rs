//! `.length` on a string.
//!
//! It used to walk the whole string on every read. A string never changes once
//! it is made, so the count is taken once when it is built; these keep the two
//! from drifting apart, and pin the answer for the shapes that are easy to get
//! wrong when a count is cached (empty, multi-byte, built by concatenation,
//! built by `repeat`, sliced out of another string).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program)
        .compile()
        .expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn a_strings_length_is_its_character_count_however_it_was_made() {
    run(r#"
        assert(''.length === 0, 'empty');
        assert('abc'.length === 3, 'ascii');
        assert('あいう'.length === 3, 'three kana are three characters');
        assert(('ab' + 'cde').length === 5, 'joined');
        assert('xy'.repeat(4).length === 8, 'repeated');
        assert('abcdef'.slice(1, 4).length === 3, 'sliced');
        assert('あいうえお'.slice(1, 3).length === 2, 'sliced multi-byte');
        assert(String(12345).length === 5, 'from a number');
        assert('  pad  '.trim().length === 3, 'trimmed');
        assert('a,b,c'.split(',')[1].length === 1, 'split out');
        assert('ABC'.toLowerCase().length === 3, 'recased');
        // The count has to survive whatever the string is used for.
        const s = 'あいう';
        assert(s.length === 3 && s.length === 3, 'read twice');
        assert(s.charAt(1) === 'い', 'and indexing still lines up with it');
        assert(s[s.length - 1] === 'う', 'last character');
    "#);
}

/// A long string answers as quickly as a short one. This does not time
/// anything -- it pins the shape that makes that true: the string is read
/// many times and the answer never changes.
#[test]
fn a_long_string_answers_the_same_every_time() {
    run(r#"
        const s = 'x'.repeat(50000) + 'あ'.repeat(50000);
        let n = 0;
        for (let i = 0; i < 100; i++) n += s.length;
        assert(n === 100 * 100000, 'read a hundred times: ' + n);
    "#);
}
