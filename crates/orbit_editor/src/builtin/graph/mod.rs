//! The graph panel: the story as blocks and links, editable.
//!
//! Blocks are the nodes of the story graph (see `orbit_project::graph`);
//! every gesture of the widget ([`view::GraphEvent`]) becomes an operation of
//! `orbit_project::graph_ops`, so each one is a single undo entry and the
//! rules are shared with the tests and the MCP server. Positions live in the
//! `.layout` sidecar (`EditorState::graph`), never in the story.

mod snarl_view;
mod view;

pub use self::view::GraphEvent;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use egui::{Color32, Key, RichText, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as ph;
use orbit_core::{NodeKind, SceneId, Story};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, Level,
    PanelDef, Plugin, PluginManifest, Selection,
};
use orbit_project::graph::{BlockId, GraphModel, SlotKind, SlotRef, SlotTarget, Terminator};
use orbit_project::graph_layout::{BLOCK_WIDTH, GraphLayout, auto_layout, block_height};
use orbit_project::graph_ops::{self, ChoiceTarget, DeletePlan};
use orbit_project::{Command, ProjectError};

use self::snarl_view::SnarlView;
use self::view::{GraphView, ViewContent, ViewNode, ViewSlot, slot_color};
use super::ids;
use super::ui_util::{accent, danger, faint, primary_button, secondary_button, weak};
use crate::theme::bold;

/// Colors of the block types.
mod palette {
    use egui::Color32;

    pub const SCENE: Color32 = Color32::from_rgb(86, 138, 226);
    pub const LABEL: Color32 = Color32::from_rgb(70, 160, 142);
    pub const CHOICE: Color32 = Color32::from_rgb(214, 148, 52);
    pub const CONDITION: Color32 = Color32::from_rgb(70, 170, 112);
    pub const END: Color32 = Color32::from_rgb(190, 92, 108);
    pub const RETURN: Color32 = Color32::from_rgb(150, 110, 210);
    pub const CALL: Color32 = Color32::from_rgb(130, 100, 200);
}

struct GraphUi {
    view: Box<dyn GraphView>,
    model: GraphModel,
    /// `(generation, revision)` the model was computed for.
    model_key: Option<(u64, u64)>,
    /// What the view currently shows (a change rebuilds its content).
    content_key: Option<u64>,
    /// Bumped when positions are replaced wholesale (reorganize).
    epoch: u64,
    seen_generation: u64,
    search: String,
    search_focus: bool,
    show_minimap: bool,
    /// Scroll to this block (and zoom in to at least this level) once its
    /// position is known.
    focus: Option<(BlockId, Option<f32>)>,
}

impl GraphUi {
    fn new() -> Self {
        Self {
            view: Box::new(SnarlView::new("orbit_graph")),
            model: GraphModel::default(),
            model_key: None,
            content_key: None,
            epoch: 0,
            seen_generation: u64::MAX,
            search: String::new(),
            search_focus: false,
            show_minimap: true,
            focus: None,
        }
    }
}

type Shared = Rc<RefCell<GraphUi>>;

pub struct GraphPlugin;

impl Plugin for GraphPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::GRAPH, "Graphe de scènes", "0.1.0")
    }
}

fn has_story(ctx: &EditorContext<'_>) -> bool {
    ctx.story().is_some_and(|s| !s.scenes.is_empty())
}

