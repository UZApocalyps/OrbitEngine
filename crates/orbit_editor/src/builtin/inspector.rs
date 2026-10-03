//! Inspector: edits the selected element. One inspector per node kind is
//! registered through the plugin API (`InspectorDef`); all of the built-in
//! ones are made of the typed field widgets of `orbit_editor_api::fields`
//! and turn each change into a `SetNodeKind` command (merged per field,
//! so typing or dragging a value is one undo step). A plugin node without an
//! inspector of its own gets a generic editor of its data.

use egui::{Frame, Margin, RichText, Ui};
use orbit_core::{
    AssetKind, CharacterId, ChoiceOption, CustomData, Expr, Node, NodeId, NodeKind, Position,
    SceneId, StringId, ValueType,
};
use orbit_editor_api::fields::{
    self, asset_picker, character_picker, expr_editor, expression_picker, field,
    label_picker, position_picker, scene_label_picker, scene_picker, seconds_field,
    transition_editor, value_editor,
};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, InspectorDef, Level, PanelDef, Plugin,
    PluginManifest, Selection, UiText, icons,
};
use orbit_project::{Command, ops};

use super::drafts::builtin_insertables;
use super::edit;
use super::ids;
use super::nodeinfo::{kind_color, kind_icon, kind_key, kind_label};
use super::thumbs;
use super::ui_util::{accent, card, danger, faint, section_title, strong, warning, weak};

pub struct InspectorPlugin;

/// `field` with a translated caption (`key` is a text key).
pub(super) fn tfield<R>(ui: &mut Ui, key: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let label = ui.t(key);
    field(ui, &label, add)
}

impl Plugin for InspectorPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::INSPECTOR, "Inspecteur", "0.2.0")
    }
}

impl EditorPlugin for InspectorPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "inspector",
            "inspector.title",
            icons::SLIDERS_HORIZONTAL,
            DockSlot::Right,
            panel,
        ));
        // One inspector per built-in kind, through the same registry a
        // plugin uses for its own node types.
        for item in builtin_insertables() {
            reg.inspector(InspectorDef::new(item.key, node_inspector));
        }
    }
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            match ctx.selection() {
                None => empty_state(ui, "inspector.empty_select"),
                Some(Selection::Scene(id)) => scene_inspector(ui, ctx, id),
                Some(Selection::Node(id)) => {
                    if ctx.selected_nodes().len() > 1 {
                        multi_selection(ui, ctx);
                    } else {
                        node_panel(ui, ctx, id);
                    }
                }
            }
            ui.add_space(12.0);
        });
}

/// A centered hint; `key` is a text key.
fn empty_state(ui: &mut Ui, key: &str) {
    let text = ui.t(key);
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(icons::CURSOR_CLICK)
                .size(28.0)
                .color(faint(ui)),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(&text).color(weak(ui)));
    });
}

fn multi_selection(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let count = ctx.selected_nodes().len();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(icons::STACK_SIMPLE)
                .size(18.0)
                .color(accent(ui)),
        );
        ui.label(strong(
            ui.tf("inspector.multi.title", &[("count", &count)]),
            16.0,
        ));
    });
    ui.add_space(8.0);
    ui.label(
        RichText::new(ui.t("inspector.multi.hint"))
            .small()
            .color(weak(ui)),
    );
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(format!(
                "{}  {}",
                icons::COPY_SIMPLE,
                ui.t("common.duplicate")
            ))
            .clicked()
        {
            edit::duplicate_selection(ctx);
        }
        if ui
            .button(format!("{}  {}", icons::COPY, ui.t("common.copy")))
            .clicked()
        {
            edit::copy_selection(ctx, false);
        }
        if ui
            .button(format!("{}  {}", icons::SCISSORS, ui.t("common.cut")))
            .clicked()
        {
            edit::copy_selection(ctx, true);
        }
    });
    ui.add_space(8.0);
    let delete = egui::Button::new(
        RichText::new(format!(
            "{}  {}",
            icons::TRASH,
            ui.t("inspector.multi.delete")
        ))
        .color(danger(ui)),
    );
    if ui.add(delete).clicked() {
        edit::delete_selection(ctx);
    }
}

fn scene_inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: SceneId) {
    let Some(scene) = ctx.story().and_then(|s| s.scene(id)) else {
        return;
    };
    let (name, count) = (scene.name.clone(), scene.nodes.len());
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(icons::FOLDER_OPEN)
                .size(18.0)
                .color(accent(ui)),
        );
        ui.label(strong(ui.t("inspector.scene.title"), 16.0));
    });
    ui.add_space(4.0);
    let mut done = false;
    if let Some(new) = tfield(ui, "common.name", |ui| {
        let hint = ui.t("inspector.scene.name_hint");
        fields::text_field(ui, ("scene_name", id), &name, &hint, 1, &mut done)
    }) && !new.is_empty()
    {
        ctx.exec_merged(
            Command::RenameScene { id, name: new },
            &format!("scene-name:{id}"),
        )
        .unwrap_or_else(|e| ctx.error(e.to_string()));
    }
    ui.add_space(6.0);
    section_title(ui, &ui.t("inspector.scene.content"));
    ui.label(ui.tn("inspector.scene.lines", count, &[]));
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui
            .button(format!(
                "{}  {}",
                icons::ARROW_UP,
                ui.t("inspector.move_up")
            ))
            .clicked()
        {
            edit::move_scene(ctx, id, -1);
        }
        if ui
            .button(format!(
                "{}  {}",
                icons::ARROW_DOWN,
                ui.t("inspector.move_down")
            ))
            .clicked()
        {
            edit::move_scene(ctx, id, 1);
        }
    });
    ui.add_space(8.0);
    let delete = egui::Button::new(
        RichText::new(format!(
            "{}  {}",
            icons::TRASH,
            ui.t("inspector.scene.delete")
        ))
        .color(danger(ui)),
    );
    if ui.add(delete).clicked() {
        edit::delete_scene(ctx, id);
    }
}

