//! One frame of editor UI: menu bar, dock, status bar, palette, dialogs,
//! toasts, global shortcuts.

use egui::{
    Align, Align2, Color32, Context, CornerRadius, Frame, Id, Key, KeyboardShortcut, Layout,
    Margin, Modifiers, Panel, RichText, Stroke, TextEdit, Ui, vec2,
};
use egui_dock::DockArea;
use orbit_editor_api::ai::AiServerStatus;
use orbit_editor_api::{
    EditorContext, EditorRegistry, EditorRequest, Level, Location, RegisteredAction, ThemeMode,
    Translator, UiText, icons, shortcut_text,
};

use crate::core::{Dialog, EditorCore, NewProjectForm, Pending, UnsavedChoice};
use crate::dock::DockViewer;
use crate::palette::{self, PaletteOutcome};
use crate::storage::RecoveryMeta;
use crate::theme::{Theme, bold};

/// What the user did in a dialog this frame.
enum DialogOutcome {
    None,
    Unsaved(Pending, UnsavedChoice),
    Recovery(bool),
    CreateProject {
        name: String,
        parent: String,
    },
    CancelNewProject,
    /// The story file changed on disk: reload (`true`) or keep ours.
    ExternalChange(bool),
}

/// Keys that edit text: global shortcuts on them are left to the text field
/// while one has focus.
fn edits_text(shortcut: &KeyboardShortcut) -> bool {
    let plain = shortcut.modifiers == Modifiers::NONE;
    matches!(
        shortcut.logical_key,
        Key::Z | Key::Y | Key::X | Key::C | Key::V
    ) || (shortcut.logical_key == Key::A && !shortcut.modifiers.shift)
        || (plain && matches!(shortcut.logical_key, Key::Delete | Key::Backspace))
}

fn modifier_count(shortcut: &KeyboardShortcut) -> u8 {
    let m = shortcut.modifiers;
    u8::from(m.ctrl || m.command) + u8::from(m.shift) + u8::from(m.alt)
}

impl EditorCore {
    /// Draws one frame of the editor into `ui` (the root `Ui` of
    /// `Context::run_ui`).
    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.first_frame();
        self.handle_shortcuts(&ctx);
        self.handle_dropped_files(&ctx);
        self.pump(&ctx);

        let palette_outcome;
        let mut dialog_outcome = DialogOutcome::None;
        let mut palette_request = false;
        let menu_clicks;
        {
            let Self {
                registry,
                state,
                session,
                dock,
                palette,
                dialog,
                theme,
                show_frame_counter,
                shared,
                ..
            } = self;
            let mut ectx =
                EditorContext::new(session.as_mut().map(|s| &mut s.project), state, registry);

            menu_clicks = menu_bar(ui, &ectx, theme, &mut palette_request);
            let mut open_assistant = false;
            if status_bar(ui, &ectx, theme, *show_frame_counter, &mut open_assistant) {
                ectx.request(EditorRequest::ShowPanel(
                    crate::builtin::panels::PROBLEMS.to_owned(),
                ));
            }
            if open_assistant {
                ectx.request(EditorRequest::ShowPanel(
                    crate::builtin::panels::ASSISTANT.to_owned(),
                ));
            }
            egui::CentralPanel::default()
                .frame(Frame::new().fill(theme.bg()))
                .show(ui, |ui| {
                    // A maximized panel (the preview) replaces the dock.
                    let maximized = ectx
                        .state()
                        .maximized_panel
                        .clone()
                        .and_then(|id| ectx.registry().find_panel(&id).map(|p| (id, p)));
                    if let Some((id, panel)) = maximized {
                        ui.push_id(("orbit_panel_ui", id.as_str()), |ui| {
                            (panel.def.ui)(ui, &mut ectx);
                        });
                        return;
                    }
                    let style = theme.dock_style(ui.style());
                    DockArea::new(dock)
                        .style(style)
                        .show_close_buttons(true)
                        .show_leaf_close_all_buttons(false)
                        .show_leaf_collapse_buttons(false)
                        .show_inside(ui, &mut DockViewer { ctx: &mut ectx });
                });

            let registry_ref: &EditorRegistry = registry;
            let search = shared.search.clone();
            palette_outcome = palette.show(&ctx, theme, |query| {
                palette::merge(
                    palette::search(registry_ref, &ectx, query),
                    palette::navigation(&search, &ectx, query),
                )
            });
            drop_overlay(&ctx, theme);
            if let Some(d) = dialog.as_mut() {
                let name = ectx
                    .project_info()
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                dialog_outcome = show_dialog(&ctx, theme, d, &name);
            }
            toasts(&ctx, theme, &mut ectx);
        }

