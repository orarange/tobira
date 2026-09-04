//! The backward-jump budget (`fuel`) is per turn, not per page lifetime, and an
//! error that escapes an event-loop job is reported rather than dropped.
//!
//! Before this, `fuel` was set only when a script was executed. Every timer,
//! animation frame and event handler afterwards drew down whatever the last
//! `<script>` had left, so a page that looped hard during load — or an
//! animation loop running long enough — hit `VmError::InfiniteLoop` in a
//! callback. That error is not catchable from JS and the event loop dropped it,
//! so the page simply stopped, silently, with a healthy-looking step count.

use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn compile(source: &str) -> tobira_engine::engine::Chunk {
    let program = Parser::new(source).parse().expect("parse");
    Compiler::new(&program).compile().expect("compile")
}

/// 0 = the callback never got to the end, 1 = it completed.
const CALLBACK: &str = r#"
    globalThis.__flag = 0;
    setTimeout(function () {
        let u = 0;
        for (let i = 0; i < 100000; i++) { u += 1; }
        globalThis.__flag = 1;
    }, 0);
"#;

fn callback_ran_after_burning(iterations: usize) -> bool {
    let src = format!("let t = 0; for (let i = 0; i < {iterations}; i++) {{ t += 1; }}\n{CALLBACK}");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&compile(&src)).expect("main script");
    vm.run_due_jobs(100);
    matches!(
        vm.eval_source("globalThis.__flag"),
        Ok(tobira_engine::engine::Value::Number(flag)) if flag == 1.0
    )
}

#[test]
fn timer_callback_runs_even_after_the_page_burned_the_script_budget() {
    // Well under the budget: this always worked.
    assert!(
        callback_ran_after_burning(1_000),
        "control: callback should run when the main script barely looped"
    );
    // Just under the budget in the main script. The callback's own loop is only
    // 100k iterations, so with a per-turn budget it has room; with the old
    // lifetime budget it died partway through.
    assert!(
        callback_ran_after_burning(950_000),
        "callback should get its own fuel budget, not the script's leftovers"
    );
}

#[test]
fn a_runaway_loop_in_one_callback_does_not_disarm_the_next() {
    // The guard must still fire — refilling per turn must not mean "never".
    let src = r#"
        globalThis.__first = 0;
        globalThis.__second = 0;
        setTimeout(function () {
            // Deliberately past the per-turn budget.
            for (let i = 0; i < 2000000; i++) { globalThis.__first = i; }
        }, 0);
        setTimeout(function () {
            let u = 0;
            for (let i = 0; i < 1000; i++) { u += 1; }
            globalThis.__second = u;
        }, 0);
    "#;
    let mut vm = Vm::new(Heap::new());
    vm.execute(&compile(src)).expect("main script");
    vm.run_due_jobs(100);

    // The runaway one was cut off...
    let errors = vm.take_job_errors();
    assert!(
        errors.iter().any(|e| e.contains("task callback")),
        "the runaway callback's error should be recorded, got {errors:?}"
    );
    // ...and the next callback still ran normally.
    assert!(
        matches!(
            vm.eval_source("globalThis.__second"),
            Ok(tobira_engine::engine::Value::Number(v)) if v == 1000.0
        ),
        "a callback that follows a runaway one should still get a full budget"
    );
}

#[test]
fn an_error_escaping_a_timer_is_recorded_not_dropped() {
    let src = r#"
        setTimeout(function () { throw new Error('boom'); }, 0);
    "#;
    let mut vm = Vm::new(Heap::new());
    vm.execute(&compile(src)).expect("main script");
    let ran = vm.run_due_jobs(100);

    let errors = vm.take_job_errors();
    assert_eq!(errors.len(), 1, "expected one recorded error, got {errors:?}");
    assert!(
        errors[0].contains("boom"),
        "the recorded error should name the throw, got {:?}",
        errors[0]
    );
    // The step count reports jobs that RAN. This one threw, so it does not count.
    assert_eq!(ran, 0, "a callback that threw should not count as work done");
    // Draining is a drain.
    assert!(vm.take_job_errors().is_empty());
}

#[test]
fn a_timer_that_succeeds_still_counts_as_work() {
    let src = "setTimeout(function () { globalThis.__x = 1; }, 0);";
    let mut vm = Vm::new(Heap::new());
    vm.execute(&compile(src)).expect("main script");
    assert_eq!(vm.run_due_jobs(100), 1);
    assert!(vm.take_job_errors().is_empty());
}

/// The reported symptom, at the scale it was reported: an animation loop that
/// spends a modest number of backward jumps per frame used to exhaust the
/// page-lifetime budget after a couple of minutes and stop dead. Three minutes
/// of virtual time at 60fps is 10,800 frames; every one of them must run.
#[test]
fn an_animation_loop_survives_three_minutes_at_60fps() {
    let src = r#"
        globalThis.__frames = 0;
        function frame() {
            // ~200 backward jumps per frame — a cheap animation, not a stress test.
            let acc = 0;
            for (let i = 0; i < 200; i++) { acc += i; }
            globalThis.__frames = globalThis.__frames + 1;
            requestAnimationFrame(frame);
        }
        requestAnimationFrame(frame);
    "#;
    let mut vm = Vm::new(Heap::new());
    vm.execute(&compile(src)).expect("main script");

    const FRAMES: u64 = 60 * 180;
    for frame in 1..=FRAMES {
        vm.pump_event_loop(frame * 1000 / 60, 10_000);
    }

    let errors = vm.take_job_errors();
    assert!(errors.is_empty(), "no frame should have failed: {errors:?}");
    let counted = match vm.eval_source("globalThis.__frames") {
        Ok(tobira_engine::engine::Value::Number(n)) => n,
        other => panic!("could not read the frame counter: {other:?}"),
    };
    assert_eq!(
        counted, FRAMES as f64,
        "every frame of three minutes at 60fps should have run"
    );
}
