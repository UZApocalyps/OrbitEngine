//! The Timeline panel (M5A-05): a dockable editor for the `Timeline` node
//! selected in the story. Tracks (target + property) are rows, keys are
//! diamonds on a time ruler. Keys are added, dragged (with snapping),
//! multi-selected, copied and deleted; each key has an easing edited with
//! the reusable curve editor; the playhead is scrubbed on the ruler and the
//! Preview shows the stage at that time; with auto-key on, editing the value
//! of a track at the playhead records a key.
//!
//! All edits go through `SetNodeKind` (merged per drag, so a drag is one undo
//! step). The model lives in `timeline_model.rs` and is tested headlessly.

use std::collections::BTreeSet;

use egui::{
    Align2, DragValue, FontId, Id, Key, Modifiers, PointerButton, Pos2,
    Rect, RichText, Sense, Ui, pos2, vec2,
};
use orbit_anim::Value;
use orbit_core::{
    AnimLoop, AnimProp, Curve, NodeId, NodeKind, Story, StoryTrack,
};
use orbit_editor_api::curve_editor::{CurveEditorOptions, curve_editor};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, PanelDef, Plugin, PluginManifest, Selection,
    PreviewCommand, Translator, UiText, icons,
};

use super::fx_text::{prop_key, target_name};
use super::inspector::commit;
use super::timeline_model::{self as model, ClipKey, KeyRef, SNAP_PX, TimeView};
use super::timeline_ui::{self as ui_w, LABEL_W, ROW_H, RULER_H, fmt_time, row_rect};
use super::ui_util::{danger, faint, weak};

const INSPECTOR_H: f32 = 200.0;

pub struct TimelinePlugin;

impl Plugin for TimelinePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(super::ids::TIMELINE, "Timeline", "0.1.0")
    }
}

impl EditorPlugin for TimelinePlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "timeline",
            "timeline.panel.title",
            icons::FILM_STRIP,
            DockSlot::Bottom,
            timeline_panel,
        ));
    }
}

// ------------------------------------------------------------------ state

#[derive(Clone)]
enum Drag {
    Keys {
        /// Tracks as they were when the drag began: every frame is computed
        /// from them, so the drag never drifts.
        origin: Vec<StoryTrack>,
        sel: BTreeSet<KeyRef>,
        /// Time of the grabbed key at the start.
        key_time: f32,
        /// Time under the pointer at the start.
        grab_time: f32,
        /// Merge key of the whole drag (one undo step).
        merge: String,
    },
    Marquee {
        start: Pos2,
        add: bool,
        base: BTreeSet<KeyRef>,
    },
    Pan,
}

#[derive(Clone)]
struct PanelState {
    node: Option<NodeId>,
    view: TimeView,
    fitted: bool,
    playhead: f32,
    playing: bool,
    loop_play: bool,
    snap: bool,
    auto_key: bool,
    link_preview: bool,
    sel: BTreeSet<KeyRef>,
    track: Option<usize>,
    clipboard: Vec<ClipKey>,
    drag: Option<Drag>,
    drags: u64,
    /// What the preview was last asked to show, to ask again only on change.
    sent: Option<(NodeId, u32, Option<u64>)>,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            node: None,
            view: TimeView::default(),
            fitted: false,
            playhead: 0.0,
            playing: false,
            loop_play: true,
            snap: true,
            auto_key: false,
            link_preview: true,
            sel: BTreeSet::new(),
            track: None,
            clipboard: Vec::new(),
            drag: None,
            drags: 0,
            sent: None,
        }
    }
}

/// What the UI pass decided; applied once the story borrow is over.
enum Act {
    /// New tracks, committed under this merge key.
    Tracks(Vec<StoryTrack>, String),
    EndMerge,
    Duration(f32),
    ConvertEffect(NodeKind),
}

/// A copy of the timeline under edit.
struct Tl {
    id: NodeId,
    kind: NodeKind,
    duration: f32,
    looping: AnimLoop,
    tracks: Vec<StoryTrack>,
}

fn timeline_of(story: &Story, id: NodeId) -> Option<Tl> {
    let node = story.node(id)?;
    match &node.kind {
        NodeKind::Timeline {
            duration,
            looping,
            tracks,
            ..
        } => Some(Tl {
            id,
            kind: node.kind.clone(),
            duration: *duration,
            looping: *looping,
            tracks: tracks.clone(),
        }),
        _ => None,
    }
}

/// The node kind with other tracks / duration.
fn rebuilt(kind: &NodeKind, new_tracks: Option<Vec<StoryTrack>>, new_duration: Option<f32>) -> NodeKind {
    match kind {
        NodeKind::Timeline {
            duration,
            looping,
            wait,
            tracks,
        } => NodeKind::Timeline {
            duration: new_duration.unwrap_or(*duration),
            looping: *looping,
            wait: *wait,
            tracks: new_tracks.unwrap_or_else(|| tracks.clone()),
        },
        other => other.clone(),
    }
}

// ------------------------------------------------------------------ panel

