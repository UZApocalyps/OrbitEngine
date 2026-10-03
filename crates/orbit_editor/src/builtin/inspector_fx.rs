//! Inspectors of the `Effect` and `Timeline` nodes (ADR 0012). The effect
//! form is the simple one; the timeline form lists tracks and keys with plain
//! fields (the dock editor with a time ruler and scrubbing is M5A-05).

use egui::{ComboBox, DragValue, RichText, Ui};
use orbit_core::{
    Angle, AnimLoop, AnimProp, AnimTarget, Color, Curve, Effect, Key, NodeId, NodeKind, Rgba,
    Story, StoryTrack, Track, TrackData, Vec2,
};
use orbit_editor_api::fields::{self, color_field, seconds_field};
use orbit_editor_api::{EditorContext, Translator, UiText};

use super::fx_text::{effect_kind_key, prop_key, target_name};
use super::inspector::{commit, curves_of, info, tfield};
use super::ui_util::weak;

type Curves = std::collections::BTreeMap<String, Curve>;

/// Every target a property can animate: camera, screen, background and each
/// character of the story.
fn candidate_targets(story: &Story, prop: AnimProp) -> Vec<AnimTarget> {
    [AnimTarget::Camera, AnimTarget::Screen, AnimTarget::Background]
        .into_iter()
        .chain(story.characters.iter().map(|c| AnimTarget::Character(c.id)))
        .filter(|t| prop.applies_to(t))
        .collect()
}

/// A target chooser listing the targets `prop` can animate.
fn target_picker(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    texts: &Translator,
    story: &Story,
    prop: AnimProp,
    current: AnimTarget,
) -> Option<AnimTarget> {
    let mut chosen = current;
    ComboBox::from_id_salt(salt)
        .selected_text(target_name(texts, story, &current))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for t in candidate_targets(story, prop) {
                ui.selectable_value(&mut chosen, t, target_name(texts, story, &t));
            }
        });
    (chosen != current).then_some(chosen)
}

/// The easing of an effect or a key: a collapsible curve editor.
fn ease_editor(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    current: &Option<Curve>,
    curves: &Curves,
) -> Option<Option<Curve>> {
    let ease = current.clone().unwrap_or_default();
    let header = format!(
        "{}: {}",
        ui.t("curve.easing"),
        orbit_editor_api::curve_editor::curve_label(&ease)
    );
    let mut out = None;
    egui::CollapsingHeader::new(header)
        .id_salt((salt, "ease"))
        .show(ui, |ui| {
            let options = orbit_editor_api::curve_editor::CurveEditorOptions {
                graph_size: 110.0,
                ..Default::default()
            };
            if let Some(c) = orbit_editor_api::curve_editor::curve_editor(
                ui,
                (salt, "curve"),
                &ease,
                curves,
                options,
            ) {
                out = Some((c != Curve::LINEAR).then_some(c));
            }
        });
    out
}

fn drag(ui: &mut Ui, value: &mut f32, speed: f64) -> bool {
    ui.add(DragValue::new(value).speed(speed).max_decimals(3))
        .changed()
}

// ---------------------------------------------------------------- effect

#[derive(Clone, Copy, PartialEq)]
enum EffectKind {
    Move,
    Shake,
    Zoom,
    Flash,
    Tint,
}

impl EffectKind {
    const ALL: [EffectKind; 5] = [
        EffectKind::Move,
        EffectKind::Shake,
        EffectKind::Zoom,
        EffectKind::Flash,
        EffectKind::Tint,
    ];

    fn of(e: &Effect) -> Self {
        match e {
            Effect::Move { .. } => EffectKind::Move,
            Effect::Shake { .. } => EffectKind::Shake,
            Effect::Zoom { .. } => EffectKind::Zoom,
            Effect::Flash { .. } => EffectKind::Flash,
            Effect::Tint { .. } => EffectKind::Tint,
        }
    }