fn node_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId) {
    let Some(node) = ctx.story().and_then(|s| s.node(id)).cloned() else {
        empty_state(ui, "inspector.gone");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(kind_icon(&node.kind))
                .size(18.0)
                .color(kind_color(&node.kind)),
        );
        ui.label(strong(kind_label(&ui.texts(), &node.kind), 16.0));
    });
    ui.label(RichText::new(id.to_string()).small().color(faint(ui)));

    // What validation says about this node.
    let findings: Vec<(Level, String)> = ctx.state().diagnostics.of(id).to_vec();
    if !findings.is_empty() {
        ui.add_space(6.0);
        for (level, message) in findings {
            let color = if level == Level::Error {
                danger(ui)
            } else {
                warning(ui)
            };
            Frame::new()
                .fill(color.gamma_multiply(0.12))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(format!(
                            "{}  {message}",
                            if level == Level::Error {
                                icons::WARNING_CIRCLE
                            } else {
                                icons::WARNING
                            }
                        ))
                        .small()
                        .color(color),
                    );
                });
        }
    }
    ui.add_space(6.0);

    // Plugin node whose plugin is absent: kept as is, read-only.
    let mut read_only = false;
    if let NodeKind::Custom { plugin_id, .. } = &node.kind
        && !ctx.registry().project.has_plugin(plugin_id)
    {
        read_only = true;
        let color = warning(ui);
        Frame::new()
            .fill(color.gamma_multiply(0.12))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        icons::LOCK_SIMPLE,
                        ui.tf("inspector.plugin_missing", &[("plugin", plugin_id)])
                    ))
                    .color(color),
                );
            });
        ui.add_space(8.0);
    }

    let key = kind_key(&node.kind);
    ui.add_enabled_ui(!read_only, |ui| match ctx.registry().inspector_for(&key) {
        Some(inspector) => (inspector.ui)(ui, ctx, id),
        None => generic_inspector(ui, ctx, id),
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .button(format!(
                "{}  {}",
                icons::COPY_SIMPLE,
                ui.t("common.duplicate")
            ))
            .clicked()
        {
            edit::duplicate_selection(ctx);
        }
        let delete = egui::Button::new(
            RichText::new(format!("{}  {}", icons::TRASH, ui.t("common.delete"))).color(danger(ui)),
        );
        if ui.add(delete).clicked() {
            edit::delete_selection(ctx);
        }
    });
}

// -------------------------------------------------------------------- commit

/// Replaces the kind of `id` with `kind`; consecutive edits of the same
/// `field` are one undo step.
pub(super) fn commit(ctx: &mut EditorContext<'_>, id: NodeId, kind: NodeKind, field: &str) {
    let command = Command::SetNodeKind {
        id,
        kind,
        expected_version: ctx.node_version(id),
    };
    if let Err(e) = ctx.exec_merged(command, &format!("node:{id}:{field}")) {
        ctx.error(e.to_string());
    }
}

fn current_kind(ctx: &EditorContext<'_>, id: NodeId) -> Option<NodeKind> {
    ctx.story()?.node(id).map(|n| n.kind.clone())
}

/// A new `Label` at the end of the scene holding `node`, plus the kind that
/// uses it, as one undo step.
fn with_new_label(
    ctx: &mut EditorContext<'_>,
    node: NodeId,
    build: impl FnOnce(NodeId) -> NodeKind,
) {
    let Some(story) = ctx.story() else { return };
    let Some((si, _)) = story.locate(node) else {
        return;
    };
    let scene = story.scenes[si].id;
    let end = story.scenes[si].nodes.len();
    let taken: Vec<String> = ops::labels(story).into_iter().map(|(_, n)| n).collect();
    let name = ops::unique_name("etiquette", taken.iter().map(String::as_str));
    let label = Node {
        id: NodeId::new(),
        kind: NodeKind::Label { name },
    };
    let label_id = label.id;
    let kind = build(label_id);
    let version = ctx.node_version(node);
    let tx_label = ctx.t("inspector.new_label");
    let result = ctx.transaction(&tx_label, |tx| {
        tx.execute(Command::InsertNode {
            scene,
            index: end,
            node: label,
        })?;
        tx.execute(Command::SetNodeKind {
            id: node,
            kind,
            expected_version: version,
        })
    });
    if let Err(e) = result {
        ctx.error(e.to_string());
    }
}

fn new_label_button(ui: &mut Ui) -> bool {
    ui.small_button(format!("{}  {}", icons::FLAG, ui.t("inspector.new_label")))
        .on_hover_text(ui.t("inspector.new_label.tip"))
        .clicked()
}

// ------------------------------------------------------------------ inspectors