fn timeline_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    if !ctx.has_project() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ui.t("defs.no_project")).color(weak(ui)));
        });
        return;
    }
    let state_id = Id::new("orbit.timeline.state");
    let mut st: PanelState = ui
        .ctx()
        .data_mut(|d| d.get_temp(state_id))
        .unwrap_or_default();

    follow_selection(ctx, &mut st);

    let mut acts: Vec<Act> = Vec::new();
    let texts = ctx.translator();
    let shown = match ctx.story().zip(st.node) {
        Some((story, id)) => timeline_of(story, id).map(|tl| {
            body(ui, story, &texts, &tl, &mut st, &mut acts);
            tl
        }),
        None => None,
    };
    match shown {
        Some(tl) => preview_sync(ctx, &tl, &mut st),
        None => empty(ui, ctx, &mut st, &mut acts),
    }
    apply(ctx, &mut st, acts);
    ui.ctx().data_mut(|d| d.insert_temp(state_id, st));
}

fn follow_selection(ctx: &EditorContext<'_>, st: &mut PanelState) {
    let Some(Selection::Node(id)) = ctx.selection() else {
        return;
    };
    let Some(story) = ctx.story() else { return };
    let animating = story
        .node(id)
        .is_some_and(|n| matches!(n.kind, NodeKind::Timeline { .. } | NodeKind::Effect(_)));
    if animating && st.node != Some(id) {
        st.node = Some(id);
        st.sel.clear();
        st.track = None;
        st.playhead = 0.0;
        st.playing = false;
        st.drag = None;
        st.fitted = false;
        st.sent = None;
    }
}

fn empty(ui: &mut Ui, ctx: &EditorContext<'_>, st: &mut PanelState, acts: &mut Vec<Act>) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icons::FILM_STRIP).size(30.0).color(weak(ui)));
        ui.add_space(6.0);
        let effect = st
            .node
            .and_then(|id| ctx.story()?.node(id))
            .and_then(|n| match &n.kind {
                NodeKind::Effect(e) => Some(e.to_timeline()),
                _ => None,
            });
        match effect {
            Some(kind) => {
                ui.label(RichText::new(ui.t("timeline.effect_hint")).color(weak(ui)));
                if ui.button(ui.t("timeline.convert")).clicked() {
                    acts.push(Act::ConvertEffect(kind));
                }
            }
            None => {
                ui.label(RichText::new(ui.t("timeline.empty")).color(weak(ui)));
            }
        }
    });
}

// ---------------------------------------------------------------- the body

fn body(
    ui: &mut Ui,
    story: &Story,
    texts: &Translator,
    tl: &Tl,
    st: &mut PanelState,
    acts: &mut Vec<Act>,
) {
    // Keep the selection and drag valid when the story changes under us
    // (undo, another edit).
    st.sel.retain(|&(t, k)| {
        tl.tracks
            .get(t)
            .is_some_and(|tr| k < model::key_count(&tr.data))
    });
    if st.track.is_some_and(|t| t >= tl.tracks.len()) {
        st.track = None;
    }

    let width = ui.available_width();
    if !st.fitted {
        st.view = TimeView::fit(tl.duration.max(model::last_key_time(&tl.tracks)), width - LABEL_W);
        st.fitted = true;
    }

    toolbar(ui, story, texts, tl, st, acts);
    advance_playhead(ui, tl, st);
    handle_keys(ui, tl, st, acts);

    let reserve = if inspector_has_content(tl, st) { INSPECTOR_H } else { 0.0 };
    let rows_h = (ui.available_height() - reserve - RULER_H - 6.0).max(ROW_H * 2.0);
    let full_w = ui.available_width();
    let canvas_w = (full_w - LABEL_W).max(80.0);

    // ---- ruler
    let (ruler_all, _) = ui.allocate_exact_size(vec2(full_w, RULER_H), Sense::hover());
    let ruler = Rect::from_min_size(
        pos2(ruler_all.left() + LABEL_W, ruler_all.top()),
        vec2(canvas_w, RULER_H),
    );
    ui_w::draw_ruler(ui, ruler, &st.view, tl.duration);
    if let Some(t) = ui_w::scrub_ruler(ui, ruler, Id::new("tl_ruler"), &st.view, tl.duration) {
        st.playing = false;
        st.playhead = snap_playhead(st, tl, t);
    }

    // ---- rows
    let mut canvas = Rect::NOTHING;
    egui::ScrollArea::vertical()
        .id_salt("tl_rows")
        .max_height(rows_h)
        .auto_shrink([false, false])
        .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR | egui::scroll_area::ScrollSource::MOUSE_WHEEL)
        .show(ui, |ui| {
            let rows = tl.tracks.len().max(1);
            let (area, _) = ui.allocate_exact_size(
                vec2(ui.available_width().max(full_w), ROW_H * rows as f32 + 4.0),
                Sense::hover(),
            );
            canvas = Rect::from_min_max(pos2(area.left() + LABEL_W, area.top()), area.right_bottom());
            draw_rows(ui, area, canvas, story, texts, tl, st, acts);
            canvas_input(ui, area, canvas, story, tl, st, acts);
        });

    // ---- wheel: ctrl zoom, shift / horizontal scroll, over the canvas
    ui_w::wheel_nav(ui, &mut st.view, ruler, canvas.bottom());

    if reserve > 0.0 {
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("tl_inspector")
            .auto_shrink([false, false])
            .show(ui, |ui| inspector(ui, story, tl, st, acts));
    }
}

