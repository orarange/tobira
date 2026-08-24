//! Per-tag DOM interface constructors have to exist as globals.
//!
//! Scripts name them for feature detection and `instanceof` guards, and a
//! missing one is a bare `ReferenceError` that takes the whole bundle down.
//! Yahoo! JAPAN died on `HTMLScriptElement is not defined`.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(src: &str) {
    let program = Parser::new(src).parse().expect("script should parse");
    let chunk = Compiler::new(&program).compile().expect("script should compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn the_common_element_interfaces_are_defined() {
    run(r#"
        const names = [
            'HTMLScriptElement', 'HTMLImageElement', 'HTMLLinkElement', 'HTMLStyleElement',
            'HTMLFormElement', 'HTMLDivElement', 'HTMLSpanElement', 'HTMLCanvasElement',
            'HTMLVideoElement', 'HTMLAudioElement', 'HTMLTableElement', 'HTMLTemplateElement',
            'HTMLHeadElement', 'HTMLBodyElement', 'HTMLHtmlElement', 'HTMLMetaElement',
            'HTMLTitleElement', 'HTMLOptionElement', 'HTMLLabelElement', 'HTMLHeadingElement',
            'HTMLInputElement', 'HTMLTextAreaElement', 'HTMLSelectElement', 'HTMLButtonElement',
            'HTMLAnchorElement', 'HTMLIFrameElement',
        ];
        for (const name of names) {
            assert(typeof globalThis[name] === 'function', name + ' should be a constructor');
        }
    "#);
}

/// Each one is a real constructor object with the usual shape, since detection
/// code reads `.prototype` and `.name` off them.
#[test]
fn interface_constructors_have_a_prototype_and_name() {
    run(r#"
        assert(typeof HTMLScriptElement.prototype === 'object');
        assert(HTMLScriptElement.name === 'HTMLScriptElement');
        assert(HTMLScriptElement.prototype.constructor === HTMLScriptElement);
    "#);
}

/// The base interfaces stay available alongside the per-tag ones.
#[test]
fn base_interfaces_are_still_defined() {
    run(r#"
        for (const name of ['EventTarget', 'Node', 'Element', 'HTMLElement', 'Document', 'Text', 'Comment', 'Window']) {
            assert(typeof globalThis[name] === 'function', name);
        }
    "#);
}