impl EditorPlugin for GraphPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        let state: Shared = Rc::new(RefCell::new(GraphUi::new()));
        {
            let state = state.clone();
            reg.panel(PanelDef::new(
                "graph",
                "Graphe",
                ph::TREE_STRUCTURE,
                DockSlot::Center,
                move |ui, ctx| panel(ui, ctx, &state),
            ));
        }
        {
            let state = state.clone();
            reg.action(
                ActionDef::new("fit", "Graphe : recadrer tout", move |ctx| {
                    state.borrow_mut().view.fit_all();
                    ctx.request(EditorRequest::ShowPanel(super::panels::GRAPH.to_owned()));
                })
                .icon(ph::CORNERS_OUT)
                .category("Graphe")
                .enabled(has_story),
            );
        }
        {
            let state = state.clone();
            reg.action(
                ActionDef::new("reorganize", "Graphe : réorganiser les blocs", move |ctx| {
                    reorganize(ctx, &state);
                })
                .icon(ph::SPARKLE)
                .category("Graphe")
                .enabled(has_story),
            );
        }
        {
            let state = state.clone();
            reg.action(
                ActionDef::new("focus", "Graphe : centrer sur la sélection", move |ctx| {
                    if let Some(block) = selected_block(ctx) {
                        focus_block(&state, block, Some(0.9));
                    }
                    ctx.request(EditorRequest::ShowPanel(super::panels::GRAPH.to_owned()));
                })
                .icon(ph::CROSSHAIR)
                .category("Graphe")
                .enabled(has_story),
            );
        }
        reg.action(
            ActionDef::new("new_scene", "Graphe : nouvelle scène", |ctx| {
                new_scene(ctx, None);
            })
            .icon(ph::FOLDER_PLUS)
            .category("Graphe")
            .enabled(|c| c.has_project()),
        );
        reg.action(
            ActionDef::new(
                "add_choice",
                "Graphe : ajouter un choix au bloc sélectionné",
                |ctx| {
                    if let Some(block) = selected_block(ctx) {
                        apply(ctx, GraphEvent::AddChoice(block));
                    }
                },
            )
            .icon(ph::GIT_BRANCH)
            .category("Graphe")
            .enabled(has_story),
        );
        reg.action(
            ActionDef::new(
                "add_condition",
                "Graphe : ajouter une condition au bloc sélectionné",
                |ctx| {
                    if let Some(block) = selected_block(ctx) {
                        apply(ctx, GraphEvent::AddCondition(block));
                    }
                },
            )
            .icon(ph::GIT_FORK)
            .category("Graphe")
            .enabled(has_story),
        );
        reg.action(
            ActionDef::new("find", "Graphe : chercher un bloc", {
                let state = state.clone();
                move |ctx| {
                    state.borrow_mut().search_focus = true;
                    ctx.request(EditorRequest::ShowPanel(super::panels::GRAPH.to_owned()));
                }
            })
            .icon(ph::MAGNIFYING_GLASS)
            .category("Graphe")
            .enabled(has_story),
        );
    }
}

// --------------------------------------------------------------- helpers

/// The block of the primary selection.
fn selected_block(ctx: &EditorContext<'_>) -> Option<BlockId> {
    let story = ctx.story()?;
    match ctx.selection()? {
        Selection::Node(id) => GraphModel::build(story).block_of(id),
        Selection::Scene(id) => story.scene(id).map(|s| BlockId::Scene(s.id)),
    }
}

fn character_names(story: &Story, ids: &[orbit_core::CharacterId]) -> String {
    let mut names: Vec<String> = ids
        .iter()
        .take(3)
        .map(|c| {
            story
                .character(*c)
                .and_then(|c| story.text(c.name_id))
                .unwrap_or("?")
                .to_owned()
        })
        .collect();
    if ids.len() > 3 {
        names.push("…".to_owned());
    }
    names.join(", ")
}

/// How a block looks: icon and color from what it does.
fn block_style(
    block: &orbit_project::graph::Block,
    entry: bool,
) -> (&'static str, Color32, String) {
    let kind = if block.has_choice() {
        (ph::GIT_BRANCH, palette::CHOICE, "Choix")
    } else if block.has_condition() {
        (ph::GIT_FORK, palette::CONDITION, "Condition")
    } else if matches!(block.terminator, Some((_, Terminator::End))) {
        (ph::STOP_CIRCLE, palette::END, "Fin")
    } else if matches!(block.terminator, Some((_, Terminator::Return))) {
        (ph::ARROW_U_UP_LEFT, palette::RETURN, "Retour")
    } else if block.has_call() {
        (ph::PHONE_CALL, palette::CALL, "Appel")
    } else if block.is_head {
        (ph::FILM_SLATE, palette::SCENE, "Scène")
    } else {
        (ph::FLAG, palette::LABEL, "Étiquette")
    };
    let role = if entry {
        "Début de l'histoire".to_owned()
    } else if block.is_head {
        "Scène".to_owned()
    } else {
        format!("Étiquette · {}", block.scene_name)
    };
    let _ = kind.2;
    (kind.0, kind.1, role)
}