fn inspector_has_content(tl: &Tl, st: &PanelState) -> bool {
    st.sel.len() == 1 || st.track.is_some_and(|t| t < tl.tracks.len())
}

fn snap_targets(tl: &Tl, st: &PanelState, skip: &BTreeSet<KeyRef>) -> Vec<f32> {
    let mut v = vec![0.0, st.playhead, tl.duration];
    for (ti, tr) in tl.tracks.iter().enumerate() {
        for (ki, t) in model::key_times(&tr.data).into_iter().enumerate() {
            if !skip.contains(&(ti, ki)) {
                v.push(t);
            }
        }
    }
    v
}

fn snap_playhead(st: &PanelState, tl: &Tl, t: f32) -> f32 {
    if !st.snap {
        return model::round_time(t);
    }
    let step = model::grid_step(st.view.pps, 40.0) / 5.0;
    let targets = snap_targets(tl, st, &BTreeSet::new());
    model::snap_time(t, Some(step), &targets, SNAP_PX / st.view.pps)
}

fn advance_playhead(ui: &mut Ui, tl: &Tl, st: &mut PanelState) {
    if !st.playing {
        return;
    }
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let end = tl.duration.max(0.0);
    st.playhead += dt;
    if st.playhead >= end {
        if st.loop_play && end > 0.0 {
            st.playhead = 0.0;
        } else {
            st.playhead = end;
            st.playing = false;
        }
    }
    ui.ctx().request_repaint();
}

// ----------------------------------------------------------------- toolbar

fn toggle(ui: &mut Ui, on: &mut bool, icon: &str, tip: &str) {
    let text = RichText::new(icon).size(15.0);
    if ui.selectable_label(*on, text).on_hover_text(tip).clicked() {
        *on = !*on;
    }
}

fn toolbar(
    ui: &mut Ui,
    story: &Story,
    texts: &Translator,
    tl: &Tl,
    st: &mut PanelState,
    acts: &mut Vec<Act>,
) {
    ui.horizontal_wrapped(|ui| {
        let title = ui.t("timeline.panel.title");
        ui.label(RichText::new(format!("{}  {}", icons::FILM_STRIP, title)).strong());
        ui.separator();

        let play = if st.playing { icons::PAUSE } else { icons::PLAY };
        if ui
            .button(RichText::new(play).size(15.0))
            .on_hover_text(ui.t("timeline.play_tip"))
            .clicked()
        {
            if !st.playing && st.playhead >= tl.duration {
                st.playhead = 0.0;
            }
            st.playing = !st.playing;
        }
        if ui
            .button(RichText::new(icons::SKIP_BACK).size(15.0))
            .on_hover_text(ui.t("timeline.rewind_tip"))
            .clicked()
        {
            st.playing = false;
            st.playhead = 0.0;
        }
        toggle(ui, &mut st.loop_play, icons::REPEAT, &ui.t("timeline.loop_tip"));
        ui.separator();
        toggle(ui, &mut st.snap, icons::MAGNET, &ui.t("timeline.snap_tip"));
        toggle(ui, &mut st.auto_key, icons::RECORD, &ui.t("timeline.autokey_tip"));
        toggle(ui, &mut st.link_preview, icons::MONITOR_PLAY, &ui.t("timeline.preview_tip"));
        ui.separator();

        ui.label(RichText::new(ui.t("timeline.time")).small().color(weak(ui)));
        let mut t = st.playhead;
        if ui
            .add(
                DragValue::new(&mut t)
                    .speed(0.01)
                    .range(0.0..=tl.duration.max(0.0))
                    .suffix(" s")
                    .max_decimals(3),
            )
            .changed()
        {
            st.playing = false;
            st.playhead = model::round_time(t);
        }
        ui.label(RichText::new(ui.t("timeline.duration")).small().color(weak(ui)));
        let mut d = tl.duration;
        if ui
            .add(
                DragValue::new(&mut d)
                    .speed(0.05)
                    .range(0.0..=3600.0)
                    .suffix(" s")
                    .max_decimals(3),
            )
            .changed()
        {
            acts.push(Act::Duration(d));
        }
        if ui
            .small_button(ui.t("timeline.fit_duration"))
            .on_hover_text(ui.t("timeline.fit_duration_tip"))
            .clicked()
        {
            let end = model::last_key_time(&tl.tracks);
            if end > 0.0 {
                acts.push(Act::Duration(end));
            }
        }
        if ui
            .small_button(icons::ARROWS_HORIZONTAL)
            .on_hover_text(ui.t("timeline.zoom_fit_tip"))
            .clicked()
        {
            st.fitted = false;
        }
        ui.separator();

        add_track_menu(ui, story, texts, tl, st, acts);

        let can_key = st.track.is_some();
        if ui
            .add_enabled(can_key, egui::Button::new(format!("{}  {}", icons::DIAMOND, ui.t("timeline.add_key"))))
            .on_hover_text(ui.t("timeline.add_key_tip"))
            .clicked()
            && let Some(ti) = st.track
        {
            add_key_at_playhead(story, tl, st, ti, acts);
        }
        let has_sel = !st.sel.is_empty();
        if ui
            .add_enabled(has_sel, egui::Button::new(icons::TRASH))
            .on_hover_text(ui.t("timeline.delete_keys_tip"))
            .clicked()
        {
            delete_selected(tl, st, acts);
        }
        if ui
            .add_enabled(has_sel, egui::Button::new(icons::COPY))
            .on_hover_text(ui.t("timeline.copy_tip"))
            .clicked()
        {
            st.clipboard = model::copy_keys(&tl.tracks, &st.sel);
        }
        if ui
            .add_enabled(!st.clipboard.is_empty(), egui::Button::new(icons::CLIPBOARD))
            .on_hover_text(ui.t("timeline.paste_tip"))
            .clicked()
        {
            paste(tl, st, acts);
        }
    });
}

