//! The scene editor: the linear list of the lines of a scene (dialogue,
//! choices, stage and sound commands, logic), edited in place.
//!
//! The list is virtualized (only the visible rows are drawn, row heights come
//! from a cache rebuilt when the project revision changes), rows are painted
//! directly for speed, and every change is deferred as a [`RowAction`] applied
//! after drawing, through `orbit_project` operations: one undo entry per user
//! gesture.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, vec2,
};
use orbit_core::{CharacterId, Node, NodeId, NodeKind, SceneId, Story, TransitionKind};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, Level,
    PanelDef, Plugin, PluginManifest, PreviewStatus, Selection, UiText, icons,
};
use orbit_project::ops;

use super::drafts::all_insertables;
use super::edit;
use super::ids;
use super::nodeinfo::{kind_color, kind_icon, truncate};
use super::ui_util::{accent, danger, faint, primary_button, secondary_button, warning, weak};
use crate::theme::bold;

const GAP: f32 = 4.0;
const GHOST_HEIGHT: f32 = 52.0;

/// Height of a row (gap included).
fn row_height(kind: &NodeKind, editing: bool) -> f32 {
    match kind {
        NodeKind::Say { .. } if editing => 176.0,
        NodeKind::Say { .. } => 72.0,
        NodeKind::Choice { options } => 48.0 + 26.0 * options.len() as f32,
        NodeKind::Label { .. } => 38.0,
        _ => 46.0,
    }
}

/// Row geometry cached per project revision: `tops[i]` is the offset of row
/// `i`; `tops[n]` the end of the last row.
struct Layout {
    key: (u64, SceneId, Option<NodeId>, usize),
    tops: Vec<f32>,
}

impl Layout {
    fn build(key: (u64, SceneId, Option<NodeId>, usize), nodes: &[Node]) -> Self {
        let mut tops = Vec::with_capacity(nodes.len() + 1);
        let mut y = 0.0;
        for n in nodes {
            tops.push(y);
            y += row_height(&n.kind, key.2 == Some(n.id));
        }
        tops.push(y);
        Self { key, tops }
    }

    fn total(&self) -> f32 {
        self.tops.last().copied().unwrap_or(0.0)
    }

    /// Rows intersecting `[min, max]`.
    fn visible(&self, min: f32, max: f32) -> std::ops::Range<usize> {
        let n = self.tops.len().saturating_sub(1);
        let first = self.tops.partition_point(|t| *t <= min).saturating_sub(1);
        let last = self.tops.partition_point(|t| *t < max);
        first.min(n)..last.min(n)
    }

    /// Insertion boundary (0..=n) closest to `y`.
    fn boundary(&self, y: f32) -> usize {
        let n = self.tops.len() - 1;
        let i = self.tops.partition_point(|t| *t <= y);
        if i == 0 {
            return 0;
        }
        let below = self.tops[(i - 1).min(n)];
        let above = self.tops[i.min(n)];
        if (y - below) <= (above - y) {
            i - 1
        } else {
            i.min(n)
        }
    }
}

#[derive(Default)]
struct SceneUi {
    editing: Option<NodeId>,
    focus_edit: bool,
    quick: Option<(String, usize)>,
    rename: Option<(SceneId, String)>,
    layout: Option<Layout>,
    drag: Option<Vec<NodeId>>,
    last_primary: Option<NodeId>,
    scroll_to: Option<NodeId>,
    anchor: Option<NodeId>,
    /// A frame ago the selection changed because of a click in this panel.
    clicked_here: bool,
}

type Shared = Rc<RefCell<SceneUi>>;

pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::SCENE, "Éditeur de scène", "0.1.0")
    }
}

fn has_nodes_selected(ctx: &EditorContext<'_>) -> bool {
    !ctx.selected_nodes().is_empty()
}

impl EditorPlugin for ScenePlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        let state: Shared = Rc::default();
        {
            let state = state.clone();
            reg.panel(PanelDef::new(
                "scene",
                "scene.panel.title",
                icons::LIST_DASHES,
                DockSlot::Center,
                move |ui, ctx| panel(ui, ctx, &state),
            ));
        }
        {
            let state = state.clone();
            reg.action(
                ActionDef::new("quick_insert", "scene.action.quick_insert", move |ctx| {
                    if ctx.has_project() {
                        state.borrow_mut().quick = Some((String::new(), 0));
                        ctx.request(EditorRequest::ShowPanel(super::panels::SCENE.to_owned()));
                    }
                })
                .icon(icons::PLUS_CIRCLE)
                .category("category.edit")
                .shortcut(Modifiers::COMMAND, egui::Key::I)
                .menu_group("menu.edit.insert")
                .menu("edit", 205)
                .enabled(|c| c.story().is_some_and(|s| !s.scenes.is_empty())),
            );
        }
        for (id, label, icon, order, key) in [
            (
                "insert_choice",
                "scene.action.insert_choice",
                icons::GIT_BRANCH,
                206,
                "Choice",
            ),
            (
                "insert_background",
                "scene.action.insert_background",
                icons::IMAGE,
                207,
                "Background",
            ),
        ] {
            reg.action(
                ActionDef::new(id, label, move |ctx| {
                    edit::insert_kind(ctx, key);
                })
                .icon(icon)
                .category("category.edit")
                .menu_group("menu.edit.insert")
                .menu("edit", order)
                .enabled(|c| c.story().is_some_and(|s| !s.scenes.is_empty())),
            );
        }
        {
            let state = state.clone();
            reg.action(
                ActionDef::new("edit_line", "scene.action.edit_line", move |ctx| {
                    if let Some(Selection::Node(id)) = ctx.selection() {
                        ctx.state_mut().inline_edit = Some(id);
                    }
                    let _ = &state;
                })
                .icon(icons::PENCIL_SIMPLE)
                .category("category.edit")
                .menu_group("menu.edit.insert")
                .menu("edit", 204)
                .enabled(|c| matches!(c.selection(), Some(Selection::Node(_)))),
            );
        }
        reg.action(
            ActionDef::new(
                "duplicate",
                "scene.action.duplicate",
                edit::duplicate_selection,
            )
            .icon(icons::COPY_SIMPLE)
            .category("category.edit")
            .shortcut(Modifiers::COMMAND, Key::D)
            .menu_group("menu.edit.blocks")
            .menu("edit", 220)
            .enabled(has_nodes_selected),
        );
        reg.action(
            ActionDef::new("copy", "scene.action.copy", |c| {
                edit::copy_selection(c, false)
            })
            .icon(icons::COPY)
            .category("category.edit")
            .shortcut(Modifiers::COMMAND, Key::C)
            .menu_group("menu.edit.blocks")
            .menu("edit", 230)
            .enabled(has_nodes_selected),
        );
        reg.action(
            ActionDef::new("cut", "scene.action.cut", |c| edit::copy_selection(c, true))
                .icon(icons::SCISSORS)
                .category("category.edit")
                .shortcut(Modifiers::COMMAND, Key::X)
                .menu_group("menu.edit.blocks")
                .menu("edit", 231)
                .enabled(has_nodes_selected),
        );
        reg.action(
            ActionDef::new("paste", "scene.action.paste", edit::paste)
                .icon(icons::CLIPBOARD_TEXT)
                .category("category.edit")
                .shortcut(Modifiers::COMMAND, Key::V)
                .menu_group("menu.edit.blocks")
                .menu("edit", 232)
                .enabled(edit::can_paste),
        );
        reg.action(
            ActionDef::new("select_all", "scene.action.select_all", edit::select_all)
                .category("category.edit")
                .shortcut(Modifiers::COMMAND, Key::A)
                .menu_group("menu.edit.blocks")
                .menu("edit", 240)
                .enabled(|c| c.story().is_some_and(|s| !s.scenes.is_empty())),
        );
        reg.action(
            ActionDef::new("move_up", "scene.action.move_up", |c| edit::nudge(c, -1))
                .icon(icons::ARROW_UP)
                .category("category.edit")
                .shortcut(Modifiers::ALT, Key::ArrowUp)
                .menu_group("menu.edit.blocks")
                .menu("edit", 250)
                .enabled(has_nodes_selected),
        );
        reg.action(
            ActionDef::new("move_down", "scene.action.move_down", |c| edit::nudge(c, 1))
                .icon(icons::ARROW_DOWN)
                .category("category.edit")
                .shortcut(Modifiers::ALT, Key::ArrowDown)
                .menu_group("menu.edit.blocks")
                .menu("edit", 251)
                .enabled(has_nodes_selected),
        );
        reg.action(
            ActionDef::new("new_scene", "scene.action.new_scene", edit::new_scene)
                .icon(icons::FOLDER_PLUS)
                .category("category.scene")
                .menu_group("menu.edit.scenes")
                .menu("edit", 300)
                .enabled(|c| c.has_project()),
        );
        {
            let state = state.clone();
            reg.action(
                ActionDef::new("rename_scene", "scene.action.rename_scene", move |ctx| {
                    if let (Some(id), Some(story)) = (edit::current_scene(ctx), ctx.story())
                        && let Some(scene) = story.scene(id)
                    {
                        state.borrow_mut().rename = Some((id, scene.name.clone()));
                        ctx.request(EditorRequest::ShowPanel(super::panels::SCENE.to_owned()));
                    }
                })
                .icon(icons::PENCIL_SIMPLE)
                .category("category.scene")
                .shortcut(Modifiers::NONE, Key::F2)
                .menu_group("menu.edit.scenes")
                .menu("edit", 301)
                .enabled(|c| c.story().is_some_and(|s| !s.scenes.is_empty())),
            );
        }
        reg.action(
            ActionDef::new("delete_scene", "scene.action.delete_scene", |ctx| {
                if let Some(id) = edit::current_scene(ctx) {
                    edit::delete_scene(ctx, id);
                }
            })
            .icon(icons::FOLDER_MINUS)
            .category("category.scene")
            .menu_group("menu.edit.scenes")
            .menu("edit", 302)
            .enabled(|c| c.story().is_some_and(|s| !s.scenes.is_empty())),
        );
        reg.action(
            ActionDef::new("scene_up", "scene.action.scene_up", |ctx| {
                if let Some(id) = edit::current_scene(ctx) {
                    edit::move_scene(ctx, id, -1);
                }
            })
            .category("category.scene")
            .enabled(|c| c.story().is_some_and(|s| s.scenes.len() > 1)),
        );
        reg.action(
            ActionDef::new("scene_down", "scene.action.scene_down", |ctx| {
                if let Some(id) = edit::current_scene(ctx) {
                    edit::move_scene(ctx, id, 1);
                }
            })
            .category("category.scene")
            .enabled(|c| c.story().is_some_and(|s| s.scenes.len() > 1)),
        );
    }
}

