use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn new_vm() -> Vm {
    Vm::new(Heap::new())
}

fn execute_script(vm: &mut Vm, source: &str) {
    let program = Parser::new(source).parse().expect("script should parse");
    let chunk = Compiler::new(&program)
        .compile()
        .expect("script should compile");
    vm.execute(&chunk).expect("script should execute");
}

#[test]
fn async_generator_for_await_basic() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let sum = 0;
        let out = "";
        async function* g() { yield 1; yield 2; }
        (async () => {
            for await (const x of g()) {
                sum += x;
                out += x;
            }
        })();
        "#,
    );
    execute_script(&mut vm, "assert(sum === 3); assert(out === '12');");
}

#[test]
fn async_generator_yield_promise_unwraps() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let value = 0;
        async function* g() { yield Promise.resolve(42); }
        (async () => {
            for await (const x of g()) {
                value = x;
            }
        })();
        "#,
    );
    execute_script(&mut vm, "assert(value === 42);");
}

#[test]
fn async_generator_mix_await_and_yield() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let out = "";
        async function* g() {
            const a = await Promise.resolve(1);
            yield a;
            await null;
            yield 2;
        }
        (async () => {
            for await (const x of g()) {
                out += x;
            }
        })();
        "#,
    );
    execute_script(&mut vm, "assert(out === '12');");
}

#[test]
fn async_generator_next_returns_promise() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let saw_then = false;
        let first = "";
        let done = "";
        async function* g() { yield 1; }
        const it = g();
        const p = it.next();
        saw_then = typeof p.then === "function";
        p.then(result => {
            first = result.value + ":" + result.done;
        });
        it.next().then(result => {
            done = result.value + ":" + result.done;
        });
        "#,
    );
    execute_script(
        &mut vm,
        "assert(saw_then === true); assert(first === '1:false'); assert(done === 'undefined:true');",
    );
}

#[test]
fn async_generator_return_settles() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let returned = "";
        let next_result = "";
        async function* g() { yield 1; yield 2; }
        const it = g();
        it.next();
        const ret = it.return(99);
        returned = typeof ret.then === "function";
        ret.then(result => {
            returned = result.value + ":" + result.done;
            return it.next();
        }).then(result => {
            next_result = result.value + ":" + result.done;
        });
        "#,
    );
    execute_script(
        &mut vm,
        "assert(returned === '99:true'); assert(next_result === 'undefined:true');",
    );
}

#[test]
fn async_generator_completed_next_resolves() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let first = "";
        let second = "";
        let third = "";
        async function* g() { yield 1; }
        const it = g();
        it.next().then(result => {
            first = result.value + ":" + result.done;
        });
        it.next().then(result => {
            second = result.value + ":" + result.done;
        });
        it.next().then(result => {
            third = result.value + ":" + result.done;
        });
        "#,
    );
    execute_script(
        &mut vm,
        "assert(first === '1:false'); assert(second === 'undefined:true'); assert(third === 'undefined:true');",
    );
}

#[test]
fn async_generator_completed_queue_drains() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let first = "";
        let second = "";
        let third = "";
        async function* g() {
            await null;
            yield 1;
        }
        const it = g();
        const p1 = it.next();
        const p2 = it.next();
        const p3 = it.next();
        p1.then(result => {
            first = result.value + ":" + result.done;
        });
        p2.then(result => {
            second = result.value + ":" + result.done;
        });
        p3.then(result => {
            third = result.value + ":" + result.done;
        });
        "#,
    );
    execute_script(
        &mut vm,
        "assert(first === '1:false'); assert(second === 'undefined:true'); assert(third === 'undefined:true');",
    );
}

#[test]
fn async_generator_symbol_async_iterator_identity() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        async function* g() { yield 1; }
        const it = g();
        assert(it[Symbol.asyncIterator]() === it);
        "#,
    );
}

#[test]
fn async_generator_for_await_sync_iterables() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let sum = 0;
        (async () => {
            for await (const x of [1, Promise.resolve(2), 3]) {
                sum += x;
            }
        })();
        "#,
    );
    execute_script(&mut vm, "assert(sum === 6);");
}

#[test]
fn async_generator_queueing_next_calls() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let first = "";
        let second = "";
        async function* g() {
            yield await Promise.resolve(1);
            yield 2;
        }
        const it = g();
        const p1 = it.next();
        const p2 = it.next();
        p1.then(result => { first = result.value + ":" + result.done; });
        p2.then(result => { second = result.value + ":" + result.done; });
        "#,
    );
    execute_script(&mut vm, "assert(first === '1:false'); assert(second === '2:false');");
}

#[test]
fn async_generator_throw_rejects_next() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let reason = "";
        async function* g() {
            throw 'boom';
        }
        g().next().catch(err => {
            reason = err;
        });
        "#,
    );
    execute_script(&mut vm, "assert(reason === 'boom');");
}
