//! Clip editing UI of the rig editor (M5B-06): the **Clips** panel (list,
//! properties, cross-fade preview) and the **Timeline** panel (tracks for
//! bones, slots, deforms and the draw order, built on the widgets the story
//! timeline shares in `timeline_ui`), plus the animation state the viewport
//! reads: the playhead, play / loop, auto-key and the cross-fade preview.
//!
//! The clip data model and the key recording are in [`super::clips`]
//! (headless). Every edit goes through [`RigDoc::edit`]; a key drag is one
//! gesture (one undo step), like the bone handles of the viewport.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use egui::{ComboBox, DragValue, Id, Rect, RichText, Sense, Ui, pos2, vec2};
use orbit_anim::rig::{Attachment, AttachmentId, BoneId, Rig, RigClip, RigPose, SlotId, Xform};
use orbit_anim::{Curve, Loop};
use orbit_editor_api::curve_editor::{CurveEditorOptions, curve_editor};
use orbit_editor_api::{EditorContext, icons};

use super::clips::{self, Channel, KeyValue, Target};
use super::model::{self, OpError};
use super::panels::{name_field, report};
use super::{State, set_pose_mode};
use crate::builtin::timeline_model::{self as tm, KeyRef, SNAP_PX, TimeView};
use crate::builtin::timeline_ui::{self as ui_w, LABEL_W, ROW_H, RULER_H, fmt_time, row_rect};
use crate::builtin::ui_util::{danger, faint, section_title, weak};

const INSPECTOR_H: f32 = 190.0;

// ------------------------------------------------------------------ state

/// A cross-fade preview: `from` plays, then `to` fades in (see
/// [`clips::crossfade`]).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Fade {
    pub from: String,
    pub to: String,
    pub blend: f32,
    pub hold: f32,
    pub clock: f32,
    pub playing: bool,
}

#[derive(Clone)]
enum Drag {
    Keys {
        /// The clip as it was when the drag began: each frame is computed
        /// from it, so the drag never drifts.
        origin: RigClip,
        sel: BTreeSet<KeyRef>,
        key_time: f32,
        grab_time: f32,
    },
    Marquee {
        start: egui::Pos2,
        add: bool,
        base: BTreeSet<KeyRef>,
    },
    Pan,
}

/// Everything the clip editor remembers between frames (never saved).
pub(super) struct AnimState {
    /// The clip being edited and previewed (`None`: setup pose).
    pub clip: Option<String>,
    pub playhead: f32,
    pub playing: bool,
    pub loop_play: bool,
    /// Posing in pose mode records keys at the playhead.
    pub auto_key: bool,
    pub snap: bool,
    pub fade: Option<Fade>,
    /// Cross-fade form.
    pub fade_to: String,
    pub fade_blend: f32,
    pub fade_hold: f32,
    /// The story's named curves (to resolve and to pick).
    pub curves: Rc<BTreeMap<String, Curve>>,
    view: TimeView,
    fitted: bool,
    sel: BTreeSet<KeyRef>,
    track: Option<usize>,
    drag: Option<Drag>,
    deform_vertex: usize,
    last_tick: u64,
}

impl Default for AnimState {
    fn default() -> Self {
        Self {
            clip: None,
            playhead: 0.0,
            playing: false,
            loop_play: true,
            auto_key: false,
            snap: true,
            fade: None,
            fade_to: String::new(),
            fade_blend: 0.3,
            fade_hold: 0.5,
            curves: Rc::new(BTreeMap::new()),
            view: TimeView::default(),
            fitted: false,
            sel: BTreeSet::new(),
            track: None,
            drag: None,
            deform_vertex: 0,
            last_tick: 0,
        }
    }
}

impl AnimState {
    /// A new document: forget the clip and the preview, keep the settings.
    pub fn reset(&mut self) {
        *self = AnimState {
            loop_play: self.loop_play,
            auto_key: self.auto_key,
            snap: self.snap,
            curves: self.curves.clone(),
            ..AnimState::default()
        };
    }
}

impl State {
    /// The clip being edited, in the document.
    pub(super) fn clip(&self) -> Option<&RigClip> {
        self.doc.as_ref()?.rig().clips.get(self.anim.clip.as_ref()?)
    }

    /// The animated pose shown in pose mode (the clip at the playhead, or the
    /// cross-fade preview); `None` when nothing is previewed.
    pub(super) fn anim_pose(&self) -> Option<RigPose> {
        if !self.pose_mode {
            return None;
        }
        let prepared = self.prepared.as_ref()?;
        let curves = self.anim.curves.clone();
        let resolve = |n: &str| curves.get(n);
        if let Some(f) = &self.anim.fade {
            let state = clips::crossfade(&f.from, &f.to, f.blend, f.hold);
            return Some(prepared.sample_blend_with(&state, f.clock, resolve));
        }
        let clip = self.clip()?;
        Some(prepared.sample_clip_local_with(clip, self.anim.playhead, resolve))
    }

    /// Whether posing records keys right now.
    pub(super) fn recording(&self) -> bool {
        self.pose_mode && self.anim.auto_key && self.anim.fade.is_none() && self.clip().is_some()
    }

    pub(super) fn set_playhead(&mut self, t: f32) {
        let t = tm::round_time(t);
        if (t - self.anim.playhead).abs() > 1e-6 {
            self.anim.playhead = t;
            // A throw-away pose belongs to one instant.
            self.pose.clear();
            self.pose_rev += 1;
        }
    }

    pub(super) fn select_clip(&mut self, name: Option<String>) {
        if self.anim.clip == name {
            return;
        }
        self.anim.clip = name;
        self.anim.sel.clear();
        self.anim.track = None;
        self.anim.drag = None;
        self.anim.playing = false;
        self.anim.fade = None;
        self.anim.fitted = false;
        self.pose.clear();
        self.pose_rev += 1;
        if self.anim.clip.is_some() {
            set_pose_mode(self, true);
        }
    }

    /// Records a bone's local transform at the playhead (inside the open
    /// gesture, so a whole drag is one undo step).
    pub(super) fn record_bone_edit(&mut self, id: &BoneId, x: Xform) {
        let Some(name) = self.anim.clip.clone() else { return };
        let t = self.anim.playhead;
        let curves = self.anim.curves.clone();
        let Some(doc) = self.doc.as_mut() else { return };
        doc.begin_gesture();
        let _ = doc.edit(|rig| {
            let bone = model::bone(rig, id).cloned().ok_or(OpError::Unknown)?;
            let clip = rig.clips.get_mut(&name).ok_or(OpError::Unknown)?;
            clips::record_bone(clip, &bone, &x, t, |n| curves.get(n));
            Ok(())
        });
        self.pose_rev += 1;
    }

