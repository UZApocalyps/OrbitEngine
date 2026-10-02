//! One frame of editor UI: menu bar, dock, status bar, palette, dialogs,
//! toasts, global shortcuts.

use egui::{
    Align, Align2, Color32, Context, CornerRadius, Frame, Id, Key, KeyboardShortcut, Layout,
    Margin, Modifiers, Panel, RichText, Stroke, TextEdit, Ui, vec2,
};
use egui_dock::DockArea;
use orbit_editor_api::{
    EditorContext, EditorRegistry, EditorRequest, Level, ThemeMode, icons, shortcut_text,
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
    CreateProject { name: String, parent: String },
    CancelNewProject,
}

/// Keys that edit text: global shortcuts on them are left to the text field
/// while one has focus.
fn edits_text(shortcut: &KeyboardShortcut) -> bool {
    let plain = shortcut.modifiers == Modifiers::NONE;
    matches!(
        shortcut.logical_key,
        Key::Z | Key::Y | Key::X | Key::C | Key::V | Key::A
    ) || (plain && matches!(shortcut.logical_key, Key::Delete | Key::Backspace))
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
                ..
            } = self;
            let mut ectx =
                EditorContext::new(session.as_mut().map(|s| &mut s.project), state, registry);

            menu_clicks = menu_bar(ui, &ectx, theme, &mut palette_request);
            status_bar(ui, &ectx, theme, *show_frame_counter);
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
            palette_outcome = palette.show(&ctx, theme, |query| {
                palette::search(registry_ref, &ectx, query)
            });
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
            PaletteOutcome::Close | PaletteOutcome::None => {}
        }
        self.apply_dialog_outcome(dialog_outcome);
        self.pump(&ctx);
        // No text field focused: the live-edit merge session is over.
        if !ctx.egui_wants_keyboard_input()
            && let Some(session) = self.session.as_mut()
        {
            session.project.end_merge();
        }
        self.after_frame(&ctx);
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
                ui.menu_button(&menu.label, |ui| {
                    ui.set_min_width(260.0);
                    let mut last_group: Option<i32> = None;
                    for action in registry.menu_actions(&menu.id) {
                        let order = action.def.menu.as_ref().map_or(0, |m| m.order);
                        let group = order.div_euclid(100);
                        if last_group.is_some_and(|g| g != group) {
                            ui.separator();
                        }
                        last_group = Some(group);
                        let label = action.label(ectx);
                        let text = if action.def.icon.is_empty() {
                            format!("      {label}")
                        } else {
                            format!("{}   {label}", action.def.icon)
                        };
                        let mut button = egui::Button::new(text);
                        if let Some(shortcut) = action.shortcuts().next() {
                            button = button.shortcut_text(shortcut_text(shortcut));
                        }
                        let enabled = (action.def.enabled)(ectx);
                        if ui.add_enabled(enabled, button).clicked() {
                            clicked.push(action.def.id.clone());
                            ui.close();
                        }
                    }
                });
            }

            // Right side: project and a palette hint.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let hint = format!(
                    "{}  Rechercher une commande   Ctrl+K",
                    icons::MAGNIFYING_GLASS
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

// -------------------------------------------------------------- status bar

fn status_bar(ui: &mut Ui, ectx: &EditorContext<'_>, theme: &Theme, frame_counter: bool) {
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
                                "modifications non enregistrées",
                            )
                        } else {
                            (icons::CHECK_CIRCLE, theme.success(), "enregistré")
                        };
                        ui.label(RichText::new(icon).color(color));
                        ui.label(RichText::new(text).small().color(color));
                        ui.separator();
                        ui.label(
                            RichText::new(info.root.join(&info.story_file).display().to_string())
                                .small()
                                .color(theme.text_weak()),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new("Aucun projet ouvert")
                                .small()
                                .color(theme.text_faint()),
                        );
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if frame_counter {
                        let frames = ectx.state().debug_frames.unwrap_or(0);
                        ui.label(
                            RichText::new(format!("frames : {frames}"))
                                .small()
                                .color(theme.text_faint()),
                        );
                        ui.separator();
                    }
                    let mode = match ectx.state().theme {
                        ThemeMode::Dark => "sombre",
                        ThemeMode::Light => "clair",
                    };
                    ui.label(
                        RichText::new(format!("thème {mode}"))
                            .small()
                            .color(theme.text_faint()),
                    );
                });
            });
        });
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
                    ui.label(RichText::new("Enregistrer les modifications ?").font(bold(17.0)));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "« {project_name} » contient des modifications non enregistrées."
                        ))
                        .color(theme.text_weak()),
                    );
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary(ui, theme, "Enregistrer").clicked() {
                            outcome = DialogOutcome::Unsaved(then.clone(), UnsavedChoice::Save);
                        }
                        if ui.button("Ne pas enregistrer").clicked() {
                            outcome = DialogOutcome::Unsaved(then.clone(), UnsavedChoice::Discard);
                        }
                        if ui.button("Annuler").clicked() {
                            outcome = DialogOutcome::Unsaved(then.clone(), UnsavedChoice::Cancel);
                        }
                    });
                });
            if escape {
                outcome = DialogOutcome::Unsaved(then, UnsavedChoice::Cancel);
            }
        }
        Dialog::Recovery { meta } => {
            let age = describe_age(meta);
            egui::Modal::new(Id::new("orbit_recovery"))
                .frame(dialog_frame(theme, ctx))
                .show(ctx, |ui| {
                    ui.set_width(430.0);
                    ui.label(
                        RichText::new(format!("{}  Restaurer le travail non enregistré ?", icons::WARNING))
                            .font(bold(17.0)),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "La dernière session de « {project_name} » s'est terminée sans enregistrer \
                             (sauvegarde automatique {age}). Voulez-vous retrouver ces modifications ?"
                        ))
                        .color(theme.text_weak()),
                    );
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary(ui, theme, "Restaurer").clicked() {
                            outcome = DialogOutcome::Recovery(true);
                        }
                        if ui.button("Ignorer").clicked() {
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
            ui.label(RichText::new("Nouveau projet").font(bold(17.0)));
            ui.add_space(10.0);
            ui.label(RichText::new("Nom").small().color(theme.text_weak()));
            let name = ui.add(
                TextEdit::singleline(&mut form.name)
                    .hint_text("Mon visual novel")
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
                RichText::new("Dossier parent")
                    .small()
                    .color(theme.text_weak()),
            );
            ui.horizontal(|ui| {
                let width = ui.available_width() - 110.0;
                ui.add(TextEdit::singleline(&mut form.parent).desired_width(width));
                if ui
                    .button(format!("{}  Parcourir…", icons::FOLDER_OPEN))
                    .clicked()
                    && let Some(folder) = rfd::FileDialog::new()
                        .set_title("Dossier qui contiendra le projet")
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
                        RichText::new("Créer")
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
                if ui.button("Annuler").clicked() {
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

/// "il y a 3 min" for a recovery snapshot.
fn describe_age(meta: &RecoveryMeta) -> String {
    let now = orbit_platform::unix_time_ms();
    let secs = now.saturating_sub(meta.saved_at_ms) / 1000;
    match secs {
        0..=59 => "il y a moins d'une minute".to_owned(),
        60..=3599 => format!("il y a {} min", secs / 60),
        3600..=86_399 => format!("il y a {} h", secs / 3600),
        _ => format!("il y a {} j", secs / 86_400),
    }
}
