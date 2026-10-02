//! The scene view ("Éditeur de vue"): the stage as it is at the selected
//! node, with the characters editable in place.
//!
//! The picture is the real runtime: the host (`crate::preview_host`)
//! simulates the story up to the selected node, draws that stage with the
//! game's own renderer into a texture and publishes where each character
//! stands (`EditorState::scene_view`). This panel draws the texture and puts
//! egui gizmos on top, computed from the same numbers:
//!
//! * the character placed by the selected `Show` is framed with handles;
//!   dragging it makes the `Show` a free position, updated live, **one undo
//!   entry per drag**; the handles scale it around its feet; Shift keeps the
//!   ratio; Escape cancels the drag;
//! * arrows nudge (Shift for a bigger step); the toolbar mirrors on X / Y;
//! * positions snap to the named places and to a grid, and the guides show
//!   left / center / right and the 16:9 safe margins;
//! * a click on another character selects the `Show` that placed it.
//!
//! All the geometry is in [`geom`] (pure, tested); this file is the egui
//! plumbing and the translation of gestures into project commands.

mod geom;

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontId, Id, Key, Pos2, Rect, RichText,
    Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use orbit_core::{NodeId, Position};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, Location,
    PanelDef, Plugin, PluginManifest, SceneViewActor, Selection, icons,
};
use orbit_project::ops::{self, ShowLook};

use self::geom::{CANVAS, Handle, NamedPoint, Pose, SnapOptions};
use super::ids;
use super::preview::fit_16_9;
use super::ui_util::{accent, faint, strong, warning, weak};

pub struct SceneViewPlugin;

const STATUS_HEIGHT: f32 = 22.0;
const HANDLE_POINTS: f32 = 6.0;
/// Box used for a character whose image is not available (no GPU, missing
/// file): it can still be placed.
const FALLBACK_SIZE: [f32; 2] = [300.0, 500.0];

impl Plugin for SceneViewPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::SCENEVIEW, "Éditeur de vue", "0.1.0")
    }
}

impl EditorPlugin for SceneViewPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "sceneview",
            "Éditeur de vue",
            icons::FRAME_CORNERS,
            DockSlot::RightTop,
            panel,
        ));
        reg.action(
            ActionDef::new("flip_x", "Personnage : miroir horizontal", |ctx| {
                flip_selected(ctx, true);
            })
            .icon(icons::FLIP_HORIZONTAL)
            .category("Éditeur de vue")
            .enabled(|c| selected_show(c).is_some()),
        );
        reg.action(
            ActionDef::new("flip_y", "Personnage : miroir vertical", |ctx| {
                flip_selected(ctx, false);
            })
            .icon(icons::FLIP_VERTICAL)
            .category("Éditeur de vue")
            .enabled(|c| selected_show(c).is_some()),
        );
        reg.action(
            ActionDef::new("fullscreen", "Éditeur de vue : plein panneau", |ctx| {
                ctx.request(EditorRequest::ToggleMaximizePanel(
                    super::panels::SCENEVIEW.to_owned(),
                ));
            })
            .icon(icons::CORNERS_OUT)
            .category("Éditeur de vue"),
        );
    }
}

// ------------------------------------------------------------- selection

/// The node whose stage the view shows: the selected node, or the first one
/// of a selected scene.
fn wanted_node(ctx: &EditorContext<'_>) -> Option<NodeId> {
    match ctx.selection()? {
        Selection::Node(id) => Some(id),
        Selection::Scene(scene) => ctx.story()?.scene(scene)?.nodes.first().map(|n| n.id),
    }
}

/// The selected node when it is a `Show`.
fn selected_show(ctx: &EditorContext<'_>) -> Option<NodeId> {
    match ctx.selection()? {
        Selection::Node(id) => ops::show_look(ctx.story()?, id).map(|_| id),
        Selection::Scene(_) => None,
    }
}

