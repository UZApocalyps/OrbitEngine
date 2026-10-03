//! Property-based round trips (ADR 0015 §5): IR → text → IR is the identity
//! (ids included), text → IR → text is stable, and the command diff reaches
//! the parsed story.

use orbit_core::{
    AnimLoop, AnimProp, AnimTarget, Angle, AssetEntry, AssetId, AssetKind, BinOp, Character, CharacterId,
    ChoiceOption, Color, Curve, CustomData, Ease, Effect, Key, Rgba, StoryTrack, Track, TrackData, Vec2,
    Expr, Node, NodeId, NodeKind, Position, Scene, SceneId, Story, Transition, UnOp, Value,
};
use orbit_dsl::{Scope, apply, diff, parse, print_scene, print_story};
use orbit_project::Project;
use proptest::prelude::*;

// ------------------------------------------------------------- strategies

fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[a-z_][a-z0-9_. ]{0,6}",
        Just("jump".to_owned()),
        Just("say \"x\" -> #@ #x".to_owned()),
        Just(String::new()),
    ]
}

fn var_name() -> impl Strategy<Value = String> {
    "[a-z_][a-z0-9_.]{0,6}".prop_filter("literal", |s| s != "true" && s != "false")
}

fn finite_f32() -> impl Strategy<Value = f32> {
    any::<f32>().prop_filter("finite", |f| f.is_finite())
}

fn finite_f64() -> impl Strategy<Value = f64> {
    any::<f64>().prop_filter("finite", |f| f.is_finite())
}

fn expr() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        any::<bool>().prop_map(Expr::lit),
        any::<i64>().prop_map(Expr::lit),
        finite_f64().prop_map(Expr::lit),
        any::<String>().prop_map(|s| Expr::Lit(Value::Str(s))),
        var_name().prop_map(Expr::Var),
    ];
    leaf.prop_recursive(4, 24, 2, |inner| {
        let unop = prop_oneof![Just(UnOp::Not), Just(UnOp::Neg)];
        let binop = prop_oneof![
            Just(BinOp::Add),
            Just(BinOp::Sub),
            Just(BinOp::Mul),
            Just(BinOp::Div),
            Just(BinOp::Eq),
            Just(BinOp::Ne),
            Just(BinOp::Lt),
            Just(BinOp::Le),
            Just(BinOp::Gt),
            Just(BinOp::Ge),
            Just(BinOp::And),
            Just(BinOp::Or),
        ];
        prop_oneof![
            (unop, inner.clone()).prop_map(|(op, e)| Expr::Unary(op, Box::new(e))),
            (binop, inner.clone(), inner).prop_map(|(op, l, r)| Expr::binary(op, l, r)),
        ]
    })
}

fn custom_data() -> impl Strategy<Value = CustomData> {
    let leaf = prop_oneof![
        Just(CustomData::Null),
        any::<bool>().prop_map(CustomData::Bool),
        any::<i64>().prop_map(CustomData::Int),
        finite_f64().prop_map(CustomData::Float),
        Just(CustomData::Float(f64::INFINITY)),
        Just(CustomData::Float(f64::NEG_INFINITY)),
        any::<String>().prop_map(CustomData::Str),
    ];
    leaf.prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(CustomData::List),
            prop::collection::vec((any::<String>(), inner), 0..4).prop_map(CustomData::Map),
        ]
    })
}

fn curve() -> impl Strategy<Value = Curve> {
    prop_oneof![
        (0..Ease::ALL.len()).prop_map(|i| Curve::Preset(Ease::ALL[i])),
        (finite_f32(), finite_f32(), finite_f32(), finite_f32())
            .prop_map(|(a, b, c, d)| Curve::bezier(a, b, c, d)),
        text().prop_map(Curve::Named),
    ]
}

fn transition() -> impl Strategy<Value = Option<Transition>> {
    let ease = || prop::option::of(curve());
    prop_oneof![
        Just(None),
        (finite_f32(), ease()).prop_map(|(d, e)| Some(Transition {
            ease: e,
            ..Transition::dissolve(d)
        })),
        (finite_f32(), any::<(u8, u8, u8)>(), ease()).prop_map(|(d, (r, g, b), e)| Some(Transition {
            ease: e,
            ..Transition::fade(Color::rgb(r, g, b), d)
        })),
    ]
}