fn build_content(
    ctx: &EditorContext<'_>,
    story: &Story,
    model: &GraphModel,
    layout: &GraphLayout,
    selected: &HashSet<BlockId>,
    highlighted: &HashSet<BlockId>,
    signature: u64,
) -> ViewContent {
    let diagnostics = &ctx.state().diagnostics;
    let entry = model.entry();
    let nodes = model
        .blocks
        .iter()
        .map(|block| {
            let (icon, accent, role) = block_style(block, entry == Some(block.id));
            let mut meta = Vec::new();
            let mut line = match block.lines {
                0 => "Aucune réplique".to_owned(),
                1 => "1 réplique".to_owned(),
                n => format!("{n} répliques"),
            };
            let who = character_names(story, &block.speakers);
            if !who.is_empty() || block.narrator {
                let who = if block.narrator && who.is_empty() {
                    "narrateur".to_owned()
                } else if block.narrator {
                    format!("{who} + narrateur")
                } else {
                    who
                };
                line.push_str(&format!(" · {who}"));
            }
            meta.push(line);
            let slots = block
                .slots
                .iter()
                .map(|slot| {
                    let (target, broken) = match &slot.target {
                        SlotTarget::Block(b) => (Some(*b), false),
                        SlotTarget::Missing(_) => (None, true),
                        SlotTarget::None => (None, false),
                    };
                    let label = match slot.kind {
                        SlotKind::Option(_) => format!("« {} »", slot.label),
                        SlotKind::Next if target.is_none() => "fin de partie".into(),
                        SlotKind::Else if target.is_none() => "sinon · suite".into(),
                        _ => slot.label.clone(),
                    };
                    ViewSlot {
                        slot: SlotRef {
                            block: block.id,
                            node: slot.node,
                            kind: slot.kind,
                        },
                        label,
                        // A block that ends the game is drawn in the "end" color.
                        color: if slot.kind == SlotKind::Next && target.is_none() {
                            palette::END
                        } else {
                            slot_color(slot.kind)
                        },
                        target,
                        broken,
                        conditional: slot.conditional,
                        soft: matches!(slot.kind, SlotKind::Next)
                            || (slot.kind == SlotKind::Else && target.is_none()),
                    }
                })
                .collect();
            // Worst finding over the nodes of the block.
            let mut problem: Option<(Level, String)> = None;
            let mut count = 0;
            for id in &block.nodes {
                for (level, message) in diagnostics.of(*id) {
                    count += 1;
                    let worse = match (&problem, level) {
                        (None, _) => true,
                        (Some((Level::Error, _)), _) => false,
                        (Some(_), Level::Error) => true,
                        _ => false,
                    };
                    if worse {
                        problem = Some((*level, message.clone()));
                    }
                }
            }
            if let Some((_, text)) = &mut problem
                && count > 1
            {
                text.push_str(&format!(" (+ {} autre(s))", count - 1));
            }
            let pos = layout.get(block.id).unwrap_or((0.0, 0.0));
            (
                ViewNode {
                    id: block.id,
                    title: block.title.clone(),
                    subtitle: role,
                    icon,
                    accent,
                    meta,
                    first_line: block.first_line.clone(),
                    slots,
                    problem,
                    orphan: model.unreachable.contains(&block.id),
                    dead_nodes: block.dead_nodes,
                    selected: selected.contains(&block.id),
                    highlighted: highlighted.contains(&block.id),
                    height: block_height(block),
                },
                pos,
            )
        })
        .collect();
    ViewContent { nodes, signature }
}

fn matches_of(story: &Story, model: &GraphModel, query: &str) -> Vec<BlockId> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    model
        .blocks
        .iter()
        .filter(|b| {
            b.title.to_lowercase().contains(&q)
                || b.scene_name.to_lowercase().contains(&q)
                || b.first_line
                    .as_deref()
                    .is_some_and(|l| l.to_lowercase().contains(&q))
                || b.slots.iter().any(|s| s.label.to_lowercase().contains(&q))
                || b.speakers.iter().any(|c| {
                    story
                        .character(*c)
                        .and_then(|c| story.text(c.name_id))
                        .is_some_and(|n| n.to_lowercase().contains(&q))
                })
        })
        .map(|b| b.id)
        .collect()
}

fn selected_set(ctx: &EditorContext<'_>, model: &GraphModel) -> HashSet<BlockId> {
    let mut out: HashSet<BlockId> = ctx
        .selected_nodes()
        .into_iter()
        .filter_map(|n| model.block_of(n))
        .collect();
    if let Some(Selection::Scene(id)) = ctx.selection()
        && model.block(BlockId::Scene(id)).is_some()
    {
        out.insert(BlockId::Scene(id));
    }
    out
}

