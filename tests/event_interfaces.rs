//! Event and structural DOM interface names have to exist as globals.
//!
//! Scripts reference them for `instanceof` and feature detection, and a missing
//! one is a bare `ReferenceError` that stops the whole bundle. Yahoo! JAPAN
//! died on `MessageEvent is not defined`.

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
fn the_event_interfaces_are_defined() {
    run(r#"
        const names = [
            'Event', 'CustomEvent', 'UIEvent', 'FocusEvent', 'InputEvent', 'KeyboardEvent',
            'MouseEvent', 'PointerEvent', 'DragEvent', 'WheelEvent', 'TouchEvent',
            'MessageEvent', 'ErrorEvent', 'ProgressEvent', 'StorageEvent', 'PopStateEvent',
            'HashChangeEvent', 'PageTransitionEvent', 'CloseEvent', 'BeforeUnloadEvent',
            'SubmitEvent', 'AnimationEvent', 'TransitionEvent', 'CompositionEvent', 'ClipboardEvent',
        ];
        for (const name of names) {
            assert(typeof globalThis[name] === 'function', name + ' should be a constructor');
        }
    "#);
}

/// They construct real event objects, not empty stubs, so the usual fields read
/// back and the object can be dispatched.
#[test]
fn constructed_events_carry_their_type() {
    run(r#"
        const e = new MessageEvent('message');
        assert(e.type === 'message');
        assert(typeof e.preventDefault === 'function');

        const p = new PopStateEvent('popstate');
        assert(p.type === 'popstate');

        const c = new CustomEvent('thing', { detail: 42 });
        assert(c.type === 'thing');
    "#);
}

#[test]
fn structural_interfaces_are_defined() {
    run(r#"
        const names = [
            'NodeList', 'HTMLCollection', 'DOMTokenList', 'NamedNodeMap', 'Attr',
            'CSSStyleDeclaration', 'DOMException', 'Navigator', 'History', 'Location',
            'Screen', 'Range', 'Selection', 'ShadowRoot',
        ];
        for (const name of names) {
            assert(typeof globalThis[name] === 'function', name);
            assert(typeof globalThis[name].prototype === 'object', name + '.prototype');
        }
    "#);
}

/// The interface globals are distinct from the live instances of the same name.
#[test]
fn interface_names_do_not_shadow_the_instances() {
    run(r#"
        assert(typeof Location === 'function');
        assert(typeof History === 'function');
        assert(typeof Navigator === 'function');
    "#);
}