fn flip_selected(ctx: &mut EditorContext<'_>, x_axis: bool) {
    let Some(node) = selected_show(ctx) else {
        return;
    };
    let Some(mut look) = ctx.story().and_then(|s| ops::show_look(s, node)) else {
        return;
    };
    if x_axis {
        look.flip_x = !look.flip_x;
    } else {
        look.flip_y = !look.flip_y;
    }
    ctx.ops_or_toast(|p| ops::set_show_look(p, node, &look));
}

// ---------------------------------------------------------------- toggles

#[derive(Clone, Copy, Debug, PartialEq)]
struct Toggles {
    /// Left / center / right guides and the safe margins.
    guides: bool,
    /// Snap to the named positions.
    snap: bool,
    grid: bool,
}

fn toggles(ctx: &EditorContext<'_>) -> Toggles {
    let get =
        |key: &str, default: bool| ctx.pref(ids::SCENEVIEW, key).map_or(default, |v| v == "1");
    Toggles {
        guides: get("guides", true),
        snap: get("snap", true),
        grid: get("grid", false),
    }
}

fn set_toggle(ctx: &mut EditorContext<'_>, key: &str, on: bool) {
    ctx.set_pref(ids::SCENEVIEW, key, if on { "1" } else { "0" });
}

// ------------------------------------------------------------------- drag

#[derive(Clone, Debug)]
enum Mode {
    /// Moving the anchor; `grab` is the pointer's offset from it (virtual px).
    Move {
        grab: [f32; 2],
    },
    Scale(Handle),
}

/// A gesture in progress. Lives in egui's memory between frames.
#[derive(Clone, Debug)]
struct Drag {
    node: NodeId,
    mode: Mode,
    /// The `Show`'s look when the gesture began.
    start_look: ShowLook,
    start: Pose,
    base: [f32; 2],
    /// The pose being edited (what the gizmo draws).
    current: Pose,
    /// Snapped onto this named position.
    named: Option<Position>,
    guides: (Option<f32>, Option<f32>),
    /// At least one command was applied (an undo entry exists).
    edited: bool,
}

fn drag_id() -> Id {
    Id::new("orbit_sceneview_drag")
}

/// The `Show` look a drag stands for.
fn look_of(drag: &Drag) -> ShowLook {
    let mut look = drag.start_look.clone();
    match drag.mode {
        Mode::Move { .. } => {
            look.position = drag.named.clone().unwrap_or(Position::Free {
                x: drag.current.x,
                y: drag.current.y,
            });
        }
        Mode::Scale(_) => look.set_effective_scale(drag.current.scale),
    }
    look
}

/// Applies the drag's look live; every frame of one gesture merges into one
/// undo entry.
fn apply_drag(ctx: &mut EditorContext<'_>, drag: &mut Drag) {
    let look = look_of(drag);
    let Some(story) = ctx.story() else { return };
    match ops::show_look_command(story, drag.node, &look) {
        Ok(command) => {
            let key = format!("sceneview:{}", drag.node);
            match ctx.exec_merged(command, &key) {
                Ok(()) => drag.edited = true,
                Err(e) => ctx.error(e.to_string()),
            }
        }
        Err(e) => ctx.error(e.to_string()),
    }
}

/// Everything the pointer logic needs about the selected character.
#[derive(Clone, Debug)]
struct Target {
    node: NodeId,
    look: ShowLook,
    pose: Pose,
    base: [f32; 2],
    default_scale: f32,
}

fn target_of(ctx: &EditorContext<'_>, node: NodeId, actors: &[SceneViewActor]) -> Option<Target> {
    let story = ctx.story()?;
    let look = ops::show_look(story, node)?;
    let actor = actors.iter().find(|a| a.source == Some(node))?;
    let default_scale = match story.node(node).map(|n| &n.kind) {
        Some(orbit_core::NodeKind::Show { character, .. }) => {
            story.character(*character).map_or(1.0, |c| c.default_scale)
        }
        _ => 1.0,
    };
    let base = if actor.base_size[0] > 0.0 && actor.base_size[1] > 0.0 {
        actor.base_size
    } else {
        FALLBACK_SIZE
    };
    Some(Target {
        node,
        look,
        pose: Pose {
            x: actor.x,
            y: actor.y,
            scale: actor.scale,
        },
        base,
        default_scale,
    })
}