/// The inspector registered for every built-in kind.
fn node_inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId) {
    let Some(kind) = current_kind(ctx, id) else {
        return;
    };
    match kind {
        NodeKind::Say { speaker, text_id } => say(ui, ctx, id, speaker, text_id),
        NodeKind::Label { name } => label(ui, ctx, id, &name),
        NodeKind::Choice { options } => choice(ui, ctx, id, &options),
        NodeKind::Jump { target } => jump(ui, ctx, id, target, true),
        NodeKind::JumpScene { scene, label } => jump_scene(ui, ctx, id, scene, label),
        NodeKind::Call { label } => jump(ui, ctx, id, label, false),
        NodeKind::SetVar { var, value } => set_var(ui, ctx, id, &var, &value),
        NodeKind::If {
            condition,
            then_target,
            else_target,
        } => condition_node(ui, ctx, id, &condition, then_target, else_target),
        NodeKind::Background { asset, transition } => {
            background(ui, ctx, id, asset, transition);
        }
        shown @ NodeKind::Show { .. } => show(ui, ctx, id, &shown),
        NodeKind::Hide {
            character,
            transition,
        } => hide(ui, ctx, id, character, transition),
        NodeKind::PlayMusic {
            asset,
            fade_in,
            looping,
        } => play_music(ui, ctx, id, asset, fade_in, looping),
        NodeKind::StopMusic { fade_out } => {
            tfield(ui, "inspector.fade_out", |ui| {
                if let Some(v) = seconds_field(ui, fade_out) {
                    commit(ctx, id, NodeKind::StopMusic { fade_out: v }, "fade_out");
                }
            });
        }
        NodeKind::PlaySound { asset } => {
            tfield(ui, "inspector.sound", |ui| {
                asset_row(ui, ctx, id, AssetKind::Sound, asset, |a| {
                    NodeKind::PlaySound { asset: a }
                });
            });
        }
        NodeKind::Wait { seconds } => {
            tfield(ui, "inspector.wait_duration", |ui| {
                if let Some(v) = seconds_field(ui, seconds) {
                    commit(ctx, id, NodeKind::Wait { seconds: v }, "seconds");
                }
            });
        }
        NodeKind::Effect(effect) => super::inspector_fx::effect(ui, ctx, id, effect),
        NodeKind::Timeline {
            duration,
            looping,
            wait,
            tracks,
        } => super::inspector_fx::timeline(ui, ctx, id, duration, looping, wait, tracks),
        NodeKind::Return => {
            info(ui, "inspector.return_info");
        }
        NodeKind::End => {
            info(ui, "inspector.end_info");
        }
        NodeKind::Custom { .. } => generic_inspector(ui, ctx, id),
        kind @ (NodeKind::ShowScene3d { .. }
        | NodeKind::Camera3d { .. }
        | NodeKind::HideScene3d { .. }) => scene3d_inspector(ui, ctx, id, kind),
    }
}

/// The inspector of the 3D scene commands (ADR 0014): scene asset, camera
/// name (empty: the first camera of the scene) and transition. Choosing the
/// camera from the scene's list is M6-11 work.
fn scene3d_inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, kind: NodeKind) {
    let (scene, camera, transition) = match &kind {
        NodeKind::ShowScene3d {
            scene,
            camera,
            transition,
        } => (Some(*scene), Some(camera.clone().unwrap_or_default()), transition.clone()),
        NodeKind::Camera3d { camera, transition } => (None, Some(camera.clone()), transition.clone()),
        NodeKind::HideScene3d { transition } => (None, None, transition.clone()),
        _ => return,
    };
    let rebuild = |scene_id: Option<orbit_core::AssetId>,
                   camera: Option<String>,
                   transition: Option<orbit_core::Transition>| match &kind {
        NodeKind::ShowScene3d { scene, .. } => NodeKind::ShowScene3d {
            scene: scene_id.unwrap_or(*scene),
            camera: camera.filter(|c| !c.trim().is_empty()),
            transition,
        },
        NodeKind::Camera3d { .. } => NodeKind::Camera3d {
            camera: camera.unwrap_or_default(),
            transition,
        },
        _ => NodeKind::HideScene3d { transition },
    };
    if let Some(current) = scene {
        tfield(ui, "inspector.scene3d", |ui| {
            asset_row(ui, ctx, id, AssetKind::Scene3d, current, |a| {
                rebuild(Some(a), camera.clone(), transition.clone())
            });
        });
    }
    if let Some(name) = &camera {
        let mut done = false;
        if let Some(new) = tfield(ui, "inspector.camera3d", |ui| {
            let hint = ui.t("inspector.camera3d.hint");
            fields::text_field(ui, ("camera3d", id), name, &hint, 1, &mut done)
        }) {
            commit(ctx, id, rebuild(None, Some(new), transition.clone()), "camera");
        }
        if done {
            ctx.end_merge();
        }
    }
    tfield(ui, "inspector.transition", |ui| {
        if let Some(t) =
            transition_editor(ui, ("scene3d_transition", id), transition.clone(), &curves_of(ctx))
        {
            commit(ctx, id, rebuild(None, camera.clone(), t), "transition");
        }
    });
}

/// A muted paragraph; `key` is a text key (a literal that is not a key is
/// shown as is).
pub(super) fn info(ui: &mut Ui, key: &str) {
    let text = ui.t(key);
    ui.add_space(4.0);
    ui.label(RichText::new(text).color(weak(ui)));
}

fn say(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    speaker: Option<CharacterId>,
    text_id: StringId,
) {
    let Some(story) = ctx.story() else { return };
    let text = story.text(text_id).unwrap_or_default().to_owned();
    let mut change: Option<Option<CharacterId>> = None;
    tfield(ui, "inspector.character", |ui| {
        let narrator = ui.t("node.narrator");
        change = character_picker(ui, ("say_speaker", id), story, speaker, Some(&narrator));
    });
    let mut new_text = None;
    let mut done = false;
    tfield(ui, "inspector.text", |ui| {
        let hint = ui.t("inspector.say.hint");
        new_text = fields::text_field(ui, ("say_text", id), &text, &hint, 5, &mut done);
        ui.label(
            RichText::new(ui.tn("inspector.say.chars", text.chars().count(), &[]))
                .small()
                .color(faint(ui)),
        );
    });
    let (version, key) = (ctx.node_version(id), format!("say-text:{id}"));
    if let Some(s) = change {
        let command = Command::SetSay {
            id,
            speaker: s,
            text: text.clone(),
            expected_version: version,
        };
        ctx.exec_or_toast(command);
    }
    if let Some(t) = new_text {
        let command = Command::SetSay {
            id,
            speaker,
            text: t,
            expected_version: version,
        };
        if let Err(e) = ctx.exec_merged(command, &key) {
            ctx.error(e.to_string());
        }
    }
    if done {
        ctx.end_merge();
    }
}

