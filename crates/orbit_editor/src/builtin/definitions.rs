//! The story's definitions: the characters panel (names, colors, expressions
//! with previews, default position, custom stage positions) and the
//! variables panel (declaration, type, initial value, scope, renames).
//! Renames update their usages in one undo step, or are refused with a
//! message.

use std::cell::Cell;
use std::rc::Rc;

use egui::{Color32, CornerRadius, DragValue, Frame, Margin, RichText, Ui};
use orbit_core::{
    AssetKind, CharacterId, Color, Expression, NodeKind, Position, PositionDef, Value, ValueType,
    VarDecl, VarScope,
};
use orbit_editor_api::fields::{self, asset_picker, position_picker, to_color32, type_name};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, PanelDef, Plugin, PluginManifest,
    UiText, icons,
};
use orbit_project::{Command, ops};

use super::anim_edit;
use super::edit;
use super::ids;

use super::ui_util::{accent, card, danger, faint, primary_button, section_title, strong, weak};

/// Colors offered for characters.
const PALETTE: [(u8, u8, u8); 10] = [
    (232, 120, 160),
    (90, 150, 230),
    (96, 190, 140),
    (236, 168, 70),
    (176, 124, 236),
    (86, 200, 210),
    (230, 100, 90),
    (170, 190, 80),
    (150, 156, 170),
    (240, 214, 120),
];

fn empty_project(ui: &mut Ui) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(ui.t("defs.no_project")).color(weak(ui)));
    });
}

// ----------------------------------------------------------------- characters

pub struct CharactersPlugin {
    thumbs: crate::rig_host::SharedThumbs,
}

impl CharactersPlugin {
    pub fn new(thumbs: crate::rig_host::SharedThumbs) -> Self {
        Self { thumbs }
    }
}

impl Plugin for CharactersPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::CHARACTERS, "Personnages", "0.1.0")
    }
}

#[derive(Default)]
struct CharactersUi {
    selected: Cell<Option<CharacterId>>,
    rigs: super::rig_expr::RigUi,
}

impl EditorPlugin for CharactersPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        let state = Rc::new(CharactersUi {
            selected: Cell::new(None),
            rigs: super::rig_expr::RigUi::new(self.thumbs.clone()),
        });
        reg.panel(PanelDef::new(
            "characters",
            "defs.characters.title",
            icons::USERS_THREE,
            DockSlot::Right,
            move |ui, ctx| characters_panel(ui, ctx, &state),
        ));
    }
}

fn characters_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, state: &CharactersUi) {
    if !ctx.has_project() {
        empty_project(ui);
        return;
    }
    // A problem or a search hit points at a character: select it.
    if let Some(orbit_editor_api::Location::Character(id)) = ctx.state().pending_reveal.clone() {
        ctx.state_mut().pending_reveal = None;
        state.selected.set(Some(id));
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(strong(ui.t("defs.characters.title"), 16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, &format!("{}  {}", icons::PLUS, ui.t("defs.new")))
                        .clicked()
                    {
                        add_character(ctx, state);
                    }
                });
            });
            ui.add_space(6.0);

            let Some(story) = ctx.story() else { return };
            if story.characters.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(icons::USERS_THREE)
                            .size(30.0)
                            .color(faint(ui)),
                    );
                    ui.add_space(6.0);
                    ui.label(RichText::new(ui.t("defs.characters.empty")).color(weak(ui)));
                });
            }
            // The character chips.
            let chips: Vec<(CharacterId, String, Color32)> = story
                .characters
                .iter()
                .map(|c| {
                    (
                        c.id,
                        story.text(c.name_id).unwrap_or("?").to_owned(),
                        to_color32(c.color),
                    )
                })
                .collect();
            let mut selected = state
                .selected
                .get()
                .filter(|s| chips.iter().any(|c| c.0 == *s))
                .or_else(|| chips.first().map(|c| c.0));
            ui.horizontal_wrapped(|ui| {
                for (id, name, color) in &chips {
                    let is = selected == Some(*id);
                    let text = RichText::new(format!("●  {name}")).color(*color);
                    if ui.selectable_label(is, text).clicked() {
                        selected = Some(*id);
                    }
                }
            });
            state.selected.set(selected);
            if let Some(id) = selected {
                ui.add_space(8.0);
                character_details(ui, ctx, state, id);
            }
            ui.add_space(16.0);
            positions_section(ui, ctx);
            ui.add_space(12.0);
        });
}