// ------------------------------------------------------------------ panel

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let wanted = wanted_node(ctx);
    {
        let sv = &mut ctx.state_mut().scene_view;
        sv.visible = true;
        sv.wanted_node = wanted;
    }
    let flags = toolbar(ui, ctx);
    ui.add_space(2.0);

    let avail = ui.available_size();
    let size = vec2(avail.x.max(40.0), (avail.y - STATUS_HEIGHT).max(40.0));
    let (area, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let picture = fit_16_9(area);
    let ppp = ui.ctx().pixels_per_point();
    {
        let sv = &mut ctx.state_mut().scene_view;
        sv.picture_rect = Some(picture);
        sv.wanted_size = [
            (picture.width() * ppp).round() as u32,
            (picture.height() * ppp).round() as u32,
        ];
    }
    if response.has_focus() {
        // Arrows nudge the character: they must not move the focus.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
    }

    // What the host published, when it is about the node we ask for.
    let (actors, texture, notice, available) = {
        let sv = &ctx.state().scene_view;
        let current = wanted.is_some() && sv.node == wanted;
        (
            if current {
                sv.actors.clone()
            } else {
                Vec::new()
            },
            sv.texture,
            sv.notice.clone(),
            sv.available,
        )
    };
    let show = selected_show(ctx);
    let target = show.and_then(|n| target_of(ctx, n, &actors));
    let points: Vec<NamedPoint> = ctx.story().map(geom::named_points).unwrap_or_default();
    let named_for_guides: Vec<(String, f32, f32)> =
        points.iter().map(|p| (p.name.clone(), p.x, p.y)).collect();

    let view = View { picture };
    let mut drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id()));
    // A drag of a node that is no longer selected is over.
    if drag.as_ref().is_some_and(|d| Some(d.node) != show) {
        drag = None;
        ctx.end_merge();
    }

    interact(
        ui,
        ctx,
        &response,
        &view,
        &actors,
        target.as_ref(),
        &points,
        flags,
        &mut drag,
    );

    ctx.state_mut().scene_view.interacting = drag.is_some();
    ui.data_mut(|data| match &drag {
        Some(d) => {
            data.insert_temp(drag_id(), d.clone());
        }
        None => data.remove::<Drag>(drag_id()),
    });

    paint(
        ui,
        &view,
        &response,
        texture,
        &actors,
        target.as_ref(),
        drag.as_ref(),
        flags,
        &named_for_guides,
        PaintInfo {
            available,
            has_project: ctx.has_project(),
            wanted: wanted.is_some(),
            scene_view_node_ready: !actors.is_empty() || ctx.state().scene_view.node == wanted,
        },
    );
    status_line(ui, ctx, target.as_ref(), drag.as_ref(), notice.as_deref());
}

/// The picture's place on screen and the maps between screen points and the
/// virtual canvas.
struct View {
    picture: Rect,
}

impl View {
    /// Screen points per virtual pixel.
    fn k(&self) -> f32 {
        self.picture.width() / CANVAS[0]
    }

    fn to_virtual(&self, p: Pos2) -> [f32; 2] {
        [
            (p.x - self.picture.left()) / self.k(),
            (p.y - self.picture.top()) / self.k(),
        ]
    }

    fn to_screen(&self, v: [f32; 2]) -> Pos2 {
        pos2(
            self.picture.left() + v[0] * self.k(),
            self.picture.top() + v[1] * self.k(),
        )
    }

    fn rect(&self, b: [f32; 4]) -> Rect {
        Rect::from_two_pos(self.to_screen([b[0], b[1]]), self.to_screen([b[2], b[3]]))
    }
}

// ---------------------------------------------------------------- toolbar

