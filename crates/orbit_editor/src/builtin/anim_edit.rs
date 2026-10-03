//! Animated expressions in the characters panel: a still image or a frame
//! animation (sprite sheet / image sequence), with fps and loop mode.
//!
//! The first half is the pure model (edits of an [`Expression`], unit
//! tested); the second half is the egui widget built on it. The expression's
//! `image` stays the poster of the animation: the model keeps it on the first
//! frame (or on the sheet) so thumbnails and fallbacks always have a valid
//! picture.

use egui::{ComboBox, DragValue, RichText, Ui, vec2};
use orbit_core::{AnimFrames, Animation, AssetId, AssetKind, Expression, LoopMode, Motion};
use orbit_editor_api::fields::{self, asset_picker};
use orbit_editor_api::{EditorContext, UiText, icons};

use super::thumbs;
use super::ui_util::weak;

/// Frames per second of a new animation.
pub const DEFAULT_FPS: f32 = 12.0;
/// What the fps field accepts (validation only requires `> 0`).
pub const FPS_MIN: f32 = 0.5;
pub const FPS_MAX: f32 = 120.0;

/// What an expression is, as the panel offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Still,
    Sheet,
    Sequence,
    /// A skeletal clip of a `.rig.ron`.
    Rig,
}

/// The kind of an expression.
pub fn kind_of(e: &Expression) -> Kind {
    if e.rigged().is_some() {
        return Kind::Rig;
    }
    match e.animation().map(|a| &a.frames) {
        Some(AnimFrames::SpriteSheet { .. }) => Kind::Sheet,
        Some(AnimFrames::Sequence(_)) => Kind::Sequence,
        None => Kind::Still,
    }
}

/// Switches the expression to `kind`, keeping what can be kept (the picture,
/// fps and loop mode, the frames when going between sheet and sequence).
pub fn set_kind(e: &mut Expression, kind: Kind) {
    // Becoming a rig needs a rig asset: see `rig_expr::switch_to_rig`.
    if kind == Kind::Rig || (kind_of(e) == kind && !(kind == Kind::Still && e.motion.is_some())) {
        return;
    }
    let (fps, loop_mode) = e
        .animation()
        .map_or((DEFAULT_FPS, LoopMode::Loop), |a| (a.fps, a.loop_mode));
    e.motion = match kind {
        Kind::Still => None,
        Kind::Rig => return,
        Kind::Sheet => {
            let (asset, count) = match e.animation().map(|a| &a.frames) {
                Some(AnimFrames::Sequence(v)) if !v.is_empty() => (v[0], v.len() as u32),
                _ => (e.image, 4),
            };
            Some(Motion::Animation(Animation {
                frames: AnimFrames::SpriteSheet {
                    asset,
                    cols: count,
                    rows: 1,
                    frame_count: count,
                    frame_size: None,
                },
                fps,
                loop_mode,
            }))
        }
        Kind::Sequence => {
            let frames = match e.animation().map(|a| &a.frames) {
                Some(AnimFrames::SpriteSheet { asset, .. }) => vec![*asset],
                _ => vec![e.image],
            };
            Some(Motion::Animation(Animation {
                frames: AnimFrames::Sequence(frames),
                fps,
                loop_mode,
            }))
        }
    };
    sync_poster(e);
}

/// Points the poster `image` at the first frame (the sheet, or the first
/// image of the sequence). Does nothing for a still or an empty sequence.
pub fn sync_poster(e: &mut Expression) {
    let first = match e.animation().map(|a| &a.frames) {
        Some(AnimFrames::SpriteSheet { asset, .. }) => Some(*asset),
        Some(AnimFrames::Sequence(v)) => v.first().copied(),
        None => None,
    };
    if let Some(first) = first {
        e.image = first;
    }
}

/// Makes the expression a frame sequence of `frames` at `fps` (looping),
/// e.g. the frames of an imported GIF. Does nothing without frames.
pub fn set_sequence(e: &mut Expression, frames: Vec<AssetId>, fps: f32) {
    if frames.is_empty() {
        return;
    }
    e.motion = Some(Motion::Animation(Animation {
        frames: AnimFrames::Sequence(frames),
        fps: fps.clamp(FPS_MIN, FPS_MAX),
        loop_mode: LoopMode::Loop,
    }));
    sync_poster(e);
}

