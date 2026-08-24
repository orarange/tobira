//! When a call fails because the callee is not a function, the message names
//! the receiver too. On a minified bundle the property name alone ("catch is
//! not a function") says nothing about which object came up short, and finding
//! out was most of the work.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn error_message(src: &str) -> String {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    match vm.execute(&chunk) {
        Ok(_) => panic!("script was expected to fail"),
        Err(error) => format!("{error:?}"),
    }
}

#[test]
fn names_the_receiver_and_its_own_keys() {
    let message = error_message("const o = { a: 1, b: 2 }; o.missing();");
    assert!(message.contains("missing is not a function"), "{message}");
    assert!(message.contains("Object"), "{message}");
    assert!(message.contains('a') && message.contains('b'), "own keys missing from: {message}");
}

/// The receiver's tag distinguishes a real Promise from a look-alike, which is
/// exactly the question that arises when `.catch` is missing.
#[test]
fn a_promise_receiver_is_named_as_one() {
    let message = error_message("Promise.resolve(1).notAMethod();");
    assert!(message.contains("notAMethod is not a function"), "{message}");
    assert!(message.contains("Promise"), "{message}");

    let message = error_message("({}).catch();");
    assert!(message.contains("catch is not a function"), "{message}");
    assert!(message.contains("Object"), "{message}");
    assert!(!message.contains("Promise"), "a plain object must not read as a Promise: {message}");
}

#[test]
fn indexed_calls_are_described_too() {
    let message = error_message("const o = { x: 1 }; o['nope']();");
    assert!(message.contains("nope is not a function"), "{message}");
    assert!(message.contains("Object"), "{message}");
}

/// A bare call to a non-function still reports the value's type; there is no
/// receiver to name.
#[test]
fn a_bare_call_still_reports_the_value_type() {
    let message = error_message("const f = undefined; f();");
    assert!(message.contains("undefined"), "{message}");
}