fn toolbar(ui: &mut Ui, ctx: &mut EditorContext<'_>) -> Toggles {
    let mut flags = toggles(ctx);
    let look = selected_show(ctx).and_then(|n| ctx.story().and_then(|s| ops::show_look(s, n)));
    ui.horizontal(|ui| {
        ui.add_enabled_ui(look.is_some(), |ui| {
            let (fx, fy) = look
                .as_ref()
                .map_or((false, false), |l| (l.flip_x, l.flip_y));
            if ui
                .selectable_label(fx, format!("{}  Miroir X", icons::FLIP_HORIZONTAL))
                .on_hover_text("Inverse le personnage de gauche à droite")
                .clicked()
            {
                flip_selected(ctx, true);
            }
            if ui
                .selectable_label(fy, format!("{}  Miroir Y", icons::FLIP_VERTICAL))
                .on_hover_text("Inverse le personnage de haut en bas")
                .clicked()
            {
                flip_selected(ctx, false);
            }
        });
        ui.separator();
        let toggle = |ui: &mut Ui, on: &mut bool, icon: &str, label: &str, tip: &str| {
            let before = *on;
            if ui
                .selectable_label(*on, format!("{icon}  {label}"))
                .on_hover_text(tip)
                .clicked()
            {
                *on = !*on;
            }
            *on != before
        };
        if toggle(
            ui,
            &mut flags.guides,
            icons::RULER,
            "Repères",
            "Gauche / centre / droite et marges de sécurité 16:9",
        ) {
            set_toggle(ctx, "guides", flags.guides);
        }
        if toggle(
            ui,
            &mut flags.snap,
            icons::MAGNET,
            "Magnétisme",
            "Aimante aux positions nommées (Alt : désactive pendant le glissement)",
        ) {
            set_toggle(ctx, "snap", flags.snap);
        }
        if toggle(
            ui,
            &mut flags.grid,
            icons::GRID_FOUR,
            "Grille",
            "Aimante à une grille de 5 %",
        ) {
            set_toggle(ctx, "grid", flags.grid);
        }
    });
    flags
}

// ------------------------------------------------------------ interaction