        self.state
            .requests
            .extend(menu_clicks.into_iter().map(EditorRequest::RunAction));
        if palette_request {
            self.palette.open();
        }
        match palette_outcome {
            PaletteOutcome::Run(id) => self.state.requests.push(EditorRequest::RunAction(id)),
            PaletteOutcome::Goto(location) => self.goto(location),
            PaletteOutcome::Close | PaletteOutcome::None => {}
        }
        self.apply_dialog_outcome(dialog_outcome);
        self.pump(&ctx);
        // No text field focused and no drag in progress: the live-edit merge
        // session is over.
        if !ctx.egui_wants_keyboard_input()
            && !ctx.input(|i| i.pointer.any_down())
            && let Some(session) = self.session.as_mut()
        {
            session.project.end_merge();
        }
        self.after_frame(&ctx);
    }

    /// Shows a thing of the project (palette navigation).
    fn goto(&mut self, location: Location) {
        let mut ectx = EditorContext::new(
            self.session.as_mut().map(|s| &mut s.project),
            &mut self.state,
            &self.registry,
        );
        crate::builtin::reveal::reveal(&mut ectx, location);
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        if self.dialog.is_some() {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        let mut bindings: Vec<(KeyboardShortcut, String)> = self
            .registry
            .actions()
            .iter()
            .flat_map(|a| a.shortcuts().map(|s| (*s, a.def.id.clone())))
            .collect();
        // `Ctrl+Shift+Z` must be tried before `Ctrl+Z`.
        bindings.sort_by_key(|(s, _)| std::cmp::Reverse(modifier_count(s)));
        for (shortcut, id) in bindings {
            if typing && edits_text(&shortcut) {
                continue;
            }
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                self.state.requests.push(EditorRequest::RunAction(id));
                break;
            }
        }
    }

    fn apply_dialog_outcome(&mut self, outcome: DialogOutcome) {
        match outcome {
            DialogOutcome::None => {}
            DialogOutcome::Unsaved(then, choice) => {
                self.dialog = None;
                self.resolve_unsaved(then, choice);
            }
            DialogOutcome::Recovery(restore) => {
                self.dialog = None;
                self.resolve_recovery(restore);
            }
            DialogOutcome::CancelNewProject => self.dialog = None,
            DialogOutcome::ExternalChange(reload) => {
                self.dialog = None;
                self.resolve_external_change(reload);
            }
            DialogOutcome::CreateProject { name, parent } => {
                match self.create_and_open(std::path::Path::new(parent.trim()), &name) {
                    Ok(()) => {
                        // `open_project` may have opened a confirmation instead.
                        if matches!(self.dialog, Some(Dialog::NewProject(_))) {
                            self.dialog = None;
                        }
                    }
                    Err(e) => {
                        if let Some(Dialog::NewProject(form)) = &mut self.dialog {
                            form.error = Some(e);
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- menu bar

/// Draws the menu bar; returns the actions the user clicked.
fn menu_bar(
    ui: &mut Ui,
    ectx: &EditorContext<'_>,
    theme: &Theme,
    palette_request: &mut bool,
) -> Vec<String> {
    let frame = Frame::new()
        .fill(theme.panel())
        .inner_margin(Margin::symmetric(8, 3))
        .stroke(Stroke::new(1.0, theme.border()));
    let mut clicked: Vec<String> = Vec::new();
    Panel::top("orbit_menubar").frame(frame).show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            let registry = ectx.registry();
            for menu in registry.menus() {
                ui.menu_button(ectx.t(&menu.label), |ui| {
                    ui.set_min_width(260.0);
                    let mut last_group: Option<i32> = None;
                    let actions = registry.menu_actions(&menu.id);
                    let mut placed: Vec<&str> = Vec::new();
                    for action in &actions {
                        let order = action.def.menu.as_ref().map_or(0, |m| m.order);
                        let group = order.div_euclid(100);
                        if let Some(submenu) = action.def.menu_group.as_deref() {
                            if placed.contains(&submenu) {
                                continue;
                            }
                            placed.push(submenu);
                        }
                        if last_group.is_some_and(|g| g != group) {
                            ui.separator();
                        }
                        last_group = Some(group);
                        match action.def.menu_group.as_deref() {
                            Some(submenu) => {
                                ui.menu_button(ectx.t(submenu), |ui| {
                                    for item in actions
                                        .iter()
                                        .filter(|a| a.def.menu_group.as_deref() == Some(submenu))
                                    {
                                        if menu_item(ui, ectx, item) {
                                            clicked.push(item.def.id.clone());
                                            ui.close();
                                        }
                                    }
                                });
                            }
                            None => {
                                if menu_item(ui, ectx, action) {
                                    clicked.push(action.def.id.clone());
                                    ui.close();
                                }
                            }
                        }
                    }
                });
            }

            // Right side: project and a palette hint.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let hint = format!(
                    "{}  {}   Ctrl+K",
                    icons::MAGNIFYING_GLASS,
                    ectx.t("palette.search_hint")
                );
                let pill = egui::Button::new(RichText::new(hint).small().color(theme.text_faint()))
                    .fill(theme.bg())
                    .stroke(Stroke::new(1.0, theme.border()))
                    .corner_radius(CornerRadius::same(8));
                if ui.add(pill).clicked() {
                    *palette_request = true;
                }
                if let Some(info) = ectx.project_info() {
                    ui.add_space(8.0);
                    let dot = if info.modified { "● " } else { "" };
                    ui.label(
                        RichText::new(format!("{dot}{}", info.name))
                            .font(bold(13.0))
                            .color(if info.modified {
                                theme.warning()
                            } else {
                                theme.text()
                            }),
                    );
                }
            });
        });
    });
    clicked
}

