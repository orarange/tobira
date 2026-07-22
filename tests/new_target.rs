// Regression tests for new.target.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn new_target_defined_under_new() {
    run(r#"
        function F() { return new.target !== undefined; }
        assert(new F() instanceof F);
        assert(F() === false);
    "#);
}

#[test]
fn new_target_is_the_constructor() {
    run(r#"
        let captured;
        function F() { captured = new.target; }
        new F();
        assert(captured === F);
    "#);
}

#[test]
fn new_target_guard_pattern() {
    run(r#"
        function MustUseNew() {
            if (!new.target) throw new Error('use new');
            this.ok = true;
        }
        assert(new MustUseNew().ok === true);
        let threw = false;
        try { MustUseNew(); } catch (e) { threw = true; }
        assert(threw === true);
    "#);
}

#[test]
fn new_target_in_class_constructor() {
    run(r#"
        class Base {
            constructor() { this.created = new.target === Base; }
        }
        assert(new Base().created === true);
    "#);
}

/// Known limitation: a native `super()` call is lowered to `Opcode::Call`, not
/// through the construct path, so `new.target` inside the base constructor is
/// `undefined` instead of the derived constructor. Transpiled subclasses go via
/// `Reflect.construct` and are unaffected (see the `_createSuper` test below).
#[test]
fn native_super_does_not_yet_propagate_new_target() {
    run(r#"
        var seen = 'unset';
        class A { constructor() { seen = new.target; } }
        class B extends A {}
        new B();
        assert(seen === undefined);
    "#);
}

#[test]
fn reflect_construct_uses_new_target_prototype() {
    run(r#"
        function Base() {}
        function Derived() {}
        Derived.prototype = Object.create(Base.prototype, {
            constructor: { value: Derived, writable: true, configurable: true }
        });

        const instance = Reflect.construct(Base, [], Derived);
        assert(Object.getPrototypeOf(instance) === Derived.prototype);
    "#);
}

#[test]
fn reflect_construct_sets_new_target_inside_constructor() {
    run(r#"
        var seen;
        function Base() { seen = new.target; }
        function Derived() {}

        Reflect.construct(Base, [], Derived);
        assert(seen === Derived);
    "#);
}

#[test]
fn ordinary_new_sets_new_target_to_constructor() {
    run(r#"
        var seen;
        function Base() { seen = new.target; }

        new Base();
        assert(seen === Base);
    "#);
}

#[test]
fn plain_call_leaves_new_target_undefined() {
    run(r#"
        var seen = 1;
        function Base() { seen = new.target; }

        Base();
        assert(seen === undefined);
    "#);
}

#[test]
fn reflect_construct_without_new_target_uses_target_prototype() {
    run(r#"
        function Base() {}

        const instance = Reflect.construct(Base, []);
        assert(Object.getPrototypeOf(instance) === Base.prototype);
    "#);
}

#[test]
fn reflect_construct_rejects_non_constructor_new_target() {
    run(r#"
        function Base() {}

        var numberThrew = false;
        try {
            Reflect.construct(Base, [], 42);
        } catch (e) {
            numberThrew = e instanceof TypeError;
        }

        var objectThrew = false;
        try {
            Reflect.construct(Base, [], {});
        } catch (e) {
            objectThrew = e instanceof TypeError;
        }

        assert(numberThrew);
        assert(objectThrew);
    "#);
}

#[test]
fn reflect_construct_preserves_arguments() {
    run(r#"
        function Base(a, b, c) {
            this.total = a + b + c;
        }

        const instance = Reflect.construct(Base, [2, 3, 5]);
        assert(instance.total === 10);
    "#);
}

#[test]
fn babel_create_super_shape_uses_subclass_instance_prototype() {
    run(r#"
        function check(condition, message) {
            if (!condition) {
                throw message;
            }
        }

        function _getPrototypeOf(o) {
            return Object.getPrototypeOf(o);
        }

        function _setPrototypeOf(o, p) {
            Object.setPrototypeOf(o, p);
            return o;
        }

        function _inherits(subClass, superClass) {
            subClass.prototype = Object.create(superClass && superClass.prototype, {
                constructor: { value: subClass, writable: true, configurable: true }
            });
            _setPrototypeOf(subClass, superClass);
        }

        function _possibleConstructorReturn(self, call) {
            if (call && (typeof call === "object" || typeof call === "function")) {
                return call;
            }
            return self;
        }

        function _createSuper(Derived) {
            return function () {
                var Super = _getPrototypeOf(Derived), result;
                var NewTarget = _getPrototypeOf(this).constructor;
                result = Reflect.construct(Super, arguments, NewTarget);
                return _possibleConstructorReturn(this, result);
            };
        }

        function Base(name) {
            this.name = name;
        }
        Base.prototype.describe = function () {
            return "base:" + this.name;
        };

        var _super;
        function Derived(name) {
            return _possibleConstructorReturn(this, _super.call(this, name));
        }
        _inherits(Derived, Base);
        _super = _createSuper(Derived);
        Derived.prototype.child = function () {
            return "child:" + this.name;
        };

        var instance = new Derived("ok");
        check(Object.getPrototypeOf(instance) === Derived.prototype, "derived prototype");
        check(instance instanceof Base, "base chain");
        check(instance.child() === "child:ok", "child method");
        check(instance.describe() === "base:ok", "base method");
    "#);
}
