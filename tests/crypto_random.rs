use tobira_engine::engine::{Compiler, Heap, Parser, Vm};

fn run(source: &str) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    vm.execute(&chunk).expect("execute");
}

#[test]
fn typeof_window_global_reports_real_type_not_undefined() {
    // Regression: `typeof crypto` (and other window-globals) must report the
    // real type, not "undefined" — the uuid library guards with
    // `typeof crypto !== 'undefined' && crypto.getRandomValues`, which used to
    // fail because the typeof path skipped the window-global fallback.
    run(r#"
        assert(typeof crypto === 'object');
        assert(typeof navigator === 'object');
        assert(typeof localStorage === 'object');
        // The uuid feature-detection idiom must now succeed.
        const detected = typeof crypto !== 'undefined' && crypto.getRandomValues;
        assert(typeof detected === 'function');
        // A genuinely undeclared name must still be "undefined" (no throw).
        assert(typeof __definitely_not_declared_xyz__ === 'undefined');
    "#);
}

#[test]
fn get_random_values_writes_integer_typed_array() {
    run(
        r#"
        const a = new Uint8Array(16);
        const r = crypto.getRandomValues(a);
        assert(r === a);
        assert(a.length === 16);
        let allInRange = true;
        for (let i = 0; i < a.length; i++) {
            if (a[i] < 0 || a[i] > 255 || (a[i] | 0) !== a[i]) allInRange = false;
        }
        assert(allInRange);
    "#,
    );
}

#[test]
fn random_uuid_has_version_and_variant_shape() {
    run(
        r#"
        const u = crypto.randomUUID();
        assert(typeof u === 'string');
        assert(u.length === 36);
        assert(u[14] === '4');
        assert(u[8] === '-' && u[13] === '-' && u[18] === '-' && u[23] === '-');
        const u2 = crypto.randomUUID();
        assert(u !== u2);
    "#,
    );
}

#[test]
fn get_random_values_rejects_non_typed_array() {
    run(
        r#"
        let threw = false;
        try {
            crypto.getRandomValues({});
        } catch (e) {
            threw = (e instanceof TypeError);
        }
        assert(threw);
    "#,
    );
}