fn add_track_menu(
    ui: &mut Ui,
    story: &Story,
    texts: &Translator,
    tl: &Tl,
    _st: &mut PanelState,
    acts: &mut Vec<Act>,
) {
    ui.menu_button(format!("{}  {}", icons::PLUS, ui.t("timeline.add_track")), |ui| {
        for prop in AnimProp::ALL {
            let targets = super::inspector_fx::candidate_targets(story, prop);
            if targets.is_empty() {
                continue;
            }
            ui.menu_button(texts.get(prop_key(prop)), |ui| {
                for target in targets {
                    if ui.button(target_name(texts, story, &target)).clicked() {
                        let mut tracks = tl.tracks.clone();
                        tracks.push(StoryTrack {
                            target,
                            prop,
                            data: super::inspector_fx::prop_data(prop),
                        });
                        acts.push(Act::Tracks(tracks, format!("tl:{}:addtrack:{}", tl.id, tl.tracks.len())));
                        acts.push(Act::EndMerge);
                        ui.close();
                    }
                }
            });
        }
    });
}

// ------------------------------------------------------------- key actions

fn resolver<'a>(story: &'a Story) -> impl Fn(&str) -> Option<&'a Curve> + 'a {
    move |name| story.curves.get(name)
}

fn add_key_at_playhead(story: &Story, tl: &Tl, st: &mut PanelState, ti: usize, acts: &mut Vec<Act>) {
    let mut tracks = tl.tracks.clone();
    let Some(tr) = tracks.get_mut(ti) else { return };
    if let Some(ki) = model::add_key_at(&mut tr.data, st.playhead, resolver(story)) {
        st.sel = std::iter::once((ti, ki)).collect();
        acts.push(Act::Tracks(tracks, format!("tl:{}:key:{}", tl.id, st.drags)));
        acts.push(Act::EndMerge);
        st.drags += 1;
    }
}

fn delete_selected(tl: &Tl, st: &mut PanelState, acts: &mut Vec<Act>) {
    let mut tracks = tl.tracks.clone();
    if model::delete_keys(&mut tracks, &st.sel) > 0 {
        acts.push(Act::Tracks(tracks, format!("tl:{}:del:{}", tl.id, st.drags)));
        acts.push(Act::EndMerge);
        st.drags += 1;
    }
    st.sel.clear();
}

fn paste(tl: &Tl, st: &mut PanelState, acts: &mut Vec<Act>) {
    let mut tracks = tl.tracks.clone();
    let new_sel = model::paste_keys(&mut tracks, &st.clipboard, st.playhead, st.track);
    if !new_sel.is_empty() {
        acts.push(Act::Tracks(tracks, format!("tl:{}:paste:{}", tl.id, st.drags)));
        acts.push(Act::EndMerge);
        st.drags += 1;
        st.sel = new_sel;
    }
}

/// Keyboard shortcuts, only while the pointer is over the panel and no text
/// field has the focus.
fn handle_keys(ui: &mut Ui, tl: &Tl, st: &mut PanelState, acts: &mut Vec<Act>) {
    if !ui.rect_contains_pointer(ui.max_rect()) || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let (del, copy, paste_k, space, all) = ui.input_mut(|i| {
        (
            i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
            i.consume_key(Modifiers::COMMAND, Key::C),
            i.consume_key(Modifiers::COMMAND, Key::V),
            i.consume_key(Modifiers::NONE, Key::Space),
            i.consume_key(Modifiers::COMMAND, Key::A),
        )
    });
    if del && !st.sel.is_empty() {
        delete_selected(tl, st, acts);
    }
    if copy && !st.sel.is_empty() {
        st.clipboard = model::copy_keys(&tl.tracks, &st.sel);
    }
    if paste_k && !st.clipboard.is_empty() {
        paste(tl, st, acts);
    }
    if space {
        if !st.playing && st.playhead >= tl.duration {
            st.playhead = 0.0;
        }
        st.playing = !st.playing;
    }
    if all {
        st.sel = tl
            .tracks
            .iter()
            .enumerate()
            .flat_map(|(ti, tr)| (0..model::key_count(&tr.data)).map(move |ki| (ti, ki)))
            .collect();
    }
}