    /// A fresh effect of this kind with the timing of `old`.
    fn build(self, old: &Effect, story: &Story) -> Effect {
        let (duration, ease, wait) = (old.duration(), old.ease().cloned(), old.wait());
        let character = story
            .characters
            .first()
            .map_or(AnimTarget::Background, |c| AnimTarget::Character(c.id));
        match self {
            EffectKind::Move => Effect::Move {
                target: character,
                dx: 0.1,
                dy: 0.0,
                duration,
                ease,
                wait,
            },
            EffectKind::Shake => Effect::Shake {
                target: AnimTarget::Camera,
                amplitude: 0.02,
                duration,
                ease,
                wait,
            },
            EffectKind::Zoom => Effect::Zoom {
                factor: 1.2,
                duration,
                ease,
                wait,
            },
            EffectKind::Flash => Effect::Flash {
                color: Color::WHITE,
                duration,
                ease,
                wait,
            },
            EffectKind::Tint => Effect::Tint {
                target: AnimTarget::Screen,
                color: Color::BLACK,
                strength: 0.5,
                duration,
                ease,
                wait,
            },
        }
    }
}

pub(super) fn effect(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, current: Effect) {
    let Some(story) = ctx.story() else { return };
    let texts = ui.texts();
    let curves = curves_of(ctx);
    let mut e = current.clone();

    tfield(ui, "inspector.fx.kind", |ui| {
        let kind = EffectKind::of(&e);
        let mut chosen = kind;
        ComboBox::from_id_salt(("fx_kind", id))
            .selected_text(texts.get(effect_kind_key(&e)))
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for k in EffectKind::ALL {
                    let label = texts.get(effect_kind_key(&k.build(&e, story)));
                    ui.selectable_value(&mut chosen, k, label);
                }
            });
        if chosen != kind {
            e = chosen.build(&e, story);
        }
    });

    match &mut e {
        Effect::Move { target, dx, dy, .. } => {
            tfield(ui, "inspector.fx.target", |ui| {
                if let Some(t) =
                    target_picker(ui, ("fx_target", id), &texts, story, AnimProp::Offset, *target)
                {
                    *target = t;
                }
            });
            tfield(ui, "inspector.fx.dx", |ui| drag(ui, dx, 0.005));
            tfield(ui, "inspector.fx.dy", |ui| drag(ui, dy, 0.005));
        }
        Effect::Shake {
            target, amplitude, ..
        } => {
            tfield(ui, "inspector.fx.target", |ui| {
                if let Some(t) =
                    target_picker(ui, ("fx_target", id), &texts, story, AnimProp::Shake, *target)
                {
                    *target = t;
                }
            });
            tfield(ui, "inspector.fx.amplitude", |ui| drag(ui, amplitude, 0.001));
        }
        Effect::Zoom { factor, .. } => {
            tfield(ui, "inspector.fx.factor", |ui| {
                if drag(ui, factor, 0.01) {
                    *factor = factor.max(0.01);
                }
            });
        }
        Effect::Flash { color, .. } => {
            tfield(ui, "field.color", |ui| {
                if let Some(c) = color_field(ui, *color) {
                    *color = c;
                }
            });
        }
        Effect::Tint {
            target,
            color,
            strength,
            ..
        } => {
            tfield(ui, "inspector.fx.target", |ui| {
                if let Some(t) =
                    target_picker(ui, ("fx_target", id), &texts, story, AnimProp::Tint, *target)
                {
                    *target = t;
                }
            });
            tfield(ui, "field.color", |ui| {
                if let Some(c) = color_field(ui, *color) {
                    *color = c;
                }
            });
            tfield(ui, "inspector.fx.strength", |ui| {
                ui.add(egui::Slider::new(strength, 0.0..=1.0).max_decimals(2));
            });
        }
    }

    tfield(ui, "field.duration", |ui| {
        let mut d = e.duration();
        if let Some(v) = seconds_field(ui, d) {
            d = v;
            match &mut e {
                Effect::Move { duration, .. }
                | Effect::Shake { duration, .. }
                | Effect::Zoom { duration, .. }
                | Effect::Flash { duration, .. }
                | Effect::Tint { duration, .. } => *duration = d,
            }
        }
    });
    let wait_label = ui.t("inspector.fx.wait");
    if let Some(w) = fields::checkbox(ui, e.wait(), &wait_label) {
        match &mut e {
            Effect::Move { wait, .. }
            | Effect::Shake { wait, .. }
            | Effect::Zoom { wait, .. }
            | Effect::Flash { wait, .. }
            | Effect::Tint { wait, .. } => *wait = w,
        }
    }
    if e.duration() > 0.0 {
        let current_ease = e.ease().cloned();
        if let Some(new) = ease_editor(ui, ("fx_ease", id), &current_ease, &curves) {
            match &mut e {
                Effect::Move { ease, .. }
                | Effect::Shake { ease, .. }
                | Effect::Zoom { ease, .. }
                | Effect::Flash { ease, .. }
                | Effect::Tint { ease, .. } => *ease = new,
            }
        }
    }
    info(ui, "inspector.fx.relative_info");

    ui.add_space(6.0);
    let convert = ui
        .small_button(ui.t("inspector.fx.to_timeline"))
        .on_hover_text(ui.t("inspector.fx.to_timeline.tip"))
        .clicked();
    if convert {
        commit(ctx, id, e.to_timeline(), "to_timeline");
    } else if e != current {
        commit(ctx, id, NodeKind::Effect(e), "effect");
    }
}