    /// Keys the throw-away pose (auto-key off) at the playhead, then drops it.
    pub(super) fn key_pose(&mut self) -> usize {
        let Some(name) = self.anim.clip.clone() else { return 0 };
        let t = self.anim.playhead;
        let curves = self.anim.curves.clone();
        let poses: Vec<(BoneId, Xform)> = self.pose.iter().map(|(k, v)| (k.clone(), *v)).collect();
        let Some(doc) = self.doc.as_mut() else { return 0 };
        let mut n = 0;
        let _ = doc.edit(|rig| {
            for (id, x) in &poses {
                let Some(bone) = model::bone(rig, id).cloned() else { continue };
                let clip = rig.clips.get_mut(&name).ok_or(OpError::Unknown)?;
                n += usize::from(clips::record_bone(clip, &bone, x, t, |n| curves.get(n)));
            }
            Ok(())
        });
        self.pose.clear();
        self.pose_rev += 1;
        n
    }

    /// Replaces the edited clip with `clip` (one undo step, or part of the
    /// open gesture).
    fn commit_clip(&mut self, clip: RigClip) {
        let Some(name) = self.anim.clip.clone() else { return };
        if let Some(doc) = self.doc.as_mut() {
            let _ = doc.edit(|rig| {
                *rig.clips.get_mut(&name).ok_or(OpError::Unknown)? = clip;
                Ok(())
            });
        }
        self.pose_rev += 1;
    }

    /// Runs `f` on the edited clip (and the rig, read only) as one step.
    fn edit_clip(&mut self, f: impl FnOnce(&Rig, &mut RigClip) -> bool) {
        let Some(name) = self.anim.clip.clone() else { return };
        let Some(doc) = self.doc.as_ref() else { return };
        let Some(mut clip) = doc.rig().clips.get(&name).cloned() else { return };
        if f(doc.rig(), &mut clip) {
            self.commit_clip(clip);
        }
    }
}

/// Advances the playhead and the cross-fade clock (once per egui pass, from
/// whichever panel is drawn first).
pub(super) fn tick(ui: &Ui, st: &mut State) {
    let pass = ui.ctx().cumulative_pass_nr();
    if st.anim.last_tick == pass {
        return;
    }
    st.anim.last_tick = pass;
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    if st.anim.playing {
        match st.clip().map(|c| c.duration) {
            Some(d) => {
                let (t, on) = ui_w::advance(st.anim.playhead, dt, d, st.anim.loop_play);
                st.anim.playing = on;
                st.set_playhead(t);
                ui.ctx().request_repaint();
            }
            None => st.anim.playing = false,
        }
    }
    let looping = st.anim.loop_play;
    if let Some(f) = &mut st.anim.fade
        && f.playing
    {
        f.clock += dt;
        let len = clips::crossfade_length(f.hold, f.blend);
        if f.clock >= len {
            if looping {
                f.clock = 0.0;
            } else {
                f.clock = len;
                f.playing = false;
            }
        }
        st.pose_rev += 1;
        ui.ctx().request_repaint();
    }
}

/// Keeps the clip, the selection and the drag valid when the document
/// changes under us (undo, deleted bones).
fn validate(st: &mut State) {
    let exists = st.anim.clip.as_ref().is_some_and(|n| st.doc.as_ref().is_some_and(|d| d.rig().clips.contains_key(n)));
    if !exists && st.anim.clip.is_some() {
        st.select_clip(None);
    }
    if let Some(f) = &st.anim.fade {
        let ok = |n: &str| st.doc.as_ref().is_some_and(|d| d.rig().clips.contains_key(n));
        if !ok(&f.from) || !ok(&f.to) {
            st.anim.fade = None;
            st.pose_rev += 1;
        }
    }
    let sel_ok: Vec<KeyRef> = match st.clip() {
        Some(c) => st
            .anim
            .sel
            .iter()
            .copied()
            .filter(|&(t, k)| c.tracks.get(t).is_some_and(|tr| k < clips::key_count(tr)))
            .collect(),
        None => Vec::new(),
    };
    st.anim.sel = sel_ok.into_iter().collect();
    let tracks = st.clip().map_or(0, |c| c.tracks.len());
    if st.anim.track.is_some_and(|t| t >= tracks) {
        st.anim.track = None;
    }
}

fn refresh_curves(ctx: &EditorContext<'_>, st: &mut State) {
    if let Some(story) = ctx.story()
        && *st.anim.curves != story.curves
    {
        st.anim.curves = Rc::new(story.curves.clone());
        st.pose_rev += 1;
    }
}

fn bone_name(rig: &Rig, id: &BoneId) -> String {
    model::bone(rig, id).map_or_else(|| id.to_string(), |b| b.name.clone())
}

fn slot_name(rig: &Rig, id: &SlotId) -> String {
    model::slot(rig, id).map_or_else(|| id.to_string(), |s| s.name.clone())
}

fn channel_key(c: Channel) -> &'static str {
    match c {
        Channel::Rotate => "rig.clip.chan.rotate",
        Channel::Translate => "rig.clip.chan.translate",
        Channel::Scale => "rig.clip.chan.scale",
        Channel::Attachment => "rig.clip.chan.attachment",
        Channel::Color => "rig.clip.chan.color",
        Channel::Deform => "rig.clip.chan.deform",
        Channel::DrawOrder => "rig.clip.chan.draw_order",
    }
}

fn track_label(ctx: &EditorContext<'_>, rig: &Rig, t: &orbit_anim::rig::RigTrack) -> String {
    let (ch, target) = clips::track_kind(t);
    let who = match &target {
        Target::Bone(b) => bone_name(rig, b),
        Target::Slot(s) => slot_name(rig, s),
        Target::Deform(s, a) => format!("{} / {}", slot_name(rig, s), a),
        Target::DrawOrder => String::new(),
    };
    let what = ctx.t(channel_key(ch));
    if who.is_empty() { what } else { format!("{who}  ·  {what}") }
}

/// Every channel that can be added, for the "add track" menu.
fn addable(rig: &Rig) -> Vec<(String, Vec<(Channel, Target)>)> {
    let mut out: Vec<(String, Vec<(Channel, Target)>)> = Vec::new();
    for b in &rig.bones {
        out.push((
            b.name.clone(),
            [Channel::Rotate, Channel::Translate, Channel::Scale]
                .into_iter()
                .map(|c| (c, Target::Bone(b.id.clone())))
                .collect(),
        ));
    }
    for s in &rig.slots {
        let mut v = vec![(Channel::Attachment, Target::Slot(s.id.clone())), (Channel::Color, Target::Slot(s.id.clone()))];
        let mut seen = BTreeSet::new();
        for skin in &rig.skins {
            for (id, a) in &skin.attachments {
                if matches!(a, Attachment::Mesh(_)) && seen.insert(id.clone()) {
                    v.push((Channel::Deform, Target::Deform(s.id.clone(), id.clone())));
                }
            }
        }
        out.push((s.name.clone(), v));
    }
    out
}