/// One entry of a menu; `true` when clicked.
fn menu_item(ui: &mut Ui, ectx: &EditorContext<'_>, action: &RegisteredAction) -> bool {
    let label = match &action.def.menu_label {
        Some(text) => ectx.t(text),
        None => action.label(ectx),
    };
    let checked = action.def.checked.as_ref().is_some_and(|f| f(ectx));
    let text = if checked {
        format!("{}   {label}", icons::CHECK)
    } else if action.def.icon.is_empty() || action.def.checked.is_some() {
        format!("      {label}")
    } else {
        format!("{}   {label}", action.def.icon)
    };
    let mut button = egui::Button::new(text);
    if let Some(shortcut) = action.shortcuts().next() {
        button = button.shortcut_text(shortcut_text(&ectx.translator(), shortcut));
    }
    let enabled = (action.def.enabled)(ectx);
    ui.add_enabled(enabled, button).clicked()
}

// -------------------------------------------------------------- status bar

fn status_bar(
    ui: &mut Ui,
    ectx: &EditorContext<'_>,
    theme: &Theme,
    frame_counter: bool,
    open_assistant: &mut bool,
) -> bool {
    let mut open_problems = false;
    let frame = Frame::new()
        .fill(theme.panel())
        .inner_margin(Margin::symmetric(12, 3))
        .stroke(Stroke::new(1.0, theme.border()));
    Panel::bottom("orbit_statusbar")
        .frame(frame)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                match ectx.project_info() {
                    Some(info) => {
                        let (icon, color, text) = if info.modified {
                            (
                                icons::DOT_OUTLINE,
                                theme.warning(),
                                ectx.t("status.unsaved"),
                            )
                        } else {
                            (icons::CHECK_CIRCLE, theme.success(), ectx.t("status.saved"))
                        };
                        ui.label(RichText::new(icon).color(color));
                        ui.label(RichText::new(text).small().color(color));
                        ui.separator();
                        ui.label(
                            RichText::new(info.root.join(&info.story_file).display().to_string())
                                .small()
                                .color(theme.text_weak()),
                        );
                        ui.separator();
                        open_problems = problem_badges(ui, ectx, theme);
                    }
                    None => {
                        ui.label(
                            RichText::new(ectx.t("status.no_project"))
                                .small()
                                .color(theme.text_faint()),
                        );
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // The workspace version (also the installers' version).
                    ui.label(
                        RichText::new(format!("OrbitEngine {}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(theme.text_faint()),
                    );
                    ui.separator();
                    if frame_counter {
                        let frames = ectx.state().debug_frames.unwrap_or(0);
                        ui.label(
                            RichText::new(ectx.tf("status.frames", &[("count", &frames)]))
                                .small()
                                .color(theme.text_faint()),
                        );
                        ui.separator();
                    }
                    let mode = match ectx.state().theme {
                        ThemeMode::Dark => ectx.t("status.theme_dark"),
                        ThemeMode::Light => ectx.t("status.theme_light"),
                    };
                    ui.label(RichText::new(mode).small().color(theme.text_faint()));
                    // The live MCP: an always-visible robot (grey when the server
                    // is off, green when it listens, accent while the AI edits);
                    // a click opens the "Assistant IA" panel.
                    ui.separator();
                    let ai = &ectx.state().ai;
                    let editing = ai.edited_within(2.5);
                    let (color, tip) = match &ai.status {
                        AiServerStatus::Stopped => {
                            (theme.text_faint(), ectx.t("status.ai_stopped"))
                        }
                        AiServerStatus::Listening => {
                            (theme.success(), ectx.t("status.ai_listening"))
                        }
                        AiServerStatus::Connected { clients } => {
                            (theme.success(), ectx.tn("status.ai_clients", *clients, &[]))
                        }
                        AiServerStatus::Failed(e) => {
                            (theme.danger(), ectx.tf("status.ai_failed", &[("error", e)]))
                        }
                    };
                    let (color, tip) = if editing && ai.status.is_running() {
                        (theme.accent(), ectx.t("status.ai_editing"))
                    } else {
                        (color, tip)
                    };
                    let response = ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!(
                                    "{} {}",
                                    icons::ROBOT,
                                    ectx.t("status.ai_label")
                                ))
                                .small()
                                .color(color),
                            )
                            .frame(false),
                        )
                        .on_hover_text(format!("{tip}\n{}", ectx.t("status.ai_click")));
                    if response.clicked() {
                        *open_assistant = true;
                    }
                    if editing {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(400));
                    }
                });
            });
        });
    open_problems
}

