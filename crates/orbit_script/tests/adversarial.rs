//! Adversarial scripts (security review of the Rhai sandbox, ADR 0015,
//! docs/security/rhai-review.md). A script comes with the game, and a
//! downloaded game is untrusted: whatever it does must end as a script
//! error, never as a crash, a freeze or a change of the game state.
//!
//! Every run happens on a thread with a 1 MiB stack (the main thread of a
//! Windows player, the default of wasm), in debug builds (bigger frames
//! than release): a stack overflow aborts the whole test binary.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use orbit_core::{ScalarType, Value};
use orbit_script::{Limits, MAX_IMPORT_DEPTH, ScriptEngine, TestRun};
use orbit_vm::ScriptHost;

const STACK: usize = 1 << 20;

fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(f)
        .expect("thread")
        .join()
        .expect("no panic")
}

fn vars() -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("gold".to_owned(), Value::Int(10)),
        ("ratio".to_owned(), Value::Float(0.5)),
        ("name".to_owned(), Value::Str("Ada".into())),
        (
            "bag".to_owned(),
            Value::list(ScalarType::Int, vec![Value::Int(1)]),
        ),
    ])
}

fn run_with(modules: &[(&str, &str)], source: &str) -> (TestRun, Duration) {
    let modules: BTreeMap<String, String> = modules
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let source = source.to_owned();
    on_small_stack(move || {
        let engine = ScriptEngine::new(modules);
        let start = Instant::now();
        let run = engine.test_run(&source, &vars(), 7);
        (run, start.elapsed())
    })
}

fn run(source: &str) -> TestRun {
    run_with(&[], source).0
}

/// The script fails, changes nothing, and the message contains `needle`.
fn fails(source: &str, needle: &str) {
    fails_with(&[], source, needle);
}

fn fails_with(modules: &[(&str, &str)], source: &str, needle: &str) {
    let (run, took) = run_with(modules, source);
    let err = run
        .result
        .as_ref()
        .expect_err(&format!("`{source}` should fail"));
    assert!(
        err.message.contains(needle),
        "`{source}`: expected `{needle}` in `{}`",
        err.message
    );
    assert!(run.changes.is_empty(), "a failed script changes nothing");
    assert!(took < Duration::from_secs(10), "`{source}` took {took:?}");
}

// ------------------------------------------------------------- escapes