// ------------------------------------------------------------ clips panel

pub(super) fn clips_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, state: &Rc<RefCell<State>>) {
    let mut guard = state.borrow_mut();
    let st = &mut *guard;
    st.begin_frame(ui);
    refresh_curves(ctx, st);
    tick(ui, st);
    validate(st);
    if st.doc.is_none() {
        super::panels::empty_state(ui, ctx, st);
        return;
    }
    let names: Vec<String> = st.doc.as_ref().map(|d| d.rig().clips.keys().cloned().collect()).unwrap_or_default();
    let current = st.anim.clip.clone();

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if ui.button(format!("{}  {}", icons::PLUS, ctx.t("rig.clip.add"))).clicked()
            && let Some(doc) = st.doc.as_mut()
            && let Some(n) = report(ctx, doc.edit(|r| clips::add_clip(r, "clip")))
        {
            st.select_clip(Some(n));
        }
        if ui
            .add_enabled(current.is_some(), egui::Button::new(icons::COPY))
            .on_hover_text(ctx.t("rig.clip.duplicate"))
            .clicked()
            && let (Some(from), Some(doc)) = (current.clone(), st.doc.as_mut())
            && let Some(n) = report(ctx, doc.edit(|r| clips::duplicate_clip(r, &from)))
        {
            st.select_clip(Some(n));
        }
        if ui
            .add_enabled(current.is_some(), egui::Button::new(icons::TRASH))
            .on_hover_text(ctx.t("rig.clip.delete"))
            .clicked()
            && let (Some(name), Some(doc)) = (current.clone(), st.doc.as_mut())
        {
            report(ctx, doc.edit(|r| clips::delete_clip(r, &name)));
        }
    });
    ui.separator();

    // ---- list
    let mut pick: Option<Option<String>> = None;
    egui::ScrollArea::vertical().id_salt("rig_clip_list").max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
        if ui.selectable_label(current.is_none(), format!("{}  {}", icons::BONE, ctx.t("rig.clip.setup"))).clicked() {
            pick = Some(None);
        }
        for n in &names {
            let label = format!("{}  {}", icons::FILM_STRIP, n);
            if ui.selectable_label(current.as_deref() == Some(n), label).clicked() {
                pick = Some(Some(n.clone()));
            }
        }
        if names.is_empty() {
            ui.label(RichText::new(ctx.t("rig.clip.none")).color(weak(ui)));
        }
    });
    if let Some(p) = pick {
        st.select_clip(p);
    }

    // ---- properties of the selected clip
    if let Some(name) = current.clone()
        && let Some(clip) = st.doc.as_ref().and_then(|d| d.rig().clips.get(&name)).cloned()
    {
        ui.separator();
        section_title(ui, &ctx.t("rig.clip.properties"));
        if let Some(new) = name_field(ui, ("rig_clip_name", &name), &name)
            && let Some(doc) = st.doc.as_mut()
            && report(ctx, doc.edit(|r| clips::rename_clip(r, &name, &new))).is_some()
        {
            st.anim.clip = Some(new.trim().to_owned());
        }
        ui.horizontal(|ui| {
            ui.label(ctx.t("rig.clip.duration"));
            let mut d = clip.duration;
            if ui.add(DragValue::new(&mut d).speed(0.05).range(0.0..=3600.0).suffix(" s").max_decimals(3)).changed()
                && let Some(doc) = st.doc.as_mut()
            {
                let _ = doc.edit(|r| clips::set_duration(r, &name, d));
            }
            if ui.small_button(ctx.t("rig.clip.fit")).on_hover_text(ctx.t("rig.clip.fit_tip")).clicked() {
                let end = clips::last_key_time(&clip);
                if end > 0.0
                    && let Some(doc) = st.doc.as_mut()
                {
                    let _ = doc.edit(|r| clips::set_duration(r, &name, end));
                }
            }
        });
        let mut kind = match clip.looping {
            Loop::Once => 0,
            Loop::Loop(_) => 1,
            Loop::Forever => 2,
            Loop::PingPong => 3,
        };
        let labels = ["rig.clip.loop.once", "rig.clip.loop.count", "rig.clip.loop.forever", "rig.clip.loop.pingpong"];
        let mut count = if let Loop::Loop(n) = clip.looping { n } else { 2 };
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(ctx.t("rig.clip.loop"));
            ComboBox::from_id_salt("rig_clip_loop").selected_text(ctx.t(labels[kind])).show_ui(ui, |ui| {
                for (i, l) in labels.iter().enumerate() {
                    changed |= ui.selectable_value(&mut kind, i, ctx.t(l)).changed();
                }
            });
            if kind == 1 {
                changed |= ui.add(DragValue::new(&mut count).range(1..=999).suffix("x")).changed();
            }
        });
        if changed && let Some(doc) = st.doc.as_mut() {
            let looping = match kind {
                0 => Loop::Once,
                1 => Loop::Loop(count.max(1)),
                2 => Loop::Forever,
                _ => Loop::PingPong,
            };
            let _ = doc.edit(|r| clips::set_loop(r, &name, looping));
        }
    }

    // ---- cross-fade preview
    ui.separator();
    egui::CollapsingHeader::new(RichText::new(ctx.t("rig.clip.fade")).strong()).default_open(false).show(ui, |ui| {
        crossfade_form(ui, ctx, st, &names, current.as_deref());
    });
}

fn crossfade_form(ui: &mut Ui, ctx: &EditorContext<'_>, st: &mut State, names: &[String], current: Option<&str>) {
    if names.len() < 2 {
        ui.label(RichText::new(ctx.t("rig.clip.fade_need_two")).color(weak(ui)));
        return;
    }
    if !names.contains(&st.anim.fade_to) {
        st.anim.fade_to = names.iter().find(|n| Some(n.as_str()) != current).cloned().unwrap_or_default();
    }
    let from = current.map_or_else(|| names[0].clone(), str::to_owned);
    ui.label(RichText::new(ctx.tf("rig.clip.fade_from", &[("clip", &from)])).small());
    ComboBox::from_id_salt("rig_fade_to").selected_text(st.anim.fade_to.clone()).show_ui(ui, |ui| {
        for n in names {
            ui.selectable_value(&mut st.anim.fade_to, n.clone(), n);
        }
    });
    ui.horizontal(|ui| {
        ui.label(ctx.t("rig.clip.fade_hold"));
        ui.add(DragValue::new(&mut st.anim.fade_hold).speed(0.05).range(0.0..=30.0).suffix(" s"));
        ui.label(ctx.t("rig.clip.fade_blend"));
        ui.add(DragValue::new(&mut st.anim.fade_blend).speed(0.02).range(0.0..=10.0).suffix(" s"));
    });
    ui.horizontal(|ui| {
        let running = st.anim.fade.is_some();
        if ui.button(format!("{}  {}", icons::PLAY, ctx.t("rig.clip.fade_play"))).clicked() {
            st.anim.fade = Some(Fade {
                from,
                to: st.anim.fade_to.clone(),
                blend: st.anim.fade_blend,
                hold: st.anim.fade_hold,
                clock: 0.0,
                playing: true,
            });
            st.anim.playing = false;
            set_pose_mode(st, true);
            st.pose.clear();
            st.pose_rev += 1;
        }
        if ui.add_enabled(running, egui::Button::new(format!("{}  {}", icons::STOP, ctx.t("rig.clip.fade_stop")))).clicked() {
            st.anim.fade = None;
            st.pose_rev += 1;
        }
    });
    if let Some(f) = st.anim.fade.clone() {
        let len = clips::crossfade_length(f.hold, f.blend);
        let mut clock = f.clock;
        if ui.add(egui::Slider::new(&mut clock, 0.0..=len).suffix(" s")).changed()
            && let Some(live) = st.anim.fade.as_mut()
        {
            live.clock = clock;
            live.playing = false;
            st.pose_rev += 1;
        }
    }
}