/// The error / warning counters of the live validation; `true` when clicked.
fn problem_badges(ui: &mut Ui, ectx: &EditorContext<'_>, theme: &Theme) -> bool {
    let diagnostics = &ectx.state().diagnostics;
    let (errors, warnings) = (diagnostics.error_count(), diagnostics.warning_count());
    let mut clicked = false;
    let badge = |ui: &mut Ui, icon: &str, n: usize, color: Color32, tip: String| -> bool {
        let color = if n == 0 { theme.text_faint() } else { color };
        let response = ui
            .add(
                egui::Button::new(RichText::new(format!("{icon} {n}")).small().color(color))
                    .frame(false),
            )
            .on_hover_text(tip);
        response.clicked()
    };
    clicked |= badge(
        ui,
        icons::WARNING_CIRCLE,
        errors,
        theme.danger(),
        ectx.tn("status.errors_tip", errors, &[]),
    );
    clicked |= badge(
        ui,
        icons::WARNING,
        warnings,
        theme.warning(),
        ectx.tn("status.warnings_tip", warnings, &[]),
    );
    clicked
}

/// While files are dragged over the window: a veil that says where they go.
fn drop_overlay(ctx: &Context, theme: &Theme) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::new("orbit_drop_overlay"),
    ));
    painter.rect_filled(rect, 0.0, theme.accent_soft(60));
    painter.rect_stroke(
        rect.shrink(10.0),
        CornerRadius::same(14),
        Stroke::new(2.5, theme.accent()),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        format!("{}  {}", icons::UPLOAD_SIMPLE, ctx.t("dialog.drop_title")),
        bold(22.0),
        theme.text(),
    );
    painter.text(
        rect.center() + vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        ctx.t("dialog.drop_hint"),
        egui::FontId::proportional(14.0),
        theme.text_weak(),
    );
}

// ----------------------------------------------------------------- toasts