fn anim_mut(e: &mut Expression) -> Option<&mut Animation> {
    match &mut e.motion {
        Some(Motion::Animation(a)) => Some(a),
        _ => None,
    }
}

pub fn set_fps(e: &mut Expression, fps: f32) {
    if let Some(a) = anim_mut(e) {
        a.fps = fps.clamp(FPS_MIN, FPS_MAX);
    }
}

pub fn set_loop_mode(e: &mut Expression, mode: LoopMode) {
    if let Some(a) = anim_mut(e) {
        a.loop_mode = mode;
    }
}

/// Sets the sprite sheet image.
pub fn set_sheet_asset(e: &mut Expression, new: AssetId) {
    if let Some(Animation {
        frames: AnimFrames::SpriteSheet { asset, .. },
        ..
    }) = anim_mut(e)
    {
        *asset = new;
    }
    sync_poster(e);
}

/// Sets the grid; the frame count is kept within `1..=cols * rows`.
pub fn set_grid(e: &mut Expression, new_cols: u32, new_rows: u32) {
    if let Some(Animation {
        frames:
            AnimFrames::SpriteSheet {
                cols,
                rows,
                frame_count,
                ..
            },
        ..
    }) = anim_mut(e)
    {
        *cols = new_cols.max(1);
        *rows = new_rows.max(1);
        *frame_count = (*frame_count).clamp(1, cols.saturating_mul(*rows));
    }
}

/// Sets the number of frames, within `1..=cols * rows`.
pub fn set_frame_count(e: &mut Expression, count: u32) {
    if let Some(Animation {
        frames:
            AnimFrames::SpriteSheet {
                cols,
                rows,
                frame_count,
                ..
            },
        ..
    }) = anim_mut(e)
    {
        *frame_count = count.clamp(1, cols.saturating_mul(*rows).max(1));
    }
}

/// Sets (or clears) the explicit size of a cell, in pixels.
pub fn set_frame_size(e: &mut Expression, size: Option<[u32; 2]>) {
    if let Some(Animation {
        frames: AnimFrames::SpriteSheet { frame_size, .. },
        ..
    }) = anim_mut(e)
    {
        *frame_size = size.map(|[w, h]| [w.max(1), h.max(1)]);
    }
}

fn sequence_mut(e: &mut Expression) -> Option<&mut Vec<AssetId>> {
    match anim_mut(e) {
        Some(Animation {
            frames: AnimFrames::Sequence(v),
            ..
        }) => Some(v),
        _ => None,
    }
}

/// Appends a frame to the sequence.
pub fn push_frame(e: &mut Expression, asset: AssetId) {
    if let Some(v) = sequence_mut(e) {
        v.push(asset);
    }
    sync_poster(e);
}

/// Replaces frame `i` of the sequence.
pub fn set_frame(e: &mut Expression, i: usize, asset: AssetId) {
    if let Some(slot) = sequence_mut(e).and_then(|v| v.get_mut(i)) {
        *slot = asset;
    }
    sync_poster(e);
}

/// Removes frame `i` of the sequence.
pub fn remove_frame(e: &mut Expression, i: usize) {
    if let Some(v) = sequence_mut(e)
        && i < v.len()
    {
        v.remove(i);
    }
    sync_poster(e);
}

/// Moves frame `i` one place up (`delta < 0`) or down (`delta > 0`).
/// `false` when it is already at the end.
pub fn move_frame(e: &mut Expression, i: usize, delta: isize) -> bool {
    let Some(v) = sequence_mut(e) else {
        return false;
    };
    let Some(j) = i.checked_add_signed(delta).filter(|j| *j < v.len()) else {
        return false;
    };
    if i >= v.len() {
        return false;
    }
    v.swap(i, j);
    sync_poster(e);
    true
}

// ---------------------------------------------------------------- widget