// ---------------------------------------------------------- timeline panel

pub(super) fn timeline_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, state: &Rc<RefCell<State>>) {
    let mut guard = state.borrow_mut();
    let st = &mut *guard;
    st.begin_frame(ui);
    refresh_curves(ctx, st);
    tick(ui, st);
    validate(st);
    if st.doc.is_none() {
        super::panels::empty_state(ui, ctx, st);
        return;
    }
    let (Some(name), Some(clip)) = (st.anim.clip.clone(), st.clip().cloned()) else {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(ctx.t("rig.clip.select_hint")).color(weak(ui))));
        return;
    };
    let rig = st.doc.as_ref().map(|d| d.rig().clone()).expect("document");

    if !st.anim.fitted {
        let w = ui.available_width() - LABEL_W;
        st.anim.view = TimeView::fit(clip.duration.max(clips::last_key_time(&clip)), w);
        st.anim.fitted = true;
    }
    toolbar(ui, ctx, st, &rig, &name, &clip);
    handle_keys(ui, st, &clip);

    let reserve = if st.anim.sel.len() == 1 || st.anim.track.is_some() { INSPECTOR_H } else { 0.0 };
    let rows_h = (ui.available_height() - reserve - RULER_H - 6.0).max(ROW_H * 2.0);
    let full_w = ui.available_width();
    let canvas_w = (full_w - LABEL_W).max(80.0);

    let (ruler_all, _) = ui.allocate_exact_size(vec2(full_w, RULER_H), Sense::hover());
    let ruler = Rect::from_min_size(pos2(ruler_all.left() + LABEL_W, ruler_all.top()), vec2(canvas_w, RULER_H));
    ui_w::draw_ruler(ui, ruler, &st.anim.view, clip.duration);
    if let Some(t) = ui_w::scrub_ruler(ui, ruler, Id::new("rig_tl_ruler"), &st.anim.view, clip.duration) {
        st.anim.playing = false;
        let t = snap_playhead(st, &clip, t);
        st.set_playhead(t);
    }

    let mut canvas = Rect::NOTHING;
    egui::ScrollArea::vertical()
        .id_salt("rig_tl_rows")
        .max_height(rows_h)
        .auto_shrink([false, false])
        .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR | egui::scroll_area::ScrollSource::MOUSE_WHEEL)
        .show(ui, |ui| {
            let rows = clip.tracks.len().max(1);
            let (area, _) =
                ui.allocate_exact_size(vec2(ui.available_width().max(full_w), ROW_H * rows as f32 + 4.0), Sense::hover());
            canvas = Rect::from_min_max(pos2(area.left() + LABEL_W, area.top()), area.right_bottom());
            draw_rows(ui, ctx, st, &rig, &clip, area, canvas);
            canvas_input(ui, st, &clip, area, canvas);
        });
    ui_w::wheel_nav(ui, &mut st.anim.view, ruler, canvas.bottom());

    if reserve > 0.0 {
        ui.separator();
        egui::ScrollArea::vertical().id_salt("rig_tl_inspector").auto_shrink([false, false]).show(ui, |ui| {
            inspector(ui, ctx, st, &rig, &clip);
        });
    }
}

fn toggle(ui: &mut Ui, on: &mut bool, icon: &str, tip: &str) {
    if ui.selectable_label(*on, RichText::new(icon).size(15.0)).on_hover_text(tip).clicked() {
        *on = !*on;
    }
}

fn toolbar(ui: &mut Ui, ctx: &EditorContext<'_>, st: &mut State, rig: &Rig, name: &str, clip: &RigClip) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{}  {}", icons::FILM_STRIP, name)).strong());
        ui.separator();
        let play = if st.anim.playing { icons::PAUSE } else { icons::PLAY };
        if ui.button(RichText::new(play).size(15.0)).on_hover_text(ctx.t("rig.clip.play_tip")).clicked() {
            if !st.anim.playing && st.anim.playhead >= clip.duration {
                st.set_playhead(0.0);
            }
            st.anim.fade = None;
            st.anim.playing = !st.anim.playing;
            if st.anim.playing {
                set_pose_mode(st, true);
            }
        }
        if ui.button(RichText::new(icons::SKIP_BACK).size(15.0)).on_hover_text(ctx.t("rig.clip.rewind_tip")).clicked() {
            st.anim.playing = false;
            st.set_playhead(0.0);
        }
        toggle(ui, &mut st.anim.loop_play, icons::REPEAT, &ctx.t("rig.clip.loop_tip"));
        ui.separator();
        toggle(ui, &mut st.anim.snap, icons::MAGNET, &ctx.t("rig.clip.snap_tip"));
        toggle(ui, &mut st.anim.auto_key, icons::RECORD, &ctx.t("rig.clip.autokey_tip"));
        if ui
            .add_enabled(!st.pose.is_empty(), egui::Button::new(format!("{}  {}", icons::DIAMOND, ctx.t("rig.clip.key_pose"))))
            .on_hover_text(ctx.t("rig.clip.key_pose_tip"))
            .clicked()
        {
            st.key_pose();
        }
        ui.separator();
        ui.label(RichText::new(ctx.t("rig.clip.time")).small().color(weak(ui)));
        let mut t = st.anim.playhead;
        if ui.add(DragValue::new(&mut t).speed(0.01).range(0.0..=clip.duration.max(0.0)).suffix(" s").max_decimals(3)).changed() {
            st.anim.playing = false;
            st.set_playhead(t);
        }
        if ui.small_button(icons::ARROWS_HORIZONTAL).on_hover_text(ctx.t("rig.clip.zoom_fit_tip")).clicked() {
            st.anim.fitted = false;
        }
        ui.separator();

        let mut chosen: Option<(Channel, Target)> = None;
        ui.menu_button(format!("{}  {}", icons::PLUS, ctx.t("rig.clip.add_track")), |ui| {
            for (who, list) in addable(rig) {
                ui.menu_button(who, |ui| {
                    for (c, target) in list {
                        let label = match &target {
                            Target::Deform(_, a) => format!("{}  {}", ctx.t(channel_key(c)), a),
                            _ => ctx.t(channel_key(c)),
                        };
                        if ui.button(label).clicked() {
                            chosen = Some((c, target));
                            ui.close();
                        }
                    }
                });
            }
            if ui.button(ctx.t(channel_key(Channel::DrawOrder))).clicked() {
                chosen = Some((Channel::DrawOrder, Target::DrawOrder));
                ui.close();
            }
        });
        if let Some((c, target)) = chosen {
            let t = st.anim.playhead;
            let n = name.to_owned();
            if let Some(doc) = st.doc.as_mut() {
                let mut added = None;
                let _ = doc.edit(|r| {
                    added = clips::add_track(r, &n, c, &target, t);
                    Ok(())
                });
                st.anim.track = added;
            }
            st.pose_rev += 1;
        }

        let has_track = st.anim.track.is_some();
        if ui
            .add_enabled(has_track, egui::Button::new(format!("{}  {}", icons::DIAMOND, ctx.t("rig.clip.add_key"))))
            .on_hover_text(ctx.t("rig.clip.add_key_tip"))
            .clicked()
        {
            add_key_at_playhead(st);
        }
        if ui
            .add_enabled(!st.anim.sel.is_empty(), egui::Button::new(icons::TRASH))
            .on_hover_text(ctx.t("rig.clip.delete_keys_tip"))
            .clicked()
        {
            delete_selected(st);
        }
    });
}