fn hash_of(parts: &[u64]) -> u64 {
    // FNV-1a over the parts.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

// ---------------------------------------------------------------- the panel

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &Shared) {
    if !ctx.has_project() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(ph::TREE_STRUCTURE)
                    .size(40.0)
                    .color(faint(ui)),
            );
            ui.label(RichText::new("Aucun projet ouvert").font(bold(16.0)));
            ui.label(
                RichText::new("Ouvrez un projet pour voir le graphe de son histoire.")
                    .color(weak(ui)),
            );
            ui.add_space(8.0);
            if primary_button(ui, "Ouvrir un projet…").clicked() {
                ctx.request(EditorRequest::PickOpenProject);
            }
        });
        return;
    }
    let Some(story) = ctx.story().cloned() else {
        return;
    };
    let revision = ctx.project().map_or(0, |p| p.revision());
    let generation = ctx.state().graph.generation;

    // New project: start from a fitted view.
    {
        let mut st = shared.borrow_mut();
        if st.seen_generation != generation {
            st.seen_generation = generation;
            st.model_key = None;
            st.content_key = None;
            st.search.clear();
            st.view.fit_all();
        }
        if st.model_key != Some((generation, revision)) {
            st.model = GraphModel::build(&story);
            st.model_key = Some((generation, revision));
            st.content_key = None;
        }
    }
    if story.scenes.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Ce projet n'a pas encore de scène").font(bold(16.0)));
            ui.add_space(8.0);
            if primary_button(ui, "Créer la première scène").clicked() {
                new_scene(ctx, None);
            }
        });
        return;
    }

    // Positions for blocks that have none.
    {
        let st = shared.borrow();
        let graph = &mut ctx.state_mut().graph;
        let mut layout = std::mem::take(&mut graph.layout);
        if layout.place_missing(&st.model) {
            graph.dirty |= graph.persist;
        }
        if layout.positions.len() > st.model.blocks.len() * 2 + 64 {
            layout.prune(&st.model);
            graph.dirty |= graph.persist;
        }
        graph.layout = layout;
    }

    // Debug motion (`--autoscroll`): pans and zooms a little every frame.
    if let Some(speed) = ctx.state().debug_autoscroll {
        let t = ui.input(|i| i.time) as f32;
        shared.borrow_mut().view.nudge(
            vec2(speed, speed * 0.5 * (t * 2.0).sin()),
            1.0 + 0.004 * (t * 1.5).cos(),
        );
        ui.ctx().request_repaint();
    }

    // A pending "scroll to this block".
    {
        let mut st = shared.borrow_mut();
        if let Some((block, zoom)) = st.focus.take() {
            let height = st.model.block(block).map(block_height);
            if let (Some((x, y)), Some(h)) = (ctx.state().graph.layout.get(block), height) {
                st.view
                    .focus(pos2(x + BLOCK_WIDTH / 2.0, y + h / 2.0), zoom);
            }
        }
    }

    let toolbar_events = toolbar(ui, ctx, shared, &story);

    // Content of the view.
    {
        let mut st = shared.borrow_mut();
        let matches: HashSet<BlockId> = matches_of(&story, &st.model, &st.search)
            .into_iter()
            .collect();
        let selected = selected_set(ctx, &st.model);
        let diag_revision = ctx.state().diagnostics.revision;
        let signature = hash_of(&[generation, revision, diag_revision, st.epoch]);
        let mut sel_hash: Vec<u64> = selected
            .iter()
            .map(|b| match b {
                BlockId::Scene(s) => s.as_u128() as u64,
                BlockId::Label(n) => n.as_u128() as u64,
            })
            .collect();
        sel_hash.sort_unstable();
        let mut parts = vec![signature, matches.len() as u64];
        parts.extend(sel_hash);
        parts.extend(matches.iter().map(|b| match b {
            BlockId::Scene(s) => s.as_u128() as u64,
            BlockId::Label(n) => n.as_u128() as u64,
        }));
        let key = hash_of(&parts);
        if st.content_key != Some(key) {
            let content = build_content(
                ctx,
                &story,
                &st.model,
                &ctx.state().graph.layout,
                &selected,
                &matches,
                signature,
            );
            st.view.set_content(content);
            st.content_key = Some(key);
        }
    }

    // The canvas.
    let canvas = ui.available_rect_before_wrap();
    let events = {
        let mut st = shared.borrow_mut();
        st.view.show(ui)
    };
    let hovered = ui.rect_contains_pointer(canvas);
    let mut events = events;
    events.extend(toolbar_events);
    if hovered && !ui.ctx().egui_wants_keyboard_input() {
        let (fit, delete) = ui.input(|i| {
            (
                i.key_pressed(Key::F) && i.modifiers.is_none(),
                i.key_pressed(Key::Delete),
            )
        });
        if fit {
            shared.borrow_mut().view.fit_all();
        }
        if delete && let Some(b) = selected_block(ctx) {
            events.push(GraphEvent::Delete(b));
        }
    }
    minimap(ui, ctx, shared, canvas);
    stats_overlay(ui, shared, canvas);
    confirm_dialog(ui, ctx);

    if !events.is_empty() {
        ui.ctx().request_repaint();
    }
    for event in events {
        handle(ctx, shared, event);
    }
}