// ------------------------------------------------------------------ selection

/// New selection after a click on `clicked`: plain click selects it alone,
/// Ctrl toggles it, Shift selects the range from the anchor. Returns the
/// selected ids (reading order) and the anchor to remember.
pub fn click_selection(
    nodes: &[NodeId],
    selected: &[NodeId],
    anchor: Option<NodeId>,
    clicked: NodeId,
    ctrl: bool,
    shift: bool,
) -> (Vec<NodeId>, NodeId) {
    let index_of = |id: NodeId| nodes.iter().position(|n| *n == id);
    if shift && let (Some(a), Some(c)) = (anchor.and_then(index_of), index_of(clicked)) {
        let (lo, hi) = (a.min(c), a.max(c));
        return (nodes[lo..=hi].to_vec(), nodes[a]);
    }
    if ctrl {
        let mut set: Vec<NodeId> = selected.to_vec();
        if let Some(pos) = set.iter().position(|id| *id == clicked) {
            set.remove(pos);
        } else {
            set.push(clicked);
        }
        set.sort_by_key(|id| index_of(*id).unwrap_or(usize::MAX));
        return (set, clicked);
    }
    (vec![clicked], clicked)
}

// ----------------------------------------------------------------------- panel

#[derive(Clone, Debug)]
enum RowAction {
    Select {
        id: NodeId,
        ctrl: bool,
        shift: bool,
    },
    /// Right click: the row joins the selection if it is not in it.
    ContextSelect(NodeId),
    StartEdit(NodeId),
    SetText {
        id: NodeId,
        speaker: Option<CharacterId>,
        text: String,
    },
    EndEdit,
    EnterNew(NodeId),
    BackspaceEmpty(NodeId),
    SetSpeaker {
        id: NodeId,
        speaker: Option<CharacterId>,
    },
    PlayFrom(NodeId),
    /// Jump to the scene a `JumpScene` targets (its start or its label).
    GoToScene(SceneId, Option<NodeId>),
    DragStart(NodeId),
    AddAtEnd,
    Insert(String),
    Duplicate,
    Copy,
    Cut,
    Paste,
    Delete,
    Move(i32),
    /// Ctrl+Z / Ctrl+Y inside the text field: the project's own history
    /// (live text edits are commands, merged into one step per session).
    Undo,
    Redo,
    /// An asset dragged from the library was dropped on a background /
    /// music / sound row: the row now uses it.
    DropAsset(NodeId, orbit_core::AssetId),
}

fn empty_state(ui: &mut Ui, icon: &str, title: &str, hint: &str) {
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icon).size(36.0).color(faint(ui)));
        ui.add_space(8.0);
        ui.label(RichText::new(title).font(bold(16.0)));
        ui.add_space(4.0);
        ui.label(RichText::new(hint).color(weak(ui)));
        ui.add_space(14.0);
    });
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &Shared) {
    if !ctx.has_project() {
        empty_state(
            ui,
            icons::FILM_SLATE,
            &ui.t("scene.empty.no_project.title"),
            &ui.t("scene.empty.no_project.hint"),
        );
        ui.vertical_centered(|ui| {
            if primary_button(ui, &ui.t("scene.empty.open_project")).clicked() {
                ctx.request(EditorRequest::PickOpenProject);
            }
            ui.add_space(6.0);
            if secondary_button(ui, &ui.t("scene.empty.new_project")).clicked() {
                ctx.request(EditorRequest::NewProjectDialog);
            }
        });
        return;
    }
    let no_scene = ctx.story().is_some_and(|s| s.scenes.is_empty());
    if no_scene {
        empty_state(
            ui,
            icons::FOLDER_PLUS,
            &ui.t("scene.empty.no_scene.title"),
            &ui.t("scene.empty.no_scene.hint"),
        );
        ui.vertical_centered(|ui| {
            if primary_button(ui, &ui.t("scene.empty.create_scene")).clicked() {
                edit::new_scene(ctx);
            }
        });
        return;
    }
    let Some(scene_id) = edit::current_scene(ctx) else {
        return;
    };
    ctx.state_mut().current_scene = Some(scene_id);

    {
        let mut st = shared.borrow_mut();
        if let Some(id) = ctx.state_mut().inline_edit.take() {
            st.editing = Some(id);
            st.focus_edit = true;
            st.scroll_to = Some(id);
        }
        // A new primary selection from elsewhere (explorer, undo): show it.
        let primary = match ctx.selection() {
            Some(Selection::Node(id)) => Some(id),
            _ => None,
        };
        if primary != st.last_primary {
            if !st.clicked_here {
                st.scroll_to = primary;
            }
            st.last_primary = primary;
        }
        st.clicked_here = false;
        if let Some(e) = st.editing
            && ctx.story().is_some_and(|s| s.locate(e).is_none())
        {
            st.editing = None;
        }
        if st.editing.is_some() && st.editing != primary {
            // Selection moved away from the line being edited.
            st.editing = None;
            ctx.end_merge();
        }
    }

    header(ui, ctx, shared, scene_id);
    ui.add_space(4.0);
    keyboard(ui, ctx, shared, scene_id);
    list(ui, ctx, shared, scene_id);
    quick_insert(ui, ctx, shared);
}

