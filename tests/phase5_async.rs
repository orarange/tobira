use tobira_engine::engine::{Compiler, Heap, Parser, TickResult, Vm};

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

// Browser expected: Promise.resolve queues its .then callback in the same microtask checkpoint.
#[test]
fn promise_resolve_then() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let result = 0;
        Promise.resolve(41).then(value => {
            result = value + 1;
        });
        "#,
    );
    execute_script(&mut vm, "assert(result === 42);");
}

// Browser expected: rejected promises dispatch catch handlers in a microtask and preserve the reason value.
#[test]
fn promise_reject_catch() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let message = "";
        Promise.reject("boom").catch(reason => {
            message = reason;
        });
        "#,
    );
    execute_script(&mut vm, "assert(message === 'boom');");
}

// Browser expected: Promise chains pass return values through each successive microtask reaction.
#[test]
fn promise_chain_values() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let result = 0;
        Promise.resolve(2)
            .then(value => value * 3)
            .then(value => {
                result = value + 1;
            });
        "#,
    );
    execute_script(&mut vm, "assert(result === 7);");
}

// Browser expected: Promise.all fulfills once every input is fulfilled, preserving input order.
#[test]
fn promise_all_basic() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let result = "";
        Promise.all([Promise.resolve("a"), 2, Promise.resolve("c")]).then(values => {
            result = values.join("");
        });
        "#,
    );
    execute_script(&mut vm, "assert(result === 'a2c');");
}

// Browser expected: Promise.race settles from the first settled input even if later entries are pending timers.
#[test]
fn promise_race_basic() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let result = "";
        Promise.race([
            Promise.resolve("fast"),
            new Promise(resolve => setTimeout(() => resolve("slow"), 0))
        ]).then(value => {
            result = value;
        });
        "#,
    );
    execute_script(&mut vm, "assert(result === 'fast');");
}

// Browser expected: Promise.allSettled always fulfills and reports per-entry status/value pairs.
#[test]
fn promise_allsettled() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let summary = "";
        Promise.allSettled([Promise.resolve(1), Promise.reject("no")]).then(results => {
            summary =
                results[0].status + ":" + results[0].value + "," +
                results[1].status + ":" + results[1].reason;
        });
        "#,
    );
    execute_script(&mut vm, "assert(summary === 'fulfilled:1,rejected:no');");
}

// Browser expected: Promise.any fulfills from the first fulfillment and ignores earlier rejections.
#[test]
fn promise_any_basic() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let result = "";
        Promise.any([
            Promise.reject("bad"),
            Promise.resolve("ok")
        ]).then(value => {
            result = value;
        });
        "#,
    );
    execute_script(&mut vm, "assert(result === 'ok');");
}

// Browser expected: microtasks run before timer macrotasks scheduled for the same turn.
#[test]
fn microtask_before_timeout() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let log = "";
        Promise.resolve().then(() => {
            log += "micro";
        });
        setTimeout(() => {
            log += " timer";
        }, 0);
        "#,
    );
    execute_script(&mut vm, "assert(log === 'micro');");
    assert_eq!(vm.event_loop_tick(0, false), TickResult::DidWork);
    execute_script(&mut vm, "assert(log === 'micro timer');");
}

// Browser expected: queueMicrotask preserves FIFO ordering within the same checkpoint.
#[test]
fn queuemicrotask_ordering() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let log = "";
        queueMicrotask(() => { log += "a"; });
        queueMicrotask(() => { log += "b"; });
        assert(log === "");
        "#,
    );
    execute_script(&mut vm, "assert(log === 'ab');");
}

// Browser expected: promise reactions flush before setTimeout(0) runs on the next tick.
#[test]
fn promise_then_before_settimeout() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let log = "";
        setTimeout(() => {
            log += "timeout";
        }, 0);
        Promise.resolve().then(() => {
            log += "promise";
        });
        "#,
    );
    execute_script(&mut vm, "assert(log === 'promise');");
    assert_eq!(vm.event_loop_tick(0, false), TickResult::DidWork);
    execute_script(&mut vm, "assert(log === 'promisetimeout');");
}

// Browser expected: async functions return Promise instances immediately and settle later.
#[test]
fn async_fn_returns_promise() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let saw_then = false;
        let resolved = 0;
        async function value() { return 42; }
        const promise = value();
        saw_then = typeof promise.then === "function";
        promise.then(v => {
            resolved = v;
        });
        "#,
    );
    execute_script(
        &mut vm,
        "assert(saw_then === true); assert(resolved === 42);",
    );
}

// Browser expected: awaiting an already-fulfilled promise resumes the async function in a later microtask.
#[test]
fn await_resolved_promise() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let result = 0;
        async function run() {
            result = await Promise.resolve(7);
        }
        run();
        "#,
    );
    execute_script(&mut vm, "assert(result === 7);");
}

// Browser expected: await wraps non-promise values with Promise.resolve before resuming.
#[test]
fn await_value_wrapping() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let result = 0;
        async function run() {
            result = await 9;
        }
        run();
        "#,
    );
    execute_script(&mut vm, "assert(result === 9);");
}

// Browser expected: uncaught exceptions inside async functions reject the returned promise.
#[test]
fn async_exception_rejects() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let message = "";
        async function fail() {
            throw new Error("boom");
        }
        fail().catch(error => {
            message = error.message;
        });
        "#,
    );
    execute_script(&mut vm, "assert(message === 'boom');");
}

// Browser expected: explicit async returns resolve the outer promise with the returned value.
#[test]
fn async_return_value() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let result = 0;
        async function run() {
            return 5;
        }
        run().then(value => {
            result = value;
        });
        "#,
    );
    execute_script(&mut vm, "assert(result === 5);");
}

// Browser expected: nested promise chains wait on returned promises, so inner callbacks run before outer continuation.
#[test]
fn nested_promise_chain_ordering() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let log = [];
        Promise.resolve()
            .then(() => Promise.resolve().then(() => log.push("b")))
            .then(() => log.push("c"));
        "#,
    );
    execute_script(&mut vm, "assert(log.join(',') === 'b,c');");
}

// Browser expected: async await resumes as a promise microtask and keeps FIFO ordering with explicit .then callbacks.
#[test]
fn async_await_vs_then_ordering() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let log = [];
        async function run() {
            log.push("start");
            await 0;
            log.push("after await");
        }
        run();
        Promise.resolve().then(() => log.push("then"));
        "#,
    );
    execute_script(
        &mut vm,
        "assert(log.join(',') === 'start,after await,then');",
    );
}

// Browser expected: multiple awaits in one async function resume sequentially, preserving source order.
#[test]
fn multiple_awaits_sequential() {
    let mut vm = new_vm();
    execute_script(
        &mut vm,
        r#"
        let log = [];
        async function run() {
            log.push(await Promise.resolve("a"));
            log.push(await Promise.resolve("b"));
            log.push("c");
        }
        run();
        "#,
    );
    execute_script(&mut vm, "assert(log.join('') === 'abc');");
}