fn label(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, name: &str) {
    let mut done = false;
    if let Some(new) = tfield(ui, "inspector.label.name", |ui| {
        let hint = ui.t("inspector.label.hint");
        fields::text_field(ui, ("label_name", id), name, &hint, 1, &mut done)
    }) {
        commit(ctx, id, NodeKind::Label { name: new }, "name");
    }
    if done {
        ctx.end_merge();
    }
    let Some(story) = ctx.story() else { return };
    let users = story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter(|n| n.kind.targets().contains(&id))
        .count();
    let users_text = ui.tn("inspector.label.users", users, &[]);
    info(ui, &users_text);
}

fn jump(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, target: NodeId, is_jump: bool) {
    let Some(story) = ctx.story() else { return };
    let mut chosen = None;
    tfield(
        ui,
        if is_jump {
            "inspector.jump.go_to"
        } else {
            "inspector.jump.call"
        },
        |ui| {
            chosen = label_picker(ui, ("jump_target", id), story, Some(target), None);
        },
    );
    let build = move |t: NodeId| {
        if is_jump {
            NodeKind::Jump { target: t }
        } else {
            NodeKind::Call { label: t }
        }
    };
    if let Some(Some(t)) = chosen {
        commit(ctx, id, build(t), "target");
    }
    ui.add_space(4.0);
    if new_label_button(ui) {
        with_new_label(ctx, id, build);
    }
}

fn jump_scene(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    scene: orbit_core::SceneId,
    label: Option<NodeId>,
) {
    let Some(story) = ctx.story() else { return };
    let mut new_scene = None;
    let mut new_label = None;
    tfield(ui, "inspector.jump_scene.target", |ui| {
        new_scene = scene_picker(ui, ("jump_scene", id), story, scene);
    });
    tfield(ui, "inspector.jump_scene.label", |ui| {
        let start = ui.t("inspector.jump_scene.start");
        new_label = scene_label_picker(ui, ("jump_scene_label", id), story, scene, label, &start);
    });
    let target_node = story.scene_entry(scene, label);
    if let Some(s) = new_scene {
        // Another scene: the label of the old one no longer applies.
        commit(
            ctx,
            id,
            NodeKind::JumpScene {
                scene: s,
                label: None,
            },
            "scene",
        );
    } else if let Some(l) = new_label {
        commit(ctx, id, NodeKind::JumpScene { scene, label: l }, "label");
    }
    info(ui, "inspector.jump_scene.info");
    ui.add_space(4.0);
    if let Some(node) = target_node
        && ui
            .small_button(format!(
                "{}  {}",
                icons::ARROW_SQUARE_OUT,
                ui.t("inspector.jump_scene.go")
            ))
            .clicked()
    {
        ctx.select(Selection::Node(node));
    }
}

fn choice(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, options: &[ChoiceOption]) {
    let Some(story) = ctx.story() else { return };
    section_title(ui, &ui.t("inspector.choice.options"));
    // What to do after the loop (the story is borrowed while drawing).
    enum Do {
        Text(StringId, String),
        Target(usize, NodeId),
        NewLabel(usize),
        Condition(usize, Option<Expr>),
        Remove(usize),
        Move(usize, i32),
        Add,
    }
    let mut action: Option<Do> = None;
    let count = options.len();
    for (i, option) in options.iter().enumerate() {
        card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(strong(
                    ui.tf("inspector.choice.option_n", &[("n", &(i + 1))]),
                    12.5,
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(count > 1, egui::Button::new(icons::TRASH).frame(false))
                        .on_hover_text(ui.t("inspector.choice.remove"))
                        .clicked()
                    {
                        action = Some(Do::Remove(i));
                    }
                    if ui
                        .add_enabled(
                            i + 1 < count,
                            egui::Button::new(icons::ARROW_DOWN).frame(false),
                        )
                        .clicked()
                    {
                        action = Some(Do::Move(i, 1));
                    }
                    if ui
                        .add_enabled(i > 0, egui::Button::new(icons::ARROW_UP).frame(false))
                        .clicked()
                    {
                        action = Some(Do::Move(i, -1));
                    }
                });
            });
            let text = story.text(option.text_id).unwrap_or_default();
            let mut done = false;
            let text_hint = ui.t("inspector.choice.text_hint");
            if let Some(t) =
                fields::text_field(ui, ("choice_text", id, i), text, &text_hint, 2, &mut done)
            {
                action = Some(Do::Text(option.text_id, t));
            }
            tfield(ui, "inspector.choice.leads_to", |ui| {
                if let Some(Some(t)) = label_picker(
                    ui,
                    ("choice_target", id, i),
                    story,
                    Some(option.target),
                    None,
                ) {
                    action = Some(Do::Target(i, t));
                }
            });
            if new_label_button(ui) {
                action = Some(Do::NewLabel(i));
            }
            ui.add_space(4.0);
            let conditional = ui.t("inspector.choice.conditional");
            if let Some(on) = fields::checkbox(ui, option.condition.is_some(), &conditional) {
                action = Some(Do::Condition(i, on.then(|| Expr::lit(true))));
            }
            if let Some(cond) = &option.condition
                && let Some(c) = condition_editor(ui, ("choice_cond", id, i), story, cond)
            {
                action = Some(Do::Condition(i, Some(c)));
            }
        });
        ui.add_space(4.0);
    }
    if ui
        .button(format!("{}  {}", icons::PLUS, ui.t("inspector.choice.add")))
        .clicked()
    {
        action = Some(Do::Add);
    }
    let Some(action) = action else { return };
    let with_options = |f: &dyn Fn(&mut Vec<ChoiceOption>)| {
        let mut o = options.to_vec();
        f(&mut o);
        NodeKind::Choice { options: o }
    };
    match action {
        Do::Text(string, text) => {
            let command = Command::SetString {
                id: string,
                text: Some(text),
            };
            if let Err(e) = ctx.exec_merged(command, &format!("opt-text:{string}")) {
                ctx.error(e.to_string());
            }
        }
        Do::Target(i, t) => {
            let kind = with_options(&|o| o[i].target = t);
            commit(ctx, id, kind, &format!("opt-target:{i}"));
        }
        Do::NewLabel(i) => {
            let mut o = options.to_vec();
            with_new_label(ctx, id, move |l| {
                o[i].target = l;
                NodeKind::Choice { options: o }
            });
        }
        Do::Condition(i, c) => {
            let kind = with_options(&|o| o[i].condition = c.clone());
            commit(ctx, id, kind, &format!("opt-cond:{i}"));
        }
        Do::Remove(i) => {
            let string = options[i].text_id;
            let kind = with_options(&|o| {
                o.remove(i);
            });
            let version = ctx.node_version(id);
            let tx_label = ctx.t("inspector.choice.remove_tx");
            let result = ctx.transaction(&tx_label, |tx| {
                tx.execute(Command::SetNodeKind {
                    id,
                    kind,
                    expected_version: version,
                })?;
                tx.execute(Command::SetString {
                    id: string,
                    text: None,
                })
            });
            if let Err(e) = result {
                ctx.error(e.to_string());
            }
        }
        Do::Move(i, d) => {
            let j = (i as i32 + d) as usize;
            let kind = with_options(&|o| o.swap(i, j));
            commit(ctx, id, kind, "opt-move");
        }
        Do::Add => add_option(ctx, id, options),
    }
}