fn add_character(ctx: &mut EditorContext<'_>, state: &CharactersUi) {
    let Some(story) = ctx.story() else { return };
    let n = story.characters.len();
    let default_name = ctx.t("defs.character.default_name");
    let name = ops::unique_name(
        &default_name,
        story
            .characters
            .iter()
            .filter_map(|c| story.text(c.name_id)),
    );
    let (r, g, b) = PALETTE[n % PALETTE.len()];
    if let Some(id) = ctx.ops_or_toast(|p| ops::add_character(p, &name, Color::rgb(r, g, b))) {
        state.selected.set(Some(id));
    }
}

/// Number of nodes using a character.
fn usage(ctx: &EditorContext<'_>, id: CharacterId) -> (usize, usize) {
    let Some(story) = ctx.story() else {
        return (0, 0);
    };
    let mut says = 0;
    let mut stage = 0;
    for n in story.scenes.iter().flat_map(|s| &s.nodes) {
        match &n.kind {
            NodeKind::Say { speaker, .. } if *speaker == Some(id) => says += 1,
            NodeKind::Show { character, .. } | NodeKind::Hide { character, .. }
                if *character == id =>
            {
                stage += 1;
            }
            _ => {}
        }
    }
    (says, stage)
}

fn character_details(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    state: &CharactersUi,
    id: CharacterId,
) {
    let Some(story) = ctx.story() else { return };
    let Some(def) = story.character(id).cloned() else {
        return;
    };
    let name = story.text(def.name_id).unwrap_or("").to_owned();
    let (says, stage) = usage(ctx, id);

    // Edits made by the widgets, applied once the story borrow is over.
    let mut pending: Vec<(Command, Option<String>)> = Vec::new();
    let mut done = false;
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        // Name and color.
        section_title(ui, &ui.t("defs.identity"));
        if let Some(n) = fields::text_field(
            ui,
            ("char_name", id),
            &name,
            &ui.t("defs.display_name"),
            1,
            &mut done,
        ) {
            pending.push((
                Command::SetString {
                    id: def.name_id,
                    text: Some(n),
                },
                Some(format!("char-name:{id}")),
            ));
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(ui.t("defs.color")).small().color(weak(ui)));
            if let Some(c) = fields::color_field(ui, def.color) {
                let mut new = def.clone();
                new.color = c;
                pending.push((
                    Command::SetCharacter {
                        id: new.id,
                        character: Some(new),
                        index: None,
                    },
                    Some(format!("char-color:{id}")),
                ));
            }
            for (r, g, b) in PALETTE {
                let swatch = Color32::from_rgb(r, g, b);
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(9), swatch);
                if def.color == Color::rgb(r, g, b) {
                    ui.painter().circle_stroke(
                        rect.center(),
                        10.0,
                        egui::Stroke::new(1.5, ui.visuals().text_color()),
                    );
                }
                if resp.clicked() {
                    let mut new = def.clone();
                    new.color = Color::rgb(r, g, b);
                    pending.push((
                        Command::SetCharacter {
                            id: new.id,
                            character: Some(new),
                            index: None,
                        },
                        None,
                    ));
                }
            }
        });

        // Default position.
        ui.add_space(8.0);
        ui.label(
            RichText::new(ui.t("defs.default_position"))
                .small()
                .color(weak(ui)),
        );
        let mut change: Option<Option<Position>> = None;
        ui.horizontal(|ui| {
            let has = def.default_position.is_some();
            if let Some(on) = fields::checkbox(ui, has, "") {
                change = Some(on.then_some(Position::Center));
            }
            if let Some(p) = &def.default_position
                && let Some(new) = position_picker(ui, ("char_pos", id), story, p)
            {
                change = Some(Some(new));
            }
        });
        if let Some(p) = change {
            let mut new = def.clone();
            new.default_position = p;
            pending.push((
                Command::SetCharacter {
                    id: new.id,
                    character: Some(new),
                    index: None,
                },
                None,
            ));
        }

        // Default scale: what a `Show` uses unless it sets its own.
        ui.add_space(8.0);
        ui.label(
            RichText::new(ui.t("defs.default_scale"))
                .small()
                .color(weak(ui)),
        );
        ui.horizontal(|ui| {
            let reset = ui
                .add_enabled(
                    def.default_scale != 1.0,
                    egui::Button::new(icons::ARROW_COUNTER_CLOCKWISE).small(),
                )
                .on_hover_text(ui.t("defs.reset_scale"))
                .clicked();
            let new_scale = if reset {
                Some(1.0)
            } else {
                fields::scale_slider(ui, def.default_scale)
            };
            if let Some(v) = new_scale {
                let mut new = def.clone();
                new.default_scale = v;
                pending.push((
                    Command::SetCharacter {
                        id: new.id,
                        character: Some(new),
                        index: None,
                    },
                    (!reset).then(|| format!("char-scale:{id}")),
                ));
            }
        });
    });
    for (command, key) in pending {
        let result = match key {
            Some(k) => ctx.exec_merged(command, &k),
            None => ctx.exec(command),
        };
        if let Err(e) = result {
            ctx.error(e.to_string());
        }
    }
    if done {
        ctx.end_merge();
    }
    ui.add_space(8.0);
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            section_title(ui, &ui.t("defs.expressions"));
        });
        expressions(ui, ctx, &def, &state.rigs);
    });

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let says_text = ui.tn("defs.says", says, &[]);
        let stage_text = ui.tn("defs.stages", stage, &[]);
        ui.label(
            RichText::new(ui.tf(
                "defs.usage",
                &[("says", &says_text), ("stage", &stage_text)],
            ))
            .small()
            .color(weak(ui)),
        );
    });
    ui.add_space(4.0);
    let delete = egui::Button::new(
        RichText::new(format!(
            "{}  {}",
            icons::TRASH,
            ui.t("defs.delete_character")
        ))
        .color(danger(ui)),
    );
    if ui.add(delete).clicked() {
        match ctx.ops(|p| ops::delete_character(p, id)) {
            Ok(()) => state.selected.set(None),
            Err(_) => {
                let says_text = ctx.tn("defs.says", says, &[]);
                let stage_text = ctx.tn("defs.stages", stage, &[]);
                let message = ctx.tf(
                    "defs.cannot_delete",
                    &[
                        ("name", &name),
                        ("says", &says_text),
                        ("stage", &stage_text),
                    ],
                );
                ctx.error(message);
            }
        }
    }
}