fn toasts(ctx: &Context, theme: &Theme, ectx: &mut EditorContext<'_>) {
    let now = std::time::Instant::now();
    ectx.state_mut().expire_toasts(now);
    let shown: Vec<_> = ectx.state().toasts.iter().rev().take(4).cloned().collect();
    if shown.is_empty() {
        return;
    }
    egui::Area::new(Id::new("orbit_toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -44.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
                for toast in shown {
                    let (icon, color) = match toast.level {
                        Level::Info => (icons::CHECK_CIRCLE, theme.success()),
                        Level::Warn => (icons::WARNING, theme.warning()),
                        Level::Error => (icons::WARNING_CIRCLE, theme.danger()),
                    };
                    Frame::new()
                        .fill(theme.surface())
                        .stroke(Stroke::new(1.0, theme.border()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .shadow(ctx.global_style().visuals.popup_shadow)
                        .show(ui, |ui| {
                            ui.set_max_width(380.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon).size(16.0).color(color));
                                ui.label(RichText::new(&toast.message).color(theme.text()));
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        });
}

// ---------------------------------------------------------------- dialogs

fn dialog_frame(theme: &Theme, ctx: &Context) -> Frame {
    Frame::new()
        .fill(theme.panel())
        .stroke(Stroke::new(1.0, theme.border()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(20))
        .shadow(ctx.global_style().visuals.popup_shadow)
}

fn show_dialog(
    ctx: &Context,
    theme: &Theme,
    dialog: &mut Dialog,
    project_name: &str,
) -> DialogOutcome {
    let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let mut outcome = DialogOutcome::None;
    match dialog {
        Dialog::ConfirmUnsaved { then } => {
            let then = then.clone();
            egui::Modal::new(Id::new("orbit_confirm"))
                .frame(dialog_frame(theme, ctx))
                .show(ctx, |ui| {
                    ui.set_width(400.0);
                    ui.label(RichText::new(ctx.t("dialog.unsaved_title")).font(bold(17.0)));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(ctx.tf("dialog.unsaved_body", &[("name", &project_name)]))
                            .color(theme.text_weak()),
                    );
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary(ui, theme, &ctx.t("common.save")).clicked() {
                            outcome = DialogOutcome::Unsaved(then.clone(), UnsavedChoice::Save);
                        }
                        if ui.button(ctx.t("dialog.dont_save")).clicked() {
                            outcome = DialogOutcome::Unsaved(then.clone(), UnsavedChoice::Discard);
                        }
                        if ui.button(ctx.t("common.cancel")).clicked() {
                            outcome = DialogOutcome::Unsaved(then.clone(), UnsavedChoice::Cancel);
                        }
                    });
                });
            if escape {
                outcome = DialogOutcome::Unsaved(then, UnsavedChoice::Cancel);
            }
        }
        Dialog::Recovery { meta } => {
            let age = describe_age(&ctx.translator().unwrap_or_default(), meta);
            egui::Modal::new(Id::new("orbit_recovery"))
                .frame(dialog_frame(theme, ctx))
                .show(ctx, |ui| {
                    ui.set_width(430.0);
                    ui.label(
                        RichText::new(format!(
                            "{}  {}",
                            icons::WARNING,
                            ctx.t("dialog.recovery_title")
                        ))
                        .font(bold(17.0)),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(ctx.tf(
                            "dialog.recovery_body",
                            &[("name", &project_name), ("age", &age)],
                        ))
                        .color(theme.text_weak()),
                    );
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary(ui, theme, &ctx.t("dialog.restore")).clicked() {
                            outcome = DialogOutcome::Recovery(true);
                        }
                        if ui.button(ctx.t("dialog.ignore")).clicked() {
                            outcome = DialogOutcome::Recovery(false);
                        }
                    });
                });
            // Escape does not decide for the user: the snapshot stays on disk.
            let _ = escape;
        }
        Dialog::NewProject(form) => {
            outcome = new_project_dialog(ctx, theme, form, escape);
        }
        Dialog::ExternalChange { conflict } => {
            let conflict = *conflict;
            egui::Modal::new(Id::new("orbit_external_change"))
                .frame(dialog_frame(theme, ctx))
                .show(ctx, |ui| {
                    ui.set_width(440.0);
                    ui.label(
                        RichText::new(format!(
                            "{}  {}",
                            icons::WARNING,
                            ctx.t("dialog.external_title")
                        ))
                        .font(bold(17.0)),
                    );
                    ui.add_space(6.0);
                    let key = if conflict {
                        "dialog.external_conflict"
                    } else {
                        "dialog.external_body"
                    };
                    let text = ctx.tf(key, &[("name", &project_name)]);
                    ui.label(RichText::new(text).color(theme.text_weak()));
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let reload = ctx.t(if conflict {
                            "dialog.reload_lose"
                        } else {
                            "dialog.reload"
                        });
                        if primary(ui, theme, &reload).clicked() {
                            outcome = DialogOutcome::ExternalChange(true);
                        }
                        let keep = ctx.t(if conflict {
                            "dialog.keep_mine"
                        } else {
                            "dialog.ignore"
                        });
                        if ui.button(keep).clicked() {
                            outcome = DialogOutcome::ExternalChange(false);
                        }
                    });
                });
            // Escape does not decide for the user.
            let _ = escape;
        }
    }
    outcome
}

