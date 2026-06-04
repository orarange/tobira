// Regression tests for the Event / CustomEvent constructors (the pure
// JS-object behavior; addEventListener + dispatchEvent end-to-end is covered by
// the engine_host bin tests, which have a DOM to dispatch on).

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn event_default_properties() {
    run(r#"
        const e = new Event('click');
        assert(e.type === 'click');
        assert(e.bubbles === false);
        assert(e.cancelable === false);
        assert(e.composed === false);
        assert(e.defaultPrevented === false);
        assert(e.target === null);
    "#);
}

#[test]
fn event_init_options() {
    run(r#"
        const e = new Event('submit', { bubbles: true, cancelable: true, composed: true });
        assert(e.bubbles === true);
        assert(e.cancelable === true);
        assert(e.composed === true);
    "#);
}

#[test]
fn prevent_default_sets_flag_when_cancelable() {
    run(r#"
        const e = new Event('x', { cancelable: true });
        assert(e.defaultPrevented === false);
        e.preventDefault();
        assert(e.defaultPrevented === true);
    "#);
}

#[test]
fn prevent_default_is_noop_when_not_cancelable() {
    run(r#"
        const e = new Event('x');
        e.preventDefault();
        assert(e.defaultPrevented === false);
    "#);
}

#[test]
fn event_methods_present_and_stop_propagation() {
    run(r#"
        const e = new Event('x');
        assert(typeof e.preventDefault === 'function');
        assert(typeof e.stopPropagation === 'function');
        assert(typeof e.stopImmediatePropagation === 'function');
        assert(e.cancelBubble === false);
        e.stopPropagation();
        assert(e.cancelBubble === true);
    "#);
}

#[test]
fn custom_event_carries_detail() {
    run(r#"
        const e = new CustomEvent('ping', { detail: { count: 5, label: 'hi' } });
        assert(e.type === 'ping');
        assert(e.detail.count === 5);
        assert(e.detail.label === 'hi');
    "#);
}

#[test]
fn custom_event_without_detail_is_null() {
    run(r#"
        const e = new CustomEvent('ping');
        assert(e.detail === null);
    "#);
}