fn add_option(ctx: &mut EditorContext<'_>, id: NodeId, options: &[ChoiceOption]) {
    let Some(story) = ctx.story() else { return };
    let target = options
        .last()
        .map(|o| o.target)
        .or_else(|| ops::labels(story).first().map(|(l, _)| *l));
    let text_id = StringId::new();
    let n = options.len() + 1;
    let version = ctx.node_version(id);
    let tx_label = ctx.t("inspector.choice.add");
    let option_text = ctx.tf("inspector.choice.option_n", &[("n", &n)]);
    let make = |target: NodeId| {
        let mut o = options.to_vec();
        o.push(ChoiceOption {
            text_id,
            target,
            condition: None,
        });
        NodeKind::Choice { options: o }
    };
    let result = match target {
        Some(target) => {
            let kind = make(target);
            ctx.transaction(&tx_label, |tx| {
                tx.execute(Command::SetString {
                    id: text_id,
                    text: Some(option_text.clone()),
                })?;
                tx.execute(Command::SetNodeKind {
                    id,
                    kind,
                    expected_version: version,
                })
            })
        }
        None => {
            // No label to point at yet: make one.
            let Some((si, _)) = story.locate(id) else {
                return;
            };
            let scene = story.scenes[si].id;
            let end = story.scenes[si].nodes.len();
            let label = Node {
                id: NodeId::new(),
                kind: NodeKind::Label {
                    name: "etiquette".into(),
                },
            };
            let kind = make(label.id);
            ctx.transaction(&tx_label, |tx| {
                tx.execute(Command::SetString {
                    id: text_id,
                    text: Some(option_text.clone()),
                })?;
                tx.execute(Command::InsertNode {
                    scene,
                    index: end,
                    node: label,
                })?;
                tx.execute(Command::SetNodeKind {
                    id,
                    kind,
                    expected_version: version,
                })
            })
        }
    };
    if let Err(e) = result {
        ctx.error(e.to_string());
    }
}

/// Condition editor: the no-code builder (variable, comparison, value,
/// "all / any" groups) when the expression allows it, otherwise (or on
/// request, "Expression mode") the text form with the parser.
fn condition_editor(
    ui: &mut Ui,
    salt: impl fields::Salt,
    story: &orbit_core::Story,
    cond: &Expr,
) -> Option<Expr> {
    orbit_editor_api::varwidgets::expr_condition_editor(ui, salt, story, cond)
}

