//! The inspector of a hotspot map (point & click navigation): its
//! background and look, then one card per zone (tooltip, where it leads,
//! visible / enabled conditions, hover image and sound, a look of its own,
//! the actions a click runs). The zones themselves are drawn in the scene
//! view; a zone can also be a picture (only its opaque pixels react).

use egui::{DragValue, RichText, Ui};
use orbit_core::{
    AssetKind, Expr, HotspotMap, HotspotShape, HotspotStyle, Node, NodeId, NodeKind, SceneId,
    StringId,
};
use orbit_editor_api::fields::{self, asset_picker, label_picker};
use orbit_editor_api::{EditorContext, UiText, icons};
use orbit_project::{Command, ops};

use super::hotspot_edit::{self, ShapeKind, new_zone, select_zone, selected_zone};
use super::inspector_hotspot_actions::{ActionEdit, actions_editor};
use super::inspector::{commit, info, new_label_button, tfield, with_new_label};
use super::ui_util::{accent, card, danger, section_title, strong, weak};

/// Replaces the map of `id` (one undo step per `field` streak).
pub(super) fn commit_map(ctx: &mut EditorContext<'_>, id: NodeId, map: HotspotMap, field: &str) {
    commit(ctx, id, NodeKind::Hotspots(map), field);
}

/// Appends a zone of `shape` to the map `id` (with its tooltip string), as
/// one undo step, and selects it. The zone leads where the last one does,
/// else to the first label of the story, else to a new label.
pub(super) fn add_zone(
    ctx: &mut EditorContext<'_>,
    egui_ctx: &egui::Context,
    id: NodeId,
    shape: HotspotShape,
) {
    let Some(story) = ctx.story() else { return };
    let Some(NodeKind::Hotspots(map)) = story.node(id).map(|n| &n.kind) else {
        return;
    };
    let Some((si, _)) = story.locate(id) else { return };
    let mut map = map.clone();
    let existing = map
        .hotspots
        .last()
        .map(|h| h.target)
        .or_else(|| ops::labels(story).first().map(|(l, _)| *l));
    let new_label = existing.is_none().then(|| {
        let taken: Vec<String> = ops::labels(story).into_iter().map(|(_, n)| n).collect();
        Node {
            id: NodeId::new(),
            kind: NodeKind::Label {
                name: ops::unique_name("zone", taken.iter().map(String::as_str)),
            },
        }
    });
    let scene = story.scenes[si].id;
    let end = story.scenes[si].nodes.len();
    let target = existing.unwrap_or_else(|| new_label.as_ref().map_or(id, |l| l.id));
    let zone = new_zone(shape, target);
    let text_id = zone.text_id;
    let n = map.hotspots.len() + 1;
    map.hotspots.push(zone);
    let index = map.hotspots.len() - 1;
    let version = ctx.node_version(id);
    let text = ctx.tf("inspector.hotspots.zone_n", &[("n", &n)]);
    let tx_label = ctx.t("inspector.hotspots.add");
    let result = ctx.transaction(&tx_label, |tx| {
        tx.execute(Command::SetString { id: text_id, text: Some(text.clone()) })?;
        if let Some(label) = new_label {
            tx.execute(Command::InsertNode { scene, index: end, node: label })?;
        }
        tx.execute(Command::SetNodeKind {
            id,
            kind: NodeKind::Hotspots(map),
            expected_version: version,
        })
    });
    match result {
        Ok(()) => select_zone(egui_ctx, id, Some(index)),
        Err(e) => ctx.error(e.to_string()),
    }
}