fn expressions(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    def: &orbit_core::Character,
    rigs: &super::rig_expr::RigUi,
) {
    let Some(story) = ctx.story() else { return };
    enum Do {
        Rename(String, String),
        Asset(usize, orbit_core::AssetId),
        Remove(usize),
        Add,
        Import(usize),
        Replace(usize, Expression, Option<String>),
        ImportFrame(usize),
        ImportAnimation(usize),
    }
    let mut action: Option<Do> = None;
    if def.expressions.is_empty() {
        ui.label(
            RichText::new(ui.t("defs.no_expressions"))
                .small()
                .color(weak(ui)),
        );
    }
    for (i, e) in def.expressions.iter().enumerate() {
        Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    anim_edit::preview(ui, ctx, story, rigs, (def.id, i), e, egui::vec2(54.0, 68.0));
                    ui.vertical(|ui| {
                        // The name is committed on Enter / focus loss, so the
                        // references follow in one step.
                        let buf_id = ui.make_persistent_id(("expr_name", def.id, &e.name));
                        let mut buffer = ui
                            .data(|d| d.get_temp::<String>(buf_id))
                            .unwrap_or_else(|| e.name.clone());
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut buffer)
                                .id(buf_id.with("edit"))
                                .desired_width(f32::INFINITY),
                        );
                        if resp.lost_focus() {
                            if buffer != e.name {
                                action = Some(Do::Rename(e.name.clone(), buffer.clone()));
                            }
                            ui.data_mut(|d| d.remove::<String>(buf_id));
                        } else if buffer != e.name || resp.has_focus() {
                            ui.data_mut(|d| d.insert_temp(buf_id, buffer));
                        }
                        let root = ctx.project_info().map(|p| p.root.clone()).unwrap_or_default();
                        match anim_edit::editor(ui, story, &root, rigs, (def.id, i), e) {
                            Some(anim_edit::Outcome::Replace(new, key)) => {
                                action = Some(Do::Replace(i, new, key));
                            }
                            Some(anim_edit::Outcome::ImportFrame) => {
                                action = Some(Do::ImportFrame(i));
                            }
                            Some(anim_edit::Outcome::ImportAnimation) => {
                                action = Some(Do::ImportAnimation(i));
                            }
                            None => {}
                        }
                        if matches!(anim_edit::kind_of(e), anim_edit::Kind::Still | anim_edit::Kind::Rig) {
                            if let Some(a) = asset_picker(
                                ui,
                                ("expr_asset", def.id, i),
                                story,
                                AssetKind::Image,
                                e.image,
                            ) {
                                action = Some(Do::Asset(i, a));
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui
                                .small_button(format!(
                                    "{}  {}",
                                    icons::FILE_PLUS,
                                    ui.t("defs.file")
                                ))
                                .clicked()
                            {
                                action = Some(Do::Import(i));
                            }
                            let used = ops::expression_in_use(story, def.id, &e.name);
                            let remove = ui
                                .add_enabled(
                                    !used,
                                    egui::Button::new(
                                        RichText::new(format!(
                                            "{}  {}",
                                            icons::TRASH,
                                            ui.t("common.delete")
                                        ))
                                        .color(danger(ui)),
                                    )
                                    .small(),
                                )
                                .on_disabled_hover_text(ui.t("defs.expression_in_use"));
                            if remove.clicked() {
                                action = Some(Do::Remove(i));
                            }
                        });
                    });
                });
            });
        ui.add_space(4.0);
    }
    if ui
        .button(format!("{}  {}", icons::PLUS, ui.t("defs.add_expression")))
        .clicked()
    {
        action = Some(Do::Add);
    }

    let Some(action) = action else { return };
    let mut new = def.clone();
    match action {
        Do::Rename(old, new_name) => {
            if let Err(e) = ctx.ops(|p| ops::rename_expression(p, def.id, &old, &new_name)) {
                ctx.error(e.to_string());
            }
            return;
        }
        Do::Asset(i, a) => new.expressions[i].image = a,
        Do::Replace(i, expression, key) => {
            new.expressions[i] = expression;
            let command = Command::SetCharacter {
                id: new.id,
                character: Some(new),
                index: None,
            };
            let result = match key {
                Some(k) => ctx.exec_merged(command, &k),
                None => ctx.exec(command),
            };
            if let Err(e) = result {
                ctx.error(e.to_string());
            }
            return;
        }
        Do::ImportFrame(i) => {
            let Some(a) = edit::import_asset_dialog(ctx, AssetKind::Image) else {
                return;
            };
            anim_edit::push_frame(&mut new.expressions[i], a);
        }
        Do::ImportAnimation(i) => {
            let Some((frames, fps)) = edit::import_animation_dialog(ctx) else {
                return;
            };
            anim_edit::set_sequence(&mut new.expressions[i], frames, fps);
        }
        Do::Remove(i) => {
            new.expressions.remove(i);
        }
        Do::Import(i) => {
            let Some(a) = edit::import_asset_dialog(ctx, AssetKind::Image) else {
                return;
            };
            new.expressions[i].image = a;
        }
        Do::Add => {
            let image = ctx.story().and_then(|s| {
                s.assets
                    .assets
                    .iter()
                    .find(|a| a.kind == AssetKind::Image)
                    .map(|a| a.id)
            });
            let image = match image {
                Some(i) => i,
                None => {
                    let message = ctx.t("defs.no_image");
                    ctx.info(message);
                    let Some(a) = edit::import_asset_dialog(ctx, AssetKind::Image) else {
                        return;
                    };
                    a
                }
            };
            let name = ops::unique_name(
                if def.expressions.is_empty() {
                    "neutral"
                } else {
                    "expression"
                },
                def.expressions.iter().map(|e| e.name.as_str()),
            );
            new.expressions.push(Expression::still(name, image));
        }
    }
    ctx.exec_or_toast(Command::SetCharacter {
        id: new.id,
        character: Some(new),
        index: None,
    });
}