// -------------------------------------------------------------- timeline

fn prop_data(prop: AnimProp) -> TrackData {
    match prop {
        AnimProp::Offset | AnimProp::Scale | AnimProp::Pan => {
            let one = if prop == AnimProp::Scale { 1.0 } else { 0.0 };
            TrackData::Vec2(Track::new(vec![
                Key::new(0.0, Vec2::new(one, one)),
                Key::new(1.0, Vec2::new(one, one)),
            ]))
        }
        AnimProp::Rotation => TrackData::Angle(Track::new(vec![
            Key::new(0.0, Angle(0.0)),
            Key::new(1.0, Angle(0.0)),
        ])),
        AnimProp::Opacity | AnimProp::Zoom => TrackData::Scalar(Track::new(vec![
            Key::new(0.0, 1.0),
            Key::new(1.0, 1.0),
        ])),
        AnimProp::Shake => TrackData::Scalar(Track::new(vec![
            Key::new(0.0, 0.0),
            Key::new(1.0, 0.0),
        ])),
        AnimProp::Tint => TrackData::Color(Track::new(vec![
            Key::new(0.0, Rgba::new(0.0, 0.0, 0.0, 0.0)),
            Key::new(1.0, Rgba::new(0.0, 0.0, 0.0, 0.5)),
        ])),
    }
}

/// Edits the keys of one track: time (kept between its neighbours), value,
/// easing towards the next key, removal and addition. `edit` draws the value.
fn keys_ui<T: Clone>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    texts: &Translator,
    curves: &Curves,
    keys: &mut Vec<Key<T>>,
    edit: &mut dyn FnMut(&mut Ui, &mut T) -> bool,
) -> bool {
    let mut changed = false;
    let mut remove = None;
    for i in 0..keys.len() {
        let lo = if i == 0 { 0.0 } else { keys[i - 1].t + 0.01 };
        let hi = keys.get(i + 1).map_or(3600.0, |k| k.t - 0.01).max(lo);
        ui.horizontal(|ui| {
            let mut t = keys[i].t;
            if ui
                .add(
                    DragValue::new(&mut t)
                        .speed(0.02)
                        .range(lo..=hi)
                        .suffix(" s")
                        .max_decimals(2),
                )
                .on_hover_text(texts.get("inspector.fx.key_time"))
                .changed()
            {
                keys[i].t = t;
                changed = true;
            }
            if edit(ui, &mut keys[i].value) {
                changed = true;
            }
            if i + 1 < keys.len() {
                let label = orbit_editor_api::curve_editor::curve_label(&keys[i].ease);
                ComboBox::from_id_salt((salt, "key_ease", i))
                    .selected_text(label)
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for e in orbit_core::Ease::ALL {
                            let c = Curve::Preset(e);
                            if ui.selectable_label(keys[i].ease == c, e.name()).clicked() {
                                keys[i].ease = c;
                                changed = true;
                            }
                        }
                        for name in curves.keys() {
                            let c = Curve::Named(name.clone());
                            if ui.selectable_label(keys[i].ease == c, name).clicked() {
                                keys[i].ease = c;
                                changed = true;
                            }
                        }
                    })
                    .response
                    .on_hover_text(texts.get("inspector.fx.key_ease"));
            }
            if ui
                .small_button(orbit_editor_api::icons::TRASH)
                .on_hover_text(texts.get("inspector.fx.remove_key"))
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        keys.remove(i);
        changed = true;
    }
    if ui
        .small_button(format!(
            "{}  {}",
            orbit_editor_api::icons::PLUS,
            texts.get("inspector.fx.add_key")
        ))
        .clicked()
    {
        let (t, value, ease) = match keys.last() {
            Some(k) => (k.t + 1.0, k.value.clone(), Curve::default()),
            None => {
                // An empty track: the caller re-seeds it from its property.
                return changed;
            }
        };
        keys.push(Key::with_ease(t, value, ease));
        changed = true;
    }
    changed
}