fn toolbar(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    shared: &Shared,
    story: &Story,
) -> Vec<GraphEvent> {
    let mut events = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if ui
            .button(format!("{}  Recadrer", ph::CORNERS_OUT))
            .on_hover_text("Cadrer tous les blocs (F)")
            .clicked()
        {
            shared.borrow_mut().view.fit_all();
        }
        if ui
            .button(format!("{}  Réorganiser", ph::SPARKLE))
            .on_hover_text("Replacer tous les blocs automatiquement")
            .clicked()
        {
            reorganize(ctx, shared);
        }
        if ui
            .button(format!("{}  Nouvelle scène", ph::FOLDER_PLUS))
            .on_hover_text("Créer une scène (avec une étiquette et une fin)")
            .clicked()
        {
            events.push(GraphEvent::NewScene((f32::NAN, f32::NAN)));
        }
        ui.separator();
        {
            let mut st = shared.borrow_mut();
            let focus = std::mem::take(&mut st.search_focus);
            let response = ui.add(
                egui::TextEdit::singleline(&mut st.search)
                    .hint_text(format!("{} Chercher un bloc", ph::MAGNIFYING_GLASS))
                    .desired_width(190.0),
            );
            if focus {
                response.request_focus();
            }
            let matches = matches_of(story, &st.model, &st.search);
            let jump = response.has_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if response.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                st.search.clear();
            }
            if jump && let Some(first) = matches.first().copied() {
                drop(st);
                events.push(GraphEvent::Select(vec![first]));
                focus_block(shared, first, Some(0.7));
            } else if !st.search.trim().is_empty() {
                let count = matches.len();
                ui.label(
                    RichText::new(match count {
                        0 => "aucun résultat".to_owned(),
                        1 => "1 résultat".to_owned(),
                        n => format!("{n} résultats"),
                    })
                    .color(weak(ui))
                    .size(12.0),
                );
                if response.has_focus() || response.lost_focus() {
                    let mut chosen = None;
                    egui::Area::new(egui::Id::new("orbit_graph_search_results"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(response.rect.left_bottom() + vec2(0.0, 2.0))
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.set_min_width(260.0);
                                for id in matches.iter().take(8) {
                                    if let Some(b) = st.model.block(*id) {
                                        let text = format!(
                                            "{}  {}  ·  {}",
                                            if b.is_head { ph::FILM_SLATE } else { ph::FLAG },
                                            b.title,
                                            b.scene_name
                                        );
                                        if ui.selectable_label(false, text).clicked() {
                                            chosen = Some(*id);
                                        }
                                    }
                                }
                            });
                        });
                    if let Some(id) = chosen {
                        drop(st);
                        events.push(GraphEvent::Select(vec![id]));
                        focus_block(shared, id, Some(0.7));
                    }
                }
            }
        }
    });
    ui.add_space(2.0);
    events
}

fn focus_block(shared: &Shared, block: BlockId, min_zoom: Option<f32>) {
    shared.borrow_mut().focus = Some((block, min_zoom));
}