fn set_var(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, var: &str, value: &Expr) {
    let Some(story) = ctx.story() else { return };
    let declared = story.variables.iter().any(|v| v.name == var);
    let mut new_var = None;
    tfield(ui, "inspector.variable", |ui| {
        let mut chosen = var.to_owned();
        let choose = ui.t("inspector.setvar.choose");
        egui::ComboBox::from_id_salt(("setvar_var", id))
            .selected_text(if var.is_empty() {
                choose
            } else {
                var.to_owned()
            })
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for v in &story.variables {
                    ui.selectable_value(&mut chosen, v.name.clone(), &v.name);
                }
            });
        if chosen != var {
            new_var = Some(chosen);
        }
        let mut done = false;
        let name_hint = ui.t("inspector.setvar.name_hint");
        if let Some(t) = fields::text_field(ui, ("setvar_name", id), var, &name_hint, 1, &mut done)
        {
            new_var = Some(t);
        }
        if !declared && !var.is_empty() {
            ui.label(
                RichText::new(ui.t("inspector.setvar.undeclared"))
                    .small()
                    .color(warning(ui)),
            );
        }
    });
    // A declared variable: the no-code operation (set, add, toggle...),
    // or the expression in "Expression mode".
    if declared {
        let mut handled = None;
        tfield(ui, "inspector.setvar.new_value", |ui| {
            handled = orbit_editor_api::varwidgets::setvar_value_editor(
                ui,
                ("setvar_op", id),
                story,
                var,
                value,
            );
        });
        if let Some(new_value) = handled {
            // Moving a plain operation to another variable keeps what fits.
            let new_value = new_value.or_else(|| {
                let moved = new_var.as_deref()?;
                let op = orbit_core::VarOp::from_expr(var, value)?;
                let vars = orbit_editor_api::varwidgets::story_vars(story);
                Some(orbit_editor_api::varwidgets::with_var(&op, &vars, moved).to_expr())
            });
            if new_var.is_some() || new_value.is_some() {
                let key = if new_var.is_some() { "var" } else { "value" };
                let kind = NodeKind::SetVar {
                    var: new_var.unwrap_or_else(|| var.to_owned()),
                    value: new_value.unwrap_or_else(|| value.clone()),
                };
                commit(ctx, id, kind, key);
            }
            return;
        }
    }
    // Value: typed widget for a constant, expression text otherwise.
    let ty = fields::var_type(story, var);
    let mode_id = ui.make_persistent_id(("setvar_mode", id));
    let is_lit = matches!(value, Expr::Lit(_));
    let calc = ui.data(|d| d.get_temp::<bool>(mode_id)).unwrap_or(false) || !is_lit;
    let mut new_value = None;
    tfield(ui, "inspector.setvar.new_value", |ui| {
        ui.horizontal(|ui| {
            let mut calc_now = calc;
            let (fixed, compute) = (
                ui.t("inspector.setvar.fixed"),
                ui.t("inspector.setvar.compute"),
            );
            ui.selectable_value(&mut calc_now, false, fixed);
            ui.selectable_value(&mut calc_now, true, compute);
            if calc_now != calc {
                ui.data_mut(|d| d.insert_temp(mode_id, calc_now));
                if !calc_now && !is_lit {
                    new_value = Some(Expr::Lit(fields::default_value(
                        ty.unwrap_or(ValueType::Int),
                    )));
                }
            }
        });
        match (calc, value) {
            (false, Expr::Lit(v)) => {
                if let Some(nv) = value_editor(ui, v) {
                    new_value = Some(Expr::Lit(nv));
                }
                // Switch the type of the constant when the variable is not declared.
                if ty.is_none() {
                    ui.horizontal(|ui| {
                        for (key, t) in [
                            ("inspector.setvar.type.number", ValueType::Int),
                            ("inspector.setvar.type.bool", ValueType::Bool),
                            ("inspector.setvar.type.text", ValueType::Str),
                        ] {
                            if ui.small_button(ui.t(key)).clicked() && v.value_type() != t {
                                new_value = Some(Expr::Lit(fields::default_value(t)));
                            }
                        }
                    });
                }
            }
            _ => {
                if let Some(e) = expr_editor(ui, ("setvar_expr", id), story, value, false) {
                    new_value = Some(e);
                }
            }
        }
    });
    if new_var.is_some() || new_value.is_some() {
        let key = if new_var.is_some() { "var" } else { "value" };
        let kind = NodeKind::SetVar {
            var: new_var.unwrap_or_else(|| var.to_owned()),
            value: new_value.unwrap_or_else(|| value.clone()),
        };
        commit(ctx, id, kind, key);
    }
}

fn condition_node(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    condition: &Expr,
    then_target: NodeId,
    else_target: Option<NodeId>,
) {
    let Some(story) = ctx.story() else { return };
    let mut new_cond = None;
    tfield(ui, "inspector.condition", |ui| {
        new_cond = condition_editor(ui, ("if_cond", id), story, condition);
    });
    let mut new_then = None;
    tfield(ui, "inspector.if.then", |ui| {
        new_then = label_picker(ui, ("if_then", id), story, Some(then_target), None);
    });
    let mut new_else = None;
    tfield(ui, "inspector.if.else", |ui| {
        let next = ui.t("inspector.if.continue");
        new_else = label_picker(ui, ("if_else", id), story, else_target, Some(&next));
    });
    let rebuild =
        |condition: Expr, then_target: NodeId, else_target: Option<NodeId>| NodeKind::If {
            condition,
            then_target,
            else_target,
        };
    if let Some(c) = new_cond {
        commit(ctx, id, rebuild(c, then_target, else_target), "condition");
    }
    if let Some(Some(t)) = new_then {
        commit(ctx, id, rebuild(condition.clone(), t, else_target), "then");
    }
    if let Some(e) = new_else {
        commit(ctx, id, rebuild(condition.clone(), then_target, e), "else");
    }
    ui.add_space(4.0);
    if new_label_button(ui) {
        let condition = condition.clone();
        with_new_label(ctx, id, move |l| rebuild(condition, l, else_target));
    }
}

/// An asset chooser with a preview (images) and an "add a file" button.
fn asset_row(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    kind: AssetKind,
    current: orbit_core::AssetId,
    build: impl Fn(orbit_core::AssetId) -> NodeKind,
) {
    let Some(story) = ctx.story() else { return };
    if kind == AssetKind::Image {
        thumbs::show(
            ui,
            ctx,
            story,
            current,
            egui::vec2(ui.available_width().min(240.0), 135.0),
        );
        ui.add_space(4.0);
    }
    let mut chosen = asset_picker(ui, ("asset", id), story, kind, current);
    if ui
        .small_button(format!(
            "{}  {}",
            icons::FILE_PLUS,
            ui.t("inspector.asset.add_file")
        ))
        .on_hover_text(ui.t("inspector.asset.add_file.tip"))
        .clicked()
        && let Some(new) = edit::import_asset_dialog(ctx, kind)
    {
        chosen = Some(new);
    }
    if let Some(a) = chosen {
        commit(ctx, id, build(a), "asset");
    }
}