fn header(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &Shared, scene_id: SceneId) {
    let Some(story) = ctx.story() else { return };
    let Some(scene) = story.scene(scene_id) else {
        return;
    };
    let scene_name = scene.name.clone();
    let count = scene.nodes.len();
    let scenes: Vec<(SceneId, String, usize)> = story
        .scenes
        .iter()
        .map(|s| (s.id, s.name.clone(), s.nodes.len()))
        .collect();
    let index = scenes.iter().position(|s| s.0 == scene_id).unwrap_or(0);
    let mut switch_to = None;
    let mut commit_rename: Option<(SceneId, String)> = None;
    let mut cancel_rename = false;

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let renaming = shared.borrow().rename.as_ref().map(|r| r.0) == Some(scene_id);
                if renaming {
                    let mut st = shared.borrow_mut();
                    if let Some((_, text)) = st.rename.as_mut() {
                        let resp = ui.add(
                            egui::TextEdit::singleline(text)
                                .id(Id::new("scene_rename"))
                                .desired_width(220.0)
                                .font(bold(15.0)),
                        );
                        if !resp.has_focus() && !resp.lost_focus() {
                            resp.request_focus();
                        }
                        if resp.lost_focus() {
                            if ui.input(|i| i.key_pressed(Key::Escape)) {
                                cancel_rename = true;
                            } else {
                                commit_rename = Some((scene_id, text.trim().to_owned()));
                            }
                        }
                    }
                } else {
                    egui::ComboBox::from_id_salt("scene_switch")
                        .selected_text(
                            RichText::new(format!("{}  {scene_name}", icons::FOLDER_OPEN))
                                .font(bold(14.0)),
                        )
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            for (id, name, n) in &scenes {
                                let label = format!("{name}   ({n})");
                                if ui.selectable_label(*id == scene_id, label).clicked() {
                                    switch_to = Some(*id);
                                }
                            }
                        });
                    if ui.available_width() > 400.0 {
                        ui.label(
                            RichText::new(ui.tn("scene.lines", count, &[]))
                                .small()
                                .color(faint(ui)),
                        );
                    }
                }

                ui.menu_button(RichText::new(icons::DOTS_THREE_VERTICAL).size(16.0), |ui| {
                    if ui
                        .button(format!(
                            "{}  {}",
                            icons::FOLDER_PLUS,
                            ui.t("scene.menu.new_scene")
                        ))
                        .clicked()
                    {
                        ctx.run_action("orbit.scene.new_scene");
                        ui.close();
                    }
                    if ui
                        .button(format!(
                            "{}  {}",
                            icons::PENCIL_SIMPLE,
                            ui.t("scene.menu.rename")
                        ))
                        .clicked()
                    {
                        ctx.run_action("orbit.scene.rename_scene");
                        ui.close();
                    }
                    ui.add_enabled_ui(index > 0, |ui| {
                        if ui
                            .button(format!(
                                "{}  {}",
                                icons::ARROW_UP,
                                ui.t("scene.menu.move_up")
                            ))
                            .clicked()
                        {
                            ctx.run_action("orbit.scene.scene_up");
                            ui.close();
                        }
                    });
                    ui.add_enabled_ui(index + 1 < scenes.len(), |ui| {
                        if ui
                            .button(format!(
                                "{}  {}",
                                icons::ARROW_DOWN,
                                ui.t("scene.menu.move_down")
                            ))
                            .clicked()
                        {
                            ctx.run_action("orbit.scene.scene_down");
                            ui.close();
                        }
                    });
                    ui.separator();
                    if ui
                        .button(
                            RichText::new(format!(
                                "{}  {}",
                                icons::TRASH,
                                ui.t("scene.menu.delete")
                            ))
                            .color(danger(ui)),
                        )
                        .clicked()
                    {
                        ctx.run_action("orbit.scene.delete_scene");
                        ui.close();
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    insert_menu(ui, ctx);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!(
                                    "{}  {}",
                                    icons::PLUS,
                                    ui.t("scene.add_line_short")
                                ))
                                .color(Color32::WHITE)
                                .font(bold(13.0)),
                            )
                            .fill(accent(ui))
                            .corner_radius(CornerRadius::same(7)),
                        )
                        .on_hover_text(ui.t("scene.add_line_tip"))
                        .clicked()
                    {
                        edit::add_say(ctx);
                    }
                });
            });
        });

    if let Some(id) = switch_to {
        ctx.select(Selection::Scene(id));
        shared.borrow_mut().editing = None;
    }
    if let Some((id, name)) = commit_rename {
        shared.borrow_mut().rename = None;
        if !name.is_empty() && name != scene_name {
            ctx.exec_or_toast(orbit_project::Command::RenameScene { id, name });
        }
    }
    if cancel_rename {
        shared.borrow_mut().rename = None;
    }
}

/// The "+ Ajouter" menu: every node type, grouped.
fn insert_menu(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let items = all_insertables(ctx.registry());
    ui.menu_button(
        RichText::new(format!("{}  {}", icons::PLUS_CIRCLE, ui.t("common.add"))),
        |ui| {
            let texts = ctx.translator();
            let mut group = "";
            for item in &items {
                if item.group != group {
                    if !group.is_empty() {
                        ui.separator();
                    }
                    group = item.group;
                    ui.label(
                        RichText::new(item.group_text(&texts).to_uppercase())
                            .font(bold(10.0))
                            .color(faint(ui)),
                    );
                }
                if ui
                    .button(format!("{}  {}", item.icon, item.label_text(&texts)))
                    .clicked()
                {
                    edit::insert_kind(ctx, &item.key);
                    ui.close();
                }
            }
            ui.separator();
            ui.label(
                RichText::new(ui.t("scene.quick_menu_hint"))
                    .small()
                    .color(faint(ui)),
            );
        },
    )
    .response
    .on_hover_text(ui.t("scene.add_other_tip"));
}

/// Arrow keys, Enter and Escape in the list, while no text field is active
/// and the pointer is over the panel.
fn keyboard(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &Shared, scene_id: SceneId) {
    if ui.ctx().egui_wants_keyboard_input() || !ui.ui_contains_pointer() {
        return;
    }
    if shared.borrow().quick.is_some() {
        return;
    }
    let (up, down, home, end, enter, escape, shift) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowUp)
                || i.consume_key(Modifiers::SHIFT, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown)
                || i.consume_key(Modifiers::SHIFT, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::Home),
            i.consume_key(Modifiers::NONE, Key::End),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
            i.modifiers.shift,
        )
    });
    if !(up || down || home || end || enter || escape) {
        return;
    }
    let primary = match ctx.selection() {
        Some(Selection::Node(id)) => Some(id),
        _ => None,
    };
    let (ids, primary_is_say) = {
        let Some(story) = ctx.story() else { return };
        let Some(scene) = story.scene(scene_id) else {
            return;
        };
        (
            scene.nodes.iter().map(|n| n.id).collect::<Vec<NodeId>>(),
            primary.is_some_and(|p| {
                matches!(story.node(p).map(|n| &n.kind), Some(NodeKind::Say { .. }))
            }),
        )
    };
    let pos = primary.and_then(|p| ids.iter().position(|n| *n == p));
    let n = ids.len();
    let target = if n == 0 {
        None
    } else if up {
        Some(pos.map_or(n - 1, |p| p.saturating_sub(1)))
    } else if down {
        Some(pos.map_or(0, |p| (p + 1).min(n - 1)))
    } else if home {
        Some(0)
    } else if end {
        Some(n - 1)
    } else {
        None
    };
    if let Some(t) = target {
        let id = ids[t];
        let anchor = shared.borrow().anchor.or(primary);
        if shift && let Some(a) = anchor {
            let selected = click_selection(&ids, &ctx.selected_nodes(), Some(a), id, false, true);
            {
                let mut st = shared.borrow_mut();
                st.anchor = Some(a);
                st.scroll_to = Some(id);
            }
            ctx.select_nodes(selected.0, id);
        } else {
            {
                let mut st = shared.borrow_mut();
                st.anchor = Some(id);
                st.scroll_to = Some(id);
            }
            ctx.select(Selection::Node(id));
        }
    }
    if enter && primary_is_say {
        let mut st = shared.borrow_mut();
        st.editing = primary;
        st.focus_edit = true;
    }
    if escape
        && ctx.selected_nodes().len() > 1
        && let Some(p) = primary
    {
        ctx.select(Selection::Node(p));
    }
}

// ------------------------------------------------------------------------ list

struct RowEnv<'a> {
    story: &'a Story,
    ui_selected: &'a [NodeId],
    primary: Option<NodeId>,
    playing: Option<NodeId>,
    diagnostics: &'a orbit_editor_api::Diagnostics,
    dragging: Option<&'a [NodeId]>,
    editing: Option<NodeId>,
    focus_edit: bool,
    /// The row under the pointer, to show its handle.
    index_base: usize,
}

