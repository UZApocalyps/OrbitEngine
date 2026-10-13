use std::collections::BTreeMap;
use std::sync::Arc;

use orbit_core::{
    Expr, Node, NodeId, NodeKind, Scene, SceneId, ScriptBranch, Story, Value, VarDecl,
};
use orbit_vm::{StepResult, Vm};

use super::*;

fn vars() -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("gold".to_owned(), Value::Int(10)),
        ("where".to_owned(), Value::Int(0)),
        ("name".to_owned(), Value::Str("Ada".into())),
        ("ratio".to_owned(), Value::Float(0.5)),
        (
            "bag".to_owned(),
            Value::list(ScalarType::Str, vec![Value::Str("key".into())]),
        ),
    ])
}

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

/// A story: label, script, label `win` + End, label `lose` + End.
fn story(source: &str) -> (Story, NodeId, NodeId, NodeId) {
    let mut story = Story::new("t");
    for (name, value) in vars() {
        story.variables.push(VarDecl::game(name, value));
    }
    let start = node(NodeKind::Label { name: "start".into() });
    let win = node(NodeKind::Label { name: "win".into() });
    let lose = node(NodeKind::Label { name: "lose".into() });
    let script = node(NodeKind::Script {
        source: source.into(),
        branches: vec![ScriptBranch {
            name: "win".into(),
            target: win.id,
        }],
        on_error: Some(lose.id),
    });
    let ids = (start.id, win.id, lose.id);
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "s".into(),
        nodes: vec![
            start,
            script,
            node(NodeKind::End),
            win,
            mark(1),
            node(NodeKind::End),
            lose,
            mark(2),
            node(NodeKind::End),
        ],
    });
    (story, ids.0, ids.1, ids.2)
}

/// `where = n`: tells which label was reached.
fn mark(n: i64) -> Node {
    node(NodeKind::SetVar {
        var: "where".into(),
        value: Expr::lit(n),
    })
}

/// 1: `win` / `yes`, 2: `lose` / `no`, 0: neither.
fn reached(vm: &Vm) -> i64 {
    match vm.variable("where") {
        Some(Value::Int(n)) => *n,
        _ => 0,
    }
}

fn vm(story: Story, modules: BTreeMap<String, String>) -> Vm {
    let mut vm = Vm::from_start(Arc::new(story)).unwrap();
    vm.set_script_host(Some(ScriptEngine::shared(modules)));
    vm
}

/// Runs to the end and returns the label reached last.
fn run(vm: &mut Vm) {
    for _ in 0..10 {
        if matches!(vm.step(), StepResult::End) {
            return;
        }
    }
    panic!("the story did not end");
}

#[test]
fn a_script_reads_writes_and_branches() {
    let (s, ..) = story(
        r#"
        let g = get_var("gold");
        set_var("gold", g + 5);
        set_var("name", get_var("name") + "!");
        set_var("bag", get_var("bag") + ["map"]);
        if g > 3 { branch("win") }
        "#,
    );
    let mut vm = vm(s, BTreeMap::new());
    run(&mut vm);
    assert_eq!(vm.variable("gold"), Some(&Value::Int(15)));
    assert_eq!(vm.variable("name"), Some(&Value::Str("Ada!".into())));
    assert_eq!(
        vm.variable("bag"),
        Some(&Value::list(
            ScalarType::Str,
            vec![Value::Str("key".into()), Value::Str("map".into())]
        ))
    );
    assert!(vm.take_warnings().is_empty());
}