fn add_key_at_playhead(st: &mut State) {
    let (Some(ti), t) = (st.anim.track, st.anim.playhead) else { return };
    let curves = st.anim.curves.clone();
    let mut new_sel = None;
    st.edit_clip(|_, clip| {
        let Some(tr) = clip.tracks.get_mut(ti) else { return false };
        match clips::add_key_at(tr, t, |n| curves.get(n)) {
            Some(ki) => {
                new_sel = Some((ti, ki));
                true
            }
            None => false,
        }
    });
    if let Some(k) = new_sel {
        st.anim.sel = std::iter::once(k).collect();
    }
}

fn delete_selected(st: &mut State) {
    let sel = st.anim.sel.clone();
    st.edit_clip(|_, clip| clips::delete_keys(clip, &sel) > 0);
    st.anim.sel.clear();
}

fn handle_keys(ui: &Ui, st: &mut State, _clip: &RigClip) {
    if !ui.rect_contains_pointer(ui.max_rect()) {
        return;
    }
    let del = ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
    if del && !st.anim.sel.is_empty() {
        delete_selected(st);
    }
}

fn snap_targets(clip: &RigClip, playhead: f32, skip: &BTreeSet<KeyRef>) -> Vec<f32> {
    let mut v = vec![0.0, playhead, clip.duration];
    for (ti, times) in clips::rows(clip).into_iter().enumerate() {
        for (ki, t) in times.into_iter().enumerate() {
            if !skip.contains(&(ti, ki)) {
                v.push(t);
            }
        }
    }
    v
}

fn snap_playhead(st: &State, clip: &RigClip, t: f32) -> f32 {
    if !st.anim.snap {
        return tm::round_time(t);
    }
    let step = tm::grid_step(st.anim.view.pps, 40.0) / 5.0;
    tm::snap_time(t, Some(step), &snap_targets(clip, st.anim.playhead, &BTreeSet::new()), SNAP_PX / st.anim.view.pps)
}

fn draw_rows(ui: &mut Ui, ctx: &EditorContext<'_>, st: &mut State, rig: &Rig, clip: &RigClip, area: Rect, canvas: Rect) {
    let visible = ui.clip_rect();
    if clip.tracks.is_empty() {
        ui.painter().text(
            area.left_top() + vec2(10.0, 8.0),
            egui::Align2::LEFT_TOP,
            ctx.t("rig.clip.no_tracks"),
            egui::FontId::proportional(13.0),
            weak(ui),
        );
    }
    let mut remove: Option<usize> = None;
    for (row, tr) in clip.tracks.iter().enumerate() {
        let r = row_rect(area, row);
        if !r.intersects(visible) {
            continue;
        }
        ui_w::draw_row_bg(ui, r, canvas, &st.anim.view, clip.duration, row, st.anim.track == Some(row));
        let label_rect = Rect::from_min_max(r.left_top(), pos2(canvas.left() - 4.0, r.bottom()));
        ui.painter().with_clip_rect(label_rect).text(
            pos2(label_rect.left() + 8.0, r.center().y),
            egui::Align2::LEFT_CENTER,
            track_label(ctx, rig, tr),
            egui::FontId::proportional(12.5),
            ui.visuals().text_color(),
        );
        let trash = Rect::from_center_size(pos2(label_rect.right() - 12.0, r.center().y), vec2(20.0, 20.0));
        let trash_resp =
            ui.interact(trash, Id::new(("rig_tl_trash", row)), Sense::click()).on_hover_text(ctx.t("rig.clip.remove_track"));
        ui.painter().text(
            trash.center(),
            egui::Align2::CENTER_CENTER,
            icons::TRASH,
            egui::FontId::proportional(13.0),
            if trash_resp.hovered() { danger(ui) } else { faint(ui) },
        );
        if trash_resp.clicked() {
            remove = Some(row);
            continue;
        }
        let label_resp = ui.interact(
            Rect::from_min_max(label_rect.left_top(), pos2(trash.left(), r.bottom())),
            Id::new(("rig_tl_label", row)),
            Sense::click(),
        );
        if label_resp.clicked() {
            st.anim.track = Some(row);
        }
        let times = clips::key_times(tr);
        let animated = clips::segments_animated(tr);
        ui_w::draw_lane(ui, r, canvas, &st.anim.view, &times, &animated, |ki| st.anim.sel.contains(&(row, ki)));
    }
    ui_w::draw_playhead(ui, area, canvas, &st.anim.view, st.anim.playhead);
    if let Some(row) = remove {
        st.edit_clip(|_, c| {
            if row < c.tracks.len() {
                c.tracks.remove(row);
                true
            } else {
                false
            }
        });
        st.anim.sel.clear();
        st.anim.track = None;
    }
}

