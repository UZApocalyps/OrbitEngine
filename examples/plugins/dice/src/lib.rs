//! `orbit.dice`: an example OrbitEngine plugin (M8-04 / M8-05).
//!
//! * Custom node `roll`: rolls `count` dice of `sides` faces, adds
//!   `modifier`, and stores the total in the story variable `target` (an
//!   integer variable declared in the project). It emits a `rolled` event
//!   (the faces) and can hold the story for `pause` seconds so the game can
//!   show the dice.
//! * Custom UI action `orbit.dice.reroll` (`UiAction::Custom { name:
//!   "orbit.dice.reroll", args: [variable, sides] }`): sets `variable` to a
//!   fresh roll of one die.
//!
//! Rollback and save safety: the plugin keeps **no state**. Its only output
//! is a story variable, and its randomness is the VM's seeded RNG (part of
//! the saved state): loading a save or rolling back and replaying gives the
//! same faces.
//!
//! To use it in a game, copy this folder into the project's `plugins/`
//! folder and export (the exporter builds a player with it; it needs a Rust
//! toolchain). See the manual, "Plugins".

use orbit_core::{CustomData, Value};
use orbit_plugin_api::{
    CustomNode, FieldDef, FieldKind, NodeTypeDef, Plugin, PluginContext, PluginManifest,
    PluginOutcome, ProjectRegistry, RuntimePlugin, RuntimeRegistry, default_data_of,
};

pub const PLUGIN_ID: &str = "orbit.dice";
pub const ROLL: &str = "roll";
pub const REROLL: &str = "reroll";

/// The plugin (both halves: editor schema and runtime handlers).
#[derive(Default)]
pub struct DicePlugin;

/// What the exporter's generated player calls.
pub fn orbit_plugins() -> Vec<Box<dyn RuntimePlugin>> {
    vec![Box::new(DicePlugin)]
}

/// Fields of a `roll` node, shown by the editor's inspector.
pub fn roll_fields() -> Vec<FieldDef> {
    vec![
        FieldDef::new("count", "Dice", FieldKind::Int { min: 1, max: 20 }),
        FieldDef::new("sides", "Faces", FieldKind::Int { min: 2, max: 100 }),
        FieldDef::new("modifier", "Modifier", FieldKind::Int { min: -100, max: 100 }),
        FieldDef::new(
            "target",
            "Store in",
            FieldKind::Variable(Some(orbit_core::ValueType::Int)),
        )
        .with_help("Integer variable receiving the total"),
        FieldDef::new("pause", "Pause (s)", FieldKind::Float { min: 0.0, max: 10.0 }),
    ]
}

fn roll_default() -> CustomData {
    let mut data = default_data_of(&roll_fields());
    if let CustomData::Map(entries) = &mut data {
        for (k, v) in entries.iter_mut() {
            match k.as_str() {
                "count" => *v = CustomData::Int(1),
                "sides" => *v = CustomData::Int(6),
                _ => {}
            }
        }
    }
    data
}

fn int(data: &CustomData, key: &str, default: i64) -> i64 {
    data.get(key).and_then(CustomData::as_int).unwrap_or(default)
}

/// Rolls `count` dice of `sides` faces with the VM's RNG.
fn roll_dice(ctx: &mut PluginContext<'_>, count: i64, sides: i64) -> Vec<i64> {
    (0..count.clamp(1, 20))
        .map(|_| ctx.random_range(1, sides.clamp(2, 100)))
        .collect()
}

fn run_roll(node: &CustomNode<'_>, ctx: &mut PluginContext<'_>) -> PluginOutcome {
    let data = node.data;
    let Some(target) = data.get("target").and_then(CustomData::as_str) else {
        ctx.fail("no target variable");
        return PluginOutcome::Continue;
    };
    let faces = roll_dice(ctx, int(data, "count", 1), int(data, "sides", 6));
    let total = faces.iter().sum::<i64>() + int(data, "modifier", 0);
    if ctx.set(target, total).is_err() {
        // `fail` was called by `set`: nothing is applied.
        return PluginOutcome::Continue;
    }
    ctx.emit(
        "rolled",
        CustomData::Map(vec![
            (
                "faces".into(),
                CustomData::List(faces.into_iter().map(CustomData::Int).collect()),
            ),
            ("total".into(), CustomData::Int(total)),
        ]),
    );
    match data.get("pause") {
        Some(CustomData::Float(s)) if *s > 0.0 => PluginOutcome::Wait(*s as f32),
        _ => PluginOutcome::Continue,
    }
}

fn run_reroll(args: &[String], ctx: &mut PluginContext<'_>) {
    let Some(var) = args.first() else {
        ctx.fail("reroll needs a variable name");
        return;
    };
    let sides = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(6);
    let face = roll_dice(ctx, 1, sides)[0];
    if !matches!(ctx.var(var), Some(Value::Int(_))) {
        ctx.fail(format!("`{var}` is not an integer variable"));
        return;
    }
    let _ = ctx.set(var, face);
}

fn manifest() -> PluginManifest {
    PluginManifest::new(PLUGIN_ID, "Dice", env!("CARGO_PKG_VERSION"))
}

impl Plugin for DicePlugin {
    fn manifest(&self) -> PluginManifest {
        manifest()
    }

    fn register(&self, reg: &mut ProjectRegistry) {
        let _ = reg.register_node_type(NodeTypeDef {
            kind: ROLL.into(),
            label: "Roll dice".into(),
            icon: "dice".into(),
            default_data: roll_default,
            validate: None,
            fields: roll_fields(),
        });
    }
}

impl RuntimePlugin for DicePlugin {
    fn manifest(&self) -> PluginManifest {
        manifest()
    }