#[allow(clippy::too_many_arguments)]
fn interact(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    response: &egui::Response,
    view: &View,
    actors: &[SceneViewActor],
    target: Option<&Target>,
    points: &[NamedPoint],
    flags: Toggles,
    drag: &mut Option<Drag>,
) {
    let (mods, escape) = ui.input(|i| (i.modifiers, i.key_pressed(Key::Escape)));
    let k = view.k();
    let radius = HANDLE_POINTS * 1.8 / k;
    let snap = SnapOptions {
        named: flags.snap && !mods.alt,
        grid: flags.grid && !mods.alt,
        threshold: [
            geom::SNAP_POINTS / view.picture.width().max(1.0),
            geom::SNAP_POINTS / view.picture.height().max(1.0),
        ],
    };

    // Focus: a press on the picture gives the keyboard to the view.
    if response.is_pointer_button_down_on() {
        response.request_focus();
    }

    // ---- begin a drag on the selected character
    if response.drag_started_by(egui::PointerButton::Primary)
        && drag.is_none()
        && let Some(t) = target
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        let p = view.to_virtual(origin);
        let b = geom::bounds(&t.pose, t.base);
        let mode = match geom::hit_handle(b, p, radius) {
            Some(h) => Some(Mode::Scale(h)),
            None if geom::contains(b, p) => {
                let a = geom::anchor(&t.pose);
                Some(Mode::Move {
                    grab: [p[0] - a[0], p[1] - a[1]],
                })
            }
            None => None,
        };
        if let Some(mode) = mode {
            *drag = Some(Drag {
                node: t.node,
                mode,
                start_look: t.look.clone(),
                start: t.pose,
                base: t.base,
                current: t.pose,
                named: None,
                guides: (None, None),
                edited: false,
            });
        }
    }

    // ---- follow the pointer
    if let Some(d) = drag.as_mut() {
        if escape {
            // Cancel: undo what this gesture did so far.
            let edited = d.edited;
            ctx.end_merge();
            if edited {
                let _ = ctx.undo();
            }
            *drag = None;
        } else if response.dragged()
            && let Some(pos) = response.interact_pointer_pos().or(response.hover_pos())
        {
            let p = view.to_virtual(pos);
            match d.mode.clone() {
                Mode::Move { grab } => {
                    let wanted = [(p[0] - grab[0]) / CANVAS[0], (p[1] - grab[1]) / CANVAS[1]];
                    let s = geom::snap_position(wanted, points, snap);
                    d.current.x = s.x;
                    d.current.y = s.y;
                    d.named = s.named;
                    d.guides = (s.guide_x, s.guide_y);
                }
                Mode::Scale(handle) => {
                    let mut scale = geom::scale_with_handle(
                        handle,
                        d.start.scale,
                        d.base,
                        geom::anchor(&d.start),
                        p,
                        mods.shift,
                    );
                    if snap.named {
                        let target_default = target.map_or(1.0, |t| t.default_scale);
                        scale = geom::snap_scale(scale, &[1.0, target_default]);
                    }
                    d.current.scale = scale;
                }
            }
            let mut owned = d.clone();
            apply_drag(ctx, &mut owned);
            *d = owned;
        }
        if response.drag_stopped() {
            ctx.end_merge();
            *drag = None;
        }
    }

    // ---- keyboard: arrows nudge the selected character
    if drag.is_none()
        && response.has_focus()
        && let Some(t) = target
    {
        let (dx, dy, big) = ui.input(|i| {
            let step =
                |neg: Key, pos: Key| i8::from(i.key_pressed(pos)) - i8::from(i.key_pressed(neg));
            (
                step(Key::ArrowLeft, Key::ArrowRight),
                step(Key::ArrowUp, Key::ArrowDown),
                i.modifiers.shift,
            )
        });
        if dx != 0 || dy != 0 {
            let [x, y] = geom::nudged(&t.pose, [dx, dy], big);
            let mut look = t.look.clone();
            look.position = Position::Free { x, y };
            if let Some(story) = ctx.story()
                && let Ok(command) = ops::show_look_command(story, t.node, &look)
            {
                ctx.exec_or_toast(command);
            }
        }
    }

    // ---- a click selects the character under the pointer
    if response.clicked_by(egui::PointerButton::Primary)
        && drag.is_none()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let p = view.to_virtual(pos);
        let hit = actors.iter().rev().find(|a| {
            let base = if a.base_size[0] > 0.0 {
                a.base_size
            } else {
                FALLBACK_SIZE
            };
            let b = geom::bounds(
                &Pose {
                    x: a.x,
                    y: a.y,
                    scale: a.scale,
                },
                base,
            );
            geom::contains(b, p)
        });
        if let Some(source) = hit.and_then(|a| a.source)
            && Some(source) != target.map(|t| t.node)
        {
            ctx.reveal(Location::Node(source));
        }
    }

    // ---- cursor
    if let Some(d) = drag.as_ref() {
        ui.ctx().set_cursor_icon(match d.mode {
            Mode::Move { .. } => CursorIcon::Grabbing,
            Mode::Scale(h) => cursor_of(h),
        });
    } else if let (Some(t), Some(pos)) = (target, response.hover_pos()) {
        let p = view.to_virtual(pos);
        let b = geom::bounds(&t.pose, t.base);
        if let Some(h) = geom::hit_handle(b, p, radius) {
            ui.ctx().set_cursor_icon(cursor_of(h));
        } else if geom::contains(b, p) {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
    }
}

fn cursor_of(handle: Handle) -> CursorIcon {
    match handle {
        Handle::TopLeft | Handle::BottomRight => CursorIcon::ResizeNwSe,
        Handle::TopRight | Handle::BottomLeft => CursorIcon::ResizeNeSw,
        Handle::Left | Handle::Right => CursorIcon::ResizeHorizontal,
        Handle::Top => CursorIcon::ResizeVertical,
    }
}

// ----------------------------------------------------------------- drawing