fn canvas_input(ui: &mut Ui, st: &mut State, clip: &RigClip, area: Rect, canvas: Rect) {
    let resp = ui.interact(canvas, Id::new("rig_tl_canvas"), Sense::click_and_drag());
    let mods = ui.input(|i| i.modifiers);
    let toggle_mod = mods.shift || mods.command;
    let pointer = resp.interact_pointer_pos();
    let rows = clips::rows(clip);
    let view = st.anim.view;

    if resp.drag_started()
        && let Some(p) = pointer
    {
        if ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle)) {
            st.anim.drag = Some(Drag::Pan);
        } else if let Some(hit) = ui_w::hit_key(canvas, area, &view, &rows, p) {
            if !st.anim.sel.contains(&hit) {
                if !toggle_mod {
                    st.anim.sel.clear();
                }
                st.anim.sel.insert(hit);
            }
            st.anim.track = Some(hit.0);
            if let Some(doc) = st.doc.as_mut() {
                doc.begin_gesture();
            }
            st.anim.drag = Some(Drag::Keys {
                origin: clip.clone(),
                sel: st.anim.sel.clone(),
                key_time: rows[hit.0][hit.1],
                grab_time: view.time_at(p.x - canvas.left()),
            });
        } else {
            st.anim.drag = Some(Drag::Marquee { start: p, add: toggle_mod, base: st.anim.sel.clone() });
        }
    }

    if resp.dragged()
        && let Some(p) = pointer
        && let Some(drag) = st.anim.drag.clone()
    {
        match drag {
            Drag::Keys { origin, sel, key_time, grab_time } => {
                let t = view.time_at(p.x - canvas.left());
                let wanted = key_time + (t - grab_time);
                let snapped = if st.anim.snap && !mods.alt {
                    let step = tm::grid_step(view.pps, 40.0) / 5.0;
                    let targets = snap_targets(&origin, st.anim.playhead, &sel);
                    tm::snap_time(wanted, Some(step), &targets, SNAP_PX / view.pps)
                } else {
                    tm::round_time(wanted)
                };
                let mut moved = origin;
                st.anim.sel = clips::move_keys(&mut moved, &sel, snapped - key_time);
                st.commit_clip(moved);
            }
            Drag::Marquee { start, add, base } => {
                let rect = Rect::from_two_pos(start, p);
                let mut picked = if add { base } else { BTreeSet::new() };
                picked.extend(ui_w::keys_in_rect(canvas, area, &view, &rows, rect));
                st.anim.sel = picked;
                ui_w::draw_marquee(ui, rect);
            }
            Drag::Pan => {
                let dx = ui.input(|i| i.pointer.delta().x);
                st.anim.view.scroll(-dx);
            }
        }
    }

    if resp.drag_stopped() {
        if matches!(st.anim.drag, Some(Drag::Keys { .. }))
            && let Some(doc) = st.doc.as_mut()
        {
            doc.end_gesture();
        }
        st.anim.drag = None;
    }

    if resp.double_clicked()
        && let Some(p) = pointer
        && ui_w::hit_key(canvas, area, &view, &rows, p).is_none()
        && let Some(row) = ui_w::row_at(area, rows.len(), p)
    {
        let t = snap_playhead(st, clip, view.time_at(p.x - canvas.left()).max(0.0));
        st.anim.track = Some(row);
        let curves = st.anim.curves.clone();
        let mut sel = None;
        st.edit_clip(|_, c| {
            let Some(tr) = c.tracks.get_mut(row) else { return false };
            sel = clips::add_key_at(tr, t, |n| curves.get(n)).map(|k| (row, k));
            sel.is_some()
        });
        if let Some(k) = sel {
            st.anim.sel = std::iter::once(k).collect();
        }
    } else if resp.clicked()
        && let Some(p) = pointer
    {
        match ui_w::hit_key(canvas, area, &view, &rows, p) {
            Some(hit) => {
                if toggle_mod {
                    if !st.anim.sel.remove(&hit) {
                        st.anim.sel.insert(hit);
                    }
                } else {
                    st.anim.sel = std::iter::once(hit).collect();
                }
                st.anim.track = Some(hit.0);
            }
            None => {
                if !toggle_mod {
                    st.anim.sel.clear();
                }
                st.anim.track = ui_w::row_at(area, rows.len(), p);
            }
        }
    }
}

// --------------------------------------------------------------- inspector

/// Edits a key value; returns the new one when changed.
fn value_editor(ui: &mut Ui, ctx: &EditorContext<'_>, rig: &Rig, target: &Target, value: &KeyValue) -> Option<KeyValue> {
    let mut out = None;
    match value {
        KeyValue::Angle(a) => {
            let mut v = *a;
            if ui.add(DragValue::new(&mut v).speed(0.5).suffix(" deg").max_decimals(2)).changed() {
                out = Some(KeyValue::Angle(v));
            }
        }
        KeyValue::Vec2(x, y) => {
            let (mut vx, mut vy) = (*x, *y);
            ui.label("x");
            let a = ui.add(DragValue::new(&mut vx).speed(0.05).max_decimals(3)).changed();
            ui.label("y");
            let b = ui.add(DragValue::new(&mut vy).speed(0.05).max_decimals(3)).changed();
            if a || b {
                out = Some(KeyValue::Vec2(vx, vy));
            }
        }
        KeyValue::Color(c) => {
            let mut rgba = [c.r, c.g, c.b, c.a];
            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                out = Some(KeyValue::Color(orbit_anim::Rgba::new(rgba[0], rgba[1], rgba[2], rgba[3])));
            }
        }
        KeyValue::Attachment(cur) => {
            let options: Vec<AttachmentId> = match target {
                Target::Slot(s) => slot_attachments(rig, s),
                _ => Vec::new(),
            };
            let none = ctx.t("rig.clip.attachment_none");
            let mut pick = cur.clone();
            ComboBox::from_id_salt("rig_key_attachment")
                .selected_text(cur.as_ref().map_or(none.clone(), ToString::to_string))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut pick, None, none);
                    for a in options {
                        let label = a.to_string();
                        ui.selectable_value(&mut pick, Some(a), label);
                    }
                });
            if pick != *cur {
                out = Some(KeyValue::Attachment(pick));
            }
        }
        KeyValue::Deform(n) => {
            ui.label(ctx.tf("rig.clip.deform_vertices", &[("n", n)]));
        }
        KeyValue::Order(order) => {
            // Move a slot one step: a swap with its neighbour.
            let mut new = order.clone();
            for i in 0..order.len() {
                ui.horizontal(|ui| {
                    ui.label(slot_name(rig, &order[i]));
                    if ui.add_enabled(i > 0, egui::Button::new(icons::ARROW_UP).small()).clicked() {
                        new.swap(i, i - 1);
                    }
                    if ui.add_enabled(i + 1 < order.len(), egui::Button::new(icons::ARROW_DOWN).small()).clicked() {
                        new.swap(i, i + 1);
                    }
                });
            }
            if new != *order {
                out = Some(KeyValue::Order(new));
            }
        }
    }
    out
}

fn slot_attachments(rig: &Rig, slot: &SlotId) -> Vec<AttachmentId> {
    // Attachments are keyed by id in the skins; the slot's own ones are the
    // ids that its bone's setup and the skins share, so offer every id.
    let _ = slot;
    let mut seen = BTreeSet::new();
    rig.skins.iter().flat_map(|s| s.attachments.keys().cloned()).filter(|a| seen.insert(a.clone())).collect()
}