/// Overview of the whole graph, bottom right; click or drag to move there.
fn minimap(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &Shared, canvas: egui::Rect) {
    let mut st = shared.borrow_mut();
    if !st.show_minimap || st.model.blocks.len() < 2 {
        return;
    }
    let layout = &ctx.state().graph.layout;
    let mut bounds = egui::Rect::NOTHING;
    let mut rects = Vec::with_capacity(st.model.blocks.len());
    for b in &st.model.blocks {
        if let Some((x, y)) = layout.get(b.id) {
            let r = egui::Rect::from_min_size(pos2(x, y), vec2(BLOCK_WIDTH, block_height(b)));
            bounds = bounds.union(r);
            rects.push((b.id, r));
        }
    }
    let visible = st.view.visible_rect();
    if let Some(v) = visible {
        bounds = bounds.union(v);
    }
    if !bounds.is_finite() {
        return;
    }
    let size = vec2(190.0, 124.0);
    let origin = canvas.right_bottom() - size - vec2(12.0, 12.0);
    let map = egui::Rect::from_min_size(origin, size);
    let scale = ((size.x - 10.0) / bounds.width())
        .min((size.y - 10.0) / bounds.height())
        .max(1e-6);
    let offset = map.center().to_vec2() - bounds.center().to_vec2() * scale;
    let to_map = |p: egui::Pos2| (p.to_vec2() * scale + offset).to_pos2();
    let selected = selected_set(ctx, &st.model);
    let mut center_on = None;
    egui::Area::new(egui::Id::new("orbit_graph_minimap"))
        .order(egui::Order::Foreground)
        .fixed_pos(origin)
        .show(ui.ctx(), |ui| {
            let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(
                rect,
                6.0,
                ui.visuals().extreme_bg_color.gamma_multiply(0.92),
            );
            painter.rect_stroke(
                rect,
                6.0,
                ui.visuals().widgets.noninteractive.bg_stroke,
                egui::StrokeKind::Inside,
            );
            for (id, r) in &rects {
                let color = if selected.contains(id) {
                    Color32::from_rgb(120, 170, 255)
                } else if st.model.unreachable.contains(id) {
                    Color32::from_gray(110)
                } else {
                    Color32::from_gray(170)
                };
                painter.rect_filled(
                    egui::Rect::from_min_max(to_map(r.min), to_map(r.max)).expand(0.5),
                    1.0,
                    color,
                );
            }
            if let Some(v) = visible {
                painter.rect_stroke(
                    egui::Rect::from_min_max(to_map(v.min), to_map(v.max)),
                    1.0,
                    egui::Stroke::new(1.5, accent(ui)),
                    egui::StrokeKind::Inside,
                );
            }
            if (response.clicked() || response.dragged())
                && let Some(p) = response.interact_pointer_pos()
            {
                let g = (p.to_vec2() - offset) / scale;
                center_on = Some(g.to_pos2());
            }
        });
    if let Some(p) = center_on {
        st.view.focus(p, None);
    }
}

/// Counts, bottom left of the canvas.
fn stats_overlay(ui: &mut Ui, shared: &Shared, canvas: egui::Rect) {
    let st = shared.borrow();
    let edges = st.model.edges().len();
    let orphans = st.model.unreachable.len();
    let mut text = format!("{} blocs · {edges} liens", st.model.blocks.len());
    if orphans > 0 {
        text.push_str(&format!(" · {orphans} orphelin(s)"));
    }
    egui::Area::new(egui::Id::new("orbit_graph_stats"))
        .order(egui::Order::Foreground)
        .fixed_pos(canvas.left_bottom() + vec2(12.0, -26.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            ui.label(RichText::new(text).size(12.0).color(weak(ui)));
        });
}

fn confirm_dialog(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let Some(block) = ctx.state().graph.pending_delete else {
        return;
    };
    let Some(story) = ctx.story() else { return };
    let Ok(plan) = graph_ops::delete_plan(story, block) else {
        ctx.state_mut().graph.pending_delete = None;
        return;
    };
    let lines = say_count(story, &plan);
    let model = GraphModel::build(story);
    let title = model.block(block).map(|b| {
        if b.is_head {
            b.scene_name.clone()
        } else {
            b.title.clone()
        }
    });
    let title = title.unwrap_or_default();
    let mut choice = None;
    egui::Window::new("Supprimer ?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.set_max_width(380.0);
            let what = if plan.scene {
                format!("la scène « {title} » et tout son contenu")
            } else {
                format!("le bloc « {title} »")
            };
            ui.label(RichText::new(format!("Supprimer {what} ?")).font(bold(14.0)));
            ui.add_space(4.0);
            if lines > 0 {
                ui.label(format!("{lines} réplique(s) seront supprimée(s)."));
            }
            if !plan.incoming.is_empty() {
                ui.label(format!(
                    "{} lien(s) venant d'autres blocs seront coupés (sauts, options, conditions).",
                    plan.incoming.len()
                ));
            }
            ui.label(
                RichText::new("Annulable avec Ctrl+Z.")
                    .color(weak(ui))
                    .size(12.0),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let delete = egui::Button::new(RichText::new("Supprimer").color(Color32::WHITE))
                    .fill(danger(ui));
                if ui.add(delete).clicked() {
                    choice = Some(true);
                }
                if secondary_button(ui, "Annuler").clicked() {
                    choice = Some(false);
                }
            });
        });
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        choice = Some(false);
    }
    match choice {
        Some(true) => handle_with(ctx, &model, GraphEvent::DeleteConfirmed(block)),
        Some(false) => ctx.state_mut().graph.pending_delete = None,
        None => {}
    }
}

fn reorganize(ctx: &mut EditorContext<'_>, shared: &Shared) {
    let Some(story) = ctx.story() else { return };
    let model = GraphModel::build(story);
    let positions = auto_layout(&model);
    let graph = &mut ctx.state_mut().graph;
    graph.layout = GraphLayout::default();
    for (id, pos) in positions {
        graph.layout.set(id, pos);
    }
    graph.persist = true;
    graph.dirty = true;
    let mut st = shared.borrow_mut();
    st.epoch += 1;
    st.content_key = None;
    st.view.fit_all();
}