fn list(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &Shared, scene_id: SceneId) {
    let revision = ctx.project().map_or(0, orbit_project::Project::revision);
    let mut actions: Vec<RowAction> = Vec::new();
    let mut pointer_drop: Option<(usize, Vec<NodeId>)> = None;
    {
        let Some(story) = ctx.story() else { return };
        let Some(scene) = story.scene(scene_id) else {
            return;
        };
        let mut st = shared.borrow_mut();
        let key = (revision, scene_id, st.editing, scene.nodes.len());
        if st.layout.as_ref().is_none_or(|l| l.key != key) {
            st.layout = Some(Layout::build(key, &scene.nodes));
        }
        if scene.nodes.is_empty() {
            drop(st);
            empty_state(
                ui,
                icons::CHAT_TEXT,
                &ui.t("scene.empty.scene.title"),
                &ui.t("scene.empty.scene.hint"),
            );
            ui.vertical_centered(|ui| {
                if primary_button(ui, &ui.t("scene.empty.add_first")).clicked() {
                    actions.push(RowAction::AddAtEnd);
                }
            });
            apply_actions(ctx, shared, scene_id, actions);
            return;
        }

        let selected = ctx.selected_nodes();
        let primary = match ctx.selection() {
            Some(Selection::Node(id)) => Some(id),
            _ => None,
        };
        let playing = {
            let p = &ctx.state().preview;
            (p.status != PreviewStatus::Stopped)
                .then_some(p.current_node)
                .flatten()
        };
        let drag = st.drag.clone();
        let env = RowEnv {
            story,
            ui_selected: &selected,
            primary,
            playing,
            diagnostics: &ctx.state().diagnostics,
            dragging: drag.as_deref(),
            editing: st.editing,
            focus_edit: st.focus_edit,
            index_base: 0,
        };
        let layout = st
            .layout
            .take()
            .unwrap_or_else(|| Layout::build(key, &scene.nodes));
        // Scroll request: bring the row into view.
        let mut scroll = ScrollArea::vertical().auto_shrink([false, false]);
        let viewport_h = ui.available_height();
        if let Some(target) = st.scroll_to.take()
            && let Some(i) = scene.nodes.iter().position(|n| n.id == target)
        {
            let (top, bottom) = (layout.tops[i], layout.tops[i + 1]);
            let current = ui
                .ctx()
                .data(|d| d.get_temp::<f32>(Id::new("scene_scroll_offset")))
                .unwrap_or(0.0);
            if top < current {
                scroll = scroll.vertical_scroll_offset((top - 8.0).max(0.0));
            } else if bottom > current + viewport_h - 8.0 {
                scroll = scroll.vertical_scroll_offset((bottom - viewport_h + 24.0).max(0.0));
            }
        }
        if let Some(speed) = ctx.state().debug_autoscroll {
            let current = ui
                .ctx()
                .data(|d| d.get_temp::<f32>(Id::new("scene_scroll_offset")))
                .unwrap_or(0.0);
            let max = (layout.total() - viewport_h).max(0.0);
            let next = current + speed;
            scroll = scroll.vertical_scroll_offset(if next > max { 0.0 } else { next });
            ui.ctx().request_repaint();
        }
        drop(st);

        let nodes = &scene.nodes;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            let width = ui.available_width();
            let (area, _) =
                ui.allocate_exact_size(vec2(width, layout.total() + GHOST_HEIGHT), Sense::hover());
            let range = layout.visible(viewport.min.y - 60.0, viewport.max.y + 60.0);
            for i in range {
                let h = layout.tops[i + 1] - layout.tops[i];
                let rect = Rect::from_min_size(
                    Pos2::new(area.left() + 6.0, area.top() + layout.tops[i]),
                    vec2(width - 12.0, h - GAP),
                );
                let env = RowEnv {
                    index_base: i,
                    ..clone_env(&env)
                };
                draw_row(ui, &env, &nodes[i], rect, shared, &mut actions);
            }
            // The "add" row at the end of the list.
            let ghost = Rect::from_min_size(
                Pos2::new(area.left() + 6.0, area.top() + layout.total() + 4.0),
                vec2(width - 12.0, GHOST_HEIGHT - 12.0),
            );
            if ghost.bottom() >= area.top() + viewport.min.y
                && ghost.top() <= area.top() + viewport.max.y
            {
                draw_ghost(ui, ghost, &mut actions);
            }
            // Drag and drop: insertion line and drop.
            let mut drop_target = None;
            if let Some(dragged) = shared.borrow().drag.clone()
                && let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
            {
                let y = pointer.y - area.top();
                let boundary = layout.boundary(y.clamp(0.0, layout.total()));
                let line_y = area.top() + layout.tops[boundary] - GAP / 2.0;
                let color = accent(ui);
                ui.painter().line_segment(
                    [
                        Pos2::new(area.left() + 10.0, line_y),
                        Pos2::new(area.right() - 10.0, line_y),
                    ],
                    Stroke::new(2.5, color),
                );
                ui.painter()
                    .circle_filled(Pos2::new(area.left() + 10.0, line_y), 4.0, color);
                // Edge auto-scroll.
                let clip = ui.clip_rect();
                let margin = 36.0;
                if pointer.y < clip.top() + margin {
                    ui.scroll_with_delta(vec2(0.0, 14.0));
                } else if pointer.y > clip.bottom() - margin {
                    ui.scroll_with_delta(vec2(0.0, -14.0));
                }
                ui.ctx().request_repaint();
                drop_target = Some((boundary, dragged));
            }
            drop_target
        });
        ui.ctx().data_mut(|d| {
            d.insert_temp(Id::new("scene_scroll_offset"), output.state.offset.y);
        });
        shared.borrow_mut().layout = Some(layout);
        if let Some((boundary, dragged)) = output.inner
            && ui.input(|i| i.pointer.any_released() || !i.pointer.any_down())
        {
            pointer_drop = Some((boundary, dragged));
        }
    }

    if let Some((boundary, dragged)) = pointer_drop {
        shared.borrow_mut().drag = None;
        if let Some(story) = ctx.story()
            && let Some(scene) = story.scene(scene_id)
        {
            // Index among the nodes that do not move.
            let index = scene.nodes[..boundary.min(scene.nodes.len())]
                .iter()
                .filter(|n| !dragged.contains(&n.id))
                .count();
            let before: Vec<NodeId> = scene.nodes.iter().map(|n| n.id).collect();
            let ids = dragged.clone();
            if ctx
                .ops_or_toast(|p| ops::move_nodes(p, &ids, scene_id, index))
                .is_some()
            {
                // Nothing changed when dropped on its own place: drop the
                // empty undo entry instead of keeping a no-op.
                let after: Vec<NodeId> = ctx
                    .story()
                    .and_then(|s| s.scene(scene_id))
                    .map(|s| s.nodes.iter().map(|n| n.id).collect())
                    .unwrap_or_default();
                if after == before {
                    let _ = ctx.undo();
                }
            }
        }
    }
    apply_actions(ctx, shared, scene_id, actions);
}

fn clone_env<'a>(e: &RowEnv<'a>) -> RowEnv<'a> {
    RowEnv {
        story: e.story,
        ui_selected: e.ui_selected,
        primary: e.primary,
        playing: e.playing,
        diagnostics: e.diagnostics,
        dragging: e.dragging,
        editing: e.editing,
        focus_edit: e.focus_edit,
        index_base: e.index_base,
    }
}

use egui::ScrollArea;

fn draw_ghost(ui: &mut Ui, rect: Rect, actions: &mut Vec<RowAction>) {
    let response = ui.interact(rect, Id::new("scene_ghost"), Sense::click());
    let color = if response.hovered() {
        accent(ui)
    } else {
        faint(ui)
    };
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.2, color.gamma_multiply(0.7)),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        format!("{}  {}", icons::PLUS, ui.t("scene.add_line")),
        bold(13.0),
        color,
    );
    if response.clicked() {
        actions.push(RowAction::AddAtEnd);
    }
}

fn avatar(painter: &egui::Painter, center: Pos2, color: Color32, glyph: &str, size: f32) {
    painter.circle_filled(center, size / 2.0, color);
    painter.text(
        center,
        Align2::CENTER_CENTER,
        glyph,
        bold(size * 0.5),
        Color32::WHITE,
    );
}

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_owned())
}