    fn register_runtime(&self, reg: &mut RuntimeRegistry) {
        reg.register_node(ROLL, run_roll);
        reg.register_action(REROLL, run_reroll);
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{Node, NodeId, NodeKind, Scene, SceneId, Story, StringId, VarDecl};
    use orbit_vm::{StepResult, Vm};

    use super::*;

    fn roll_node(count: i64, sides: i64, modifier: i64, target: &str, pause: f64) -> Node {
        Node {
            id: NodeId::new(),
            kind: NodeKind::Custom {
                plugin_id: PLUGIN_ID.into(),
                kind: ROLL.into(),
                version: 1,
                data: CustomData::Map(vec![
                    ("count".into(), CustomData::Int(count)),
                    ("sides".into(), CustomData::Int(sides)),
                    ("modifier".into(), CustomData::Int(modifier)),
                    ("target".into(), CustomData::Str(target.into())),
                    ("pause".into(), CustomData::Float(pause)),
                ]),
            },
        }
    }

    fn story(nodes: Vec<Node>) -> Story {
        let mut story = Story::new("dice");
        story.variables.push(VarDecl::game("total", Value::Int(0)));
        story.variables.push(VarDecl::game("name", Value::Str(String::new())));
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes,
        });
        story
    }

    fn host() -> orbit_plugin_api::SharedPluginHost {
        let mut reg = RuntimeRegistry::new();
        for p in orbit_plugins() {
            reg.install(p.as_ref()).unwrap();
        }
        reg.into_host()
    }

    fn vm(story: Story, seed: u64) -> Vm {
        let entry = story.entry().unwrap();
        let mut vm = Vm::with_seed(story, entry, seed).unwrap();
        vm.set_plugin_host(Some(host()));
        vm
    }

    fn total(vm: &Vm) -> i64 {
        match vm.variable("total") {
            Some(Value::Int(i)) => *i,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rolls_stay_in_range_and_are_deterministic() {
        for seed in 0..50 {
            let s = story(vec![roll_node(3, 6, 2, "total", 0.0)]);
            let mut a = vm(s.clone(), seed);
            let mut b = vm(s, seed);
            assert_eq!(a.step(), StepResult::End);
            b.step();
            let t = total(&a);
            assert!((5..=20).contains(&t), "{t}");
            assert_eq!(t, total(&b));
            assert!(a.take_warnings().is_empty());
        }
    }

    #[test]
    fn a_restored_snapshot_rolls_the_same_faces() {
        let s = story(vec![
            roll_node(2, 20, 0, "total", 0.0),
            Node {
                id: NodeId::new(),
                kind: NodeKind::Say {
                    speaker: None,
                    text_id: StringId::new(),
                    voice: None,
                },
            },
        ]);
        let mut vm = vm(s, 7);
        let before = vm.snapshot();
        vm.step();
        let first = total(&vm);
        let events = vm.take_events();
        assert!(events.iter().any(|e| matches!(
            e,
            orbit_vm::PresentationEvent::Custom { kind, .. } if kind == "rolled"
        )));
        vm.restore(before).unwrap();
        vm.step();
        assert_eq!(total(&vm), first);
    }

    #[test]
    fn a_pause_blocks_the_story() {
        let mut vm = vm(story(vec![roll_node(1, 6, 0, "total", 1.5)]), 1);
        assert_eq!(vm.step(), StepResult::WaitTime(1.5));
        assert_ne!(total(&vm), 0);
        assert_eq!(vm.step(), StepResult::End);
    }

    #[test]
    fn a_wrong_target_changes_nothing() {
        let mut vm = vm(story(vec![roll_node(1, 6, 0, "name", 0.0)]), 1);
        let rng_before = vm.snapshot().rng;
        vm.step();
        assert_eq!(vm.variable("name"), Some(&Value::Str(String::new())));
        assert_eq!(vm.snapshot().rng, rng_before, "the RNG is not advanced");
        assert!(!vm.take_warnings().is_empty());
    }

    #[test]
    fn the_reroll_ui_action_sets_the_variable() {
        let mut vm = vm(story(vec![roll_node(1, 6, 0, "total", 0.0)]), 3);
        vm.step();
        assert!(vm.run_ui_action("orbit.dice.reroll", &["total".into(), "4".into()]));
        assert!((1..=4).contains(&total(&vm)));
        assert!(!vm.run_ui_action("orbit.dice.reroll", &["name".into()]));
        assert!(!vm.run_ui_action("orbit.dice.nope", &[]));
    }

    #[test]
    fn the_editor_schema_validates_nodes() {
        let mut reg = ProjectRegistry::new();
        reg.install(&DicePlugin).unwrap();
        let ty = reg.node_type(PLUGIN_ID, ROLL).unwrap();
        assert_eq!(ty.def.fields.len(), 5);
        assert_eq!(
            (ty.def.default_data)().get("sides").and_then(CustomData::as_int),
            Some(6)
        );
        let good = story(vec![roll_node(2, 6, 0, "total", 0.0)]);
        assert!(reg.validate_custom(&good).is_empty());
        let bad = story(vec![roll_node(0, 6, 0, "name", 0.0)]);
        let problems = reg.validate_custom(&bad);
        assert!(problems.iter().any(|(_, m)| m.contains("count")));
        assert!(problems.iter().any(|(_, m)| m.contains("wrong type")));
        let unknown = story(vec![roll_node(1, 6, 0, "nope", 0.0)]);
        assert!(reg.validate_custom(&unknown)[0].1.contains("unknown variable"));
    }

    #[test]
    fn the_plugin_targets_api_version_1() {
        assert_eq!(orbit_plugin_api::API_VERSION, 1);
        assert_eq!(Plugin::manifest(&DicePlugin).api_version, 1);
    }
}