// ----------------------------------------------------------------- drawing

#[allow(clippy::too_many_arguments)]
fn draw_rows(
    ui: &mut Ui,
    area: Rect,
    canvas: Rect,
    story: &Story,
    texts: &Translator,
    tl: &Tl,
    st: &mut PanelState,
    acts: &mut Vec<Act>,
) {
    let visible = ui.clip_rect();
    if tl.tracks.is_empty() {
        ui.painter().text(
            area.left_top() + vec2(10.0, 8.0),
            Align2::LEFT_TOP,
            ui.t("timeline.no_tracks"),
            FontId::proportional(13.0),
            weak(ui),
        );
    }
    for (row, tr) in tl.tracks.iter().enumerate() {
        let r = row_rect(area, row);
        if !r.intersects(visible) {
            continue;
        }
        ui_w::draw_row_bg(ui, r, canvas, &st.view, tl.duration, row, st.track == Some(row));

        // Label column: "target · property" and the remove button.
        let label = format!(
            "{}  ·  {}",
            target_name(texts, story, &tr.target),
            texts.get(prop_key(tr.prop))
        );
        let label_rect = Rect::from_min_max(r.left_top(), pos2(canvas.left() - 4.0, r.bottom()));
        let clipped = ui.painter().with_clip_rect(label_rect.shrink2(vec2(0.0, 0.0)));
        clipped.text(
            pos2(label_rect.left() + 8.0, r.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.5),
            ui.visuals().text_color(),
        );
        let trash = Rect::from_center_size(pos2(label_rect.right() - 12.0, r.center().y), vec2(20.0, 20.0));
        let trash_resp = ui
            .interact(trash, Id::new(("tl_trash", row)), Sense::click())
            .on_hover_text(ui.t("timeline.remove_track"));
        ui.painter().text(
            trash.center(),
            Align2::CENTER_CENTER,
            icons::TRASH,
            FontId::proportional(13.0),
            if trash_resp.hovered() { danger(ui) } else { faint(ui) },
        );
        if trash_resp.clicked() {
            let mut tracks = tl.tracks.clone();
            tracks.remove(row);
            acts.push(Act::Tracks(tracks, format!("tl:{}:rmtrack:{}", tl.id, st.drags)));
            acts.push(Act::EndMerge);
            st.drags += 1;
            st.sel.clear();
            st.track = None;
            continue;
        }
        let label_resp = ui.interact(
            Rect::from_min_max(label_rect.left_top(), pos2(trash.left(), r.bottom())),
            Id::new(("tl_label", row)),
            Sense::click(),
        );
        if label_resp.clicked() {
            st.track = Some(row);
        }

        // Animated segments (the value changes between two keys).
        let times = model::key_times(&tr.data);
        let animated: Vec<bool> = (0..times.len().saturating_sub(1))
            .map(|w| model::key_value(&tr.data, w) != model::key_value(&tr.data, w + 1))
            .collect();
        ui_w::draw_lane(ui, r, canvas, &st.view, &times, &animated, |ki| st.sel.contains(&(row, ki)));
    }
    ui_w::draw_playhead(ui, area, canvas, &st.view, st.playhead);
}

// ------------------------------------------------------------ canvas input

fn rows_of(tl: &Tl) -> Vec<Vec<f32>> {
    tl.tracks.iter().map(|tr| model::key_times(&tr.data)).collect()
}

fn hit_key(canvas: Rect, area: Rect, tl: &Tl, st: &PanelState, p: Pos2) -> Option<KeyRef> {
    ui_w::hit_key(canvas, area, &st.view, &rows_of(tl), p)
}

fn row_at(area: Rect, tl: &Tl, p: Pos2) -> Option<usize> {
    ui_w::row_at(area, tl.tracks.len(), p)
}