/// Title and detail of a non-dialogue row.
fn describe(texts: &orbit_editor_api::Translator, story: &Story, node: &Node) -> (String, String) {
    let asset = |id| {
        story.asset_path(id).map_or_else(
            || texts.t("scene.row.unknown_asset"),
            |p| p.rsplit('/').next().unwrap_or(p).to_owned(),
        )
    };
    let who = |c| {
        super::nodeinfo::summary(
            texts,
            story,
            &Node {
                id: NodeId::new(),
                kind: NodeKind::Hide {
                    character: c,
                    transition: None,
                },
            },
        )
    };
    let transition = |t: &Option<orbit_core::Transition>| match t {
        None => String::new(),
        Some(t) => match t.kind {
            TransitionKind::Dissolve => {
                format!(
                    "  ·  {}",
                    texts.tf("scene.row.dissolve", &[("seconds", &t.duration)])
                )
            }
            TransitionKind::Fade(_) => {
                format!(
                    "  ·  {}",
                    texts.tf("scene.row.fade", &[("seconds", &t.duration)])
                )
            }
        },
    };
    let label = |id| match story.node(id).map(|n| &n.kind) {
        Some(NodeKind::Label { name }) => name.clone(),
        _ => "?".to_owned(),
    };
    match &node.kind {
        NodeKind::Label { name } => (texts.t("scene.row.label"), name.clone()),
        NodeKind::Choice { options } => (
            texts.t("scene.row.choice"),
            texts.tn("scene.row.options", options.len(), &[]),
        ),
        NodeKind::Jump { target } => (texts.t("scene.row.jump"), label(*target)),
        NodeKind::JumpScene { scene, label: l } => {
            let name = story.scene(*scene).map_or("?", |s| s.name.as_str());
            (
                texts.t("scene.row.jump_scene"),
                match l {
                    Some(l) => format!("{name} › {}", label(*l)),
                    None => name.to_owned(),
                },
            )
        }
        NodeKind::SetVar { var, value } => {
            (texts.t("scene.row.set_var"), format!("{var} = {value}"))
        }
        NodeKind::If {
            condition,
            then_target,
            else_target,
        } => (
            texts.t("scene.row.if"),
            format!(
                "{}{}",
                texts.tf(
                    "scene.row.if_then",
                    &[("condition", condition), ("target", &label(*then_target))]
                ),
                else_target.map_or(String::new(), |e| texts
                    .tf("scene.row.if_else", &[("target", &label(e))]))
            ),
        ),
        NodeKind::Background {
            asset: a,
            transition: t,
        } => (
            texts.t("scene.row.background"),
            format!("{}{}", asset(*a), transition(t)),
        ),
        NodeKind::ShowScene3d {
            scene,
            camera,
            transition: t,
        } => (
            texts.t("scene.row.scene3d"),
            format!(
                "{}{}{}",
                asset(*scene),
                camera
                    .as_ref()
                    .map_or_else(String::new, |c| format!("  ·  {c}")),
                transition(t)
            ),
        ),
        NodeKind::Camera3d {
            camera,
            transition: t,
        } => (
            texts.t("scene.row.camera3d"),
            format!("{camera}{}", transition(t)),
        ),
        NodeKind::HideScene3d { transition: t } => {
            (texts.t("scene.row.hide_scene3d"), transition(t))
        }
        NodeKind::Show {
            character,
            expression,
            position,
            transition: t,
            scale,
            scale_y,
            flip_x,
            flip_y,
        } => (
            texts.t("scene.row.show"),
            format!(
                "{} ({expression})  ·  {}{}{}",
                who(*character),
                super::nodeinfo::position_name(texts, position),
                super::nodeinfo::look_text(texts, *scale, *scale_y, *flip_x, *flip_y),
                transition(t)
            ),
        ),
        NodeKind::Hide {
            character,
            transition: t,
        } => (
            texts.t("scene.row.hide"),
            format!("{}{}", who(*character), transition(t)),
        ),
        NodeKind::PlayMusic {
            asset: a,
            fade_in,
            looping,
        } => (
            texts.t("scene.row.music"),
            format!(
                "{}{}{}",
                asset(*a),
                if *looping {
                    format!("  ·  {}", texts.t("scene.row.looping"))
                } else {
                    String::new()
                },
                if *fade_in > 0.0 {
                    format!(
                        "  ·  {}",
                        texts.tf("scene.row.fade", &[("seconds", fade_in)])
                    )
                } else {
                    String::new()
                }
            ),
        ),
        NodeKind::StopMusic { fade_out } => (
            texts.t("scene.row.stop_music"),
            if *fade_out > 0.0 {
                texts.tf("scene.row.fade", &[("seconds", fade_out)])
            } else {
                String::new()
            },
        ),
        NodeKind::PlaySound { asset: a } => (texts.t("scene.row.sound"), asset(*a)),
        NodeKind::Wait { seconds } => (texts.t("scene.row.wait"), format!("{seconds} s")),
        NodeKind::Call { label: l } => (texts.t("scene.row.call"), label(*l)),
        NodeKind::Effect(e) => super::fx_text::effect_row(texts, story, e),
        NodeKind::Timeline {
            duration,
            looping,
            wait,
            tracks,
        } => (
            texts.t("scene.row.timeline"),
            super::fx_text::timeline_detail(texts, *duration, *looping, *wait, tracks),
        ),
        NodeKind::OpenScreen { screen, .. } => (texts.t("scene.row.open_screen"), screen.clone()),
        NodeKind::CloseScreen { screen } => (texts.t("scene.row.close_screen"), screen.clone()),
        NodeKind::BackgroundLayers {
            layers,
            transition: t,
        } => (
            texts.t("scene.row.background_layers"),
            format!(
                "{}{}",
                layers.iter().map(|l| asset(l.asset)).collect::<Vec<_>>().join(" + "),
                transition(t)
            ),
        ),
        NodeKind::Particles(e) => (
            texts.t("scene.row.particles"),
            format!("{}  ·  {}", e.name, e.preset.name()),
        ),
        NodeKind::StopParticles { name } => (
            texts.t("scene.row.stop_particles"),
            if name.is_empty() { texts.t("inspector.stop_particles.all") } else { name.clone() },
        ),
        NodeKind::Script {
            source, branches, ..
        } => (
            texts.t("scene.row.script"),
            super::nodeinfo::script_summary(texts, source, branches.len()),
        ),
        NodeKind::Unlock { achievement } => (
            texts.t("scene.row.unlock"),
            story
                .achievements
                .iter()
                .find(|a| a.id == *achievement)
                .and_then(|a| story.text(a.title_id))
                .unwrap_or("?")
                .to_owned(),
        ),
        NodeKind::Return => (texts.t("scene.row.return"), String::new()),
        NodeKind::End => (texts.t("scene.row.end"), String::new()),
        NodeKind::Custom {
            plugin_id, kind, ..
        } => (texts.t("scene.row.plugin"), format!("{plugin_id}.{kind}")),
        NodeKind::Say { .. } => (String::new(), String::new()),
    }
}

fn draw_row(
    ui: &mut Ui,
    env: &RowEnv<'_>,
    node: &Node,
    rect: Rect,
    shared: &Shared,
    actions: &mut Vec<RowAction>,
) {
    let selected = env.ui_selected.contains(&node.id);
    let is_primary = env.primary == Some(node.id);
    let editing = env.editing == Some(node.id);
    let dim = env.dragging.is_some_and(|d| d.contains(&node.id));
    let hover_id = Id::new(("scene_row", node.id));
    let response = ui.interact(rect, hover_id, Sense::click_and_drag());
    let hovered = response.hovered() && env.dragging.is_none();
    let painter = ui.painter_at(rect.expand(2.0));
    // An asset dragged from the library onto a row that holds an asset.
    let wants = match &node.kind {
        NodeKind::Background { .. } => Some(orbit_core::AssetKind::Image),
        NodeKind::PlayMusic { .. } => Some(orbit_core::AssetKind::Music),
        NodeKind::PlaySound { .. } => Some(orbit_core::AssetKind::Sound),
        _ => None,
    };
    if let Some(kind) = wants {
        if let Some(drag) = response.dnd_hover_payload::<orbit_editor_api::fields::AssetDrag>() {
            let ok = drag.kind == kind;
            painter.rect_stroke(
                rect,
                CornerRadius::same(8),
                Stroke::new(2.0, if ok { accent(ui) } else { danger(ui) }),
                StrokeKind::Inside,
            );
        }
        if let Some(drag) = response.dnd_release_payload::<orbit_editor_api::fields::AssetDrag>()
            && drag.kind == kind
        {
            actions.push(RowAction::DropAsset(node.id, drag.id));
        }
    }

    // Background.
    let visuals = ui.visuals();
    let base = visuals.widgets.inactive.bg_fill;
    let fill = if selected {
        accent(ui).gamma_multiply(0.22)
    } else if hovered {
        visuals.widgets.hovered.bg_fill
    } else {
        base.gamma_multiply(0.7)
    };
    painter.rect_filled(rect, CornerRadius::same(8), fill);
    if is_primary {
        painter.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.5, accent(ui)),
            StrokeKind::Inside,
        );
    } else if selected {
        painter.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0, accent(ui).gamma_multiply(0.6)),
            StrokeKind::Inside,
        );
    }
    if env.playing == Some(node.id) {
        painter.rect_filled(
            Rect::from_min_max(rect.left_top(), Pos2::new(rect.left() + 3.0, rect.bottom())),
            CornerRadius::same(2),
            super::ui_util::success(),
        );
    }
    let content_alpha = if dim { 0.4 } else { 1.0 };

    // Grip (drag handle) and diagnostics dot.
    let mid_y = rect.center().y;
    if hovered || selected {
        painter.text(
            Pos2::new(rect.left() + 11.0, mid_y),
            Align2::CENTER_CENTER,
            icons::DOTS_SIX_VERTICAL,
            FontId::proportional(14.0),
            faint(ui),
        );
    }
    let diag = env.diagnostics.worst(node.id);
    if let Some(level) = diag {
        let color = if level == Level::Error {
            danger(ui)
        } else {
            warning(ui)
        };
        let dot = Pos2::new(rect.right() - 14.0, rect.top() + 14.0);
        painter.circle_filled(dot, 4.5, color);
        let dot_resp = ui.interact(
            Rect::from_center_size(dot, vec2(18.0, 18.0)),
            hover_id.with("diag"),
            Sense::hover(),
        );
        if dot_resp.hovered() {
            dot_resp.on_hover_ui(|ui| {
                for (l, msg) in env.diagnostics.of(node.id) {
                    let c = if *l == Level::Error {
                        danger(ui)
                    } else {
                        warning(ui)
                    };
                    ui.label(RichText::new(msg).color(c));
                }
            });
        }
    }

    let x0 = rect.left() + 26.0;
    match &node.kind {
        NodeKind::Say { speaker, text_id, .. } => {
            draw_say(
                ui,
                env,
                node,
                *speaker,
                *text_id,
                rect,
                x0,
                shared,
                actions,
                content_alpha,
            );
        }
        NodeKind::Label { name } => {
            let color = kind_color(&node.kind);
            painter.text(
                Pos2::new(x0 + 8.0, mid_y),
                Align2::CENTER_CENTER,
                icons::FLAG,
                FontId::proportional(15.0),
                color,
            );
            let galley = painter.layout_no_wrap(name.clone(), bold(13.5), color);
            let w = galley.size().x;
            painter.galley(
                Pos2::new(x0 + 24.0, mid_y - galley.size().y / 2.0),
                galley,
                color,
            );
            painter.line_segment(
                [
                    Pos2::new(x0 + 24.0 + w + 12.0, mid_y),
                    Pos2::new(rect.right() - 30.0, mid_y),
                ],
                Stroke::new(1.0, color.gamma_multiply(0.35)),
            );
        }
        NodeKind::Choice { options } => {
            let color = kind_color(&node.kind);
            chip(
                &painter,
                Pos2::new(x0 + 13.0, rect.top() + 21.0),
                icons::GIT_BRANCH,
                color,
            );
            painter.text(
                Pos2::new(x0 + 36.0, rect.top() + 21.0),
                Align2::LEFT_CENTER,
                ui.t("scene.row.player_choice"),
                bold(13.0),
                ui.visuals().text_color().gamma_multiply(content_alpha),
            );
            for (i, option) in options.iter().enumerate() {
                let y = rect.top() + 46.0 + 26.0 * i as f32;
                let box_rect = Rect::from_min_max(
                    Pos2::new(x0 + 36.0, y - 11.0),
                    Pos2::new(rect.right() - 36.0, y + 11.0),
                );
                painter.rect_filled(box_rect, CornerRadius::same(6), color.gamma_multiply(0.14));
                let text = env.story.text(option.text_id).unwrap_or("?");
                let target = match env.story.node(option.target).map(|n| &n.kind) {
                    Some(NodeKind::Label { name }) => name.clone(),
                    _ => "?".to_owned(),
                };
                let tail = format!(
                    "{}→ {target}",
                    if option.condition.is_some() {
                        ui.t("scene.row.if_prefix")
                    } else {
                        String::new()
                    }
                );
                let tail_galley =
                    painter.layout_no_wrap(tail, FontId::proportional(11.5), faint(ui));
                let tail_w = tail_galley.size().x;
                painter.galley(
                    Pos2::new(
                        box_rect.right() - tail_w - 8.0,
                        y - tail_galley.size().y / 2.0,
                    ),
                    tail_galley,
                    faint(ui),
                );
                let clip = Rect::from_min_max(
                    box_rect.left_top(),
                    Pos2::new(box_rect.right() - tail_w - 14.0, box_rect.bottom()),
                );
                painter.with_clip_rect(clip).text(
                    Pos2::new(box_rect.left() + 10.0, y),
                    Align2::LEFT_CENTER,
                    truncate(text, 80),
                    FontId::proportional(13.0),
                    ui.visuals().text_color().gamma_multiply(content_alpha),
                );
            }
        }
        _ => {
            let color = kind_color(&node.kind);
            chip(
                &painter,
                Pos2::new(x0 + 13.0, mid_y),
                kind_icon(&node.kind),
                color,
            );
            let (title, detail) = describe(&ui.texts(), env.story, node);
            let text_color = ui.visuals().text_color().gamma_multiply(content_alpha);
            let g = painter.layout_no_wrap(title, bold(13.0), text_color);
            let w = g.size().x;
            painter.galley(
                Pos2::new(x0 + 36.0, mid_y - g.size().y / 2.0),
                g,
                text_color,
            );
            if !detail.is_empty() {
                let clip = Rect::from_min_max(
                    Pos2::new(x0 + 36.0 + w + 10.0, rect.top()),
                    Pos2::new(rect.right() - 34.0, rect.bottom()),
                );
                painter.with_clip_rect(clip).text(
                    Pos2::new(x0 + 36.0 + w + 10.0, mid_y),
                    Align2::LEFT_CENTER,
                    detail,
                    FontId::proportional(13.0),
                    weak(ui),
                );
            }
        }
    }

    // "Play from here" on the primary row.
    if is_primary && !editing {
        let play = Rect::from_center_size(
            Pos2::new(rect.right() - 16.0, rect.bottom() - 16.0),
            vec2(24.0, 22.0),
        );
        let p = ui
            .interact(play, hover_id.with("play"), Sense::click())
            .on_hover_text(ui.t("scene.play_from_tip"));
        painter.text(
            play.center(),
            Align2::CENTER_CENTER,
            icons::CROSSHAIR,
            FontId::proportional(15.0),
            if p.hovered() { accent(ui) } else { weak(ui) },
        );
        if p.clicked() {
            actions.push(RowAction::PlayFrom(node.id));
        }
    }

    // A scene jump: a button (and a double click) to go to its scene.
    if let NodeKind::JumpScene { scene, label } = &node.kind
        && !editing
    {
        let go = Rect::from_center_size(
            Pos2::new(rect.right() - 18.0, rect.top() + 20.0),
            vec2(24.0, 22.0),
        );
        let b = ui
            .interact(go, hover_id.with("go"), Sense::click())
            .on_hover_text(ui.t("scene.go_to_scene"));
        painter.text(
            go.center(),
            Align2::CENTER_CENTER,
            icons::ARROW_SQUARE_OUT,
            FontId::proportional(15.0),
            if b.hovered() { accent(ui) } else { weak(ui) },
        );
        if b.clicked() || response.double_clicked() {
            actions.push(RowAction::GoToScene(*scene, *label));
        }
    }

    // Row-level interaction.
    if response.drag_started() && !editing {
        actions.push(RowAction::ContextSelect(node.id));
        actions.push(RowAction::DragStart(node.id));
    }
    if response.secondary_clicked() {
        actions.push(RowAction::ContextSelect(node.id));
    }
    if response.clicked() {
        let m = ui.input(|i| i.modifiers);
        let sole = env.ui_selected.len() == 1 && selected;
        let say = matches!(node.kind, NodeKind::Say { .. });
        if say && sole && !m.command && !m.shift && !editing && !response.double_clicked() {
            // A click on the only selected line edits it.
            actions.push(RowAction::StartEdit(node.id));
        } else if !editing {
            actions.push(RowAction::Select {
                id: node.id,
                ctrl: m.command || m.ctrl,
                shift: m.shift,
            });
        }
    }
    if response.double_clicked() && matches!(node.kind, NodeKind::Say { .. }) {
        actions.push(RowAction::StartEdit(node.id));
    }
    if !editing {
        response.context_menu(|ui| row_menu(ui, node, actions));
    }
}