fn background(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    asset: orbit_core::AssetId,
    transition: Option<orbit_core::Transition>,
) {
    tfield(ui, "inspector.image", |ui| {
        asset_row(ui, ctx, id, AssetKind::Image, asset, |a| {
            NodeKind::Background {
                asset: a,
                transition: transition.clone(),
            }
        });
    });
    tfield(ui, "inspector.transition", |ui| {
        if let Some(t) = transition_editor(ui, ("bg_transition", id), transition.clone(), &curves_of(ctx)) {
            commit(
                ctx,
                id,
                NodeKind::Background {
                    asset,
                    transition: t,
                },
                "transition",
            );
        }
    });
}

/// The inspector of a `Show`: who, which expression, and how they stand:
/// position (named or free), scale (uniform or stretched) and mirroring.
fn show(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, kind: &NodeKind) {
    let NodeKind::Show {
        character,
        expression,
        position,
        transition,
        scale,
        scale_y,
        flip_x,
        flip_y,
    } = kind
    else {
        return;
    };
    let (character, transition) = (*character, transition.clone());
    let Some(story) = ctx.story() else { return };
    let default_scale = story.character(character).map_or(1.0, |c| c.default_scale);
    // Edits are applied to a copy of the node: whatever the widget changes,
    // the rest of the node (and so of the look) is kept.
    let mut edited = kind.clone();
    let mut changed: Option<&str> = None;

    let mut new_char = None;
    tfield(ui, "inspector.character", |ui| {
        new_char = character_picker(ui, ("show_char", id), story, Some(character), None);
    });
    let mut new_expr = None;
    tfield(ui, "inspector.expression", |ui| {
        // Preview of the sprite.
        if let Some(e) = story
            .character(character)
            .and_then(|c| c.expression(expression))
        {
            thumbs::show(ui, ctx, story, e.image, egui::vec2(96.0, 120.0));
            ui.add_space(4.0);
        }
        new_expr = expression_picker(ui, ("show_expr", id), story, character, expression);
    });

    let mut new_pos = None;
    tfield(ui, "inspector.position", |ui| {
        new_pos = position_picker(ui, ("show_pos", id), story, position);
    });
    let mut new_xy: (Option<f32>, Option<f32>) = (None, None);
    if let Position::Free { x, y } = position {
        ui.horizontal(|ui| {
            new_xy.0 = fields::coord_field(ui, "X", *x);
            new_xy.1 = fields::coord_field(ui, "Y", *y);
        });
        ui.label(
            RichText::new(ui.t("inspector.show.free_hint"))
                .small()
                .color(faint(ui)),
        );
    }

    // Scale: the override, or the character's default when there is none.
    let mut new_scale: Option<Option<f32>> = None;
    let mut new_scale_y: Option<Option<f32>> = None;
    tfield(ui, "inspector.scale", |ui| {
        let shown = scale.unwrap_or(default_scale);
        ui.horizontal(|ui| {
            if let Some(v) = fields::scale_slider(ui, shown) {
                new_scale = Some(Some(v));
            }
        });
        ui.horizontal(|ui| {
            let mut stretch = scale_y.is_some();
            if ui
                .checkbox(&mut stretch, ui.t("inspector.show.stretch"))
                .on_hover_text(ui.t("inspector.show.stretch.tip"))
                .changed()
            {
                // Turning it on starts from the current scale (no jump).
                new_scale_y = Some(stretch.then_some(shown));
                if stretch && scale.is_none() {
                    new_scale = Some(Some(shown));
                }
            }
            let has_override = scale.is_some() || scale_y.is_some();
            if ui
                .add_enabled(
                    has_override,
                    egui::Button::new(format!(
                        "{}  {}",
                        icons::ARROW_COUNTER_CLOCKWISE,
                        ui.tf(
                            "inspector.show.default_scale",
                            &[("scale", &format!("{default_scale:.2}"))]
                        )
                    ))
                    .small(),
                )
                .on_hover_text(ui.t("inspector.show.default_scale.tip"))
                .clicked()
            {
                new_scale = Some(None);
                new_scale_y = Some(None);
            }
        });
        if let Some(sy) = scale_y {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Y").small().color(weak(ui)));
                if let Some(v) = fields::scale_slider(ui, *sy) {
                    new_scale_y = Some(Some(v));
                }
            });
        }
    });

    // Mirroring.
    let (mut fx, mut fy) = (*flip_x, *flip_y);
    tfield(ui, "inspector.mirror", |ui| {
        ui.horizontal(|ui| {
            ui.toggle_value(
                &mut fx,
                format!("{}  {}", icons::FLIP_HORIZONTAL, ui.t("inspector.mirror_x")),
            )
            .on_hover_text(ui.t("inspector.mirror_x.tip"));
            ui.toggle_value(
                &mut fy,
                format!("{}  {}", icons::FLIP_VERTICAL, ui.t("inspector.mirror_y")),
            )
            .on_hover_text(ui.t("inspector.mirror_y.tip"));
        });
    });
    let mut new_tr = None;
    tfield(ui, "inspector.transition", |ui| {
        new_tr = transition_editor(ui, ("show_tr", id), transition.clone(), &curves_of(ctx));
    });
    ui.add_space(6.0);
    let open_view = ui
        .button(format!(
            "{}  {}",
            icons::FRAME_CORNERS,
            ui.t("inspector.show.place")
        ))
        .on_hover_text(ui.t("inspector.show.place.tip"))
        .clicked();

    // ---- apply
    if let NodeKind::Show {
        character: c,
        expression: e,
        position: p,
        transition: t,
        scale: sc,
        scale_y: sy,
        flip_x: ffx,
        flip_y: ffy,
    } = &mut edited
    {
        if let Some(Some(nc)) = new_char {
            // Keep the expression when the new character has it, else take its first.
            *e = story
                .character(nc)
                .map(|d| {
                    if d.expression(expression).is_some() {
                        expression.clone()
                    } else {
                        d.expressions
                            .first()
                            .map(|x| x.name.clone())
                            .unwrap_or_default()
                    }
                })
                .unwrap_or_default();
            *c = nc;
            changed = Some("character");
        }
        if let Some(ne) = new_expr {
            *e = ne;
            changed = Some("expression");
        }
        if let Some(np) = new_pos {
            *p = np;
            changed = Some("position");
        }
        if let Position::Free { x, y } = p
            && (new_xy.0.is_some() || new_xy.1.is_some())
        {
            *x = new_xy.0.unwrap_or(*x);
            *y = new_xy.1.unwrap_or(*y);
            changed = Some("position_xy");
        }
        if let Some(v) = new_scale {
            *sc = v;
            changed = Some("scale");
        }
        if let Some(v) = new_scale_y {
            *sy = v;
            changed = Some("scale_y");
        }
        if fx != *flip_x {
            *ffx = fx;
            changed = Some("flip_x");
        }
        if fy != *flip_y {
            *ffy = fy;
            changed = Some("flip_y");
        }
        if let Some(nt) = new_tr {
            *t = nt;
            changed = Some("transition");
        }
    }
    if let Some(field) = changed {
        commit(ctx, id, edited, field);
    }
    if open_view {
        ctx.request(orbit_editor_api::EditorRequest::ShowPanel(
            super::panels::SCENEVIEW.to_owned(),
        ));
    }
}