#[test]
fn a_returned_name_chooses_the_branch() {
    let (s, ..) = story(r#"if get_var("gold") == 10 { "win" } else { "" }"#);
    let mut vm = vm(s, BTreeMap::new());
    vm.step();
    assert_eq!(reached(&vm), 1, "the script jumped to `win`");
}

#[test]
fn a_failing_script_applies_nothing_and_takes_its_error_exit() {
    for source in [
        r#"set_var("gold", 99); rand_int(1, 6); throw "boom";"#,
        r#"set_var("gold", 99); set_var("nope", 1);"#,
        r#"set_var("gold", 99); set_var("gold", "text");"#,
        r#"set_var("gold", 99); let x = ;"#,
        r#"set_var("gold", 99); loop { }"#,
        r#"set_var("gold", 99); fn f(n) { f(n + 1) } f(0);"#,
        r#"set_var("gold", 99); let s = "x"; loop { s += s; }"#,
    ] {
        let (s, ..) = story(source);
        let mut vm = vm(s, BTreeMap::new());
        let rng_before = vm.snapshot().rng;
        vm.step();
        assert_eq!(vm.variable("gold"), Some(&Value::Int(10)), "{source}");
        assert_eq!(vm.snapshot().rng, rng_before, "{source}: RNG not advanced");
        assert_eq!(reached(&vm), 2, "{source}");
        let warnings = vm.take_warnings();
        assert!(
            warnings.iter().any(|w| w.contains("nothing applied") || w.contains("cancelled")),
            "{source}: {warnings:?}"
        );
    }
}

#[test]
fn limits_stop_runaway_scripts_with_a_clear_message() {
    let engine = ScriptEngine::new(BTreeMap::new());
    let run = |src: &str| engine.test_run(src, &vars(), 1).result.unwrap_err();
    assert!(run("loop {}").message.contains("operation limit"));
    assert!(run("fn f(n) { f(n + 1) } f(0)").message.contains("call depth"));
    assert!(run(r#"let s = "ab"; loop { s += s; }"#).message.contains("too large"));
    assert!(run("let a = []; loop { a.push(1); }").message.contains("too large"));
}

#[test]
fn sandbox_has_no_eval_no_io_and_no_clock() {
    let engine = ScriptEngine::new(BTreeMap::new());
    for src in [
        r#"eval("1 + 1")"#,
        "timestamp()",
        r#"import "std/fs" as fs;"#,
        r#"open_file("x")"#,
    ] {
        assert!(engine.test_run(src, &vars(), 1).result.is_err(), "{src} must fail");
    }
}

#[test]
fn same_state_gives_the_same_draws_and_rollback_replays_them() {
    let src = r#"
        let a = rand_int(1, 100);
        let b = rand_float();
        set_var("gold", a);
        set_var("ratio", b);
        if chance(0.5) { branch("win") }
    "#;
    let (s, ..) = story(src);
    let mut first = vm(s.clone(), BTreeMap::new());
    let snapshot = first.snapshot();
    run(&mut first);
    let mut second = vm(s.clone(), BTreeMap::new());
    run(&mut second);
    assert_eq!(first.snapshot().vars, second.snapshot().vars);
    assert_eq!(first.snapshot().rng, second.snapshot().rng);
    assert_ne!(first.snapshot().rng, snapshot.rng, "the RNG advanced");
    // Restoring the snapshot (load / rollback) and replaying gives the same.
    let mut again = vm(s, BTreeMap::new());
    again.restore(snapshot).unwrap();
    run(&mut again);
    assert_eq!(again.snapshot().vars, first.snapshot().vars);
    assert_eq!(again.snapshot().rng, first.snapshot().rng);
}

#[test]
fn modules_are_imported_from_memory() {
    let modules = BTreeMap::from([(
        "shop".to_owned(),
        r#"
        fn price(item) { if item == "sword" { 7 } else { 1 } }
        fn buy(item) {
            let p = price(item);
            if get_var("gold") < p { throw "not enough gold"; }
            set_var("gold", get_var("gold") - p);
            set_var("bag", get_var("bag") + [item]);
        }
        "#
        .to_owned(),
    )]);
    let (s, ..) = story(r#"import "shop" as shop; shop::buy("sword");"#);
    let mut v = vm(s, modules.clone());
    run(&mut v);
    assert_eq!(v.variable("gold"), Some(&Value::Int(3)));
    // An unknown module is a diagnostic, and a run-time failure.
    let engine = ScriptEngine::new(modules);
    let d = engine.check(r#"import "nope" as n;"#, false);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!((d[0].line, d[0].column), (1, 1));
    assert!(engine.test_run(r#"import "nope" as n;"#, &vars(), 1).result.is_err());
}

#[test]
fn syntax_errors_have_a_line_and_column() {
    let engine = ScriptEngine::default();
    let d = engine.check("let a = 1;\nlet b = ;\n", false);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].line, 2);
    assert!(d[0].column > 0);
    assert!(engine.check(r#"get_var("gold") > 3"#, true).is_empty());
    assert!(!engine.check("let x = 1; x", true).is_empty(), "a condition is one expression");
    // Strict variables: an undefined name is caught before running.
    assert!(!engine.check("undefined_thing + 1", false).is_empty());
}

#[test]
fn rhai_conditions_read_only() {
    let mut story = Story::new("t");
    story.variables.push(VarDecl::game("gold", Value::Int(10)));
    story.variables.push(VarDecl::game("where", Value::Int(0)));
    let yes = node(NodeKind::Label { name: "yes".into() });
    let no = node(NodeKind::Label { name: "no".into() });
    let cond = |code: &str| NodeKind::If {
        condition: Expr::Rhai(code.into()),
        then_target: yes.id,
        else_target: Some(no.id),
    };
    for (code, expect_yes, warns) in [
        (r#"get_var("gold") > 5 && get_var("gold") % 2 == 0"#, true, false),
        (r#"get_var("gold") > 50"#, false, false),
        (r#"set_var("gold", 1); true"#, false, true),
        (r#"rand_int(1, 2) > 0"#, false, true),
        (r#"42"#, false, true),
    ] {
        let mut s = story.clone();
        s.scenes = vec![Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes: vec![
                node(cond(code)),
                yes.clone(),
                mark(1),
                node(NodeKind::End),
                no.clone(),
                mark(2),
                node(NodeKind::End),
            ],
        }];
        let mut v = vm(s, BTreeMap::new());
        v.step();
        assert_eq!(reached(&v) == 1, expect_yes, "{code}");
        assert_eq!(!v.take_warnings().is_empty(), warns, "{code}");
        assert_eq!(v.variable("gold"), Some(&Value::Int(10)));
    }
}

#[test]
fn without_a_host_scripts_are_skipped_with_a_warning() {
    let (s, ..) = story(r#"set_var("gold", 1); "win""#);
    let mut v = Vm::from_start(Arc::new(s)).unwrap();
    v.step();
    assert_eq!(v.variable("gold"), Some(&Value::Int(10)));
    assert_eq!(reached(&v), 2);
    assert!(!v.take_warnings().is_empty());
}

#[test]
fn values_round_trip_through_rhai() {
    for value in vars().into_values().chain([
        Value::Bool(true),
        Value::map(ScalarType::Int, BTreeMap::from([("a".to_owned(), Value::Int(1))])),
    ]) {
        let back = from_dynamic(&to_dynamic(&value), Some(value.value_type())).unwrap();
        assert_eq!(back, value);
    }
    // An empty array takes the element type of the variable.
    let empty = from_dynamic(&Dynamic::from_array(vec![]), Some(ValueType::List(ScalarType::Str)));
    assert_eq!(empty.unwrap(), Value::list(ScalarType::Str, vec![]));
    assert!(from_dynamic(&Dynamic::UNIT, None).is_err());
}

#[test]
fn test_run_reports_changes_logs_and_draws() {
    let engine = ScriptEngine::default();
    let run = engine.test_run(
        r#"log("hi"); print("there"); set_var("gold", rand_int(1, 3)); "done""#,
        &vars(),
        42,
    );
    assert_eq!(run.result, Ok(Some("done".into())));
    assert_eq!(run.logs, vec!["hi".to_owned(), "there".to_owned()]);
    assert_eq!(run.draws, 1);
    assert_eq!(run.changes.len(), 1);
    assert_eq!(api_version_value(&engine), API_VERSION as i64);
}

fn api_version_value(engine: &ScriptEngine) -> i64 {
    let mut out = BTreeMap::new();
    out.insert("gold".to_owned(), Value::Int(0));
    let run = engine.test_run(r#"set_var("gold", api_version());"#, &out, 0);
    match &run.changes[..] {
        [(_, _, Value::Int(v))] => *v,
        other => panic!("{other:?}"),
    }
}

#[test]
fn notify_queues_toasts_only_when_the_script_succeeds() {
    use orbit_vm::PresentationEvent;
    let notes = |vm: &mut Vm| -> Vec<orbit_vm::TextNotification> {
        vm.take_events()
            .into_iter()
            .filter_map(|e| match e {
                PresentationEvent::NotifyText(n) => Some(n),
                _ => None,
            })
            .collect()
    };
    let (s, ..) = story(
        r#"notify("Hi"); notify("Gold!", "achievement"); notify("x", #{icon: "a.png", duration: 2});"#,
    );
    let mut m = vm(s, BTreeMap::new());
    m.step();
    let got = notes(&mut m);
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].text, "Hi");
    assert_eq!(got[1].style.as_deref(), Some("achievement"));
    assert_eq!(got[2].icon.as_deref(), Some("a.png"));
    assert_eq!(got[2].duration, Some(2.0));

    let (s, ..) = story(r#"notify("never"); throw "boom";"#);
    let mut m = vm(s, BTreeMap::new());
    m.step();
    assert!(notes(&mut m).is_empty(), "a failing script shows nothing");
}