fn chip(painter: &egui::Painter, center: Pos2, glyph: &str, color: Color32) {
    painter.rect_filled(
        Rect::from_center_size(center, vec2(26.0, 26.0)),
        CornerRadius::same(7),
        color.gamma_multiply(0.22),
    );
    painter.text(
        center,
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(15.0),
        color,
    );
}

fn menu_item(ui: &mut Ui, label: String) -> bool {
    let clicked = ui.button(label).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn row_menu(ui: &mut Ui, node: &Node, actions: &mut Vec<RowAction>) {
    if matches!(node.kind, NodeKind::Say { .. })
        && menu_item(
            ui,
            format!(
                "{}  {}",
                icons::PENCIL_SIMPLE,
                ui.t("scene.row_menu.edit_text")
            ),
        )
    {
        actions.push(RowAction::StartEdit(node.id));
    }
    if menu_item(
        ui,
        format!("{}  {}", icons::CROSSHAIR, ui.t("scene.row_menu.play_from")),
    ) {
        actions.push(RowAction::PlayFrom(node.id));
    }
    let insert_after = format!(
        "{}  {}",
        icons::PLUS_CIRCLE,
        ui.t("scene.row_menu.insert_after")
    );
    ui.menu_button(insert_after, |ui| {
        for item in super::drafts::builtin_insertables() {
            if menu_item(
                ui,
                format!("{}  {}", item.icon, item.label_text(&ui.texts())),
            ) {
                actions.push(RowAction::ContextSelect(node.id));
                actions.push(RowAction::Insert(item.key));
            }
        }
    });
    ui.separator();
    if menu_item(
        ui,
        format!("{}  {}", icons::COPY_SIMPLE, ui.t("common.duplicate")),
    ) {
        actions.push(RowAction::Duplicate);
    }
    if menu_item(ui, format!("{}  {}", icons::COPY, ui.t("common.copy"))) {
        actions.push(RowAction::Copy);
    }
    if menu_item(ui, format!("{}  {}", icons::SCISSORS, ui.t("common.cut"))) {
        actions.push(RowAction::Cut);
    }
    if menu_item(
        ui,
        format!(
            "{}  {}",
            icons::CLIPBOARD_TEXT,
            ui.t("scene.row_menu.paste_after")
        ),
    ) {
        actions.push(RowAction::Paste);
    }
    ui.separator();
    if menu_item(
        ui,
        format!("{}  {}", icons::ARROW_UP, ui.t("scene.row_menu.move_up")),
    ) {
        actions.push(RowAction::Move(-1));
    }
    if menu_item(
        ui,
        format!(
            "{}  {}",
            icons::ARROW_DOWN,
            ui.t("scene.row_menu.move_down")
        ),
    ) {
        actions.push(RowAction::Move(1));
    }
    ui.separator();
    if ui
        .button(
            RichText::new(format!("{}  {}", icons::TRASH, ui.t("common.delete"))).color(danger(ui)),
        )
        .clicked()
    {
        actions.push(RowAction::Delete);
        ui.close();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_say(
    ui: &mut Ui,
    env: &RowEnv<'_>,
    node: &Node,
    speaker: Option<CharacterId>,
    text_id: orbit_core::StringId,
    rect: Rect,
    x0: f32,
    shared: &Shared,
    actions: &mut Vec<RowAction>,
    alpha: f32,
) {
    let story = env.story;
    let painter = ui.painter_at(rect.expand(2.0));
    let (name, color) = match speaker.and_then(|c| story.character(c)) {
        Some(c) => (
            story.text(c.name_id).unwrap_or("?").to_owned(),
            orbit_editor_api::fields::to_color32(c.color),
        ),
        None => (ui.t("scene.narrator"), Color32::from_rgb(140, 148, 166)),
    };
    let missing = speaker.is_some() && story.character(speaker.unwrap_or_default()).is_none();
    let center = Pos2::new(x0 + 15.0, rect.top() + 25.0);
    if speaker.is_none() {
        avatar(
            &painter,
            center,
            color.gamma_multiply(0.5),
            icons::QUOTES,
            30.0,
        );
    } else {
        avatar(&painter, center, color, &initial(&name), 30.0);
    }
    // Speaker chip: click to change the speaker.
    let name_galley = painter.layout_no_wrap(
        if missing {
            ui.t("scene.missing_character")
        } else {
            name
        },
        bold(13.0),
        color,
    );
    let name_pos = Pos2::new(x0 + 38.0, rect.top() + 8.0);
    let chip_rect = Rect::from_min_size(name_pos, name_galley.size() + vec2(24.0, 4.0));
    let chip_resp = ui.interact(
        chip_rect.expand2(vec2(2.0, 1.0)),
        Id::new(("speaker", node.id)),
        Sense::click(),
    );
    if chip_resp.hovered() {
        painter.rect_filled(
            chip_rect.expand2(vec2(2.0, 1.0)),
            CornerRadius::same(5),
            color.gamma_multiply(0.18),
        );
    }
    let name_w = name_galley.size().x;
    painter.galley(name_pos + vec2(0.0, 1.0), name_galley, color);
    painter.text(
        Pos2::new(name_pos.x + name_w + 10.0, name_pos.y + 9.0),
        Align2::CENTER_CENTER,
        icons::CARET_DOWN,
        FontId::proportional(10.0),
        faint(ui),
    );
    egui::Popup::menu(&chip_resp).show(|ui| {
        ui.set_min_width(170.0);
        if ui
            .selectable_label(
                speaker.is_none(),
                format!("{}  {}", icons::QUOTES, ui.t("scene.narrator")),
            )
            .clicked()
        {
            actions.push(RowAction::SetSpeaker {
                id: node.id,
                speaker: None,
            });
            ui.close();
        }
        for c in &story.characters {
            let n = story.text(c.name_id).unwrap_or("?");
            let label = RichText::new(format!("●  {n}"))
                .color(orbit_editor_api::fields::to_color32(c.color));
            if ui.selectable_label(speaker == Some(c.id), label).clicked() {
                actions.push(RowAction::SetSpeaker {
                    id: node.id,
                    speaker: Some(c.id),
                });
                ui.close();
            }
        }
        if story.characters.is_empty() {
            ui.label(RichText::new(ui.t("scene.no_characters")).small());
        }
    });

    let text = story.text(text_id).unwrap_or("");
    let text_left = x0 + 38.0;
    let text_rect = Rect::from_min_max(
        Pos2::new(text_left, rect.top() + 28.0),
        Pos2::new(rect.right() - 36.0, rect.bottom() - 6.0),
    );
    if env.editing == Some(node.id) {
        let edit_rect = Rect::from_min_max(
            Pos2::new(text_left - 4.0, rect.top() + 30.0),
            Pos2::new(rect.right() - 12.0, rect.bottom() - 26.0),
        );
        let mut buffer = text.to_owned();
        let id = Id::new(("say_edit", node.id));
        let focus = env.focus_edit;
        let focused = ui.memory(|m| m.has_focus(id));
        // Keys of the writing flow, before the text field sees them.
        let (enter, escape, backspace) = if focused {
            let (undo, redo) = ui.input_mut(|i| {
                let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                    || i.consume_key(Modifiers::COMMAND, Key::Y);
                (i.consume_key(Modifiers::COMMAND, Key::Z), redo)
            });
            if undo {
                actions.push(RowAction::Undo);
            }
            if redo {
                actions.push(RowAction::Redo);
            }
            ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                    buffer.is_empty() && i.consume_key(Modifiers::NONE, Key::Backspace),
                )
            })
        } else {
            (false, false, false)
        };
        let resp = ui.put(
            edit_rect,
            egui::TextEdit::multiline(&mut buffer)
                .id(id)
                .desired_width(edit_rect.width())
                .desired_rows(4)
                .font(FontId::proportional(14.0))
                .hint_text(ui.t("scene.say_hint"))
                .margin(egui::Margin::same(8)),
        );
        if focus {
            resp.request_focus();
            let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            let end = buffer.chars().count();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(end),
                )));
            state.store(ui.ctx(), id);
            shared.borrow_mut().focus_edit = false;
        }
        painter.text(
            Pos2::new(text_left, rect.bottom() - 12.0),
            Align2::LEFT_CENTER,
            ui.t("scene.edit_keys"),
            FontId::proportional(10.5),
            faint(ui),
        );
        if buffer != text {
            actions.push(RowAction::SetText {
                id: node.id,
                speaker,
                text: buffer.clone(),
            });
        }
        if backspace {
            actions.push(RowAction::BackspaceEmpty(node.id));
        } else if enter {
            actions.push(RowAction::EnterNew(node.id));
        } else if escape || (resp.lost_focus() && !focus) {
            actions.push(RowAction::EndEdit);
        }
    } else if text.is_empty() {
        painter.text(
            Pos2::new(text_left, rect.top() + 36.0),
            Align2::LEFT_CENTER,
            ui.t("scene.say_empty"),
            FontId::proportional(13.0),
            faint(ui),
        );
    } else {
        let mut job = egui::text::LayoutJob::simple(
            text.replace('\n', " ↵ "),
            FontId::proportional(13.5),
            ui.visuals().text_color().gamma_multiply(alpha),
            text_rect.width(),
        );
        job.wrap.max_rows = 2;
        job.wrap.overflow_character = Some('…');
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        painter
            .with_clip_rect(text_rect)
            .galley(text_rect.left_top(), galley, Color32::WHITE);
    }
}