fn canvas_input(
    ui: &mut Ui,
    area: Rect,
    canvas: Rect,
    story: &Story,
    tl: &Tl,
    st: &mut PanelState,
    acts: &mut Vec<Act>,
) {
    let resp = ui.interact(canvas, Id::new("tl_canvas"), Sense::click_and_drag());
    let mods = ui.input(|i| i.modifiers);
    let toggle_mod = mods.shift || mods.command;
    let pointer = resp.interact_pointer_pos();

    if resp.drag_started()
        && let Some(p) = pointer
    {
        let middle = ui.input(|i| i.pointer.button_down(PointerButton::Middle));
        if middle {
            st.drag = Some(Drag::Pan);
        } else if let Some(hit) = hit_key(canvas, area, tl, st, p) {
            if !st.sel.contains(&hit) {
                if !toggle_mod {
                    st.sel.clear();
                }
                st.sel.insert(hit);
            }
            st.track = Some(hit.0);
            let key_time = model::key_times(&tl.tracks[hit.0].data)[hit.1];
            st.drags += 1;
            st.drag = Some(Drag::Keys {
                origin: tl.tracks.clone(),
                sel: st.sel.clone(),
                key_time,
                grab_time: st.view.time_at(p.x - canvas.left()),
                merge: format!("tl:{}:drag:{}", tl.id, st.drags),
            });
        } else {
            st.drag = Some(Drag::Marquee {
                start: p,
                add: toggle_mod,
                base: st.sel.clone(),
            });
        }
    }

    if resp.dragged()
        && let Some(p) = pointer
        && let Some(drag) = st.drag.clone()
    {
        match drag {
            Drag::Keys {
                origin,
                sel,
                key_time,
                grab_time,
                merge,
            } => {
                let t = st.view.time_at(p.x - canvas.left());
                let wanted = key_time + (t - grab_time);
                let snapped = if st.snap && !mods.alt {
                    let step = model::grid_step(st.view.pps, 40.0) / 5.0;
                    let probe = PanelState { sel: sel.clone(), ..st.clone() };
                    let targets = snap_targets(
                        &Tl {
                            id: tl.id,
                            kind: tl.kind.clone(),
                            duration: tl.duration,
                            looping: tl.looping,
                            tracks: origin.clone(),
                        },
                        &probe,
                        &sel,
                    );
                    model::snap_time(wanted, Some(step), &targets, SNAP_PX / st.view.pps)
                } else {
                    model::round_time(wanted)
                };
                let mut tracks = origin;
                let new_sel = model::move_keys(&mut tracks, &sel, snapped - key_time);
                st.sel = new_sel;
                acts.push(Act::Tracks(tracks, merge));
            }
            Drag::Marquee { start, add, base } => {
                let rect = Rect::from_two_pos(start, p);
                let mut picked = if add { base } else { BTreeSet::new() };
                picked.extend(ui_w::keys_in_rect(canvas, area, &st.view, &rows_of(tl), rect));
                st.sel = picked;
                ui_w::draw_marquee(ui, rect);
            }
            Drag::Pan => {
                let dx = ui.input(|i| i.pointer.delta().x);
                st.view.scroll(-dx);
            }
        }
    }

    if resp.drag_stopped() {
        if matches!(st.drag, Some(Drag::Keys { .. })) {
            acts.push(Act::EndMerge);
        }
        st.drag = None;
    }

    if resp.double_clicked()
        && let Some(p) = pointer
        && hit_key(canvas, area, tl, st, p).is_none()
        && let Some(row) = row_at(area, tl, p)
    {
        let t = st.view.time_at(p.x - canvas.left()).max(0.0);
        let t = snap_playhead(st, tl, t);
        st.track = Some(row);
        let mut tracks = tl.tracks.clone();
        if let Some(ki) = model::add_key_at(&mut tracks[row].data, t, resolver(story)) {
            st.sel = std::iter::once((row, ki)).collect();
            acts.push(Act::Tracks(tracks, format!("tl:{}:key:{}", tl.id, st.drags)));
            acts.push(Act::EndMerge);
            st.drags += 1;
        }
    } else if resp.clicked()
        && let Some(p) = pointer
    {
        match hit_key(canvas, area, tl, st, p) {
            Some(hit) => {
                if toggle_mod {
                    if !st.sel.remove(&hit) {
                        st.sel.insert(hit);
                    }
                } else {
                    st.sel = std::iter::once(hit).collect();
                }
                st.track = Some(hit.0);
            }
            None => {
                if !toggle_mod {
                    st.sel.clear();
                }
                st.track = row_at(area, tl, p);
            }
        }
    }
}

// --------------------------------------------------------------- inspector

fn value_editor(ui: &mut Ui, value: &mut Value) -> bool {
    let mut changed = false;
    match value {
        Value::Scalar(v) => {
            changed |= ui.add(DragValue::new(v).speed(0.01).max_decimals(4)).changed();
        }
        Value::Vec2(v) => {
            ui.label("x");
            changed |= ui.add(DragValue::new(&mut v.x).speed(0.005).max_decimals(4)).changed();
            ui.label("y");
            changed |= ui.add(DragValue::new(&mut v.y).speed(0.005).max_decimals(4)).changed();
        }
        Value::Angle(a) => {
            changed |= ui
                .add(DragValue::new(&mut a.0).speed(0.5).suffix(" deg").max_decimals(2))
                .changed();
        }
        Value::Color(c) => {
            let mut rgba = [c.r, c.g, c.b, c.a];
            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                (c.r, c.g, c.b, c.a) = (rgba[0], rgba[1], rgba[2], rgba[3]);
                changed = true;
            }
        }
        Value::Name(n) => {
            changed |= ui.text_edit_singleline(n).changed();
        }
    }
    changed
}