// ------------------------------------------------------------------ events

/// Applies a gesture of the graph (what the panel does with the events of
/// the widget); also the entry point of the actions and of the tests.
pub fn apply_event(ctx: &mut EditorContext<'_>, event: GraphEvent) {
    let model = ctx.story().map(GraphModel::build);
    let Some(model) = model else { return };
    handle_with(ctx, &model, event);
}

fn apply(ctx: &mut EditorContext<'_>, event: GraphEvent) {
    apply_event(ctx, event);
}

fn handle(ctx: &mut EditorContext<'_>, shared: &Shared, event: GraphEvent) {
    let model = shared.borrow().model.clone();
    handle_with(ctx, &model, event);
}

fn report(ctx: &mut EditorContext<'_>, result: Result<(), ProjectError>) -> bool {
    match result {
        Ok(()) => true,
        Err(e) => {
            ctx.error(e.to_string());
            false
        }
    }
}

/// Selects a block: its first node, or its scene when it is empty.
fn select_block(ctx: &mut EditorContext<'_>, model: &GraphModel, block: BlockId) {
    let Some(b) = model.block(block) else { return };
    match b.head_node() {
        Some(node) => ctx.select(Selection::Node(node)),
        None => ctx.select(Selection::Scene(b.scene)),
    }
    ctx.state_mut().current_scene = Some(b.scene);
}

fn new_scene(ctx: &mut EditorContext<'_>, at: Option<(f32, f32)>) {
    let Some(story) = ctx.story() else { return };
    let name = orbit_project::ops::unique_name(
        "Nouvelle scène",
        story.scenes.iter().map(|s| s.name.as_str()),
    );
    let Some(block) = ctx.ops_or_toast(|p| graph_ops::create_scene(p, &name)) else {
        return;
    };
    place_new(ctx, block, at);
    if let Some(story) = ctx.story() {
        let model = GraphModel::build(story);
        select_block(ctx, &model, block);
    }
}

/// Positions a block that was just created at `at` (a graph point under the
/// cursor), when there is one.
fn place_new(ctx: &mut EditorContext<'_>, block: BlockId, at: Option<(f32, f32)>) {
    if let Some(pos) = at.filter(|p| p.0.is_finite() && p.1.is_finite()) {
        ctx.state_mut().graph.set_position(block, pos);
    }
}

fn scene_for_new_label(ctx: &EditorContext<'_>) -> Option<SceneId> {
    super::edit::current_scene(ctx)
}

