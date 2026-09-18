//! A subset of tc39/test262, run against the VM alone (no DOM).
//!
//! The fixtures are vendored under `tests/fixtures/test262/`: the `harness/`
//! directory and a few directories of `test/built-ins/`. This is to the
//! engine what html5lib-tests is to the parser: a score that may only go up.
//!
//! ```text
//! cargo test --release --bin tobira test262 -- --nocapture
//! TOBIRA_T262_DIR=built-ins/Math      every failure under one directory
//! TOBIRA_T262_OUT=<path>              every failure, one per line, to a file
//! TOBIRA_T262_BLESS=1                 rewrite baseline.txt from this run
//! ```
//!
//! Three outcomes, not two. **fail** is something implemented that answers
//! wrongly: a bug. **absent** is a failure in a test whose `features:` name
//! something in [`ABSENT`], left out on purpose and with a reason: a decision,
//! not a bug. Counting the two as one number would make every right decision
//! not to half-implement a feature pull the score down, and a score gets
//! pushed up.
//!
//! The gate is a set, not a rate: `baseline.txt` lists every test that
//! passes, and one that stops passing fails the run. Fixing five and breaking
//! five leaves a rate where it was. Newly passing tests are printed; bless
//! them and the diff of `baseline.txt` is the review of what moved.

use super::vm::{Vm, VmError};
use super::{Compiler, Heap, Parser};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Features left out on purpose. A feature not listed here that a test needs
/// and does not get is a **fail**, so that every absence is one somebody
/// decided on. See HANDOFF.md, 設計判断.
const ABSENT: &[(&str, &str)] = &[
    (
        "Symbol.species",
        "half of it makes core-js replace map / filter / slice; all or nothing",
    ),
    ("Symbol.replace", "same: the RegExp protocol comes in whole or not at all"),
    ("Symbol.split", "same"),
    ("Symbol.search", "same"),
    ("Symbol.matchAll", "same"),
    ("Symbol.isConcatSpreadable", "same, for concat"),
    ("Symbol.unscopables", "`with` is not something pages reach"),
    ("cross-realm", "one realm per VM; $262.createRealm has no meaning here"),
];

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test262");

/// What the YAML front matter of a test says, as far as the runner cares.
#[derive(Default, Debug)]
struct Meta {
    includes: Vec<String>,
    flags: Vec<String>,
    features: Vec<String>,
    /// `negative:` is present: the test must fail (at parse or at run time).
    negative: bool,
}

/// `key: [a, b]` on one line, or `key:` followed by `  - a` lines.
fn yaml_list(front: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = front.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix(key).and_then(|r| r.strip_prefix(':')) else {
            continue;
        };
        let rest = rest.trim();
        if let Some(inner) = rest.strip_prefix('[') {
            let inner = inner.trim_end_matches(']');
            out.extend(
                inner
                    .split(',')
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty()),
            );
        } else {
            for item in lines.by_ref() {
                match item.trim().strip_prefix("- ") {
                    Some(name) => out.push(name.trim().to_string()),
                    None => break,
                }
            }
        }
        break;
    }
    out
}

fn parse_meta(source: &str) -> Meta {
    let Some(start) = source.find("/*---") else {
        return Meta::default();
    };
    let Some(len) = source[start..].find("---*/") else {
        return Meta::default();
    };
    let front = &source[start + 5..start + len];
    Meta {
        includes: yaml_list(front, "includes"),
        flags: yaml_list(front, "flags"),
        features: yaml_list(front, "features"),
        negative: front.lines().any(|line| line.starts_with("negative:")),
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "js")
            && !path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("_FIXTURE"))
        {
            out.push(path);
        }
    }
}