// --------------------------------------------------------------------- actions

fn apply_actions(
    ctx: &mut EditorContext<'_>,
    shared: &Shared,
    scene_id: SceneId,
    actions: Vec<RowAction>,
) {
    for action in actions {
        match action {
            RowAction::Select { id, ctrl, shift } => {
                let Some(order) = ctx
                    .story()
                    .and_then(|s| s.scene(scene_id))
                    .map(|s| s.nodes.iter().map(|n| n.id).collect::<Vec<NodeId>>())
                else {
                    continue;
                };
                let (selected, anchor) = {
                    let st = shared.borrow();
                    click_selection(
                        &order,
                        &ctx.selected_nodes(),
                        st.anchor.or(match ctx.selection() {
                            Some(Selection::Node(p)) => Some(p),
                            _ => None,
                        }),
                        id,
                        ctrl,
                        shift,
                    )
                };
                let mut st = shared.borrow_mut();
                st.clicked_here = true;
                if !shift {
                    st.anchor = Some(anchor);
                }
                if st.editing.is_some() && st.editing != Some(id) {
                    st.editing = None;
                }
                drop(st);
                if selected.is_empty() {
                    ctx.clear_selection();
                } else {
                    let primary = if selected.contains(&id) {
                        id
                    } else {
                        selected[0]
                    };
                    ctx.select_nodes(selected, primary);
                }
            }
            RowAction::ContextSelect(id) => {
                if !ctx.selected_nodes().contains(&id) {
                    ctx.select(Selection::Node(id));
                }
                shared.borrow_mut().clicked_here = true;
            }
            RowAction::StartEdit(id) => {
                ctx.select(Selection::Node(id));
                let mut st = shared.borrow_mut();
                st.clicked_here = true;
                st.editing = Some(id);
                st.focus_edit = true;
            }
            RowAction::SetText { id, speaker, text } => {
                let command = orbit_project::Command::SetSay {
                    id,
                    speaker,
                    text,
                    expected_version: ctx.node_version(id),
                };
                if let Err(e) = ctx.exec_merged(command, &format!("say-text:{id}")) {
                    ctx.error(e.to_string());
                }
            }
            RowAction::EndEdit => {
                shared.borrow_mut().editing = None;
                ctx.end_merge();
            }
            RowAction::EnterNew(after) => {
                ctx.end_merge();
                let Some((si, ni)) = ctx.story().and_then(|s| s.locate(after)) else {
                    continue;
                };
                let (scene, speaker) = {
                    let story = ctx.story();
                    let story = story.map(|s| (s.scenes[si].id, &s.scenes[si].nodes[ni].kind));
                    match story {
                        Some((sc, NodeKind::Say { speaker, .. })) => (sc, *speaker),
                        Some((sc, _)) => (sc, None),
                        None => continue,
                    }
                };
                let draft = ops::NodeDraft::say(speaker, "");
                let new_id = draft.node.id;
                let label = ctx.t("scene.add_line");
                if ctx
                    .ops_or_toast(|p| ops::insert_drafts(p, scene, ni + 1, vec![draft], &label))
                    .is_some()
                {
                    ctx.select(Selection::Node(new_id));
                    let mut st = shared.borrow_mut();
                    st.editing = Some(new_id);
                    st.focus_edit = true;
                    st.scroll_to = Some(new_id);
                    st.clicked_here = true;
                }
            }
            RowAction::BackspaceEmpty(id) => {
                ctx.end_merge();
                let previous = ctx.story().and_then(|s| {
                    let (si, ni) = s.locate(id)?;
                    ni.checked_sub(1)
                        .map(|p| &s.scenes[si].nodes[p])
                        .map(|n| (n.id, matches!(n.kind, NodeKind::Say { .. })))
                });
                if ctx.ops_or_toast(|p| ops::delete_nodes(p, &[id])).is_some() {
                    let mut st = shared.borrow_mut();
                    match previous {
                        Some((prev, is_say)) => {
                            st.editing = is_say.then_some(prev);
                            st.focus_edit = is_say;
                            st.scroll_to = Some(prev);
                            drop(st);
                            ctx.select(Selection::Node(prev));
                        }
                        None => {
                            st.editing = None;
                            drop(st);
                            ctx.clear_selection();
                        }
                    }
                }
            }
            RowAction::DropAsset(id, asset) => {
                let kind = ctx.story().and_then(|s| match s.node(id).map(|n| &n.kind) {
                    Some(NodeKind::Background { transition, .. }) => Some(NodeKind::Background {
                        asset,
                        transition: transition.clone(),
                    }),
                    Some(NodeKind::PlayMusic {
                        fade_in, looping, ..
                    }) => Some(NodeKind::PlayMusic {
                        asset,
                        fade_in: *fade_in,
                        looping: *looping,
                    }),
                    Some(NodeKind::PlaySound { .. }) => Some(NodeKind::PlaySound { asset }),
                    _ => None,
                });
                if let Some(kind) = kind {
                    ctx.select(Selection::Node(id));
                    ctx.exec_or_toast(orbit_project::Command::SetNodeKind {
                        id,
                        kind,
                        expected_version: ctx.node_version(id),
                    });
                }
            }
            RowAction::SetSpeaker { id, speaker } => {
                let text = ctx
                    .story()
                    .and_then(|s| match s.node(id).map(|n| &n.kind) {
                        Some(NodeKind::Say { text_id, .. }) => s.text(*text_id).map(str::to_owned),
                        _ => None,
                    })
                    .unwrap_or_default();
                ctx.exec_or_toast(orbit_project::Command::SetSay {
                    id,
                    speaker,
                    text,
                    expected_version: ctx.node_version(id),
                });
            }
            RowAction::GoToScene(scene, label) => {
                let node = ctx.story().and_then(|s| s.scene_entry(scene, label));
                // The explorer scrolls to it and the inspector follows.
                super::reveal::reveal(
                    ctx,
                    match node {
                        Some(node) => orbit_editor_api::Location::Node(node),
                        None => orbit_editor_api::Location::Scene(scene),
                    },
                );
            }
            RowAction::PlayFrom(id) => {
                ctx.select(Selection::Node(id));
                ctx.play_from(id);
                ctx.request(EditorRequest::ShowPanel(super::panels::PREVIEW.to_owned()));
            }
            RowAction::DragStart(id) => {
                let sel = ctx.selected_nodes();
                let ids = if sel.contains(&id) { sel } else { vec![id] };
                shared.borrow_mut().drag = Some(ids);
            }
            RowAction::AddAtEnd => {
                ctx.state_mut().current_scene = Some(scene_id);
                ctx.clear_selection();
                edit::add_say(ctx);
            }
            RowAction::Insert(key) => {
                edit::insert_kind(ctx, &key);
            }
            RowAction::Duplicate => edit::duplicate_selection(ctx),
            RowAction::Copy => edit::copy_selection(ctx, false),
            RowAction::Cut => edit::copy_selection(ctx, true),
            RowAction::Paste => edit::paste(ctx),
            RowAction::Delete => edit::delete_selection(ctx),
            RowAction::Move(d) => edit::nudge(ctx, d),
            RowAction::Undo => {
                if let Err(e) = ctx.undo() {
                    ctx.error(e.to_string());
                }
            }
            RowAction::Redo => {
                if let Err(e) = ctx.redo() {
                    ctx.error(e.to_string());
                }
            }
        }
    }
}

