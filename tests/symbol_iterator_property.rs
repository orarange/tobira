use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn array_symbol_iterator_is_values_and_drives_protocol() {
    run(r#"
        assert(typeof [][Symbol.iterator] === 'function');
        assert(Array.prototype[Symbol.iterator] === Array.prototype.values);

        var it = [1, 2][Symbol.iterator]();
        var first = it.next();
        assert(first.value === 1 && first.done === false);
        var second = it.next();
        assert(second.value === 2 && second.done === false);
        var final = it.next();
        assert(final.done === true);
    "#);
}

#[test]
fn string_symbol_iterator_reads_unicode_code_points() {
    run(r#"
        assert(typeof ''[Symbol.iterator] === 'function');

        var it = 'ab'[Symbol.iterator]();
        var a = it.next();
        var b = it.next();
        var done = it.next();
        assert(a.value === 'a' && a.done === false);
        assert(b.value === 'b' && b.done === false);
        assert(done.done === true);

        var face = String.fromCodePoint(0x1F600);
        var astral = ('a' + face + 'b')[Symbol.iterator]();
        assert(astral.next().value === 'a');
        var middle = astral.next();
        assert(middle.value === face && middle.done === false);
        assert(astral.next().value === 'b');
        assert(astral.next().done === true);
    "#);
}

#[test]
fn map_and_set_symbol_iterators_alias_entries_and_values() {
    run(r#"
        assert(Map.prototype[Symbol.iterator] === Map.prototype.entries);
        var mapIt = (new Map([['a', 1]]))[Symbol.iterator]();
        var mapFirst = mapIt.next();
        assert(mapFirst.done === false);
        assert(mapFirst.value[0] === 'a' && mapFirst.value[1] === 1);
        assert(mapIt.next().done === true);

        assert(Set.prototype[Symbol.iterator] === Set.prototype.values);
        var setIt = (new Set([3]))[Symbol.iterator]();
        var setFirst = setIt.next();
        assert(setFirst.value === 3 && setFirst.done === false);
        assert(setIt.next().done === true);
    "#);
}

#[test]
fn arguments_and_typed_arrays_expose_symbol_iterator() {
    run(r#"
        (function () {
            assert(typeof arguments[Symbol.iterator] === 'function');
            assert(arguments[Symbol.iterator] === Array.prototype.values);
            assert(Object.getOwnPropertySymbols(arguments).length === 0);
            var it = arguments[Symbol.iterator]();
            assert(it.next().value === 7);
            assert(it.next().done === true);
        })(7);

        assert(Uint8Array.prototype[Symbol.iterator] === Uint8Array.prototype.values);
        var typed = new Uint8Array([4, 5]);
        var typedIt = typed[Symbol.iterator]();
        assert(typedIt.next().value === 4);
        assert(typedIt.next().value === 5);
        assert(typedIt.next().done === true);
    "#);
}

#[test]
fn babel_gate_shape_and_property_visibility() {
    run(r#"
        function hasIterator(o) {
            return typeof Symbol !== 'undefined' && o[Symbol.iterator] != null;
        }

        assert(hasIterator([]));
        assert(hasIterator(''));
        assert(hasIterator(new Map()));
        assert(hasIterator(new Set()));

        assert(Object.keys(Array.prototype).indexOf('Symbol.iterator') === -1);
        var symbols = Object.getOwnPropertySymbols(Array.prototype);
        assert(symbols.length > 0);
        assert(symbols.indexOf(Symbol.iterator) !== -1);

        var desc = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.iterator);
        assert(desc.writable === true);
        assert(desc.enumerable === false);
        assert(desc.configurable === true);
    "#);
}

#[test]
fn iterator_objects_are_iterable_and_next_is_not_enumerable() {
    run(r#"
        var arrayIt = [1, 2].values();
        assert(typeof arrayIt[Symbol.iterator] === 'function');
        assert(arrayIt[Symbol.iterator]() === arrayIt);
        assert(Object.keys(arrayIt).length === 0);
        assert(Object.getOwnPropertyDescriptor(arrayIt, 'next') === undefined);

        var arrayProto = Object.getPrototypeOf(arrayIt);
        var nextDesc = Object.getOwnPropertyDescriptor(arrayProto, 'next');
        assert(typeof nextDesc.value === 'function');
        assert(nextDesc.enumerable === false);

        var iteratorDesc = Object.getOwnPropertyDescriptor(arrayProto, Symbol.iterator);
        assert(typeof iteratorDesc.value === 'function');
        assert(iteratorDesc.enumerable === false);

        var mapIt = new Map([['a', 1]]).entries();
        assert(typeof mapIt[Symbol.iterator] === 'function');
        assert(mapIt[Symbol.iterator]() === mapIt);
        assert(Object.keys(mapIt).length === 0);
        assert(Object.getOwnPropertyDescriptor(mapIt, 'next') === undefined);

        var mapFirst = mapIt.next();
        assert(mapFirst.value[0] === 'a' && mapFirst.value[1] === 1);
        assert(mapIt.next().done === true);
    "#);
}

#[test]
fn transpiled_for_of_helper_accepts_iterator_results() {
    run(r#"
        function _createForOfIteratorHelper(o) {
            var it = typeof Symbol !== 'undefined' && o[Symbol.iterator];
            if (!it) throw new TypeError('Invalid attempt to iterate non-iterable instance');
            var iterator = it.call(o);
            return {
                s: function () {},
                n: function () {
                    var step = iterator.next();
                    return { done: step.done, value: step.value };
                },
                e: function (err) { throw err; },
                f: function () {}
            };
        }

        var values = [];
        var arrHelper = _createForOfIteratorHelper([1, 2].values());
        arrHelper.s();
        for (var step; !(step = arrHelper.n()).done;) {
            values.push(step.value);
        }
        arrHelper.f();
        assert(values.length === 2 && values[0] === 1 && values[1] === 2);

        var entries = [];
        var mapHelper = _createForOfIteratorHelper(new Map([['a', 1]]).entries());
        mapHelper.s();
        for (var mapStep; !(mapStep = mapHelper.n()).done;) {
            entries.push(mapStep.value);
        }
        mapHelper.f();
        assert(entries.length === 1);
        assert(entries[0][0] === 'a' && entries[0][1] === 1);
    "#);
}