/// What the panel must do after the widget ran.
pub enum Outcome {
    /// The expression changed (`Some(key)`: live edit, merge the undo entry).
    Replace(Expression, Option<String>),
    /// The user asked for a file to be imported as a new sequence frame.
    ImportFrame,
    /// The user asked to replace the frames with those of a GIF / APNG file.
    ImportAnimation,
}

pub(super) fn loop_label(ui: &Ui, mode: LoopMode) -> String {
    ui.t(match mode {
        LoopMode::Loop => "defs.anim.loop.loop",
        LoopMode::Once => "defs.anim.loop.once",
        LoopMode::PingPong => "defs.anim.loop.pingpong",
    })
}

fn first_image(story: &orbit_core::Story) -> Option<AssetId> {
    story
        .assets
        .assets
        .iter()
        .find(|a| a.kind == AssetKind::Image)
        .map(|a| a.id)
}

/// Draws the "still / animation" controls of expression `index` and returns
/// the requested change. For a still the caller keeps its own image picker.
pub fn editor(
    ui: &mut Ui,
    story: &orbit_core::Story,
    root: &std::path::Path,
    rigs: &super::rig_expr::RigUi,
    salt: (orbit_core::CharacterId, usize),
    e: &Expression,
) -> Option<Outcome> {
    let mut new = e.clone();
    let mut outcome: Option<Outcome> = None;
    let mut key: Option<String> = None;
    let kind = kind_of(e);

    // Still / animation / rig.
    ui.horizontal(|ui| {
        if ui
            .selectable_label(kind == Kind::Still, ui.t("defs.anim.still"))
            .clicked()
        {
            set_kind(&mut new, Kind::Still);
        }
        if ui
            .selectable_label(
                matches!(kind, Kind::Sheet | Kind::Sequence),
                ui.t("defs.anim.animation"),
            )
            .clicked()
            && matches!(kind, Kind::Still | Kind::Rig)
        {
            set_kind(&mut new, Kind::Sheet);
        }
        let has_rigs = super::rig_expr::has_rigs(story);
        let rig_button = ui
            .add_enabled(
                has_rigs || kind == Kind::Rig,
                egui::Button::selectable(kind == Kind::Rig, ui.t("defs.anim.rig")),
            )
            .on_hover_text(ui.t(if has_rigs {
                "defs.anim.rig.hint"
            } else {
                "defs.anim.rig.none"
            }))
            .on_disabled_hover_text(ui.t("defs.anim.rig.none"));
        if rig_button.clicked()
            && kind != Kind::Rig
            && let Some(rigged) = super::rig_expr::switch_to_rig(story, root, rigs, e)
        {
            new = rigged;
        }
    });
    if kind == Kind::Still {
        return (new != *e).then(|| Outcome::Replace(new, None));
    }
    if kind == Kind::Rig {
        if let Some(changed) = super::rig_expr::editor(ui, story, root, rigs, salt, &new) {
            new = changed;
        }
        return (new != *e).then(|| Outcome::Replace(new, None));
    }

    ui.horizontal(|ui| {
        if ui
            .selectable_label(kind == Kind::Sheet, ui.t("defs.anim.sheet"))
            .clicked()
        {
            set_kind(&mut new, Kind::Sheet);
        }
        if ui
            .selectable_label(kind == Kind::Sequence, ui.t("defs.anim.sequence"))
            .clicked()
        {
            set_kind(&mut new, Kind::Sequence);
        }
    });

    let Some(anim) = e.animation() else {
        return None;
    };
    match &anim.frames {
        AnimFrames::SpriteSheet {
            asset,
            cols,
            rows,
            frame_count,
            frame_size,
        } => {
            if let Some(a) = asset_picker(
                ui,
                ("anim_sheet", salt.0, salt.1),
                story,
                AssetKind::Image,
                *asset,
            ) {
                set_sheet_asset(&mut new, a);
            }
            ui.horizontal(|ui| {
                let (mut c, mut r, mut n) = (*cols, *rows, *frame_count);
                ui.label(RichText::new(ui.t("defs.anim.cols")).small().color(weak(ui)));
                if ui.add(DragValue::new(&mut c).range(1..=64)).changed() {
                    set_grid(&mut new, c, *rows);
                    key = Some("grid".into());
                }
                ui.label(RichText::new(ui.t("defs.anim.rows")).small().color(weak(ui)));
                if ui.add(DragValue::new(&mut r).range(1..=64)).changed() {
                    set_grid(&mut new, *cols, r);
                    key = Some("grid".into());
                }
                ui.label(RichText::new(ui.t("defs.anim.count")).small().color(weak(ui)));
                let max = cols.saturating_mul(*rows).max(1);
                if ui.add(DragValue::new(&mut n).range(1..=max)).changed() {
                    set_frame_count(&mut new, n);
                    key = Some("count".into());
                }
            });
            ui.horizontal(|ui| {
                if let Some(on) = fields::checkbox(ui, frame_size.is_some(), &ui.t("defs.anim.frame_size"))
                {
                    set_frame_size(&mut new, on.then_some([64, 64]));
                }
                if let Some([w, h]) = *frame_size {
                    let (mut w2, mut h2) = (w, h);
                    let cw = ui.add(DragValue::new(&mut w2).range(1..=16384).suffix(" px"));
                    ui.label("×");
                    let ch = ui.add(DragValue::new(&mut h2).range(1..=16384).suffix(" px"));
                    if cw.changed() || ch.changed() {
                        set_frame_size(&mut new, Some([w2, h2]));
                        key = Some("fsize".into());
                    }
                }
            });
        }
        AnimFrames::Sequence(frames) => {
            for (i, frame) in frames.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{}", i + 1))
                            .small()
                            .color(weak(ui)),
                    );
                    let width = (ui.available_width() - 92.0).max(80.0);
                    ui.allocate_ui(vec2(width, 22.0), |ui| {
                        if let Some(a) = asset_picker(
                            ui,
                            ("anim_frame", salt.0, salt.1, i),
                            story,
                            AssetKind::Image,
                            *frame,
                        ) {
                            set_frame(&mut new, i, a);
                        }
                    });
                    if ui
                        .add_enabled(
                            i > 0,
                            egui::Button::new(icons::ARROW_UP).small().frame(false),
                        )
                        .clicked()
                    {
                        move_frame(&mut new, i, -1);
                    }
                    if ui
                        .add_enabled(
                            i + 1 < frames.len(),
                            egui::Button::new(icons::ARROW_DOWN).small().frame(false),
                        )
                        .clicked()
                    {
                        move_frame(&mut new, i, 1);
                    }
                    if ui
                        .add(egui::Button::new(icons::X).small().frame(false))
                        .on_hover_text(ui.t("common.delete"))
                        .clicked()
                    {
                        remove_frame(&mut new, i);
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui
                    .small_button(format!("{}  {}", icons::PLUS, ui.t("defs.anim.add_frame")))
                    .clicked()
                    && let Some(a) = frames.last().copied().or_else(|| first_image(story))
                {
                    push_frame(&mut new, a);
                }
                if ui
                    .small_button(format!("{}  {}", icons::FILE_PLUS, ui.t("defs.file")))
                    .clicked()
                {
                    outcome = Some(Outcome::ImportFrame);
                }
                if ui
                    .small_button(format!("{}  {}", icons::FILE_PLUS, ui.t("defs.anim.import_gif")))
                    .on_hover_text(ui.t("defs.anim.import_gif.hint"))
                    .clicked()
                {
                    outcome = Some(Outcome::ImportAnimation);
                }
            });
        }
    }

    // fps and loop mode.
    ui.horizontal(|ui| {
        let mut fps = anim.fps;
        ui.label(RichText::new(ui.t("defs.anim.fps")).small().color(weak(ui)));
        if ui
            .add(
                DragValue::new(&mut fps)
                    .range(FPS_MIN..=FPS_MAX)
                    .speed(0.25)
                    .max_decimals(1),
            )
            .changed()
        {
            set_fps(&mut new, fps);
            key = Some("fps".into());
        }
        let mut mode = anim.loop_mode;
        ComboBox::from_id_salt(("anim_loop", salt.0, salt.1))
            .selected_text(loop_label(ui, mode))
            .show_ui(ui, |ui| {
                for m in [LoopMode::Loop, LoopMode::Once, LoopMode::PingPong] {
                    let label = loop_label(ui, m);
                    ui.selectable_value(&mut mode, m, label);
                }
            });
        if mode != anim.loop_mode {
            set_loop_mode(&mut new, mode);
        }
    });

    if outcome.is_none() && new != *e {
        let key = key.map(|k| format!("anim:{}:{}:{k}", salt.0, salt.1));
        outcome = Some(Outcome::Replace(new, key));
    }
    outcome
}