fn handle_with(ctx: &mut EditorContext<'_>, model: &GraphModel, event: GraphEvent) {
    match event {
        GraphEvent::Select(blocks) => {
            let heads: Vec<_> = blocks
                .iter()
                .filter_map(|b| model.block(*b).and_then(|b| b.head_node()))
                .collect();
            match (blocks.first(), heads.first()) {
                (Some(_), Some(&primary)) if heads.len() > 1 => ctx.select_nodes(heads, primary),
                (Some(first), _) => select_block(ctx, model, *first),
                _ => {}
            }
        }
        GraphEvent::Open(block) => {
            select_block(ctx, model, block);
            ctx.request(EditorRequest::ShowPanel(super::panels::SCENE.to_owned()));
        }
        GraphEvent::PlayFrom(block) => {
            select_block(ctx, model, block);
            match model.block(block).and_then(|b| b.head_node()) {
                Some(node) => ctx.play_from(node),
                None => ctx.warn("Ce bloc est vide : rien à jouer."),
            }
        }
        GraphEvent::Moved(block, pos) => ctx.state_mut().graph.set_position(block, pos),
        GraphEvent::MoveEnded => {}
        GraphEvent::Connect(slot, to) => {
            let result = ctx.ops(|p| graph_ops::set_slot_target(p, slot, to));
            report(ctx, result.map_err(editor_error));
        }
        GraphEvent::ConnectNew(slot, pos) => {
            let Some(story) = ctx.story() else { return };
            let name = orbit_project::ops::unique_name(
                "Nouvelle scène",
                story.scenes.iter().map(|s| s.name.as_str()),
            );
            let result = ctx.ops(|p| {
                p.group("Créer une scène liée", |p| {
                    let block = graph_ops::create_scene(p, &name)?;
                    graph_ops::set_slot_target(p, slot, block)?;
                    Ok(block)
                })
            });
            match result {
                Ok(block) => {
                    place_new(ctx, block, Some(pos));
                    if let Some(story) = ctx.story() {
                        let model = GraphModel::build(story);
                        select_block(ctx, &model, block);
                    }
                }
                Err(e) => ctx.error(e.to_string()),
            }
        }
        GraphEvent::Disconnect(slot) => {
            let result = ctx.ops(|p| graph_ops::remove_slot(p, slot));
            report(ctx, result.map_err(editor_error));
        }
        GraphEvent::NewScene(pos) => new_scene(ctx, Some(pos)),
        GraphEvent::NewLabel(pos) => {
            let Some(scene) = scene_for_new_label(ctx) else {
                return;
            };
            if let Some(block) =
                ctx.ops_or_toast(|p| graph_ops::create_label(p, scene, "Étiquette"))
            {
                place_new(ctx, block, Some(pos));
                if let Some(story) = ctx.story() {
                    let model = GraphModel::build(story);
                    select_block(ctx, &model, block);
                }
            }
        }
        GraphEvent::AddChoice(block) => {
            let created = ctx.ops_or_toast(|p| {
                graph_ops::add_choice(
                    p,
                    block,
                    vec![
                        (
                            "Option A".into(),
                            ChoiceTarget::NewScene("Branche A".into()),
                        ),
                        (
                            "Option B".into(),
                            ChoiceTarget::NewScene("Branche B".into()),
                        ),
                    ],
                )
            });
            if created.is_some() {
                select_block(ctx, model, block);
            }
        }
        GraphEvent::AddCondition(block) => {
            let created = ctx.ops_or_toast(|p| {
                graph_ops::add_condition(
                    p,
                    block,
                    ChoiceTarget::NewScene("Si vrai".into()),
                    Some(ChoiceTarget::NewScene("Sinon".into())),
                )
            });
            if created.is_some() {
                select_block(ctx, model, block);
            }
        }
        GraphEvent::AddEnd(block) => {
            ctx.ops_or_toast(|p| graph_ops::add_end(p, block));
        }
        GraphEvent::RemoveEnd(block) => {
            ctx.ops_or_toast(|p| graph_ops::remove_terminator(p, block));
        }
        GraphEvent::AddOption(slot) => {
            if let Some(node) = slot.node {
                ctx.ops_or_toast(|p| {
                    graph_ops::add_option(
                        p,
                        node,
                        "Nouvelle option",
                        ChoiceTarget::NewScene("Nouvelle branche".into()),
                    )
                });
            }
        }
        GraphEvent::Rename(block, name) => {
            let result = ctx.ops(|p| graph_ops::rename_block(p, block, &name));
            report(ctx, result.map_err(editor_error));
        }
        GraphEvent::SetOptionText(slot, text) => {
            let (Some(node), SlotKind::Option(i)) = (slot.node, slot.kind) else {
                return;
            };
            let id = ctx
                .story()
                .and_then(|s| graph_ops::option_text(s, node, i))
                .map(|(id, _)| id);
            if let Some(id) = id {
                ctx.exec_or_toast(Command::SetString {
                    id,
                    text: Some(text),
                });
            }
        }
        GraphEvent::Delete(block) => {
            let Some(story) = ctx.story() else { return };
            let Ok(plan) = graph_ops::delete_plan(story, block) else {
                return;
            };
            // Lines of dialogue or links from elsewhere are lost: ask first.
            if !plan.incoming.is_empty() || say_count(story, &plan) > 0 {
                ctx.state_mut().graph.pending_delete = Some(block);
            } else {
                delete_now(ctx, block);
            }
        }
        GraphEvent::DeleteConfirmed(block) => {
            ctx.state_mut().graph.pending_delete = None;
            delete_now(ctx, block);
        }
    }
}

/// Lines of dialogue that a deletion removes.
fn say_count(story: &Story, plan: &DeletePlan) -> usize {
    plan.nodes
        .iter()
        .filter(|n| {
            story
                .node(**n)
                .is_some_and(|n| matches!(n.kind, NodeKind::Say { .. }))
        })
        .count()
}

fn delete_now(ctx: &mut EditorContext<'_>, block: BlockId) {
    if ctx
        .ops_or_toast(|p| graph_ops::delete_block(p, block))
        .is_some()
    {
        let graph = &mut ctx.state_mut().graph;
        graph.layout.remove(block);
        graph.dirty |= graph.persist;
    }
}

fn editor_error(e: orbit_editor_api::EditorError) -> ProjectError {
    match e {
        orbit_editor_api::EditorError::Project(p) => p,
        other => ProjectError::Rejected(other.to_string()),
    }
}