fn inspector(ui: &mut Ui, ctx: &EditorContext<'_>, st: &mut State, rig: &Rig, clip: &RigClip) {
    let curves = st.anim.curves.clone();
    if st.anim.sel.len() == 1 {
        let &(ti, ki) = st.anim.sel.iter().next().expect("one key");
        let Some(tr) = clip.tracks.get(ti) else { return };
        let (Some(time), Some(value), Some(ease)) =
            (clips::key_times(tr).get(ki).copied(), clips::key_value(tr, ki), clips::key_ease(tr, ki))
        else {
            return;
        };
        let (_, target) = clips::track_kind(tr);
        ui.label(RichText::new(ctx.t("rig.clip.key")).strong());
        ui.horizontal(|ui| {
            ui.label(ctx.t("rig.clip.time"));
            let mut t = time;
            if ui.add(DragValue::new(&mut t).speed(0.01).range(0.0..=3600.0).suffix(" s").max_decimals(3)).changed() {
                let sel: BTreeSet<KeyRef> = std::iter::once((ti, ki)).collect();
                let mut c = clip.clone();
                let new_sel = clips::move_keys(&mut c, &sel, t - time);
                st.anim.sel = new_sel;
                st.commit_clip(c);
            }
            ui.separator();
            ui.label(ctx.t("rig.clip.value"));
            if let Some(new) = value_editor(ui, ctx, rig, &target, &value) {
                let mut c = clip.clone();
                if clips::set_key_value(&mut c.tracks[ti], ki, new) {
                    st.commit_clip(c);
                }
            }
        });
        let last = ki + 1 >= clips::key_count(tr);
        ui.label(RichText::new(ctx.t(if last { "rig.clip.ease_last" } else { "rig.clip.ease" })).small().color(weak(ui)));
        if !last
            && let Some(new) = curve_editor(
                ui,
                ("rig_ease", ti, ki),
                &ease,
                &curves,
                CurveEditorOptions { preview: false, graph_size: 110.0, ..Default::default() },
            )
        {
            let mut c = clip.clone();
            clips::set_key_ease(&mut c.tracks[ti], ki, new);
            st.commit_clip(c);
        }
        return;
    }

    // A track: the value at the playhead (auto-key records it).
    let Some(ti) = st.anim.track else { return };
    let Some(tr) = clip.tracks.get(ti) else { return };
    let (ch, target) = clips::track_kind(tr);
    let t = st.anim.playhead;
    let Some(value) = clips::sample_track(tr, t, |n| curves.get(n)) else {
        ui.label(RichText::new(ctx.t("rig.clip.empty_track")).color(weak(ui)));
        return;
    };
    ui.label(RichText::new(ctx.t("rig.clip.at_playhead")).strong());
    let on_key = clips::key_at(tr, t).is_some();
    let editable = st.anim.auto_key || on_key;
    ui.horizontal(|ui| {
        ui.label(fmt_time(t));
        if let KeyValue::Deform(n) = &value {
            deform_editor(ui, ctx, st, rig, clip, &target, *n, editable);
        } else if let Some(new) = ui.add_enabled_ui(editable, |ui| value_editor(ui, ctx, rig, &target, &value)).inner {
            let name = st.anim.clip.clone().unwrap_or_default();
            let rig2 = rig.clone();
            let curves = curves.clone();
            if let Some(doc) = st.doc.as_mut() {
                doc.begin_gesture();
                let _ = doc.edit(|r| {
                    if let Some(c) = r.clips.get_mut(&name) {
                        clips::record_value(c, &rig2, ch, &target, &new, t, |n| curves.get(n));
                    }
                    Ok(())
                });
            }
            st.pose_rev += 1;
        }
    });
    if !editable {
        ui.label(RichText::new(ctx.t("rig.clip.autokey_hint")).small().color(weak(ui)));
    }
}