fn hide(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    character: CharacterId,
    transition: Option<orbit_core::Transition>,
) {
    let Some(story) = ctx.story() else { return };
    let mut new_char = None;
    tfield(ui, "inspector.character", |ui| {
        new_char = character_picker(ui, ("hide_char", id), story, Some(character), None);
    });
    let mut new_tr = None;
    tfield(ui, "inspector.transition", |ui| {
        new_tr = transition_editor(ui, ("hide_tr", id), transition.clone(), &curves_of(ctx));
    });
    if let Some(Some(c)) = new_char {
        commit(
            ctx,
            id,
            NodeKind::Hide {
                character: c,
                transition,
            },
            "character",
        );
    }
    if let Some(t) = new_tr {
        commit(
            ctx,
            id,
            NodeKind::Hide {
                character,
                transition: t,
            },
            "transition",
        );
    }
}

fn play_music(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    asset: orbit_core::AssetId,
    fade_in: f32,
    looping: bool,
) {
    tfield(ui, "inspector.music", |ui| {
        asset_row(ui, ctx, id, AssetKind::Music, asset, |a| {
            NodeKind::PlayMusic {
                asset: a,
                fade_in,
                looping,
            }
        });
    });
    tfield(ui, "inspector.fade_in", |ui| {
        if let Some(v) = seconds_field(ui, fade_in) {
            commit(
                ctx,
                id,
                NodeKind::PlayMusic {
                    asset,
                    fade_in: v,
                    looping,
                },
                "fade_in",
            );
        }
    });
    ui.add_space(6.0);
    let loop_label = ui.t("inspector.music.loop");
    if let Some(l) = fields::checkbox(ui, looping, &loop_label) {
        commit(
            ctx,
            id,
            NodeKind::PlayMusic {
                asset,
                fade_in,
                looping: l,
            },
            "looping",
        );
    }
}

// --------------------------------------------------------------------- custom

/// Fallback for node kinds without a dedicated inspector: plugin nodes get
/// an editor of their open data (and the plugin's own validation messages).
fn generic_inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId) {
    let Some(story) = ctx.story() else { return };
    let Some(node) = story.node(id) else { return };
    let NodeKind::Custom {
        plugin_id,
        kind,
        version,
        data,
    } = &node.kind
    else {
        // A built-in kind always has an inspector; this is only a safety net.
        ui.label(RichText::new(super::nodeinfo::summary(&ui.texts(), story, node)).color(weak(ui)));
        return;
    };
    ui.label(
        RichText::new(ui.tf(
            "inspector.custom.data_version",
            &[("plugin", plugin_id), ("kind", kind), ("version", version)],
        ))
        .small()
        .color(weak(ui)),
    );
    let problems: Vec<String> = ctx
        .registry()
        .project
        .node_type(plugin_id, kind)
        .and_then(|t| t.def.validate.map(|v| v(data)))
        .unwrap_or_default();
    for p in problems {
        ui.label(
            RichText::new(format!("{}  {p}", icons::WARNING))
                .small()
                .color(warning(ui)),
        );
    }
    ui.add_space(4.0);
    let (plugin_id, kind, version) = (plugin_id.clone(), kind.clone(), *version);
    let data = data.clone();
    let mut changed: Option<CustomData> = None;
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        changed = fields::custom_data_editor(ui, egui::Id::new(("custom", id)), &data);
    });
    if let Some(d) = changed {
        commit(
            ctx,
            id,
            NodeKind::Custom {
                plugin_id,
                kind,
                version,
                data: d,
            },
            "data",
        );
    }
}

/// The story's named curves (a snapshot, so the widgets can borrow freely).
pub(super) fn curves_of(ctx: &EditorContext<'_>) -> std::collections::BTreeMap<String, orbit_core::Curve> {
    ctx.story().map(|s| s.curves.clone()).unwrap_or_default()
}