// ----------------------------------------------------------------- quick insert

fn quick_insert(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &Shared) {
    let Some((mut query, mut highlighted)) = shared.borrow().quick.clone() else {
        return;
    };
    let items = all_insertables(ctx.registry());
    let needle = query.to_lowercase();
    let texts = ctx.translator();
    let matches: Vec<&super::drafts::Insertable> = items
        .iter()
        .filter(|i| {
            needle.is_empty()
                || i.label_text(&texts).to_lowercase().contains(&needle)
                || i.key.to_lowercase().contains(&needle)
        })
        .collect();
    highlighted = highlighted.min(matches.len().saturating_sub(1));
    let (up, down, enter, escape) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    if up {
        highlighted = highlighted.saturating_sub(1);
    }
    if down && highlighted + 1 < matches.len() {
        highlighted += 1;
    }
    let mut chosen: Option<String> = None;
    if enter && let Some(m) = matches.get(highlighted) {
        chosen = Some(m.key.clone());
    }
    let mut close = escape;
    let panel_rect = ui.max_rect();
    egui::Area::new(Id::new("scene_quick_insert"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(
            panel_rect.center().x - 170.0,
            panel_rect.top() + 60.0,
        ))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(340.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .id(Id::new("scene_quick_query"))
                            .hint_text(ui.t("scene.quick_hint"))
                            .desired_width(f32::INFINITY),
                    );
                    resp.request_focus();
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for (n, item) in matches.iter().enumerate() {
                                let label = format!("{}  {}", item.icon, item.label_text(&texts));
                                let r = ui.selectable_label(n == highlighted, label);
                                if r.clicked() {
                                    chosen = Some(item.key.clone());
                                }
                                if n == highlighted {
                                    r.scroll_to_me(None);
                                }
                            }
                            if matches.is_empty() {
                                ui.label(RichText::new(ui.t("scene.quick_none")).color(weak(ui)));
                            }
                        });
                });
        });
    if ui.input(|i| i.pointer.any_pressed())
        && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        && !Rect::from_min_size(
            Pos2::new(panel_rect.center().x - 170.0, panel_rect.top() + 60.0),
            vec2(360.0, 380.0),
        )
        .contains(pos)
    {
        close = true;
    }
    if let Some(key) = chosen {
        shared.borrow_mut().quick = None;
        edit::insert_kind(ctx, &key);
    } else if close {
        shared.borrow_mut().quick = None;
    } else {
        shared.borrow_mut().quick = Some((query, highlighted));
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{Node, NodeId, NodeKind};

    use super::*;

    fn nodes(n: usize) -> Vec<Node> {
        (0..n)
            .map(|_| Node {
                id: NodeId::new(),
                kind: NodeKind::End,
            })
            .collect()
    }

    fn ids(ns: &[Node]) -> Vec<NodeId> {
        ns.iter().map(|n| n.id).collect()
    }

    #[test]
    fn row_titles_follow_the_language() {
        let story = Story::new("t");
        let node = Node {
            id: NodeId::new(),
            kind: NodeKind::End,
        };
        let en = crate::i18n::translator(orbit_editor_api::Lang::En);
        let fr = crate::i18n::translator(orbit_editor_api::Lang::Fr);
        assert_eq!(describe(&en, &story, &node).0, "End of story");
        assert_eq!(describe(&fr, &story, &node).0, "Fin de l'histoire");
    }

    #[test]
    fn plain_click_selects_one_node() {
        let ns = nodes(4);
        let (sel, anchor) = click_selection(&ids(&ns), &[ns[0].id], None, ns[2].id, false, false);
        assert_eq!(sel, [ns[2].id]);
        assert_eq!(anchor, ns[2].id);
    }

    #[test]
    fn ctrl_click_toggles_and_keeps_reading_order() {
        let ns = nodes(4);
        let (sel, _) = click_selection(&ids(&ns), &[ns[3].id], None, ns[1].id, true, false);
        assert_eq!(sel, [ns[1].id, ns[3].id]);
        let (sel, _) = click_selection(&ids(&ns), &sel, None, ns[3].id, true, false);
        assert_eq!(sel, [ns[1].id]);
    }

    #[test]
    fn shift_click_selects_the_range_from_the_anchor() {
        let ns = nodes(6);
        let (sel, anchor) = click_selection(
            &ids(&ns),
            &[ns[4].id],
            Some(ns[4].id),
            ns[1].id,
            false,
            true,
        );
        assert_eq!(sel, [ns[1].id, ns[2].id, ns[3].id, ns[4].id]);
        assert_eq!(anchor, ns[4].id);
    }

    #[test]
    fn layout_finds_visible_rows_and_boundaries() {
        let ns = nodes(100);
        let layout = Layout::build((0, SceneId::new(), None, 100), &ns);
        assert!((layout.total() - 100.0 * 46.0).abs() < 0.01);
        let r = layout.visible(460.0, 920.0);
        assert!(r.start <= 10 && r.end >= 20 && r.end <= 21, "{r:?}");
        assert_eq!(layout.boundary(0.0), 0);
        assert_eq!(layout.boundary(46.0 * 3.0 + 5.0), 3);
        assert_eq!(layout.boundary(46.0 * 3.0 + 40.0), 4);
        assert_eq!(layout.boundary(1.0e9), 100);
    }
}