/// Effects and timelines. `usize`s pick a character (modulo the list).
#[derive(Clone, Debug)]
enum FxSpec {
    Move(AnimSel, f32, f32, f32, Option<Curve>, bool),
    Shake(AnimSel, f32, f32, Option<Curve>, bool),
    Zoom(f32, f32, Option<Curve>, bool),
    Flash((u8, u8, u8), f32, Option<Curve>, bool),
    Tint(AnimSel, (u8, u8, u8), f32, f32, Option<Curve>, bool),
    Timeline(f32, u8, u32, bool, Vec<(AnimSel, u8, Vec<(f32, [f32; 4], Option<Curve>)>)>),
}

#[derive(Clone, Copy, Debug)]
enum AnimSel {
    Camera,
    Screen,
    Background,
    Character(usize),
}

fn anim_sel() -> impl Strategy<Value = AnimSel> {
    prop_oneof![
        Just(AnimSel::Camera),
        Just(AnimSel::Screen),
        Just(AnimSel::Background),
        any::<usize>().prop_map(AnimSel::Character),
    ]
}

fn fx_spec() -> impl Strategy<Value = FxSpec> {
    let ease = || prop::option::of(curve());
    let rgb = || any::<(u8, u8, u8)>();
    prop_oneof![
        (anim_sel(), finite_f32(), finite_f32(), finite_f32(), ease(), any::<bool>())
            .prop_map(|(t, x, y, d, e, w)| FxSpec::Move(t, x, y, d, e, w)),
        (anim_sel(), finite_f32(), finite_f32(), ease(), any::<bool>())
            .prop_map(|(t, a, d, e, w)| FxSpec::Shake(t, a, d, e, w)),
        (finite_f32(), finite_f32(), ease(), any::<bool>())
            .prop_map(|(z, d, e, w)| FxSpec::Zoom(z, d, e, w)),
        (rgb(), finite_f32(), ease(), any::<bool>())
            .prop_map(|(c, d, e, w)| FxSpec::Flash(c, d, e, w)),
        (anim_sel(), rgb(), finite_f32(), finite_f32(), ease(), any::<bool>())
            .prop_map(|(t, c, s, d, e, w)| FxSpec::Tint(t, c, s, d, e, w)),
        (
            finite_f32(),
            0u8..4,
            any::<u32>(),
            any::<bool>(),
            prop::collection::vec(
                (
                    anim_sel(),
                    any::<u8>(),
                    prop::collection::vec(
                        (finite_f32(), [finite_f32(), finite_f32(), finite_f32(), finite_f32()], ease()),
                        0..4,
                    ),
                ),
                0..4,
            ),
        )
            .prop_map(|(d, l, n, w, t)| FxSpec::Timeline(d, l, n, w, t)),
    ]
}

fn position() -> impl Strategy<Value = Position> {
    prop_oneof![
        Just(Position::Left),
        Just(Position::Center),
        Just(Position::Right),
        text().prop_map(Position::Custom),
        (finite_f32(), finite_f32()).prop_map(|(x, y)| Position::Free { x, y }),
    ]
}

/// A node with unresolved references: `usize`s index the story's nodes,
/// scenes, characters or assets (modulo their count).
#[derive(Clone, Debug)]
enum Spec {
    Label(String),
    Say(Option<usize>, String),
    Choice(Vec<(String, usize, Option<Expr>)>),
    Jump(usize),
    JumpScene(usize, Option<usize>),
    SetVar(String, Expr),
    /// `$ v += …`-shaped expressions, to exercise the VarOp form.
    Compound(String, BinOp, Expr),
    If(Expr, usize, Option<usize>),
    Bg(usize, Option<Transition>),
    Show(
        usize,
        String,
        Position,
        Option<Transition>,
        Option<f32>,
        Option<f32>,
        bool,
        bool,
    ),
    Hide(usize, Option<Transition>),
    Music(usize, f32, bool),
    Stop(f32),
    Sound(usize),
    Wait(f32),
    Call(usize),
    Return,
    End,
    Custom(String, String, u32, CustomData),
    Fx(FxSpec),
}