/// The thumbnail of an expression, playing its animation (frame by frame
/// from the cached thumbnails: a sprite sheet is cropped, a sequence swaps
/// textures). A still is drawn like any asset thumbnail.
pub fn preview(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    story: &orbit_core::Story,
    rigs: &super::rig_expr::RigUi,
    key: (orbit_core::CharacterId, usize),
    e: &Expression,
    max: egui::Vec2,
) {
    if e.rigged().is_some() {
        // The clip drawn by the host; the poster while the rig cannot play.
        if !super::rig_expr::preview(ui, ctx, story, rigs, key, e, max) {
            thumbs::show(ui, ctx, story, e.image, max);
        }
        return;
    }
    let Some(anim) = e.animation().filter(|a| a.is_animated()) else {
        thumbs::show(ui, ctx, story, e.image, max);
        return;
    };
    let time = ui.input(|i| i.time) as f32;
    let Some(frame) = anim.frame_at(time) else {
        thumbs::show(ui, ctx, story, e.image, max);
        return;
    };
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f32(1.0 / anim.fps.max(1.0)));
    let (asset, crop) = match frame {
        orbit_core::anim::FrameRef::Image(a) => (a, None),
        orbit_core::anim::FrameRef::Sheet { asset, index } => (asset, Some(index)),
    };
    let (Some(root), waker) = (thumbs::assets_root(ctx, story), ctx.state().waker.clone()) else {
        thumbs::show(ui, ctx, story, e.image, max);
        return;
    };
    let Some(texture) = thumbs::thumbnail(ui.ctx(), &waker, &root, story, asset) else {
        thumbs::show(ui, ctx, story, e.image, max);
        return;
    };
    let tex_px = [texture.size()[0] as u32, texture.size()[1] as u32];
    let (uv, cell) = match (crop, &anim.frames) {
        (
            Some(index),
            AnimFrames::SpriteSheet {
                cols,
                rows,
                frame_size,
                ..
            },
        ) => {
            let uv = orbit_core::anim::sheet_uv(index, *cols, *rows, *frame_size, tex_px)
                .unwrap_or([0.0, 0.0, 1.0, 1.0]);
            let cell = [
                (uv[2] - uv[0]) * tex_px[0] as f32,
                (uv[3] - uv[1]) * tex_px[1] as f32,
            ];
            (uv, cell)
        }
        _ => ([0.0, 0.0, 1.0, 1.0], [tex_px[0] as f32, tex_px[1] as f32]),
    };
    let scale = (max.x / cell[0].max(1.0)).min(max.y / cell[1].max(1.0));
    let size = vec2(cell[0] * scale, cell[1] * scale);
    ui.allocate_ui(max, |ui| {
        ui.add(
            egui::Image::new(&texture)
                .uv(egui::Rect::from_min_max(
                    egui::pos2(uv[0], uv[1]),
                    egui::pos2(uv[2], uv[3]),
                ))
                .fit_to_exact_size(size)
                .corner_radius(6),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::anim::FrameRef;

    fn still() -> (Expression, AssetId) {
        let img = AssetId::new();
        (Expression::still("happy", img), img)
    }

    #[test]
    fn still_to_sheet_to_sequence_keeps_the_picture() {
        let (mut e, img) = still();
        assert_eq!(kind_of(&e), Kind::Still);
        set_kind(&mut e, Kind::Sheet);
        assert_eq!(kind_of(&e), Kind::Sheet);
        let a = e.animation().unwrap();
        assert_eq!(a.fps, DEFAULT_FPS);
        assert_eq!(a.frames.assets(), vec![img]);
        assert_eq!(e.image, img);
        set_kind(&mut e, Kind::Sequence);
        assert_eq!(e.animation().unwrap().frames, AnimFrames::Sequence(vec![img]));
        set_kind(&mut e, Kind::Still);
        assert_eq!(e, Expression::still("happy", img));
    }

    #[test]
    fn an_imported_gif_becomes_a_looping_sequence_with_its_poster() {
        let (mut e, _) = still();
        let frames = vec![AssetId::new(), AssetId::new(), AssetId::new()];
        set_sequence(&mut e, frames.clone(), 12.0);
        let a = e.animation().unwrap();
        assert_eq!(a.frames, AnimFrames::Sequence(frames.clone()));
        assert_eq!((a.fps, a.loop_mode), (12.0, LoopMode::Loop));
        assert_eq!(e.image, frames[0]);
        // No frames: untouched.
        let before = e.clone();
        set_sequence(&mut e, Vec::new(), 12.0);
        assert_eq!(e, before);
    }

    #[test]
    fn sheet_edits_keep_the_frame_count_in_the_grid() {
        let (mut e, _) = still();
        set_kind(&mut e, Kind::Sheet);
        set_grid(&mut e, 3, 2);
        set_frame_count(&mut e, 6);
        set_frame_count(&mut e, 99);
        let AnimFrames::SpriteSheet { frame_count, .. } = e.animation().unwrap().frames else {
            panic!()
        };
        assert_eq!(frame_count, 6);
        set_grid(&mut e, 2, 2);
        let AnimFrames::SpriteSheet { frame_count, .. } = e.animation().unwrap().frames else {
            panic!()
        };
        assert_eq!(frame_count, 4);
        set_frame_size(&mut e, Some([0, 12]));
        let AnimFrames::SpriteSheet { frame_size, .. } = e.animation().unwrap().frames else {
            panic!()
        };
        assert_eq!(frame_size, Some([1, 12]));
        // What the editor produces is valid for the validator's rules.
        set_fps(&mut e, 0.0);
        assert!(e.animation().unwrap().fps > 0.0);
    }

    #[test]
    fn sequence_add_remove_reorder_and_poster() {
        let (mut e, first) = still();
        set_kind(&mut e, Kind::Sequence);
        let (b, c) = (AssetId::new(), AssetId::new());
        push_frame(&mut e, b);
        push_frame(&mut e, c);
        let frames = |e: &Expression| e.animation().unwrap().frames.assets();
        assert_eq!(frames(&e), vec![first, b, c]);
        assert!(move_frame(&mut e, 2, -1));
        assert_eq!(frames(&e), vec![first, c, b]);
        assert!(!move_frame(&mut e, 0, -1));
        assert!(!move_frame(&mut e, 2, 1));
        assert!(!move_frame(&mut e, 9, -1));
        // The poster follows the first frame.
        assert!(move_frame(&mut e, 1, -1));
        assert_eq!(e.image, c);
        remove_frame(&mut e, 0);
        assert_eq!(frames(&e), vec![first, b]);
        assert_eq!(e.image, first);
        remove_frame(&mut e, 7);
        assert_eq!(frames(&e).len(), 2);
        // The animation reads back through the shared frame logic.
        assert_eq!(
            e.animation().unwrap().frame_at(0.0),
            Some(FrameRef::Image(first))
        );
    }

    #[test]
    fn edits_survive_a_ron_round_trip() {
        let (mut e, _) = still();
        set_kind(&mut e, Kind::Sheet);
        set_grid(&mut e, 4, 3);
        set_frame_count(&mut e, 10);
        set_loop_mode(&mut e, LoopMode::PingPong);
        set_fps(&mut e, 24.0);
        let text = ron::to_string(&e).unwrap();
        let back: Expression = ron::from_str(&text).unwrap();
        assert_eq!(back, e);
        // Changing nothing on a still is a no-op.
        let (mut s, _) = still();
        let before = s.clone();
        set_kind(&mut s, Kind::Still);
        set_fps(&mut s, 30.0);
        set_grid(&mut s, 2, 2);
        push_frame(&mut s, AssetId::new());
        assert_eq!(s, before);
    }
}