fn inspector(ui: &mut Ui, story: &Story, tl: &Tl, st: &mut PanelState, acts: &mut Vec<Act>) {
    // One key selected: its time, value and easing.
    if st.sel.len() == 1 {
        let &(ti, ki) = st.sel.iter().next().expect("one key");
        let Some(tr) = tl.tracks.get(ti) else { return };
        let (Some(time), Some(mut value), Some(ease)) = (
            model::key_times(&tr.data).get(ki).copied(),
            model::key_value(&tr.data, ki),
            model::key_ease(&tr.data, ki),
        ) else {
            return;
        };
        ui.label(RichText::new(ui.t("timeline.key")).strong());
        ui.horizontal(|ui| {
            ui.label(ui.t("timeline.time"));
            let mut t = time;
            if ui
                .add(DragValue::new(&mut t).speed(0.01).range(0.0..=3600.0).suffix(" s").max_decimals(3))
                .changed()
            {
                let mut tracks = tl.tracks.clone();
                let sel = std::iter::once((ti, ki)).collect();
                let new_sel = model::move_keys(&mut tracks, &sel, t - time);
                st.sel = new_sel;
                acts.push(Act::Tracks(tracks, format!("tl:{}:keytime:{}:{}", tl.id, ti, ki)));
            }
            ui.separator();
            ui.label(ui.t("timeline.value"));
            if value_editor(ui, &mut value) {
                let mut tracks = tl.tracks.clone();
                model::set_key_value(&mut tracks[ti].data, ki, value);
                acts.push(Act::Tracks(tracks, format!("tl:{}:keyval:{}:{}", tl.id, ti, ki)));
            }
        });
        let last = ki + 1 >= model::key_count(&tr.data);
        ui.label(
            RichText::new(ui.t(if last { "timeline.ease_last" } else { "timeline.ease" }))
                .small()
                .color(weak(ui)),
        );
        if !last
            && let Some(new) = curve_editor(
                ui,
                ("tl_ease", tl.id, ti, ki),
                &ease,
                &story.curves,
                CurveEditorOptions {
                    preview: false,
                    graph_size: 110.0,
                    ..Default::default()
                },
            )
        {
            let mut tracks = tl.tracks.clone();
            model::set_key_ease(&mut tracks[ti].data, ki, new);
            acts.push(Act::Tracks(tracks, format!("tl:{}:ease:{}:{}", tl.id, ti, ki)));
        }
        return;
    }

    // A track: the value at the playhead (auto-key records it).
    let Some(ti) = st.track else { return };
    let Some(tr) = tl.tracks.get(ti) else { return };
    let Some(mut value) = model::sample(&tr.data, st.playhead, resolver(story)) else {
        ui.label(RichText::new(ui.t("timeline.empty_track")).color(weak(ui)));
        return;
    };
    ui.label(RichText::new(ui.t("timeline.at_playhead")).strong());
    let on_key = model::key_at(&tr.data, st.playhead);
    let editable = st.auto_key || on_key.is_some();
    ui.horizontal(|ui| {
        ui.label(fmt_time(st.playhead));
        let changed = ui
            .add_enabled_ui(editable, |ui| value_editor(ui, &mut value))
            .inner;
        if changed {
            let mut tracks = tl.tracks.clone();
            if model::put_key(&mut tracks[ti].data, st.playhead, value, None).is_some() {
                acts.push(Act::Tracks(tracks, format!("tl:{}:autokey:{}:{}", tl.id, ti, st.drags)));
            }
        }
    });
    if !editable {
        ui.label(RichText::new(ui.t("timeline.autokey_hint")).small().color(weak(ui)));
    }
}

// -------------------------------------------------------- preview and apply

/// Asks the preview to show the stage at the playhead, once per change.
fn preview_sync(ctx: &mut EditorContext<'_>, tl: &Tl, st: &mut PanelState) {
    if !st.link_preview || !ctx.state().preview.available {
        return;
    }
    let ms = (st.playhead.max(0.0) * 1000.0).round() as u32;
    let version = ctx.node_version(tl.id);
    let key = (tl.id, ms, version);
    if st.sent != Some(key) {
        st.sent = Some(key);
        ctx.preview_command(PreviewCommand::ScrubAnimation(tl.id, ms));
    }
}