fn spec() -> impl Strategy<Value = Spec> {
    let opt_f = prop_oneof![Just(None), finite_f32().prop_map(Some)];
    prop_oneof![
        3 => text().prop_map(Spec::Label),
        3 => (prop::option::of(any::<usize>()), text()).prop_map(|(s, t)| Spec::Say(s, t)),
        1 => prop::collection::vec((text(), any::<usize>(), prop::option::of(expr())), 0..4).prop_map(Spec::Choice),
        1 => any::<usize>().prop_map(Spec::Jump),
        1 => (any::<usize>(), prop::option::of(any::<usize>())).prop_map(|(s, l)| Spec::JumpScene(s, l)),
        1 => (prop_oneof![var_name(), text()], expr()).prop_map(|(v, e)| Spec::SetVar(v, e)),
        1 => (var_name(), prop_oneof![Just(BinOp::Add), Just(BinOp::Sub), Just(BinOp::Mul), Just(BinOp::Div)], expr())
            .prop_map(|(v, op, e)| Spec::Compound(v, op, e)),
        1 => (expr(), any::<usize>(), prop::option::of(any::<usize>())).prop_map(|(c, t, e)| Spec::If(c, t, e)),
        1 => (any::<usize>(), transition()).prop_map(|(a, t)| Spec::Bg(a, t)),
        1 => (any::<usize>(), text(), position(), transition(), opt_f.clone(), opt_f, any::<bool>(), any::<bool>())
            .prop_map(|(c, e, p, t, s, sy, fx, fy)| Spec::Show(c, e, p, t, s, sy, fx, fy)),
        1 => (any::<usize>(), transition()).prop_map(|(c, t)| Spec::Hide(c, t)),
        1 => (any::<usize>(), finite_f32(), any::<bool>()).prop_map(|(a, f, l)| Spec::Music(a, f, l)),
        1 => finite_f32().prop_map(Spec::Stop),
        1 => any::<usize>().prop_map(Spec::Sound),
        1 => finite_f32().prop_map(Spec::Wait),
        1 => any::<usize>().prop_map(Spec::Call),
        1 => Just(Spec::Return),
        1 => Just(Spec::End),
        1 => (text(), text(), any::<u32>(), custom_data()).prop_map(|(p, k, v, d)| Spec::Custom(p, k, v, d)),
        2 => fx_spec().prop_map(Spec::Fx),
    ]
}

const CHARACTER_NAMES: &[&str] = &["Alice", "Bob Smith", "jump", "Alice", "zoé", ""];
const ASSET_PATHS: &[&str] = &["bg/room.png", "music/theme.ogg", "a b.ogg", "bg/room.png"];
const SCENE_NAMES: &[&str] = &["intro", "Chapter 2", "intro", "end", "x.y", ""];

/// The definitions every generated story shares.
fn base_story() -> Story {
    let mut story = Story::new("prop");
    for (i, name) in CHARACTER_NAMES.iter().enumerate() {
        let name_id = story.strings.add(*name);
        story.characters.push(Character {
            id: CharacterId::from_u128(0x1000 + i as u128),
            name_id,
            color: Color::WHITE,
            expressions: Vec::new(),
            default_position: None,
            default_scale: 1.0,
        });
    }
    for (i, path) in ASSET_PATHS.iter().enumerate() {
        story.assets.assets.push(AssetEntry {
            id: AssetId::from_u128(0x2000 + i as u128),
            kind: AssetKind::Image,
            path: (*path).to_owned(),
        });
    }
    story
}