fn track_ui(
    ui: &mut Ui,
    ctx_story: &Story,
    texts: &Translator,
    curves: &Curves,
    salt: (NodeId, usize),
    track: &mut StoryTrack,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(texts.get("inspector.fx.prop")).small().color(weak(ui)));
        let mut prop = track.prop;
        ComboBox::from_id_salt((salt, "prop"))
            .selected_text(texts.get(prop_key(prop)))
            .width(150.0)
            .show_ui(ui, |ui| {
                for p in AnimProp::ALL {
                    if p.applies_to(&track.target) {
                        ui.selectable_value(&mut prop, p, texts.get(prop_key(p)));
                    }
                }
            });
        if prop != track.prop {
            track.prop = prop;
            track.data = prop_data(prop);
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(texts.get("inspector.fx.target")).small().color(weak(ui)));
        if let Some(t) = target_picker(
            ui,
            (salt, "target"),
            texts,
            ctx_story,
            track.prop,
            track.target,
        ) {
            track.target = t;
            changed = true;
        }
    });
    let pair = |ui: &mut Ui, v: &mut Vec2| {
        let a = drag(ui, &mut v.x, 0.005);
        let b = drag(ui, &mut v.y, 0.005);
        a || b
    };
    match &mut track.data {
        TrackData::Vec2(t) => {
            changed |= keys_ui(ui, (salt, "k"), texts, curves, &mut t.keys, &mut |ui, v| pair(ui, v));
        }
        TrackData::Scalar(t) => {
            changed |= keys_ui(ui, (salt, "k"), texts, curves, &mut t.keys, &mut |ui, v| drag(ui, v, 0.01));
        }
        TrackData::Angle(t) => {
            changed |= keys_ui(ui, (salt, "k"), texts, curves, &mut t.keys, &mut |ui, v| {
                ui.add(DragValue::new(&mut v.0).speed(1.0).suffix("°").max_decimals(1))
                    .changed()
            });
        }
        TrackData::Color(t) => {
            changed |= keys_ui(ui, (salt, "k"), texts, curves, &mut t.keys, &mut |ui, v| {
                let mut rgba = [v.r, v.g, v.b, v.a];
                let response = ui.color_edit_button_rgba_unmultiplied(&mut rgba);
                if response.changed() {
                    *v = Rgba::new(rgba[0], rgba[1], rgba[2], rgba[3]);
                    true
                } else {
                    false
                }
            });
        }
        TrackData::Name(_) => {}
    }
    changed
}