/// Deletes zone `i` of the map `id` and its tooltip, as one undo step.
pub(super) fn remove_zone(ctx: &mut EditorContext<'_>, egui_ctx: &egui::Context, id: NodeId, i: usize) {
    let Some(NodeKind::Hotspots(map)) = ctx.story().and_then(|s| s.node(id)).map(|n| &n.kind) else {
        return;
    };
    if i >= map.hotspots.len() {
        return;
    }
    let mut map = map.clone();
    let string = map.hotspots.remove(i).text_id;
    let version = ctx.node_version(id);
    let tx_label = ctx.t("inspector.hotspots.remove");
    let result = ctx.transaction(&tx_label, |tx| {
        tx.execute(Command::SetNodeKind {
            id,
            kind: NodeKind::Hotspots(map),
            expected_version: version,
        })?;
        tx.execute(Command::SetString { id: string, text: None })
    });
    match result {
        Ok(()) => select_zone(egui_ctx, id, None),
        Err(e) => ctx.error(e.to_string()),
    }
}

/// A `Label` and a `JumpScene` to `scene` at the end of the map's scene, and
/// zone `i` pointed at that label: "this zone goes to that scene".
fn zone_to_scene(ctx: &mut EditorContext<'_>, id: NodeId, i: usize, to: SceneId) {
    let Some(story) = ctx.story() else { return };
    let Some(NodeKind::Hotspots(map)) = story.node(id).map(|n| &n.kind) else {
        return;
    };
    let Some((si, _)) = story.locate(id) else { return };
    let scene = story.scenes[si].id;
    let end = story.scenes[si].nodes.len();
    let taken: Vec<String> = ops::labels(story).into_iter().map(|(_, n)| n).collect();
    let target_name = story.scene(to).map_or("scene", |s| s.name.as_str());
    let name = ops::unique_name(&format!("vers_{target_name}"), taken.iter().map(String::as_str));
    let label = Node { id: NodeId::new(), kind: NodeKind::Label { name } };
    let jump = Node { id: NodeId::new(), kind: NodeKind::JumpScene { scene: to, label: None } };
    let mut map = map.clone();
    let Some(zone) = map.hotspots.get_mut(i) else { return };
    zone.target = label.id;
    let version = ctx.node_version(id);
    let tx_label = ctx.t("inspector.hotspots.to_scene");
    let result = ctx.transaction(&tx_label, |tx| {
        tx.execute(Command::InsertNode { scene, index: end, node: label })?;
        tx.execute(Command::InsertNode { scene, index: end + 1, node: jump })?;
        tx.execute(Command::SetNodeKind {
            id,
            kind: NodeKind::Hotspots(map),
            expected_version: version,
        })
    });
    if let Err(e) = result {
        ctx.error(e.to_string());
    }
}

/// Editor of a style; returns the new one when it changed.
fn style_editor(ui: &mut Ui, style: &HotspotStyle) -> Option<HotspotStyle> {
    let mut s = style.clone();
    let mut changed = false;
    tfield(ui, "inspector.hotspots.color", |ui| {
        if let Some(c) = fields::color_field(ui, s.color) {
            s.color = c;
            changed = true;
        }
    });
    let mut slider = |ui: &mut Ui, key: &str, v: &mut f32, max: f32| {
        tfield(ui, key, |ui| {
            changed |= ui.add(egui::Slider::new(v, 0.0..=max).max_decimals(2)).changed();
        });
    };
    slider(ui, "inspector.hotspots.fill", &mut s.fill_alpha, 1.0);
    slider(ui, "inspector.hotspots.glow", &mut s.glow_width, 24.0);
    slider(ui, "inspector.hotspots.intensity", &mut s.intensity, 1.0);
    slider(ui, "inspector.hotspots.pulse", &mut s.pulse_speed, 4.0);
    slider(ui, "inspector.hotspots.idle", &mut s.idle_hint, 1.0);
    let hand = ui.t("inspector.hotspots.hand");
    if let Some(v) = fields::checkbox(ui, s.hand_cursor, &hand) {
        s.hand_cursor = v;
        changed = true;
    }
    changed.then_some(s)
}

/// The pixel size of an image asset (read from its file header).
fn image_size(root: Option<&std::path::Path>, story: &orbit_core::Story, image: orbit_core::AssetId) -> Option<(u32, u32)> {
    let path = story.asset_path(image)?;
    image::image_dimensions(root?.join(path)).ok()
}