fn build(scenes: Vec<(usize, Vec<Spec>)>) -> Story {
    let mut story = base_story();
    let scene_ids: Vec<SceneId> = (0..scenes.len())
        .map(|i| SceneId::from_u128(0x3000 + i as u128))
        .collect();
    let mut next = 0x10000u128;
    let ids: Vec<Vec<NodeId>> = scenes
        .iter()
        .map(|(_, specs)| {
            specs
                .iter()
                .map(|_| {
                    next += 1;
                    NodeId::from_u128(next)
                })
                .collect()
        })
        .collect();
    let all: Vec<NodeId> = ids.iter().flatten().copied().collect();
    let node = |i: usize| {
        all.get(i % all.len().max(1))
            .copied()
            .unwrap_or(NodeId::from_u128(7))
    };
    let character = |i: usize| story_character(i);
    let asset = |i: usize| AssetId::from_u128(0x2000 + (i % ASSET_PATHS.len()) as u128);
    for (si, (name, specs)) in scenes.into_iter().enumerate() {
        let mut nodes = Vec::new();
        for (ni, spec) in specs.into_iter().enumerate() {
            let kind = match spec {
                Spec::Label(name) => NodeKind::Label { name },
                Spec::Say(speaker, t) => NodeKind::Say {
                    speaker: speaker.map(character),
                    text_id: story.strings.add(t),
                },
                Spec::Choice(options) => NodeKind::Choice {
                    options: options
                        .into_iter()
                        .map(|(t, target, condition)| ChoiceOption {
                            text_id: story.strings.add(t),
                            target: node(target),
                            condition,
                        })
                        .collect(),
                },
                Spec::Jump(t) => NodeKind::Jump { target: node(t) },
                Spec::JumpScene(s, l) => {
                    let scene = scene_ids[s % scene_ids.len()];
                    let target_nodes = &ids[s % scene_ids.len()];
                    NodeKind::JumpScene {
                        scene,
                        label: l
                            .and_then(|l| target_nodes.get(l % target_nodes.len().max(1)).copied()),
                    }
                }
                Spec::SetVar(var, value) => NodeKind::SetVar { var, value },
                Spec::Compound(var, op, rhs) => NodeKind::SetVar {
                    value: Expr::binary(op, Expr::Var(var.clone()), rhs),
                    var,
                },
                Spec::If(condition, t, e) => NodeKind::If {
                    condition,
                    then_target: node(t),
                    else_target: e.map(node),
                },
                Spec::Bg(a, transition) => NodeKind::Background {
                    asset: asset(a),
                    transition,
                },
                Spec::Show(c, expression, position, transition, scale, scale_y, flip_x, flip_y) => {
                    NodeKind::Show {
                        character: character(c),
                        expression,
                        position,
                        transition,
                        scale,
                        scale_y,
                        flip_x,
                        flip_y,
                    }
                }
                Spec::Hide(c, transition) => NodeKind::Hide {
                    character: character(c),
                    transition,
                },
                Spec::Music(a, fade_in, looping) => NodeKind::PlayMusic {
                    asset: asset(a),
                    fade_in,
                    looping,
                },
                Spec::Stop(fade_out) => NodeKind::StopMusic { fade_out },
                Spec::Sound(a) => NodeKind::PlaySound { asset: asset(a) },
                Spec::Wait(seconds) => NodeKind::Wait { seconds },
                Spec::Call(l) => NodeKind::Call { label: node(l) },
                Spec::Return => NodeKind::Return,
                Spec::End => NodeKind::End,
                Spec::Custom(plugin_id, kind, version, data) => NodeKind::Custom {
                    plugin_id,
                    kind,
                    version,
                    data,
                },
                Spec::Fx(fx) => fx_node(fx, &character),
            };
            nodes.push(Node {
                id: ids[si][ni],
                kind,
            });
        }
        story.scenes.push(Scene {
            id: scene_ids[si],
            name: SCENE_NAMES[name % SCENE_NAMES.len()].to_owned(),
            nodes,
        });
    }
    story
}

fn fx_node(fx: FxSpec, character: &dyn Fn(usize) -> CharacterId) -> NodeKind {
    let target = |s: AnimSel| match s {
        AnimSel::Camera => AnimTarget::Camera,
        AnimSel::Screen => AnimTarget::Screen,
        AnimSel::Background => AnimTarget::Background,
        AnimSel::Character(i) => AnimTarget::Character(character(i)),
    };
    let color = |(r, g, b): (u8, u8, u8)| Color::rgb(r, g, b);
    match fx {
        FxSpec::Move(t, dx, dy, duration, ease, wait) => NodeKind::Effect(Effect::Move {
            target: target(t),
            dx,
            dy,
            duration,
            ease,
            wait,
        }),
        FxSpec::Shake(t, amplitude, duration, ease, wait) => NodeKind::Effect(Effect::Shake {
            target: target(t),
            amplitude,
            duration,
            ease,
            wait,
        }),
        FxSpec::Zoom(factor, duration, ease, wait) => {
            NodeKind::Effect(Effect::Zoom { factor, duration, ease, wait })
        }
        FxSpec::Flash(c, duration, ease, wait) => NodeKind::Effect(Effect::Flash {
            color: color(c),
            duration,
            ease,
            wait,
        }),
        FxSpec::Tint(t, c, strength, duration, ease, wait) => NodeKind::Effect(Effect::Tint {
            target: target(t),
            color: color(c),
            strength,
            duration,
            ease,
            wait,
        }),
        FxSpec::Timeline(duration, l, n, wait, tracks) => NodeKind::Timeline {
            duration,
            looping: match l {
                0 => AnimLoop::Once,
                1 => AnimLoop::Loop(n),
                2 => AnimLoop::Forever,
                _ => AnimLoop::PingPong,
            },
            wait,
            tracks: tracks
                .into_iter()
                .map(|(t, p, keys)| {
                    let prop = AnimProp::ALL[p as usize % AnimProp::ALL.len()];
                    let data = match prop {
                        AnimProp::Offset | AnimProp::Scale | AnimProp::Pan => {
                            TrackData::Vec2(Track::new(
                                keys.into_iter()
                                    .map(|(t, v, e)| Key::with_ease(t, Vec2::new(v[0], v[1]), e.unwrap_or_default()))
                                    .collect(),
                            ))
                        }
                        AnimProp::Rotation => TrackData::Angle(Track::new(
                            keys.into_iter()
                                .map(|(t, v, e)| Key::with_ease(t, Angle(v[0]), e.unwrap_or_default()))
                                .collect(),
                        )),
                        AnimProp::Opacity | AnimProp::Shake | AnimProp::Zoom => {
                            TrackData::Scalar(Track::new(
                                keys.into_iter()
                                    .map(|(t, v, e)| Key::with_ease(t, v[0], e.unwrap_or_default()))
                                    .collect(),
                            ))
                        }
                        AnimProp::Tint => TrackData::Color(Track::new(
                            keys.into_iter()
                                .map(|(t, v, e)| {
                                    Key::with_ease(t, Rgba::new(v[0], v[1], v[2], v[3]), e.unwrap_or_default())
                                })
                                .collect(),
                        )),
                    };
                    StoryTrack { target: target(t), prop, data }
                })
                .collect(),
        },
    }
}