fn apply(ctx: &mut EditorContext<'_>, st: &mut PanelState, acts: Vec<Act>) {
    let Some(id) = st.node else { return };
    // The kind to edit is the live one (acts may be several in one frame).
    for act in acts {
        match act {
            Act::Tracks(tracks, key) => {
                let Some(kind) = ctx.story().and_then(|s| s.node(id)).map(|n| n.kind.clone()) else {
                    continue;
                };
                commit(ctx, id, rebuilt(&kind, Some(tracks), None), &key);
            }
            Act::EndMerge => ctx.end_merge(),
            Act::Duration(d) => {
                let Some(kind) = ctx.story().and_then(|s| s.node(id)).map(|n| n.kind.clone()) else {
                    continue;
                };
                commit(ctx, id, rebuilt(&kind, None, Some(d.max(0.0))), "timeline-duration");
            }
            Act::ConvertEffect(kind) => {
                commit(ctx, id, kind, "timeline-convert");
                ctx.end_merge();
                st.fitted = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_anim::{Key as AKey, Track};
    use orbit_core::{AnimTarget, TrackData};
    use super::super::timeline_model::KeyRef;

    fn project_with_timeline() -> (orbit_project::Project, NodeId) {
        use orbit_core::{Node, Scene, SceneId};
        let id = NodeId::new();
        let mut story = Story::new("t");
        let track = StoryTrack {
            target: AnimTarget::Background,
            prop: AnimProp::Opacity,
            data: TrackData::Scalar(Track::new(vec![AKey::new(0.0, 0.0), AKey::new(1.0, 1.0)])),
        };
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes: vec![Node {
                id,
                kind: NodeKind::Timeline {
                    duration: 2.0,
                    looping: AnimLoop::Once,
                    wait: false,
                    tracks: vec![track],
                },
            }],
        });
        (orbit_project::Project::new(story), id)
    }

    fn tracks_of(p: &orbit_project::Project, id: NodeId) -> Vec<StoryTrack> {
        timeline_of(p.story(), id).unwrap().tracks
    }

    /// What `apply` does for an `Act::Tracks`.
    fn do_tracks(p: &mut orbit_project::Project, id: NodeId, tracks: Vec<StoryTrack>, key: &str) {
        let kind = p.story().node(id).unwrap().kind.clone();
        p.execute_merged(
            orbit_project::Command::SetNodeKind {
                id,
                kind: rebuilt(&kind, Some(tracks), None),
                expected_version: p.node_version(id),
            },
            key,
        )
        .unwrap();
    }

    #[test]
    fn a_whole_key_drag_is_one_undo_step() {
        let (mut p, id) = project_with_timeline();
        let original = tracks_of(&p, id);
        let sel: BTreeSet<KeyRef> = std::iter::once((0, 1)).collect();
        // Seven frames of one drag, each computed from the original tracks.
        for dt in [0.1, 0.25, 0.4, 0.55, 0.7, 0.8, 0.9] {
            let mut t = original.clone();
            model::move_keys(&mut t, &sel, dt);
            do_tracks(&mut p, id, t, "tl:drag:1");
        }
        p.end_merge();
        assert_eq!(model::key_times(&tracks_of(&p, id)[0].data), vec![0.0, 1.9]);
        // A second drag is its own step.
        let after = tracks_of(&p, id);
        let mut t = after.clone();
        model::move_keys(&mut t, &sel, -0.4);
        do_tracks(&mut p, id, t, "tl:drag:2");
        p.end_merge();
        assert_eq!(model::key_times(&tracks_of(&p, id)[0].data), vec![0.0, 1.5]);

        assert!(p.undo().unwrap());
        assert_eq!(tracks_of(&p, id), after);
        assert!(p.undo().unwrap());
        assert_eq!(tracks_of(&p, id), original, "one undo for the whole first drag");
        assert!(p.redo().unwrap());
        assert_eq!(model::key_times(&tracks_of(&p, id)[0].data), vec![0.0, 1.9]);
    }

    #[test]
    fn adding_pasting_and_deleting_keys_undo_cleanly() {
        let (mut p, id) = project_with_timeline();
        let original = tracks_of(&p, id);
        let mut t = original.clone();
        model::add_key_at(&mut t[0].data, 0.5, |_| None).unwrap();
        do_tracks(&mut p, id, t, "tl:key:1");
        p.end_merge();
        let with_key = tracks_of(&p, id);
        assert_eq!(model::key_count(&with_key[0].data), 3);

        let clip = model::copy_keys(&with_key, &std::iter::once((0, 1)).collect());
        let mut t = with_key.clone();
        model::paste_keys(&mut t, &clip, 1.5, None);
        do_tracks(&mut p, id, t, "tl:paste:1");
        p.end_merge();
        assert_eq!(model::key_times(&tracks_of(&p, id)[0].data), vec![0.0, 0.5, 1.0, 1.5]);

        let mut t = tracks_of(&p, id);
        model::delete_keys(&mut t, &std::iter::once((0, 3)).collect());
        do_tracks(&mut p, id, t, "tl:del:1");
        p.end_merge();
        assert_eq!(model::key_count(&tracks_of(&p, id)[0].data), 3);

        p.undo().unwrap();
        assert_eq!(model::key_count(&tracks_of(&p, id)[0].data), 4);
        p.undo().unwrap();
        assert_eq!(tracks_of(&p, id), with_key);
        p.undo().unwrap();
        assert_eq!(tracks_of(&p, id), original);
    }

    #[test]
    fn following_the_selection_only_takes_animating_nodes() {
        let (p, id) = project_with_timeline();
        assert!(timeline_of(p.story(), id).is_some());
        assert!(timeline_of(p.story(), NodeId::new()).is_none());
    }

    #[test]
    fn rebuilt_replaces_only_what_it_is_given() {
        let kind = NodeKind::Timeline {
            duration: 2.0,
            looping: AnimLoop::Once,
            wait: true,
            tracks: vec![],
        };
        let t = StoryTrack {
            target: AnimTarget::Background,
            prop: AnimProp::Opacity,
            data: TrackData::Scalar(Track::new(vec![AKey::new(0.0, 1.0)])),
        };
        match rebuilt(&kind, Some(vec![t]), None) {
            NodeKind::Timeline { duration, wait, tracks, .. } => {
                assert_eq!(duration, 2.0);
                assert!(wait);
                assert_eq!(tracks.len(), 1);
            }
            _ => panic!("not a timeline"),
        }
        match rebuilt(&kind, None, Some(5.0)) {
            NodeKind::Timeline { duration, tracks, .. } => {
                assert_eq!(duration, 5.0);
                assert!(tracks.is_empty());
            }
            _ => panic!("not a timeline"),
        }
    }
}