struct PaintInfo {
    available: bool,
    has_project: bool,
    wanted: bool,
    scene_view_node_ready: bool,
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &mut Ui,
    view: &View,
    response: &egui::Response,
    texture: Option<egui::TextureId>,
    actors: &[SceneViewActor],
    target: Option<&Target>,
    drag: Option<&Drag>,
    flags: Toggles,
    named: &[(String, f32, f32)],
    info: PaintInfo,
) {
    let picture = view.picture;
    let painter = ui.painter_at(response.rect);
    let accent = accent(ui);

    // The stage: the real renderer's picture, or a plain backdrop.
    painter.rect_filled(picture, CornerRadius::same(2), Color32::from_gray(24));
    let drawn = match texture {
        Some(texture) if info.available && info.wanted => {
            painter.image(
                texture,
                picture,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            true
        }
        _ => false,
    };

    // Grid.
    if flags.grid {
        let faint_line = Stroke::new(1.0, Color32::from_white_alpha(22));
        let steps = (1.0 / geom::GRID_STEP).round() as i32;
        for i in 1..steps {
            let f = i as f32 / steps as f32;
            let x = picture.left() + f * picture.width();
            let y = picture.top() + f * picture.height();
            painter.line_segment(
                [pos2(x, picture.top()), pos2(x, picture.bottom())],
                faint_line,
            );
            painter.line_segment(
                [pos2(picture.left(), y), pos2(picture.right(), y)],
                faint_line,
            );
        }
    }

    // Guides: safe margins and the named positions.
    if flags.guides {
        let line = Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 200, 255, 90));
        let m = geom::SAFE_MARGIN;
        let safe = Rect::from_min_max(
            view.to_screen([m * CANVAS[0], m * CANVAS[1]]),
            view.to_screen([(1.0 - m) * CANVAS[0], (1.0 - m) * CANVAS[1]]),
        );
        dashed_rect(&painter, safe, line);
        painter.text(
            safe.left_top() + vec2(4.0, 2.0),
            Align2::LEFT_TOP,
            "marge de sécurité",
            FontId::proportional(9.5),
            Color32::from_rgba_unmultiplied(120, 200, 255, 130),
        );
        for (name, x, y) in named {
            let top = view.to_screen([x * CANVAS[0], 0.0]);
            let bottom = view.to_screen([x * CANVAS[0], CANVAS[1]]);
            // The three built-in places are full-height guides; a custom
            // place is a cross at its spot.
            if *y >= 0.999 {
                dashed_line(&painter, top, bottom, line);
                painter.text(
                    pos2(top.x + 3.0, picture.bottom() - 3.0),
                    Align2::LEFT_BOTTOM,
                    name,
                    FontId::proportional(9.5),
                    Color32::from_rgba_unmultiplied(120, 200, 255, 150),
                );
            } else {
                let c = view.to_screen([x * CANVAS[0], y * CANVAS[1]]);
                painter.line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], line);
                painter.line_segment([c - vec2(0.0, 5.0), c + vec2(0.0, 5.0)], line);
                painter.text(
                    c + vec2(6.0, -2.0),
                    Align2::LEFT_BOTTOM,
                    name,
                    FontId::proportional(9.5),
                    Color32::from_rgba_unmultiplied(120, 200, 255, 150),
                );
            }
        }
    }

    // Characters: a faint outline each (and, without the real picture, a
    // stand-in box), the hovered one lit.
    let hover = response.hover_pos().map(|p| view.to_virtual(p));
    for a in actors {
        let selected = target.is_some_and(|t| a.source == Some(t.node));
        if selected {
            continue;
        }
        let base = if a.base_size[0] > 0.0 {
            a.base_size
        } else {
            FALLBACK_SIZE
        };
        let b = geom::bounds(
            &Pose {
                x: a.x,
                y: a.y,
                scale: a.scale,
            },
            base,
        );
        let r = view.rect(b);
        if !drawn {
            stand_in(&painter, r, a);
        }
        let lit = hover.is_some_and(|h| geom::contains(b, h));
        let stroke = if lit {
            Stroke::new(1.5, accent.gamma_multiply(0.9))
        } else {
            Stroke::new(1.0, Color32::from_white_alpha(40))
        };
        painter.rect_stroke(r, CornerRadius::ZERO, stroke, StrokeKind::Inside);
        if lit {
            painter.text(
                r.left_top() + vec2(4.0, 3.0),
                Align2::LEFT_TOP,
                "cliquer : sélectionner son « Afficher »",
                FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
    }

    // The selected character: frame, handles, snap guides.
    if let Some(t) = target {
        let pose = drag.map_or(t.pose, |d| d.current);
        let b = geom::bounds(&pose, t.base);
        let r = view.rect(b);
        if !drawn {
            stand_in_of(&painter, r, "sélection");
        }
        painter.rect_stroke(
            r,
            CornerRadius::ZERO,
            Stroke::new(2.0, accent),
            StrokeKind::Outside,
        );
        let anchor = view.to_screen(geom::anchor(&pose));
        painter.circle(anchor, 4.0, Color32::WHITE, Stroke::new(1.5, accent));
        for h in Handle::ALL {
            let c = view.to_screen(h.point(b));
            let half = if h.is_corner() {
                HANDLE_POINTS
            } else {
                HANDLE_POINTS - 1.0
            };
            let hr = Rect::from_center_size(c, vec2(half * 2.0, half * 2.0));
            painter.rect(
                hr,
                CornerRadius::same(2),
                Color32::WHITE,
                Stroke::new(1.5, accent),
                StrokeKind::Inside,
            );
        }
        if let Some(d) = drag {
            let guide = Stroke::new(1.0, Color32::from_rgb(255, 196, 64));
            if let Some(x) = d.guides.0 {
                let top = view.to_screen([x * CANVAS[0], 0.0]);
                let bottom = view.to_screen([x * CANVAS[0], CANVAS[1]]);
                painter.line_segment([top, bottom], guide);
            }
            if let Some(y) = d.guides.1 {
                let left = view.to_screen([0.0, y * CANVAS[1]]);
                let right = view.to_screen([CANVAS[0], y * CANVAS[1]]);
                painter.line_segment([left, right], guide);
            }
            // Live readout next to the anchor.
            let text = match d.mode {
                Mode::Move { .. } => match &d.named {
                    Some(p) => super::nodeinfo::position_name(p),
                    None => format!("{:.3}, {:.3}", d.current.x, d.current.y),
                },
                Mode::Scale(_) => {
                    if (d.current.scale[0] - d.current.scale[1]).abs() < 1e-4 {
                        format!("×{:.2}", d.current.scale[0])
                    } else {
                        format!("×{:.2} / ×{:.2}", d.current.scale[0], d.current.scale[1])
                    }
                }
            };
            let pos = anchor + vec2(10.0, 8.0);
            let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), Color32::WHITE);
            let box_rect = Rect::from_min_size(pos, galley.size() + vec2(10.0, 6.0));
            painter.rect_filled(
                box_rect,
                CornerRadius::same(4),
                Color32::from_black_alpha(190),
            );
            painter.galley(box_rect.min + vec2(5.0, 3.0), galley, Color32::WHITE);
        }
    }

    // Frame of the picture.
    painter.rect_stroke(
        picture,
        CornerRadius::same(2),
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        StrokeKind::Outside,
    );
    if response.has_focus() {
        painter.rect_stroke(
            picture,
            CornerRadius::same(2),
            Stroke::new(1.5, accent),
            StrokeKind::Outside,
        );
    }

    // What the picture does not say by itself.
    let message = if !info.has_project {
        Some((
            "Aucun projet ouvert",
            "Ouvrez un projet pour placer ses personnages.",
        ))
    } else if !info.wanted {
        Some((
            "Sélectionnez un nœud",
            "L'éditeur de vue montre la scène telle qu'elle est à ce nœud.",
        ))
    } else if !info.available && info.scene_view_node_ready {
        Some((
            "Aperçu indisponible",
            "Aucun GPU n'a pu être initialisé ; les cadres restent éditables.",
        ))
    } else {
        None
    };
    if let Some((title, sub)) = message {
        painter.text(
            picture.center() - vec2(0.0, 8.0),
            Align2::CENTER_CENTER,
            title,
            FontId::proportional(15.0),
            weak(ui),
        );
        painter.text(
            picture.center() + vec2(0.0, 12.0),
            Align2::CENTER_CENTER,
            sub,
            FontId::proportional(12.0),
            faint(ui),
        );
    } else if target.is_none() && info.wanted {
        // A node that is not a `Show`: say what to do.
        painter.text(
            picture.left_top() + vec2(8.0, 8.0),
            Align2::LEFT_TOP,
            "Sélectionnez un nœud « Afficher » pour placer son personnage",
            FontId::proportional(11.0),
            Color32::from_white_alpha(170),
        );
    }
}