#[test]
fn no_code_from_strings() {
    fails(r#"eval("set_var(\"gold\", 1)")"#, "eval");
    fails(r#"let f = Fn("eval"); f.call("1")"#, "eval");
    fails(r#"call(Fn("eval"), "1")"#, "eval");
}

#[test]
fn no_host_access() {
    for f in [
        "timestamp()",
        r#"open_file("x")"#,
        r#"read_file("C:/Windows/win.ini")"#,
        r#"system("calc")"#,
        r#"import_module("x")"#,
    ] {
        fails(f, "not found");
    }
    // `sleep` exists in Rhai's standard package: it froze the game (and
    // panics on wasm). It is overridden.
    fails("sleep(1000000)", "not available");
    fails("sleep(1e300)", "not available");
}

#[test]
fn imports_only_reach_in_memory_modules() {
    let mods = [("shop", "fn price() { 3 }")];
    for path in [
        "../shop",
        "scripts/shop",
        "scripts/shop.rhai",
        "/etc/passwd",
        "C:\\Windows\\win.ini",
        "\\\\server\\share\\x",
        "file:///etc/passwd",
        "shop.rhai",
        "",
    ] {
        let source = format!("import {path:?} as m; 1");
        fails_with(&mods, &source, "not found");
    }
    let (ok, _) = run_with(&mods, r#"import "shop" as s; set_var("gold", s::price());"#);
    assert_eq!(ok.result, Ok(None));
}

#[test]
fn no_curry_and_no_capturing_closures() {
    fails(r#"let f = Fn("x"); f = f.curry(1); 1"#, "property");
    fails(r#"curry(Fn("x"), 1)"#, "curry");
    fails(r#"let n = 1; let f = || n; f.call()"#, "Undefined variable");
    // Anonymous functions that capture nothing still work.
    let r = run(r#"let a = [3, 1, 2]; a.sort(|x, y| x - y); set_var("bag", a);"#);
    assert_eq!(r.result, Ok(None));
}

// ------------------------------------------------------------- modules

#[test]
fn cyclic_imports_fail_instead_of_overflowing_the_stack() {
    let mods = [
        ("a", r#"import "b" as b; fn f() { 1 }"#),
        ("b", r#"import "a" as a; fn g() { 2 }"#),
        ("me", r#"import "me" as me;"#),
    ];
    fails_with(&mods, r#"import "a" as a; 1"#, "cyclic import: a -> b -> a");
    fails_with(&mods, r#"import "me" as m; 1"#, "cyclic import: me -> me");
    // A function that imports itself again.
    let mods = [("rec", r#"fn f() { import "rec" as r; r::f() }"#)];
    fails_with(&mods, r#"import "rec" as r; r::f()"#, "");
}

#[test]
fn imports_nest_a_bounded_depth() {
    let names: Vec<String> = (0..=MAX_IMPORT_DEPTH + 1).map(|i| format!("m{i}")).collect();
    let sources: Vec<String> = (0..names.len())
        .map(|i| match names.get(i + 1) {
            Some(next) => format!("import \"{next}\" as n; fn f() {{ n::f() + 1 }}"),
            None => "fn f() { 0 }".to_owned(),
        })
        .collect();
    let mods: Vec<(&str, &str)> = names
        .iter()
        .map(String::as_str)
        .zip(sources.iter().map(String::as_str))
        .collect();
    fails_with(&mods, r#"import "m0" as m; m::f()"#, "nest more than");
    // At the limit it works.
    let start = format!("m{}", names.len() - MAX_IMPORT_DEPTH);
    let (ok, _) = run_with(&mods, &format!("import \"{start}\" as m; m::f()"));
    assert!(ok.result.is_ok(), "{:?}", ok.result);
}

#[test]
fn modules_count_against_the_call() {
    let mods = [("l", "fn f() { 1 }")];
    let many: String = (0..40).map(|i| format!("import \"l\" as l{i};\n")).collect();
    fails_with(&mods, &many, "modules");
    // Work done by modules counts against the operation limit.
    let heavy = [("h", "let x = 0; while x < 20000 { x += 1; }")];
    fails_with(
        &heavy,
        r#"import "h" as a; import "h" as b; import "h" as c; import "h" as d; 1"#,
        "operation limit",
    );
}

#[test]
fn a_failing_module_applies_nothing() {
    let mods = [("bad", r#"set_var("gold", 99); throw "no";"#)];
    fails_with(&mods, r#"set_var("gold", 50); import "bad" as b; 1"#, "no");
}

// ------------------------------------------------------- exhaustion

#[test]
fn callbacks_share_the_operation_budget() {
    // Rhai gives every callback a fresh operation counter: a comparator
    // spending ~90 000 operations per comparison ran for minutes.
    let heavy = "let n = 0; while n < 30000 { n += 1; }";
    for source in [
        format!("let a = []; for i in 0..1000 {{ a.push(i) }} a.sort(|x, y| {{ {heavy} x - y }}); 1"),
        format!("let a = []; for i in 0..1000 {{ a.push(i) }} a.map(|x| {{ {heavy} x }}); 1"),
        format!("let a = []; for i in 0..1000 {{ a.push(i) }} a.filter(|x| {{ {heavy} true }}); 1"),
        format!("let a = []; for i in 0..1000 {{ a.push(i) }} a.reduce(|s, x| {{ {heavy} s }}, 0); 1"),
        format!("let a = []; for i in 0..1000 {{ a.push(i) }} a.retain(|x| {{ {heavy} true }}); 1"),
        format!("let a = []; for i in 0..1000 {{ a.push(i) }} a.all(|x| {{ {heavy} true }}); 1"),
        format!("let a = [1, 2]; a.dedup(|x, y| {{ {heavy} false }}); a.index_of(|x| {{ {heavy} false }}); 1"),
        format!("fn f(x) {{ {heavy} x }} let a = []; for i in 0..1000 {{ a.push(i) }} a.map(Fn(\"f\")); 1"),
        "let n = 0; for i in range(0, 9223372036854775807) { n += 1; } n".to_owned(),
        "loop {}".to_owned(),
    ] {
        let (r, took) = run_with(&[], &source);
        let err = r.result.expect_err(&source);
        assert!(err.message.contains("operation limit"), "{source}: {}", err.message);
        assert!(took < Duration::from_secs(10), "{source}: {took:?}");
    }
}

#[test]
fn recursion_through_callbacks_is_bounded() {
    let (r, took) = run_with(&[], "fn f() { [3, 2, 1].sort(|x, y| { f(); x - y }) } f(); 1");
    assert!(took < Duration::from_secs(10), "{took:?}");
    let _ = r.result;
    fails(r#"fn f() { call(Fn("f")) } f()"#, "call depth");
}

#[test]
fn nested_values_are_bounded_before_the_stack_is() {
    // Arrays, maps, and both alternating. Unchecked, ~500 levels overflowed
    // a 1 MiB stack in `to_string` / `==` / `print` / `debug`.
    for wrap in ["[a]", "#{x: a}", "#{x: [a]}", "[a, a]"] {
        for tail in [
            "1",
            "a.to_string().len()",
            "`${a}`.len()",
            "(\"\" + a).len()",
            "a == a",
            "a != a",
            "[a].contains(a)",
            "a in [a]",
            "switch a { _ => 1 }",
            "print(a); 1",
            "debug(a); 1",
            "let b = a; b.len()",
        ] {
            let source = format!("let a = []; for i in 0..3000 {{ a = {wrap}; }} {tail}");
            fails(&source, "size limit");
        }
    }
    // A deep value built in a few expressions is refused when printed or
    // compared.
    let wrap = |inner: &str| format!("{}{inner}{}", "[".repeat(15), "]".repeat(15));
    let lit = format!(
        "let a = {}; let a = {}; let a = {}; let b = {}; let c = {};",
        wrap("1"),
        wrap("a"),
        wrap("a"),
        wrap("a"),
        wrap("b")
    );
    for tail in ["c.to_string()", "c == c", "`${c}`", "print(c)"] {
        let r = run(&format!("{lit} {tail}"));
        let message = r.result.unwrap_err().message;
        assert!(message.contains("size limit"), "{tail}: {message}");
    }
    // Within the limit everything works, with Rhai's usual formats.
    let limits = Limits::default();
    assert!(limits.max_array_size >= 1000);
    let r = run(
        "let a = 1; for i in 0..20 { a = #{x: [a]}; } let b = a; \
         let s = a.to_string(); print(a); debug(a); \
         if a == b && !(a != b) && [b].contains(a) { s } else { \"\" }",
    );
    let s = r.result.unwrap().unwrap();
    assert!(s.starts_with("#{\"x\": [#{\"x\": ["), "{s}");
    let r = run(r#"`${[1, "a", 'c', 2.5, true, ()]} ${#{b: 1, a: [2]}}`"#);
    assert_eq!(
        r.result.unwrap().unwrap(),
        r#"[1, "a", 'c', 2.5, true, ()] #{"a": [2], "b": 1}"#
    );
    let r = run(
        r#"if [1, [2]] == [1, [2]] && [1] != [1.5] && #{a: [1]} == #{a: [1]} && [1] != ["1"] { "yes" }"#,
    );
    assert_eq!(r.result.unwrap(), Some("yes".into()));
}

#[test]
fn growing_a_map_by_index_is_bounded() {
    // Rhai checks the value assigned by `m[key] = v`, not the map: this
    // loop grew a map past a gigabyte before the operation limit.
    fails(
        "let s = \"\"; s.pad(60000, 'x'); let m = #{}; for i in 0..15000 { m[`${i}`] = s + i; } 1",
        "size limit",
    );
    fails(
        "let s = \"\"; s.pad(60000, 'x'); let m = #{a: #{}}; for i in 0..15000 { m.a[`${i}`] = s + i; } 1",
        "size limit",
    );
    fails("let m = #{}; for i in 0..5000 { m[`${i}`] = i; } 1", "size limit");
    fails(
        "fn grow(m) { for i in 0..5000 { m[`${i}`] = i; } m } let x = grow(#{}); 1",
        "size limit",
    );
    // Many variables, each within its own limit.
    let lets: String = (0..200).map(|i| format!("let v{i} = s + {i};\n")).collect();
    fails(&format!("let s = \"\"; s.pad(60000, 'x'); {lets} 1"), "size limit");
}

#[test]
fn sizes_are_limited() {
    fails("let b = blob(100000000); 1", "size limit");
    fails("let b = blob(500); for i in 0..20 { b += b; } 1", "size limit");
    fails("let a = []; a.pad(1000000000, 0); 1", "size limit");
    fails("let s = \"x\"; s.pad(9223372036854775807, 'y'); 1", "size limit");
    fails("let s = \"x\"; for i in 0..30 { s = `${s}${s}`; } 1", "size limit");
    fails("let s = \"x\"; for i in 0..30 { s += s; } 1", "size limit");
    fails(
        "let s = \"\"; s.pad(60000, 'x'); let a = []; for i in 0..100 { a.push(s + i); } 1",
        "size limit",
    );
    fails(
        "let s = \"\"; s.pad(60000, 'x'); let m = #{}; for i in 0..2000 { m[`${i}`] = s; } 1",
        "size limit",
    );
    fails("let m = #{}; for i in 0..5000 { m[`${i}`] = i; } 1", "size limit");
}

#[test]
fn logs_are_capped() {
    let r = run("let s = \"\"; s.pad(60000, 'x'); for i in 0..10000 { print(s); debug(s); log(s); } 1");
    assert!(r.result.is_ok());
    assert!(r.logs.len() <= 64);
    assert!(r.logs.iter().all(|l| l.chars().count() <= 1024));
}

#[test]
fn source_complexity_is_limited() {
    let cases = [
        format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000)),
        "{".repeat(50_000),
        format!("[{}]", "[".repeat(50_000)),
        format!("1{}", "+1".repeat(200_000)),
        "!".repeat(100_000) + "true",
        format!("let x = 1; x{}", ".a".repeat(100_000)),
        format!("let x = 1; x{}", "[0]".repeat(100_000)),
        "#{a:".repeat(50_000),
        format!("fn f() {{ {}1 }} f()", "if true {".repeat(10_000)),
    ];
    for source in cases {
        let r = run(&source);
        assert!(r.result.is_err());
    }
}

#[test]
fn deepest_legal_recursion_fits_a_small_stack() {
    // Call depth limit, with deep expressions at every level, from the
    // deepest legal import.
    let expr = format!("{}f(n - 1){}", "(1 + ".repeat(10), ")".repeat(10));
    let body = format!("fn f(n) {{ if n <= 0 {{ 0 }} else {{ {expr} }} }}");
    let names: Vec<String> = (0..MAX_IMPORT_DEPTH).map(|i| format!("m{i}")).collect();
    let sources: Vec<String> = (0..names.len())
        .map(|i| match names.get(i + 1) {
            Some(next) => format!("import \"{next}\" as n; {body} let warm = f(25);"),
            None => format!("{body} let warm = f(25);"),
        })
        .collect();
    let mods: Vec<(&str, &str)> = names
        .iter()
        .map(String::as_str)
        .zip(sources.iter().map(String::as_str))
        .collect();
    let (r, _) = run_with(&mods, &format!("import \"m0\" as m; {body} f(28) + m::f(28)"));
    assert!(r.result.is_ok(), "{:?}", r.result);
    let (r, _) = run_with(&mods, &format!("{body} f(1000)"));
    assert!(r.result.unwrap_err().message.contains("call depth"));
}

// ---------------------------------------------------------- arithmetic

#[test]
fn arithmetic_edges_are_errors_not_panics() {
    let min = "(-9223372036854775807 - 1)";
    for (source, needle) in [
        ("9223372036854775807 + 1".to_owned(), "overflow"),
        ("2 ** 100".to_owned(), "overflow"),
        ("1 / 0".to_owned(), "zero"),
        (format!("{min} / -1"), "overflow"),
        (format!("{min} % -1"), "overflow"),
        (format!("{min}.abs()"), "overflow"),
        (format!("-{min}"), "overflow"),
        ("(1e308 * 10.0).to_int()".to_owned(), "overflow"),
        (format!("[1][{min}]"), "out of bounds"),
        (format!("\"abc\"[{min}]"), "out of bounds"),
        (format!("5.get_bits({min}, 9223372036854775807)"), "out of bounds"),
        ("for i in range(0, 10, 0) {} 1".to_owned(), "step"),
        ("parse_int(\"ff\", 99)".to_owned(), "radix"),
    ] {
        fails(&source, needle);
    }
    // Out-of-range slicing is clamped, never a panic.
    for source in [
        format!("\"abc\".sub_string({min}, 9223372036854775807)"),
        format!("let s = \"abc\"; s.crop({min}, 9223372036854775807); s"),
        format!("[1, 2, 3].extract({min}, 9223372036854775807)"),
        format!("let a = [1, 2, 3]; a.splice({min}, 9223372036854775807, [1]); a"),
        format!("let a = [1, 2]; a.chop({min}); a"),
        format!("blob(3).get({min})"),
        "1 << 200".to_owned(),
        "1 << -1".to_owned(),
        "(0.0 / 0.0).to_int()".to_owned(),
    ] {
        let _ = run(&source);
    }
}

#[test]
fn the_api_survives_extreme_arguments() {
    let min = "(-9223372036854775807 - 1)";
    for source in [
        format!("rand_int({min}, 9223372036854775807)"),
        format!("rand_int(9223372036854775807, {min})"),
        "rand_int(5, 5)".to_owned(),
        "chance(0.0 / 0.0)".to_owned(),
        "chance(1e308 * 10.0)".to_owned(),
        "chance(-1.0)".to_owned(),
        "branch(\"\")".to_owned(),
        "get_var(\"\")".to_owned(),
        "has_var(\"\\u{0}\")".to_owned(),
        "log(\"\\u{0}\\u{202e}\")".to_owned(),
    ] {
        let _ = run(&source);
    }
}

#[test]
fn non_finite_floats_never_reach_the_story() {
    fails("set_var(\"ratio\", 0.0 / 0.0)", "finite");
    fails("set_var(\"ratio\", 1e308 * 10.0)", "finite");
    fails("set_var(\"bag\", [1.0, 0.0 / 0.0])", "finite");
    let engine = ScriptEngine::default();
    let none = |_: &str| None;
    assert!(engine.eval("0.0 / 0.0", &none).is_err());
}

#[test]
fn wrong_values_cannot_be_stored() {
    for source in [
        r#"set_var("gold", "x")"#,
        r#"set_var("gold", [1])"#,
        r#"set_var("bag", [[1]])"#,
        r#"set_var("bag", [1, "x"])"#,
        r#"set_var("bag", #{a: #{}})"#,
        r#"set_var("gold", blob(3))"#,
        r#"set_var("gold", Fn("x"))"#,
        r#"set_var("gold", ())"#,
        r#"set_var("nope", 1)"#,
    ] {
        let r = run(source);
        assert!(r.result.is_err(), "{source}");
        assert!(r.changes.is_empty());
    }
}

// ------------------------------------------------------- determinism

#[test]
fn same_state_same_result() {
    let source = r#"
        let m = #{ z: 1, a: 2, m: 3, "é": 4 };
        let out = `${m.keys()} ${0.1 + 0.2} ${1.0 / 3.0} ${1e21} ${rand_int(1, 100)} ${rand_float()}`;
        log(out);
        set_var("name", out);
    "#;
    let a = run(source);
    let b = run(source);
    assert_eq!(a, b);
    assert!(a.logs[0].starts_with("[\"a\", \"m\", \"z\", \"é\"] 0.30000000000000004"), "{}", a.logs[0]);
}

// ------------------------------------------------------------- fuzzing

/// Random token soup: no input may crash, hang or leave state behind.
#[test]
fn random_programs_never_crash() {
    const TOKENS: &[&str] = &[
        "let x = ", "x", "1", "-1", "9223372036854775807", "0.0", "1e308", "\"s\"", "'c'", "[",
        "]", "#{", "}", "{", "(", ")", ";", ",", ".", "+", "-", "*", "/", "%", "**", "<<", "==",
        "!", "&&", "||", "if ", "else ", "while ", "loop ", "for i in 0..9 ", "break;",
        "return ", "fn f(a) ", "f(", "x.len()", "x.to_string()", "a", "x = [x];", "x = #{k: x};",
        "set_var(\"gold\", ", "set_var(\"bag\", ", "get_var(\"gold\")", "rand_int(", "branch(",
        "import \"m\" as m;", "m::g()", "Fn(\"f\")", ".call(", "|v| v", "sort(", "map(",
        "throw ", "try { ", "} catch (e) { ", "switch x { 1 => 2, _ => 3 }", "`${", "`", "x[0]",
        "blob(9)", "x.pad(99999, 0);", "print(", "debug(", "this", "?.", "??", "in ",
    ];
    let mods = BTreeMap::from([(
        "m".to_owned(),
        r#"fn g() { set_var("gold", 3); [1, 2] }"#.to_owned(),
    )]);
    on_small_stack(move || {
        let engine = ScriptEngine::new(mods);
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..3000 {
            let len = 1 + (next() % 40) as usize;
            let source: String = (0..len)
                .map(|_| TOKENS[(next() % TOKENS.len() as u64) as usize])
                .collect::<Vec<_>>()
                .join(" ");
            let start = Instant::now();
            let r = engine.test_run(&source, &vars(), next());
            assert!(start.elapsed() < Duration::from_secs(5), "{source}");
            if r.result.is_err() {
                assert!(r.changes.is_empty(), "{source}");
            }
            let none = |_: &str| None;
            let _ = engine.eval(&source, &none);
        }
    });
}