/// The kind of a shape (rectangle, ellipse, polygon, picture), then its
/// numbers (position and size in % of the screen) and, for a picture, the
/// picture, its alpha threshold and whether it is drawn.
fn shape_editor(
    ui: &mut Ui,
    salt: (NodeId, usize),
    story: &orbit_core::Story,
    root: Option<&std::path::Path>,
    shape: &HotspotShape,
) -> Option<HotspotShape> {
    let kind = ShapeKind::of(shape);
    let name = |k: ShapeKind| match k {
        ShapeKind::Rect => "inspector.hotspots.shape.rect",
        ShapeKind::Ellipse => "inspector.hotspots.shape.ellipse",
        ShapeKind::Polygon => "inspector.hotspots.shape.polygon",
        ShapeKind::Mask => "inspector.hotspots.shape.mask",
    };
    let first_image = story.assets.assets.iter().find(|a| a.kind == AssetKind::Image).map(|a| a.id);
    let mut converted = None;
    tfield(ui, "inspector.hotspots.shape", |ui| {
        egui::ComboBox::from_id_salt(("hotspot_kind", salt))
            .selected_text(ui.t(name(kind)))
            .show_ui(ui, |ui| {
                for k in ShapeKind::ALL {
                    let enabled = k != ShapeKind::Mask || first_image.is_some();
                    if ui.add_enabled(enabled, egui::Button::selectable(k == kind, ui.t(name(k)))).clicked() && k != kind {
                        let image = shape.mask_image().or(first_image);
                        let size = image.and_then(|i| image_size(root, story, i));
                        converted = hotspot_edit::convert_shape(shape, k, image, size);
                    }
                }
            });
    });
    if converted.is_some() {
        return converted;
    }
    if let HotspotShape::Mask { image, threshold, draw, .. } = shape {
        let mut out = None;
        tfield(ui, "inspector.hotspots.mask_image", |ui| {
            if let Some(new) = asset_picker(ui, ("hotspot_mask", salt), story, AssetKind::Image, *image) {
                // A new picture: fitted to its size, same threshold and drawing.
                if let HotspotShape::Mask { x, y, w, h, .. } =
                    hotspot_edit::mask_fit(new, image_size(root, story, new))
                {
                    out = Some(HotspotShape::Mask { image: new, x, y, w, h, threshold: *threshold, draw: *draw });
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(format!("{}  {}", icons::ARROWS_OUT, ui.t("inspector.hotspots.mask_fit")))
                .on_hover_text(ui.t("inspector.hotspots.mask_fit_tip"))
                .clicked()
                && let HotspotShape::Mask { x, y, w, h, .. } =
                    hotspot_edit::mask_fit(*image, image_size(root, story, *image))
            {
                out = Some(HotspotShape::Mask { image: *image, x, y, w, h, threshold: *threshold, draw: *draw });
            }
            if ui
                .button(ui.t("inspector.hotspots.mask_full"))
                .on_hover_text(ui.t("inspector.hotspots.mask_full_tip"))
                .clicked()
            {
                out = Some(HotspotShape::Mask {
                    image: *image,
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                    threshold: *threshold,
                    draw: *draw,
                });
            }
        });
        let mut t = *threshold;
        tfield(ui, "inspector.hotspots.mask_threshold", |ui| {
            if ui.add(egui::Slider::new(&mut t, 0.05..=1.0).max_decimals(2)).changed() {
                let mut s = shape.clone();
                if let HotspotShape::Mask { threshold, .. } = &mut s {
                    *threshold = t;
                }
                out = Some(s);
            }
        });
        let label = ui.t("inspector.hotspots.mask_draw");
        if let Some(v) = fields::checkbox(ui, *draw, &label) {
            let mut s = shape.clone();
            if let HotspotShape::Mask { draw, .. } = &mut s {
                *draw = v;
            }
            out = Some(s);
        }
        if out.is_some() {
            return out;
        }
    }
    let mut s = shape.clone();
    let mut changed = false;
    let mut num = |ui: &mut Ui, label: &str, v: &mut f32| {
        ui.label(RichText::new(label).small().color(weak(ui)));
        let mut pct = *v * 100.0;
        if ui
            .add(DragValue::new(&mut pct).speed(0.2).range(-100.0..=200.0).max_decimals(1).suffix("%"))
            .changed()
        {
            *v = pct / 100.0;
            changed = true;
        }
    };
    ui.horizontal_wrapped(|ui| match &mut s {
        HotspotShape::Rect { x, y, w, h } | HotspotShape::Mask { x, y, w, h, .. } => {
            num(ui, "x", x);
            num(ui, "y", y);
            num(ui, "w", w);
            num(ui, "h", h);
        }
        HotspotShape::Ellipse { cx, cy, rx, ry } => {
            num(ui, "x", cx);
            num(ui, "y", cy);
            num(ui, "rx", rx);
            num(ui, "ry", ry);
        }
        HotspotShape::Polygon { points } => {
            ui.label(
                RichText::new(ui.tf("inspector.hotspots.points", &[("n", &points.len())]))
                    .color(weak(ui)),
            );
        }
    });
    (changed && s.is_valid()).then_some(s)
}

/// The inspector of a `Hotspots` node.
pub(super) fn hotspots(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, map: &HotspotMap) {
    let root = ctx.project_info().map(|p| p.root.clone());
    let Some(story) = ctx.story() else { return };
    let egui_ctx = ui.ctx().clone();
    let vars = orbit_editor_api::varwidgets::story_vars(story);
    let map_scene = story.locate(id).map(|(s, _)| story.scenes[s].id);
    enum Do {
        Map(HotspotMap, &'static str),
        /// The map and a new string (a notification action), one step.
        MapText(HotspotMap, StringId, String),
        Text(StringId, String),
        NewLabel(usize),
        ToScene(usize, SceneId),
        Remove(usize),
        Add,
    }
    let mut action: Option<Do> = None;
    let with = |f: &dyn Fn(&mut HotspotMap)| {
        let mut m = map.clone();
        f(&mut m);
        m
    };

    info(ui, "inspector.hotspots.info");
    ui.add_space(4.0);
    section_title(ui, &ui.t("inspector.hotspots.map"));
    let own_bg = ui.t("inspector.hotspots.background");
    if let Some(on) = fields::checkbox(ui, map.background.is_some(), &own_bg) {
        let first = story
            .assets
            .assets
            .iter()
            .find(|a| a.kind == AssetKind::Image)
            .map(|a| a.id);
        if !on || first.is_some() {
            action = Some(Do::Map(with(&|m| m.background = if on { first } else { None }), "bg"));
        }
    }
    if let Some(bg) = map.background
        && let Some(a) = asset_picker(ui, ("hotspot_bg", id), story, AssetKind::Image, bg)
    {
        action = Some(Do::Map(with(&|m| m.background = Some(a)), "bg"));
    }
    egui::CollapsingHeader::new(ui.t("inspector.hotspots.look"))
        .id_salt(("hotspot_style", id))
        .show(ui, |ui| {
            if let Some(s) = style_editor(ui, &map.style) {
                action = Some(Do::Map(with(&|m| m.style = s.clone()), "style"));
            }
        });

    ui.add_space(6.0);
    section_title(ui, &ui.t("inspector.hotspots.zones"));
    let selected = selected_zone(&egui_ctx, id);
    let count = map.hotspots.len();
    for (i, zone) in map.hotspots.iter().enumerate() {
        let is_selected = selected == Some(i);
        card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let icon = match zone.shape {
                    HotspotShape::Rect { .. } => icons::RECTANGLE,
                    HotspotShape::Ellipse { .. } => icons::CIRCLE,
                    HotspotShape::Polygon { .. } => icons::POLYGON,
                    HotspotShape::Mask { .. } => icons::IMAGE,
                };
                let title = if zone.name.is_empty() {
                    ui.tf("inspector.hotspots.zone_n", &[("n", &(i + 1))])
                } else {
                    zone.name.clone()
                };
                let text = strong(format!("{icon}  {title}"), 12.5);
                let text = if is_selected { text.color(accent(ui)) } else { text };
                if ui.selectable_label(is_selected, text).clicked() {
                    select_zone(&egui_ctx, id, Some(i));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(count > 1, egui::Button::new(RichText::new(icons::TRASH).color(danger(ui))).frame(false))
                        .on_hover_text(ui.t("inspector.hotspots.remove"))
                        .clicked()
                    {
                        action = Some(Do::Remove(i));
                    }
                });
            });
            if !is_selected {
                return;
            }
            let mut done = false;
            let mut name = zone.name.clone();
            tfield(ui, "inspector.hotspots.name", |ui| {
                if ui.text_edit_singleline(&mut name).changed() {
                    action = Some(Do::Map(with(&|m| m.hotspots[i].name = name.clone()), "name"));
                }
            });
            let text = story.text(zone.text_id).unwrap_or_default();
            let hint = ui.t("inspector.hotspots.tooltip_hint");
            tfield(ui, "inspector.hotspots.tooltip", |ui| {
                if let Some(t) = fields::text_field(ui, ("hotspot_text", id, i), text, &hint, 1, &mut done) {
                    action = Some(Do::Text(zone.text_id, t));
                }
            });
            tfield(ui, "inspector.hotspots.leads_to", |ui| {
                if let Some(Some(t)) = label_picker(ui, ("hotspot_target", id, i), story, Some(zone.target), None) {
                    action = Some(Do::Map(with(&|m| m.hotspots[i].target = t), "target"));
                }
            });
            ui.horizontal_wrapped(|ui| {
                if new_label_button(ui) {
                    action = Some(Do::NewLabel(i));
                }
                // "Go to a scene": a label + jump_scene made for this zone.
                egui::ComboBox::from_id_salt(("hotspot_scene", id, i))
                    .selected_text(format!("{}  {}", icons::ARROW_SQUARE_OUT, ui.t("inspector.hotspots.to_scene")))
                    .show_ui(ui, |ui| {
                        for s in &story.scenes {
                            if ui.selectable_label(false, &s.name).clicked() {
                                action = Some(Do::ToScene(i, s.id));
                            }
                        }
                    })
                    .response
                    .on_hover_text(ui.t("inspector.hotspots.to_scene_tip"));
            });
            if let Some(s) = shape_editor(ui, (id, i), story, root.as_deref(), &zone.shape) {
                action = Some(Do::Map(with(&|m| m.hotspots[i].shape = s.clone()), "shape"));
            }
            ui.add_space(4.0);
            ui.label(RichText::new(ui.t("inspector.hotspots.actions")).strong());
            if let Some(scene) = map_scene
                && let Some(edit) =
                    actions_editor(ui, ("hotspot_actions", id, i), story, &vars, &zone.actions, zone.target, scene)
            {
                action = Some(match edit {
                    ActionEdit::List(list) => Do::Map(with(&|m| m.hotspots[i].actions = list.clone()), "actions"),
                    ActionEdit::WithText(list, text_id, text) => {
                        Do::MapText(with(&|m| m.hotspots[i].actions = list.clone()), text_id, text)
                    }
                    ActionEdit::Text(text_id, text) => Do::Text(text_id, text),
                });
            }
            ui.add_space(4.0);
            for (key, current, enabled) in [
                ("inspector.hotspots.visible_if", &zone.condition, false),
                ("inspector.hotspots.enabled_if", &zone.enabled, true),
            ] {
                let label = ui.t(key);
                if let Some(on) = fields::checkbox(ui, current.is_some(), &label) {
                    let c = on.then(|| Expr::lit(true));
                    let field = if enabled { "enabled" } else { "cond" };
                    action = Some(Do::Map(
                        with(&|m| {
                            if enabled {
                                m.hotspots[i].enabled = c.clone();
                            } else {
                                m.hotspots[i].condition = c.clone();
                            }
                        }),
                        field,
                    ));
                }
                if let Some(cond) = current
                    && let Some(c) = super::inspector_script::condition_field(
                        ui,
                        ctx,
                        id,
                        None,
                        ("hotspot_cond", id, i, enabled),
                        cond,
                    )
                {
                    let field = if enabled { "enabled" } else { "cond" };
                    action = Some(Do::Map(
                        with(&|m| {
                            if enabled {
                                m.hotspots[i].enabled = Some(c.clone());
                            } else {
                                m.hotspots[i].condition = Some(c.clone());
                            }
                        }),
                        field,
                    ));
                }
            }
            ui.add_space(4.0);
            for (key, current, kind) in [
                ("inspector.hotspots.hover_image", zone.hover_image, AssetKind::Image),
                ("inspector.hotspots.hover_sound", zone.hover_sound, AssetKind::Sound),
            ] {
                let label = ui.t(key);
                let first = story.assets.assets.iter().find(|a| a.kind == kind).map(|a| a.id);
                let set = |m: &mut HotspotMap, v: Option<orbit_core::AssetId>| {
                    if kind == AssetKind::Image {
                        m.hotspots[i].hover_image = v;
                    } else {
                        m.hotspots[i].hover_sound = v;
                    }
                };
                if let Some(on) = fields::checkbox(ui, current.is_some(), &label)
                    && (!on || first.is_some())
                {
                    action = Some(Do::Map(with(&|m| set(m, if on { first } else { None })), "asset"));
                }
                if let Some(a) = current
                    && let Some(new) = asset_picker(ui, ("hotspot_asset", id, i, key), story, kind, a)
                {
                    action = Some(Do::Map(with(&|m| set(m, Some(new))), "asset"));
                }
            }
            let own = ui.t("inspector.hotspots.own_look");
            if let Some(on) = fields::checkbox(ui, zone.style.is_some(), &own) {
                let s = on.then(|| map.style.clone());
                action = Some(Do::Map(with(&|m| m.hotspots[i].style = s.clone()), "zstyle"));
            }
            if let Some(style) = &zone.style
                && let Some(s) = style_editor(ui, style)
            {
                action = Some(Do::Map(with(&|m| m.hotspots[i].style = Some(s.clone())), "zstyle"));
            }
        });
        ui.add_space(4.0);
    }
    if ui
        .button(format!("{}  {}", icons::PLUS, ui.t("inspector.hotspots.add")))
        .on_hover_text(ui.t("inspector.hotspots.add_tip"))
        .clicked()
    {
        action = Some(Do::Add);
    }

    let Some(action) = action else { return };
    match action {
        Do::Map(m, field) => commit_map(ctx, id, m, field),
        Do::MapText(m, text_id, text) => {
            let version = ctx.node_version(id);
            let label = ctx.t("inspector.hotspots.act.add");
            let result = ctx.transaction(&label, |tx| {
                tx.execute(Command::SetString { id: text_id, text: Some(text.clone()) })?;
                tx.execute(Command::SetNodeKind { id, kind: NodeKind::Hotspots(m), expected_version: version })
            });
            if let Err(e) = result {
                ctx.error(e.to_string());
            }
        }
        Do::Text(string, text) => {
            let command = Command::SetString { id: string, text: Some(text) };
            if let Err(e) = ctx.exec_merged(command, &format!("hotspot-text:{string}")) {
                ctx.error(e.to_string());
            }
        }
        Do::NewLabel(i) => {
            let mut m = map.clone();
            with_new_label(ctx, id, move |l| {
                m.hotspots[i].target = l;
                NodeKind::Hotspots(m)
            });
        }
        Do::ToScene(i, s) => zone_to_scene(ctx, id, i, s),
        Do::Remove(i) => remove_zone(ctx, &egui_ctx, id, i),
        Do::Add => add_zone(
            ctx,
            &egui_ctx,
            id,
            hotspot_edit::shape_from_drag(hotspot_edit::Tool::Rect, (0.4, 0.35), (0.6, 0.65))
                .expect("a valid default zone"),
        ),
    }
}