fn stand_in(painter: &egui::Painter, r: Rect, a: &SceneViewActor) {
    stand_in_of(
        painter,
        r,
        &format!("{:?}", a.character)
            .chars()
            .take(6)
            .collect::<String>(),
    );
}

/// A box drawn instead of a missing picture.
fn stand_in_of(painter: &egui::Painter, r: Rect, label: &str) {
    painter.rect_filled(r, CornerRadius::same(3), Color32::from_white_alpha(18));
    painter.text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.0),
        Color32::from_white_alpha(120),
    );
}

fn dashed_line(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let len = (b - a).length();
    if len <= 0.0 {
        return;
    }
    let dir = (b - a) / len;
    let (dash, gap) = (5.0, 4.0);
    let mut t = 0.0;
    while t < len {
        let end = (t + dash).min(len);
        painter.line_segment([a + dir * t, a + dir * end], stroke);
        t += dash + gap;
    }
}

fn dashed_rect(painter: &egui::Painter, r: Rect, stroke: Stroke) {
    dashed_line(painter, r.left_top(), r.right_top(), stroke);
    dashed_line(painter, r.right_top(), r.right_bottom(), stroke);
    dashed_line(painter, r.right_bottom(), r.left_bottom(), stroke);
    dashed_line(painter, r.left_bottom(), r.left_top(), stroke);
}

