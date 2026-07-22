use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn dom_interface_globals_are_functions() {
    run(r#"
        assert(typeof HTMLElement === "function");
        assert(typeof EventTarget === "function");
        assert(typeof Node === "function");
        assert(typeof Element === "function");
        assert(typeof Document === "function");
    "#);
}

#[test]
fn dom_interface_constructor_metadata_matches_browsers() {
    run(r#"
        assert(HTMLElement.name === "HTMLElement");
        assert(HTMLElement.prototype.constructor === HTMLElement);

        const prototype = Object.getOwnPropertyDescriptor(HTMLElement, "prototype");
        assert(prototype.writable === false);
        assert(prototype.enumerable === false);
        assert(prototype.configurable === false);

        const name = Object.getOwnPropertyDescriptor(HTMLElement, "name");
        assert(name.value === "HTMLElement");
        assert(name.writable === false);
        assert(name.enumerable === false);
        assert(name.configurable === true);

        const constructor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "constructor");
        assert(constructor.value === HTMLElement);
        assert(constructor.writable === true);
        assert(constructor.enumerable === false);
        assert(constructor.configurable === true);
    "#);
}

#[test]
fn dom_interface_constructor_plain_call_is_illegal() {
    run(r#"
        let threw = false;
        try {
            HTMLElement();
        } catch (e) {
            threw = e instanceof TypeError && String(e.message).indexOf("Illegal constructor") !== -1;
        }
        assert(threw);
    "#);
}

#[test]
fn subclass_super_call_still_constructs() {
    run(r#"
        class X extends HTMLElement {
            constructor() {
                super();
                this.ready = true;
            }
        }
        const x = new X();
        assert(x.ready === true);
        assert(x instanceof X);
    "#);
}

#[test]
fn js_constructed_dom_interface_instances_use_prototype_instanceof() {
    run(r#"
        class X extends HTMLElement {}
        const x = new X();
        assert(x instanceof HTMLElement);
        assert(new HTMLElement() instanceof HTMLElement);
    "#);
}

#[test]
fn non_dom_objects_are_not_dom_interface_instances() {
    run(r#"
        assert(!({} instanceof HTMLElement));
        class Plain {}
        assert(!(new Plain() instanceof HTMLElement));
    "#);
}

#[test]
fn dom_interface_prototype_can_be_patched() {
    run(r#"
        HTMLElement.prototype.foo = 1;
        assert(HTMLElement.prototype.foo === 1);
    "#);
}

#[test]
fn host_document_instanceof_dom_interfaces_still_works() {
    run(r#"
        assert(document instanceof Document);
        assert(document instanceof Node);
        assert(document instanceof EventTarget);
    "#);
}