pub(super) fn timeline(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    duration: f32,
    looping: AnimLoop,
    wait: bool,
    tracks: Vec<StoryTrack>,
) {
    let Some(story) = ctx.story() else { return };
    let texts = ui.texts();
    let curves = curves_of(ctx);
    let (mut d, mut lp, mut w, mut tr) = (duration, looping, wait, tracks.clone());

    tfield(ui, "field.duration", |ui| {
        if let Some(v) = seconds_field(ui, d) {
            d = v;
        }
    });
    tfield(ui, "inspector.fx.loop", |ui| {
        #[derive(Clone, Copy, PartialEq)]
        enum Mode {
            Once,
            Repeat,
            Forever,
            PingPong,
        }
        let mode_of = |l: AnimLoop| match l {
            AnimLoop::Once => Mode::Once,
            AnimLoop::Loop(_) => Mode::Repeat,
            AnimLoop::Forever => Mode::Forever,
            AnimLoop::PingPong => Mode::PingPong,
        };
        let name = |m: Mode| {
            texts.get(match m {
                Mode::Once => "inspector.fx.loop.once",
                Mode::Repeat => "inspector.fx.loop.repeat",
                Mode::Forever => "inspector.fx.loop.forever",
                Mode::PingPong => "inspector.fx.loop.ping_pong",
            })
        };
        let mode = mode_of(lp);
        let mut chosen = mode;
        ComboBox::from_id_salt(("tl_loop", id))
            .selected_text(name(mode))
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for m in [Mode::Once, Mode::Repeat, Mode::Forever, Mode::PingPong] {
                    ui.selectable_value(&mut chosen, m, name(m));
                }
            });
        if chosen != mode {
            lp = match chosen {
                Mode::Once => AnimLoop::Once,
                Mode::Repeat => AnimLoop::Loop(2),
                Mode::Forever => AnimLoop::Forever,
                Mode::PingPong => AnimLoop::PingPong,
            };
        }
        if let AnimLoop::Loop(n) = &mut lp {
            ui.horizontal(|ui| {
                ui.label(texts.get("inspector.fx.repeat"));
                let mut v = *n;
                if ui.add(DragValue::new(&mut v).range(1..=1000)).changed() {
                    *n = v;
                }
            });
        }
    });
    let endless = matches!(lp, AnimLoop::Forever | AnimLoop::PingPong);
    if endless {
        // An endless animation cannot hold the story.
        w = false;
        info(ui, "inspector.fx.wait_forever");
    } else {
        let label = ui.t("inspector.fx.wait");
        if let Some(v) = fields::checkbox(ui, w, &label) {
            w = v;
        }
    }

    ui.add_space(6.0);
    if tr.is_empty() {
        info(ui, "inspector.fx.no_tracks");
    }
    let mut remove = None;
    for i in 0..tr.len() {
        let header = format!(
            "{}  ·  {}  ·  {}",
            target_name(&texts, story, &tr[i].target),
            texts.get(prop_key(tr[i].prop)),
            texts.plural(
                "inspector.fx.keys",
                match &tr[i].data {
                    TrackData::Scalar(t) => t.keys.len(),
                    TrackData::Vec2(t) => t.keys.len(),
                    TrackData::Color(t) => t.keys.len(),
                    TrackData::Angle(t) => t.keys.len(),
                    TrackData::Name(t) => t.keys.len(),
                },
                &[]
            )
        );
        egui::CollapsingHeader::new(header)
            .id_salt(("tl_track", id, i))
            .default_open(tr.len() <= 3)
            .show(ui, |ui| {
                track_ui(ui, story, &texts, &curves, (id, i), &mut tr[i]);
                if ui
                    .small_button(format!(
                        "{}  {}",
                        orbit_editor_api::icons::TRASH,
                        texts.get("inspector.fx.remove_track")
                    ))
                    .clicked()
                {
                    remove = Some(i);
                }
            });
    }
    if let Some(i) = remove {
        tr.remove(i);
    }
    if ui
        .small_button(format!(
            "{}  {}",
            orbit_editor_api::icons::PLUS,
            ui.t("inspector.fx.add_track")
        ))
        .clicked()
    {
        tr.push(StoryTrack {
            target: AnimTarget::Background,
            prop: AnimProp::Offset,
            data: prop_data(AnimProp::Offset),
        });
    }

    if d != duration || lp != looping || w != wait || tr != tracks {
        commit(
            ctx,
            id,
            NodeKind::Timeline {
                duration: d,
                looping: lp,
                wait: w,
                tracks: tr,
            },
            "timeline",
        );
    }
}