fn positions_section(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let Some(story) = ctx.story() else { return };
    let positions = story.positions.clone();
    section_title(ui, &ui.t("defs.positions.title"));
    ui.label(
        RichText::new(ui.t("defs.positions.hint"))
            .small()
            .color(weak(ui)),
    );
    ui.add_space(4.0);
    let mut new = positions.clone();
    let mut changed = false;
    let mut remove = None;
    for (i, p) in positions.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut name = p.name.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut name).desired_width(110.0))
                .changed()
                && !name.trim().is_empty()
                && !["left", "center", "right"].contains(&name.as_str())
                && !positions.iter().any(|q| q.name == name)
            {
                new[i].name = name;
                changed = true;
            }
            ui.label(RichText::new("x").small().color(weak(ui)));
            if ui
                .add(
                    DragValue::new(&mut new[i].x)
                        .speed(0.005)
                        .range(0.0..=1.0)
                        .max_decimals(3),
                )
                .changed()
            {
                changed = true;
            }
            ui.label(RichText::new("y").small().color(weak(ui)));
            if ui
                .add(
                    DragValue::new(&mut new[i].y)
                        .speed(0.005)
                        .range(0.0..=1.0)
                        .max_decimals(3),
                )
                .changed()
            {
                changed = true;
            }
            if ui.small_button(icons::TRASH).clicked() {
                remove = Some(i);
            }
        });
    }
    if ui
        .button(format!("{}  {}", icons::PLUS, ui.t("defs.add_position")))
        .clicked()
    {
        let name = ops::unique_name("position", positions.iter().map(|p| p.name.as_str()));
        new.push(PositionDef {
            name,
            x: 0.5,
            y: 1.0,
        });
        changed = true;
    }
    if let Some(i) = remove {
        let name = positions[i].name.clone();
        let used = story.scenes.iter().flat_map(|s| &s.nodes).any(|n| {
            matches!(&n.kind, NodeKind::Show { position: Position::Custom(c), .. } if *c == name)
        });
        if used {
            let message = ctx.tf("defs.position_in_use", &[("name", &name)]);
            ctx.error(message);
        } else {
            new.remove(i);
            changed = true;
        }
    }
    if changed {
        let _ = ctx.exec_merged(Command::SetPositions { positions: new }, "positions");
    }
}