fn new_project_dialog(
    ctx: &Context,
    theme: &Theme,
    form: &mut NewProjectForm,
    escape: bool,
) -> DialogOutcome {
    let mut outcome = DialogOutcome::None;
    egui::Modal::new(Id::new("orbit_new_project"))
        .frame(dialog_frame(theme, ctx))
        .show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new(ctx.t("dialog.new_project")).font(bold(17.0)));
            ui.add_space(10.0);
            ui.label(
                RichText::new(ctx.t("dialog.name"))
                    .small()
                    .color(theme.text_weak()),
            );
            let name = ui.add(
                TextEdit::singleline(&mut form.name)
                    .hint_text(ctx.t("dialog.name_hint"))
                    .desired_width(f32::INFINITY),
            );
            if name.changed() {
                form.error = None;
            }
            if !form.focused {
                name.request_focus();
                form.focused = true;
            }
            let enter_in_name = name.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(8.0);
            ui.label(
                RichText::new(ctx.t("dialog.parent_folder"))
                    .small()
                    .color(theme.text_weak()),
            );
            ui.horizontal(|ui| {
                let width = ui.available_width() - 110.0;
                ui.add(TextEdit::singleline(&mut form.parent).desired_width(width));
                if ui
                    .button(format!(
                        "{}  {}",
                        icons::FOLDER_OPEN,
                        ctx.t("dialog.browse")
                    ))
                    .clicked()
                    && let Some(folder) = rfd::FileDialog::new()
                        .set_title(ctx.t("dialog.pick_parent"))
                        .pick_folder()
                {
                    form.parent = folder.display().to_string();
                }
            });
            if let Some(error) = &form.error {
                ui.add_space(8.0);
                ui.label(RichText::new(error).color(theme.danger()));
            }
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ready = !form.name.trim().is_empty() && !form.parent.trim().is_empty();
                let create = ui.add_enabled(
                    ready,
                    egui::Button::new(
                        RichText::new(ctx.t("dialog.create"))
                            .color(Color32::WHITE)
                            .font(bold(13.5)),
                    )
                    .fill(theme.accent())
                    .min_size(vec2(0.0, 28.0)),
                );
                if create.clicked() || (enter_in_name && ready) {
                    outcome = DialogOutcome::CreateProject {
                        name: form.name.clone(),
                        parent: form.parent.clone(),
                    };
                }
                if ui.button(ctx.t("common.cancel")).clicked() {
                    outcome = DialogOutcome::CancelNewProject;
                }
            });
        });
    if escape {
        outcome = DialogOutcome::CancelNewProject;
    }
    outcome
}

fn primary(ui: &mut Ui, theme: &Theme, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE).font(bold(13.5)))
            .fill(theme.accent())
            .min_size(vec2(0.0, 28.0)),
    )
}

/// "3 min ago" for a recovery snapshot.
fn describe_age(texts: &Translator, meta: &RecoveryMeta) -> String {
    let now = orbit_platform::unix_time_ms();
    let secs = now.saturating_sub(meta.saved_at_ms) / 1000;
    match secs {
        0..=59 => texts.get("time.just_now"),
        60..=3599 => texts.plural("time.minutes_ago", (secs / 60) as usize, &[]),
        3600..=86_399 => texts.plural("time.hours_ago", (secs / 3600) as usize, &[]),
        _ => texts.plural("time.days_ago", (secs / 86_400) as usize, &[]),
    }
}