enum Outcome {
    Pass,
    Fail(String),
    /// Failed, and needs a feature from [`ABSENT`].
    Absent(&'static str),
    /// Module and async tests: counted, not run yet.
    Skipped,
}

fn run_source(source: &str) -> Result<(), String> {
    let program = Parser::new(source)
        .parse()
        .map_err(|error| format!("parse: {error:?}"))?;
    let chunk = Compiler::new(&program)
        .compile()
        .map_err(|error| format!("compile: {error:?}"))?;
    let mut vm = Vm::new(Heap::new());
    match vm.execute(&chunk) {
        Ok(_) => {
            vm.drain_microtasks();
            Ok(())
        }
        Err(VmError::Thrown(value)) => Err(vm.describe_thrown_value(&value)),
        Err(error) => Err(error.to_string()),
    }
}

fn run_one(path: &Path, harness: &BTreeMap<String, String>) -> Outcome {
    let Ok(test) = std::fs::read_to_string(path) else {
        return Outcome::Fail("unreadable".to_string());
    };
    let meta = parse_meta(&test);
    if meta.flags.iter().any(|f| f == "module" || f == "async") {
        return Outcome::Skipped;
    }
    let mut source = String::new();
    if meta.flags.iter().any(|f| f == "onlyStrict") {
        source.push_str("'use strict';\n");
    }
    if !meta.flags.iter().any(|f| f == "raw") {
        for name in ["assert.js", "sta.js"]
            .into_iter()
            .chain(meta.includes.iter().map(String::as_str))
        {
            match harness.get(name) {
                Some(text) => {
                    source.push_str(text);
                    source.push('\n');
                }
                None => return Outcome::Fail(format!("harness file {name} is not vendored")),
            }
        }
    }
    source.push_str(&test);

    let result = std::panic::catch_unwind(|| run_source(&source))
        .unwrap_or_else(|_| Err("the VM panicked".to_string()));
    match (result, meta.negative) {
        (Ok(()), false) | (Err(_), true) => Outcome::Pass,
        (result, _) => {
            if let Some((feature, _)) = ABSENT
                .iter()
                .find(|(feature, _)| meta.features.iter().any(|f| f == feature))
            {
                return Outcome::Absent(feature);
            }
            Outcome::Fail(match result {
                Ok(()) => "expected an error, ran to the end".to_string(),
                Err(message) => message,
            })
        }
    }
}

#[test]
fn test262_subset_conformance() {
    let root = Path::new(ROOT);
    let mut harness = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(root.join("harness")) {
        for entry in entries.filter_map(Result::ok) {
            if let (Some(name), Ok(text)) = (
                entry.file_name().to_str().map(str::to_string),
                std::fs::read_to_string(entry.path()),
            ) {
                harness.insert(name, text);
            }
        }
    }
    let mut files = Vec::new();
    collect(&root.join("test"), &mut files);
    files.sort();
    assert!(!files.is_empty(), "the fixtures should be present");

    let focus = std::env::var("TOBIRA_T262_DIR").ok();
    let test_root = root.join("test");

    // The harness is deep JS on a recursive-descent parser: give it room, and
    // keep a panic in one test from taking the rest with it.
    let worker = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let quiet = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let mut by_dir: BTreeMap<String, (usize, usize)> = BTreeMap::new();
            let mut failures: Vec<String> = Vec::new();
            let mut passing: Vec<String> = Vec::new();
            let mut absent: BTreeMap<&'static str, usize> = BTreeMap::new();
            let (mut ran, mut passed, mut skipped) = (0usize, 0usize, 0usize);
            for path in &files {
                let relative = path
                    .strip_prefix(&test_root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");
                // The directory, to four levels: `built-ins/Math`, and
                // `built-ins/Number/prototype/toFixed` on its own line.
                let parts: Vec<&str> = relative.split('/').collect();
                let depth = if parts.get(2) == Some(&"prototype") { 4 } else { 2 };
                let dir = parts[..parts.len().saturating_sub(1).min(depth)].join("/");
                // A crash no `catch_unwind` sees (an abort, a native stack
                // overflow) leaves this as the last line.
                if std::env::var("TOBIRA_T262_TRACE").is_ok() {
                    eprintln!("run {relative}");
                }
                match run_one(path, &harness) {
                    Outcome::Skipped => skipped += 1,
                    Outcome::Absent(feature) => *absent.entry(feature).or_insert(0) += 1,
                    Outcome::Pass => {
                        ran += 1;
                        passed += 1;
                        let entry = by_dir.entry(dir).or_insert((0, 0));
                        entry.0 += 1;
                        entry.1 += 1;
                        passing.push(relative);
                    }
                    Outcome::Fail(message) => {
                        ran += 1;
                        by_dir.entry(dir).or_insert((0, 0)).1 += 1;
                        let first = message.lines().next().unwrap_or("").to_string();
                        failures.push(format!("{relative}\t{first}"));
                    }
                }
            }
            std::panic::set_hook(quiet);
            (ran, passed, skipped, by_dir, failures, passing, absent)
        })
        .expect("the worker should start");
    let (ran, passed, skipped, by_dir, failures, passing, absent) = worker.join().expect("the worker should finish");

    let percent = 100.0 * passed as f64 / ran.max(1) as f64;
    let absent_total: usize = absent.values().sum();
    println!(
        "test262 subset: pass {passed} / fail {} ({percent:.1}% of pass + fail), absent {absent_total}, skipped {skipped} (module / async)",
        ran - passed
    );
    for (feature, count) in &absent {
        let reason = ABSENT.iter().find(|(f, _)| f == feature).map_or("", |(_, r)| r);
        println!("  absent {count:>4} {feature}: {reason}");
    }
    for (dir, (ok, all)) in &by_dir {
        println!("  {ok:>4} / {all:<4} {dir}");
    }
    match &focus {
        Some(want) => {
            for line in failures.iter().filter(|line| line.starts_with(want.as_str())) {
                println!("{line}");
            }
        }
        None => {
            // The commonest first lines: what to fix first.
            let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
            for line in &failures {
                *reasons.entry(line.split('\t').nth(1).unwrap_or("")).or_insert(0) += 1;
            }
            let mut ranked: Vec<_> = reasons.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            for (reason, count) in ranked.into_iter().take(15) {
                println!("  {count:>4} x {reason}");
            }
        }
    }
    if let Ok(out) = std::env::var("TOBIRA_T262_OUT") {
        let _ = std::fs::write(out, failures.join("\n"));
    }
    let baseline_path = root.join("baseline.txt");
    if std::env::var("TOBIRA_T262_BLESS").is_ok() {
        std::fs::write(&baseline_path, passing.join("\n") + "\n")
            .expect("baseline should write");
        println!("baseline.txt rewritten: {} tests", passing.len());
        return;
    }
    let baseline = std::fs::read_to_string(&baseline_path).unwrap_or_default();
    let now: std::collections::BTreeSet<&str> = passing.iter().map(String::as_str).collect();
    let before: std::collections::BTreeSet<&str> =
        baseline.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let gained: Vec<&&str> = now.difference(&before).collect();
    let lost: Vec<&&str> = before.difference(&now).collect();
    if !gained.is_empty() {
        println!("newly passing ({}), bless with TOBIRA_T262_BLESS=1:", gained.len());
        for path in gained.iter().take(40) {
            println!("  + {path}");
        }
    }
    for path in &lost {
        let why = failures
            .iter()
            .find(|line| line.starts_with(**path))
            .and_then(|line| line.split('\t').nth(1))
            .unwrap_or("");
        println!("  - {path}\t{why}");
    }
    assert!(
        lost.is_empty(),
        "{} test262 tests that passed in baseline.txt no longer pass",
        lost.len()
    );
}