// ------------------------------------------------------------------ variables

pub struct VariablesPlugin;

impl Plugin for VariablesPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::VARIABLES, "Variables", "0.1.0")
    }
}

impl EditorPlugin for VariablesPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "variables",
            "defs.variables.title",
            icons::FUNCTION,
            DockSlot::Right,
            variables_panel,
        ));
    }
}

/// Text key of the name of a scope.
fn scope_key(scope: VarScope) -> &'static str {
    match scope {
        VarScope::Game => "defs.scope.game",
        VarScope::Persistent => "defs.scope.persistent",
    }
}

fn variables_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    if !ctx.has_project() {
        empty_project(ui);
        return;
    }
    // A problem or a search hit points at a variable: highlight its row.
    let reveal_id = egui::Id::new("orbit_variable_reveal");
    if let Some(orbit_editor_api::Location::Variable(name)) = ctx.state().pending_reveal.clone() {
        let serial = ctx.state().reveal_serial;
        ctx.state_mut().pending_reveal = None;
        ui.data_mut(|d| d.insert_temp(reveal_id, (name, serial)));
    }
    let revealed: Option<String> = ui
        .data(|d| d.get_temp::<(String, u64)>(reveal_id))
        .filter(|(_, serial)| *serial == ctx.state().reveal_serial)
        .map(|(name, _)| name);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(strong(ui.t("defs.variables.title"), 16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, &format!("{}  {}", icons::PLUS, ui.t("defs.new_f")))
                        .clicked()
                    {
                        let Some(story) = ctx.story() else { return };
                        let name = ops::unique_name(
                            "variable",
                            story.variables.iter().map(|v| v.name.as_str()),
                        );
                        ctx.ops_or_toast(|p| {
                            ops::add_variable(p, VarDecl::game(name, Value::Bool(false)))
                        });
                    }
                });
            });
            ui.label(
                RichText::new(ui.t("defs.variables.scope_hint"))
                    .small()
                    .color(weak(ui)),
            );
            ui.add_space(8.0);
            let Some(story) = ctx.story() else { return };
            if story.variables.is_empty() {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(icons::FUNCTION).size(30.0).color(faint(ui)));
                    ui.add_space(6.0);
                    ui.label(RichText::new(ui.t("defs.variables.empty")).color(weak(ui)));
                });
                return;
            }
            let vars = story.variables.clone();
            for decl in &vars {
                let row = ui.scope(|ui| variable_row(ui, ctx, decl)).response;
                if revealed.as_deref() == Some(decl.name.as_str()) {
                    ui.painter().rect_stroke(
                        row.rect.expand(3.0),
                        egui::CornerRadius::same(8),
                        egui::Stroke::new(1.5, accent(ui)),
                        egui::StrokeKind::Outside,
                    );
                    row.scroll_to_me(Some(egui::Align::Center));
                }
                ui.add_space(4.0);
            }
        });
}

