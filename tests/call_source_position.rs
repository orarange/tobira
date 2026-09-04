use tobira_engine::engine::{Compiler, Heap, Parser, Value, Vm, VmError};

fn execute(source: &str) -> (Vm, Result<Value, VmError>) {
    let program = Parser::new(source).parse().expect("parse");
    let chunk = Compiler::new(&program).compile().expect("compile");
    let mut vm = Vm::new(Heap::new());
    let result = vm.execute(&chunk);
    (vm, result)
}

fn backtrace_for_error(source: &str) -> String {
    let (mut vm, result) = execute(source);
    result.expect_err("script should throw");
    vm.take_last_backtrace()
        .expect("backtrace should be captured")
}

fn frame_position(backtrace: &str, frame: &str) -> (u32, u32) {
    let line = backtrace
        .lines()
        .find(|line| line.contains(frame))
        .unwrap_or_else(|| panic!("missing frame {frame} in {backtrace}"));
    let open = line
        .rfind('(')
        .unwrap_or_else(|| panic!("missing position in {line}"));
    let close = line
        .rfind(')')
        .unwrap_or_else(|| panic!("missing position in {line}"));
    let (line_number, column_number) = line[open + 1..close]
        .split_once(':')
        .unwrap_or_else(|| panic!("bad position in {line}"));
    (
        line_number.parse().expect("line number"),
        column_number.parse().expect("column number"),
    )
}

#[test]
fn top_level_non_callable_call_has_source_position() {
    let backtrace = backtrace_for_error("var x = [];\nx();");
    let position = frame_position(&backtrace, "at <script>");
    assert_eq!(position.0, 2, "{backtrace}");
    assert!(position.1 > 0, "{backtrace}");
}

#[test]
fn named_function_non_callable_call_has_source_position() {
    let backtrace = backtrace_for_error("function boom() { var x = []; x(); }\nboom();");
    let position = frame_position(&backtrace, "at boom");
    assert_eq!(position.0, 1, "{backtrace}");
    assert!(position.1 > 0, "{backtrace}");
}

#[test]
fn new_non_constructor_has_source_position() {
    let backtrace = backtrace_for_error("var x = {}; new x();");
    let position = frame_position(&backtrace, "at <script>");
    assert_eq!(position.0, 1, "{backtrace}");
    assert!(position.1 > 0, "{backtrace}");
}

#[test]
fn same_line_calls_report_different_columns() {
    let first = backtrace_for_error("var x=[];x();");
    let second = backtrace_for_error("var x=[];          x();");
    let first_position = frame_position(&first, "at <script>");
    let second_position = frame_position(&second, "at <script>");
    assert_eq!(first_position.0, 1, "{first}");
    assert_eq!(second_position.0, 1, "{second}");
    assert_ne!(first_position.1, second_position.1, "{first}\n{second}");
}

#[test]
fn backtrace_without_call_site_position_still_works() {
    let backtrace = backtrace_for_error("null.x;");
    assert!(backtrace.contains("    at <script>"), "{backtrace}");
    assert!(!backtrace.contains("    at <script> ("), "{backtrace}");
}

#[test]
fn normal_program_with_many_calls_still_runs() {
    let (_, result) = execute(
        r#"
        function add(a, b) { return a + b; }
        function twice(value) { return add(value, value); }
        assert(twice(3) === 6);
        assert([1, 2, 3].map(x => twice(x))[2] === 6);
        "#,
    );
    result.expect("script should execute");
}