/// Offsets of one vertex of a deform track at the playhead.
#[allow(clippy::too_many_arguments)]
fn deform_editor(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    st: &mut State,
    rig: &Rig,
    clip: &RigClip,
    target: &Target,
    count: usize,
    editable: bool,
) {
    let Target::Deform(slot, att) = target else { return };
    let n = clips::mesh_vertex_count(rig, slot, att).max(count);
    if n == 0 {
        return;
    }
    st.anim.deform_vertex = st.anim.deform_vertex.min(n - 1);
    let t = st.anim.playhead;
    let curves = st.anim.curves.clone();
    let mut offsets = clips::sample_deform(clip, slot, att, t, n, |c| curves.get(c));
    let mut changed = false;
    ui.label(ctx.t("rig.clip.vertex"));
    ui.add(DragValue::new(&mut st.anim.deform_vertex).range(0..=n - 1));
    let v = st.anim.deform_vertex;
    ui.add_enabled_ui(editable, |ui| {
        ui.label("x");
        changed |= ui.add(DragValue::new(&mut offsets[v][0]).speed(0.1).max_decimals(2)).changed();
        ui.label("y");
        changed |= ui.add(DragValue::new(&mut offsets[v][1]).speed(0.1).max_decimals(2)).changed();
        if ui.small_button(ctx.t("rig.clip.deform_reset")).clicked() {
            offsets.iter_mut().for_each(|o| *o = [0.0, 0.0]);
            changed = true;
        }
    });
    if changed {
        let name = st.anim.clip.clone().unwrap_or_default();
        let (slot, att) = (slot.clone(), att.clone());
        if let Some(doc) = st.doc.as_mut() {
            doc.begin_gesture();
            let _ = doc.edit(|r| {
                if let Some(c) = r.clips.get_mut(&name) {
                    clips::record_deform(c, &slot, &att, offsets, t, |n| curves.get(n));
                }
                Ok(())
            });
        }
        st.pose_rev += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use orbit_anim::rig::Xform;

    use super::*;
    use crate::rig_host::RigLink;

    fn state_with_arm() -> (State, BoneId) {
        let mut st = State::new(Rc::new(RefCell::new(RigLink::default())));
        st.new_rig();
        let id = st.doc.as_mut().unwrap().add_bone(None, [0.0, 0.0], [50.0, 0.0]).unwrap();
        let doc = st.doc.as_mut().unwrap();
        let clip = doc.edit(|r| clips::add_clip(r, "wave")).unwrap();
        st.sync();
        st.select_clip(Some(clip));
        (st, id)
    }

    fn rotation_keys(st: &State, id: &BoneId) -> Vec<(f32, f32)> {
        let c = st.clip().unwrap();
        match clips::find_track(c, Channel::Rotate, &Target::Bone(id.clone())) {
            Some(i) => {
                let tr = &c.tracks[i];
                (0..clips::key_count(tr))
                    .map(|k| {
                        let KeyValue::Angle(a) = clips::key_value(tr, k).unwrap() else { unreachable!() };
                        (clips::key_times(tr)[k], a)
                    })
                    .collect()
            }
            None => Vec::new(),
        }
    }

    #[test]
    fn selecting_a_clip_enters_pose_mode_and_previews_it() {
        let (mut st, id) = state_with_arm();
        assert!(st.pose_mode, "selecting a clip switches to pose mode");
        // A key written straight into the clip is what the viewport shows at
        // the playhead.
        st.anim.auto_key = true;
        st.set_playhead(0.5);
        st.set_local(&id, Xform { rotation: 30.0, ..Xform::IDENTITY });
        st.doc.as_mut().unwrap().end_gesture();
        st.sync();
        st.set_playhead(0.5);
        assert!((st.world()[0].x_angle() - 30.0).abs() < 1e-3);
        st.set_playhead(2.0);
        assert!((st.world()[0].x_angle() - 30.0).abs() < 1e-3, "held after the last key");
        st.select_clip(None);
        assert!(st.world()[0].x_angle().abs() < 1e-3, "no clip: the rest pose");
    }

    #[test]
    fn auto_key_records_at_the_playhead_and_a_drag_is_one_undo_step() {
        let (mut st, id) = state_with_arm();
        st.anim.auto_key = true;
        assert!(st.recording());
        st.set_playhead(0.5);
        let depth = st.doc.as_ref().unwrap().revision;
        for r in [10.0, 20.0, 35.0] {
            st.set_local(&id, Xform { rotation: r, ..Xform::IDENTITY });
        }
        assert!(st.doc.as_ref().unwrap().revision > depth);
        assert_eq!(rotation_keys(&st, &id), vec![(0.5, 35.0)]);
        assert!(st.pose.is_empty(), "auto-key leaves no throw-away pose");
        st.doc.as_mut().unwrap().end_gesture();
        st.doc.as_mut().unwrap().undo();
        assert!(rotation_keys(&st, &id).is_empty(), "one undo removes the whole gesture");
        st.doc.as_mut().unwrap().redo();
        assert_eq!(rotation_keys(&st, &id), vec![(0.5, 35.0)]);
    }

    #[test]
    fn without_auto_key_the_pose_is_thrown_away_until_keyed() {
        let (mut st, id) = state_with_arm();
        st.set_playhead(0.25);
        let rev = st.doc.as_ref().unwrap().revision;
        st.set_local(&id, Xform { rotation: 15.0, ..Xform::IDENTITY });
        assert_eq!(st.doc.as_ref().unwrap().revision, rev, "document untouched");
        assert!((st.world()[0].x_angle() - 15.0).abs() < 1e-3);
        // Moving the playhead drops the throw-away pose.
        st.set_playhead(0.75);
        assert!(st.pose.is_empty());
        st.set_playhead(0.25);
        st.set_local(&id, Xform { rotation: 15.0, ..Xform::IDENTITY });
        assert_eq!(st.key_pose(), 1);
        assert_eq!(rotation_keys(&st, &id), vec![(0.25, 15.0)]);
        assert!(st.pose.is_empty());
        st.doc.as_mut().unwrap().undo();
        assert!(rotation_keys(&st, &id).is_empty());
    }

    #[test]
    fn rest_mode_edits_still_change_the_rest_pose_with_a_clip_selected() {
        let (mut st, id) = state_with_arm();
        st.anim.auto_key = true;
        set_pose_mode(&mut st, false);
        assert!(!st.recording());
        st.set_local(&id, Xform { x: 7.0, ..Xform::IDENTITY });
        st.doc.as_mut().unwrap().end_gesture();
        assert_eq!(st.doc.as_ref().unwrap().rig().bones[0].rest.x, 7.0);
        assert!(rotation_keys(&st, &id).is_empty());
    }

    #[test]
    fn crossfade_preview_blends_and_clears_with_its_clips() {
        let (mut st, id) = state_with_arm();
        st.anim.auto_key = true;
        st.set_local(&id, Xform { rotation: 40.0, ..Xform::IDENTITY });
        st.doc.as_mut().unwrap().end_gesture();
        let other = st.doc.as_mut().unwrap().edit(|r| clips::add_clip(r, "idle")).unwrap();
        st.sync();
        st.anim.fade = Some(Fade { from: "wave".into(), to: other, blend: 0.4, hold: 1.0, clock: 0.0, playing: false });
        st.pose_rev += 1;
        assert!((st.world()[0].x_angle() - 40.0).abs() < 1e-3, "only the first clip before the switch");
        st.anim.fade.as_mut().unwrap().clock = 1.2;
        assert!((st.world()[0].x_angle() - 20.0).abs() < 1e-3, "halfway: 40 -> 0");
        st.anim.fade.as_mut().unwrap().clock = 2.0;
        assert!(st.world()[0].x_angle().abs() < 1e-3);
        // Deleting a clip of the fade cancels the preview.
        st.doc.as_mut().unwrap().edit(|r| clips::delete_clip(r, "idle")).unwrap();
        st.sync();
        validate(&mut st);
        assert!(st.anim.fade.is_none());
    }

    #[test]
    fn deleting_or_undoing_the_edited_clip_deselects_it() {
        let (mut st, _) = state_with_arm();
        st.doc.as_mut().unwrap().edit(|r| clips::delete_clip(r, "wave")).unwrap();
        validate(&mut st);
        assert!(st.anim.clip.is_none());
        assert!(st.clip().is_none());
    }

    #[test]
    fn named_curves_shape_the_preview() {
        let (mut st, id) = state_with_arm();
        st.anim.auto_key = true;
        st.set_local(&id, Xform { rotation: 0.0, ..Xform::IDENTITY });
        st.set_playhead(1.0);
        st.set_local(&id, Xform { rotation: 40.0, ..Xform::IDENTITY });
        st.doc.as_mut().unwrap().end_gesture();
        // Key 0 would not exist (rest pose at 0.0 is a no-op): add it, then
        // ease it with a story curve.
        st.edit_clip(|_, c| {
            clips::add_key_at(&mut c.tracks[0], 0.0, |_| None);
            clips::set_key_value(&mut c.tracks[0], 0, KeyValue::Angle(0.0));
            clips::set_key_ease(&mut c.tracks[0], 0, Curve::Named("snap".into()));
            true
        });
        st.sync();
        st.set_playhead(0.5);
        assert!((st.world()[0].x_angle() - 20.0).abs() < 1e-3, "unresolved: linear");
        let mut table = BTreeMap::new();
        table.insert("snap".to_owned(), Curve::Preset(orbit_anim::Ease::InQuad));
        st.anim.curves = Rc::new(table);
        assert!((st.world()[0].x_angle() - 10.0).abs() < 1e-3, "resolved through the story's table");
    }
}