fn status_line(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    target: Option<&Target>,
    drag: Option<&Drag>,
    notice: Option<&str>,
) {
    ui.horizontal(|ui| {
        match target {
            Some(t) => {
                let pose = drag.map_or(t.pose, |d| d.current);
                let story = ctx.story();
                let name = story
                    .and_then(|s| match s.node(t.node).map(|n| &n.kind) {
                        Some(orbit_core::NodeKind::Show { character, .. }) => {
                            s.text(s.character(*character)?.name_id).map(str::to_owned)
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                ui.label(strong(name, 12.0));
                let look = drag.map_or_else(|| t.look.clone(), look_of);
                let mut text = format!(
                    "{}  ·  ×{:.2}",
                    super::nodeinfo::position_name(&look.position),
                    pose.scale[0]
                );
                if (pose.scale[0] - pose.scale[1]).abs() > 1e-4 {
                    text.push_str(&format!(" / ×{:.2}", pose.scale[1]));
                }
                if look.flip_x {
                    text.push_str("  ·  miroir X");
                }
                if look.flip_y {
                    text.push_str("  ·  miroir Y");
                }
                ui.label(RichText::new(text).size(11.5).color(weak(ui)));
            }
            None => {
                ui.label(
                    RichText::new("Aucun personnage sélectionné")
                        .size(11.5)
                        .color(weak(ui)),
                );
            }
        }
        if let Some(notice) = notice {
            ui.label(RichText::new(icons::WARNING).color(warning(ui)))
                .on_hover_text(notice);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(
                    "glisser : placer · poignées : échelle (Maj : ratio) · flèches : ajuster",
                )
                .size(10.5)
                .color(faint(ui)),
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_maps_between_screen_and_canvas() {
        let v = View {
            picture: Rect::from_min_size(pos2(100.0, 50.0), vec2(960.0, 540.0)),
        };
        assert_eq!(v.k(), 0.5);
        assert_eq!(v.to_virtual(pos2(100.0, 50.0)), [0.0, 0.0]);
        assert_eq!(v.to_virtual(pos2(580.0, 320.0)), [960.0, 540.0]);
        assert_eq!(v.to_screen([1920.0, 1080.0]), pos2(1060.0, 590.0));
    }
}