fn variable_row(ui: &mut Ui, ctx: &mut EditorContext<'_>, decl: &VarDecl) {
    enum Do {
        Rename(String),
        Type(ValueType),
        Initial(Value),
        Scope(VarScope),
        Delete,
    }
    let uses = ctx
        .story()
        .map_or(0, |s| ops::variable_users(s, &decl.name).len());
    let mut action: Option<Do> = None;
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let buf_id = ui.make_persistent_id(("var_name", &decl.name));
            let mut buffer = ui
                .data(|d| d.get_temp::<String>(buf_id))
                .unwrap_or_else(|| decl.name.clone());
            let resp = ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .id(buf_id.with("edit"))
                    .desired_width(150.0)
                    .font(crate::theme::bold(13.5)),
            );
            if resp.lost_focus() {
                if buffer != decl.name {
                    action = Some(Do::Rename(buffer.clone()));
                }
                ui.data_mut(|d| d.remove::<String>(buf_id));
            } else if buffer != decl.name || resp.has_focus() {
                ui.data_mut(|d| d.insert_temp(buf_id, buffer));
            }
            ui.label(
                RichText::new(if uses == 0 {
                    ui.t("defs.var.unused")
                } else {
                    ui.tn("defs.var.nodes", uses, &[])
                })
                .small()
                .color(faint(ui)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(RichText::new(icons::TRASH).color(danger(ui)))
                    .on_hover_text(ui.t("defs.var.delete"))
                    .clicked()
                {
                    action = Some(Do::Delete);
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let ty = decl.initial.value_type();
            egui::ComboBox::from_id_salt(("var_type", &decl.name))
                .selected_text(type_name(ui, ty))
                .width(130.0)
                .show_ui(ui, |ui| {
                    let scalars = [
                        ValueType::Bool,
                        ValueType::Int,
                        ValueType::Float,
                        ValueType::Str,
                    ];
                    // Scalars, then lists and maps of each scalar (schema v11).
                    let lists = orbit_core::ScalarType::ALL.map(ValueType::List);
                    let maps = orbit_core::ScalarType::ALL.map(ValueType::Map);
                    for (i, group) in [scalars, lists, maps].into_iter().enumerate() {
                        if i > 0 {
                            ui.separator();
                        }
                        for t in group {
                            if ui.selectable_label(t == ty, type_name(ui, t)).clicked() && t != ty
                            {
                                action = Some(Do::Type(t));
                            }
                        }
                    }
                });
            ui.label(
                RichText::new(ui.t("defs.var.initial"))
                    .small()
                    .color(weak(ui)),
            );
            if let Some(v) = fields::value_editor(ui, &decl.initial) {
                action = Some(Do::Initial(v));
            }
        });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(ui.t("defs.var.scope"))
                    .small()
                    .color(weak(ui)),
            );
            egui::ComboBox::from_id_salt(("var_scope", &decl.name))
                .selected_text(ui.t(scope_key(decl.scope)))
                .show_ui(ui, |ui| {
                    for s in [VarScope::Game, VarScope::Persistent] {
                        if ui
                            .selectable_label(decl.scope == s, ui.t(scope_key(s)))
                            .clicked()
                            && decl.scope != s
                        {
                            action = Some(Do::Scope(s));
                        }
                    }
                });
        });
    });
    let Some(action) = action else { return };
    let name = decl.name.clone();
    let result = match action {
        Do::Rename(new) => ctx.ops(|p| ops::rename_variable(p, &name, &new)),
        Do::Type(t) => {
            ctx.ops(|p| ops::update_variable(p, &name, fields::default_value(t), decl.scope))
        }
        Do::Initial(v) => {
            // Live edits of one value are one undo step.
            let mut updated = decl.clone();
            updated.initial = v;
            ctx.exec_merged(
                Command::SetVariable {
                    name: name.clone(),
                    decl: Some(updated),
                    index: None,
                },
                &format!("var-init:{name}"),
            )
        }
        Do::Scope(s) => ctx.ops(|p| ops::update_variable(p, &name, decl.initial.clone(), s)),
        Do::Delete => ctx.ops(|p| ops::delete_variable(p, &name)),
    };
    if let Err(e) = result {
        ctx.error(e.to_string());
    }
}