fn story_character(i: usize) -> CharacterId {
    CharacterId::from_u128(0x1000 + (i % CHARACTER_NAMES.len()) as u128)
}

fn story() -> impl Strategy<Value = Story> {
    prop::collection::vec((any::<usize>(), prop::collection::vec(spec(), 0..10)), 0..4)
        .prop_map(build)
}

// ------------------------------------------------------------- properties

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, .. ProptestConfig::default() })]

    #[test]
    fn ir_text_ir_is_identity(s in story()) {
        let text = print_story(&s);
        let back = parse(&text, &s, Scope::Story)
            .unwrap_or_else(|e| panic!("{e:?}\n---\n{text}"));
        prop_assert_eq!(&back, &s, "text:\n{}", text);
        // Canonical formatting is stable.
        prop_assert_eq!(print_story(&back), text);
        // Nothing to do.
        prop_assert!(diff(&s, &back).unwrap().is_empty());
    }

    #[test]
    fn each_scene_round_trips_alone(s in story()) {
        for scene in &s.scenes {
            let text = print_scene(&s, scene.id).unwrap();
            let back = parse(&text, &s, Scope::Scene(scene.id))
                .unwrap_or_else(|e| panic!("{e:?}\n---\n{text}"));
            prop_assert_eq!(&back, &s, "text:\n{}", text);
        }
    }

    /// Text edited by removing and reordering lines: the parsed story keeps
    /// every surviving id and the command diff reaches it exactly.
    #[test]
    fn edits_apply_as_commands(s in story(), keep in prop::collection::vec(any::<bool>(), 64), seed in any::<u64>()) {
        let mut edited = s.clone();
        // Drop some nodes and rotate each scene.
        let mut k = keep.iter().cycle();
        for scene in &mut edited.scenes {
            scene.nodes.retain(|_| *k.next().unwrap());
            if !scene.nodes.is_empty() {
                let r = (seed as usize) % scene.nodes.len();
                scene.nodes.rotate_left(r);
            }
        }
        // Retarget what pointed at a dropped node.
        let alive: std::collections::HashSet<NodeId> =
            edited.scenes.iter().flat_map(|s| &s.nodes).map(|n| n.id).collect();
        for n in edited.scenes.iter_mut().flat_map(|s| &mut s.nodes) {
            if n.kind.targets().iter().any(|t| !alive.contains(t)) {
                n.kind = NodeKind::End;
            }
        }
        // Drop a scene sometimes (and what targets it).
        if seed % 3 == 0 && !edited.scenes.is_empty() {
            let gone = edited.scenes.remove((seed as usize / 3) % edited.scenes.len());
            let gone_nodes: std::collections::HashSet<NodeId> = gone.nodes.iter().map(|n| n.id).collect();
            for n in edited.scenes.iter_mut().flat_map(|s| &mut s.nodes) {
                if n.kind.target_scene() == Some(gone.id) || n.kind.targets().iter().any(|t| gone_nodes.contains(t)) {
                    n.kind = NodeKind::End;
                }
            }
        }
        // `edited` uses texts of `s`'s string table, so print against it.
        let mut view = edited.clone();
        view.strings = s.strings.clone();
        let text = print_story(&view);
        let target = parse(&text, &s, Scope::Story)
            .unwrap_or_else(|e| panic!("{e:?}\n---\n{text}"));
        prop_assert_eq!(&target.scenes, &edited.scenes);
        let mut project = Project::new(s.clone());
        apply(&mut project, &text, Scope::Story).unwrap();
        prop_assert_eq!(project.story(), &target);
        // Undo restores the base exactly.
        project.undo().unwrap();
        prop_assert_eq!(project.story(), &s);
    }
}
