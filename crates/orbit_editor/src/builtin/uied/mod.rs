//! The interface editor (`orbit.ui`): the screens, theme and splash screens
//! of the project's `ui/` folder, with the real runtime as the live preview.
//!
//! One dockable panel with three tabs:
//!
//! * **Screens**: the screens of the project (new from a template, rename,
//!   duplicate, delete, "stands in for" a system screen), the widget tree
//!   of the selected one (add, delete, duplicate, move, drag and drop) and
//!   the properties of the selected widget;
//! * **Theme**: the named styles (add, rename, delete, edit);
//! * **Splashes**: the splash screens, the mandatory engine one shown as a
//!   locked first row.
//!
//! The logic (tree operations, templates, save / load, undo) is in
//! [`model`] and unit tested; [`props`] has the property editors; this file
//! is the egui shell around them. The preview is the Preview panel put in
//! "UI mode" ([`UiPreviewRequest`]).

mod model;
mod props;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use egui::{Align2, Color32, CornerRadius, FontId, RichText, ScrollArea, Sense, Ui, vec2};
use orbit_core::AssetKind;
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, PanelDef,
    Plugin, PluginManifest, Translator, UiPreviewRequest, UiText as _, icons,
};
use orbit_ui::{
    Issue, ScreenAsset, Severity, SplashContent, SplashDef, UiBundle, WidgetDef, WidgetKind,
    asset::SYSTEM_SCREENS,
};

use self::model::{KindTag, Template, UiDoc, WidgetPath};
use self::props::Env;
use super::ids;
use super::panels;
use super::ui_util::{accent, accent_soft, danger, faint, icon_button, section_title, warning, weak};

pub struct UiEditorPlugin {
    state: Rc<RefCell<State>>,
}

impl Default for UiEditorPlugin {
    fn default() -> Self {
        Self {
            state: Rc::new(RefCell::new(State::default())),
        }
    }
}

impl Plugin for UiEditorPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::UI_EDITOR, "Interface editor", "0.1.0")
    }
}

impl EditorPlugin for UiEditorPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        // Saving the project saves the interface documents too.
        let saver = self.state.clone();
        let save_link = reg.ui_link.clone();
        reg.project.on_project_event(move |event| {
            if matches!(event, orbit_plugin_api::ProjectEvent::BeforeSave)
                && let Ok(mut st) = saver.try_borrow_mut()
                && let Some(doc) = st.doc.as_mut()
                && doc.dirty()
                && doc.save().is_ok()
            {
                save_link.dirty.set(false);
            }
        });
        let state = self.state.clone();
        let link = reg.ui_link.clone();
        reg.panel(PanelDef::new(
            "editor",
            "uied.title",
            icons::BROWSERS,
            DockSlot::Hidden,
            move |ui, ctx| panel(ui, ctx, &state, &link),
        ));
        reg.action(
            ActionDef::new("open", "uied.open", |ctx| {
                ctx.request(EditorRequest::ShowPanel(panels::UI_EDITOR.to_owned()));
                ctx.request(EditorRequest::ShowPanel(panels::PREVIEW.to_owned()));
            })
            .icon(icons::BROWSERS)
            .category("category.tools")
            .menu("tools", 150)
            .enabled(|c| c.has_project()),
        );
    }
}

// ------------------------------------------------------------------ state

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Screens,
    Theme,
    Splashes,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Off,
    Screen,
    Splashes,
}

struct State {
    root: Option<PathBuf>,
    doc: Option<UiDoc>,
    error: Option<String>,
    tab: Tab,
    screen: Option<String>,
    widget: WidgetPath,
    style: Option<String>,
    splash: Option<usize>,
    add_kind: KindTag,
    rename: String,
    rename_for: String,
    style_rename: String,
    style_rename_for: String,
    new_style: String,
    mode: Mode,
    sent: Option<(Mode, String, u64, String)>,
    /// Language of the game texts in the preview (`""`: the story's own).
    preview_lang: String,
    saw_active: bool,
    /// The last click was inside this panel.
    focused: bool,
    /// The screen the preview showed last time we looked (to notice when a
    /// click inside it navigated).
    last_shown: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            root: None,
            doc: None,
            error: None,
            tab: Tab::Screens,
            screen: None,
            widget: Vec::new(),
            style: None,
            splash: None,
            add_kind: KindTag::Button,
            rename: String::new(),
            rename_for: String::new(),
            style_rename: String::new(),
            style_rename_for: String::new(),
            new_style: String::new(),
            mode: Mode::Off,
            sent: None,
            preview_lang: String::new(),
            saw_active: false,
            focused: false,
            last_shown: None,
        }
    }
}

impl State {
    /// Loads the `ui/` folder of the open project (again when the project
    /// changed).
    fn sync(&mut self, ctx: &mut EditorContext<'_>) {
        let root = ctx.project_info().map(|p| p.root.clone());
        if root == self.root {
            return;
        }
        *self = Self {
            root: root.clone(),
            tab: self.tab,
            ..Self::default()
        };
        if self.mode != Mode::Off || self.sent.is_some() {
            ctx.state_mut().preview.ui_request = Some(UiPreviewRequest::Off);
        }
        if let Some(root) = root {
            self.load(&root.join("ui"));
        }
    }

    fn load(&mut self, dir: &std::path::Path) {
        match UiDoc::load(dir) {
            Ok(doc) => {
                self.doc = Some(doc);
                self.error = None;
            }
            Err(e) => {
                self.doc = None;
                self.error = Some(e);
            }
        }
    }

    /// Shares the dirty flag and the keyboard focus with the host, applies
    /// the undo / redo it routed here (Ctrl+Z / Ctrl+Y while this panel has
    /// the focus).
    fn sync_link(&mut self, ui: &Ui, link: &orbit_editor_api::UiEditorLink) {
        if ui.input(|i| i.pointer.any_pressed()) {
            self.focused = ui.rect_contains_pointer(ui.max_rect());
        }
        link.focus.set(self.focused);
        let mut steps = link.history.replace(0);
        if let Some(doc) = self.doc.as_mut() {
            while steps != 0 {
                if steps < 0 {
                    doc.undo();
                    steps += 1;
                } else {
                    doc.redo();
                    steps -= 1;
                }
            }
        }
        self.publish_dirty(link);
    }

    fn publish_dirty(&self, link: &orbit_editor_api::UiEditorLink) {
        link.dirty.set(self.doc.as_ref().is_some_and(UiDoc::dirty));
    }

    /// Keeps the selection pointing at things that exist.
    fn ensure_selection(&mut self) {
        let Some(doc) = &self.doc else { return };
        let b = &doc.bundle;
        if self.screen.as_ref().is_none_or(|id| b.screen(id).is_none()) {
            self.screen = b.screens.first().map(|s| s.id.clone());
            self.widget.clear();
        }
        if let Some(screen) = self.screen.as_ref().and_then(|id| b.screen(id))
            && model::get(&screen.root, &self.widget).is_none()
        {
            self.widget.clear();
        }
        if let Some(theme) = &b.theme {
            if self.style.as_ref().is_none_or(|n| !theme.styles.contains_key(n)) {
                self.style = theme.styles.keys().next().cloned();
            }
        } else {
            self.style = None;
        }
        if self.splash.is_some_and(|i| i >= b.splashes.len()) {
            self.splash = b.splashes.len().checked_sub(1);
        }
    }

    /// Keeps the preview in step with the document: shows the selected
    /// screen (or the splashes) while the mode asks for it, sends a new
    /// version after every edit.
    fn drive_preview(&mut self, ctx: &mut EditorContext<'_>) {
        let active = ctx.state().preview.ui_active;
        if self.mode != Mode::Off {
            if active {
                self.saw_active = true;
            } else if self.saw_active {
                // The preview was closed from its own panel, or the game
                // was played.
                self.mode = Mode::Off;
                self.saw_active = false;
                self.sent = None;
                return;
            }
        }
        self.follow_preview(ctx);
        let Some(doc) = &self.doc else { return };
        let want = match self.mode {
            Mode::Off => None,
            Mode::Screen => self.screen.clone().map(|id| (Mode::Screen, id)),
            Mode::Splashes => Some((Mode::Splashes, String::new())),
        };
        match want {
            Some((mode, id)) => {
                let key = (mode, id.clone(), doc.revision, self.preview_lang.clone());
                if self.sent.as_ref() != Some(&key) {
                    let bundle_ron = doc.bundle_ron();
                    ctx.state_mut().preview.ui_language =
                        (!self.preview_lang.is_empty()).then(|| self.preview_lang.clone());
                    ctx.state_mut().preview.ui_request = Some(match mode {
                        Mode::Splashes => UiPreviewRequest::Splashes { bundle_ron },
                        _ => UiPreviewRequest::Screen { bundle_ron, id },
                    });
                    self.sent = Some(key);
                }
            }
            None => {
                if self.sent.take().is_some() {
                    ctx.state_mut().preview.ui_request = Some(UiPreviewRequest::Off);
                    self.saw_active = false;
                }
            }
        }
    }

    /// What happened inside the preview: a click that navigated moves the
    /// selection to the screen now shown; a click on a widget of the screen
    /// selects it (tree and inspector). Also tells the preview which widget
    /// to outline.
    fn follow_preview(&mut self, ctx: &mut EditorContext<'_>) {
        let (shown, pick) = {
            let p = &mut ctx.state_mut().preview;
            (p.ui_shown.clone(), p.ui_pick.take())
        };
        if shown != self.last_shown {
            self.last_shown.clone_from(&shown);
            if self.mode == Mode::Screen
                && let Some(id) = shown
                && self.screen.as_deref() != Some(id.as_str())
                && self.doc.as_ref().is_some_and(|d| d.bundle.screen(&id).is_some())
            {
                self.select_screen(&id);
            }
        }
        if let Some((screen, path)) = pick
            && self.mode == Mode::Screen
            && self.screen.as_deref() == Some(screen.as_str())
            && self.screen().is_some_and(|s| model::get(&s.root, &path).is_some())
        {
            self.widget = path;
            self.tab = Tab::Screens;
        }
        let outline = (self.mode == Mode::Screen && self.tab == Tab::Screens)
            .then(|| self.widget.clone());
        ctx.state_mut().preview.ui_highlight = outline;
    }

    fn select_screen(&mut self, id: &str) {
        self.screen = Some(id.to_owned());
        self.widget.clear();
    }

    fn screen(&self) -> Option<&ScreenAsset> {
        self.doc.as_ref()?.bundle.screen(self.screen.as_deref()?)
    }
}

// ------------------------------------------------------------------ panel

fn panel(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    state: &Rc<RefCell<State>>,
    link: &orbit_editor_api::UiEditorLink,
) {
    let mut st = state.borrow_mut();
    st.sync(ctx);
    st.sync_link(ui, link);
    if !ctx.has_project() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ctx.t("uied.no_project")).color(weak(ui)));
        });
        return;
    }
    if st.doc.is_none() {
        ui.add_space(12.0);
        ui.label(RichText::new(ctx.t("uied.load_failed")).color(danger(ui)));
        if let Some(e) = &st.error {
            ui.label(RichText::new(e).small().color(weak(ui)));
        }
        if ui.button(ctx.t("uied.retry")).clicked()
            && let Some(root) = st.root.clone()
        {
            st.load(&root.join("ui"));
        }
        return;
    }
    st.ensure_selection();
    toolbar(ui, ctx, &mut st);
    ui.separator();
    // Reserve room for the diagnostics under the tab.
    let diag_height = 78.0;
    let body_height = (ui.available_height() - diag_height).max(120.0);
    ui.allocate_ui(vec2(ui.available_width(), body_height), |ui| {
        ui.set_min_height(body_height);
        match st.tab {
            Tab::Screens => screens_tab(ui, ctx, &mut st),
            Tab::Theme => theme_tab(ui, ctx, &mut st),
            Tab::Splashes => splash_tab(ui, ctx, &mut st),
        }
    });
    diagnostics(ui, ctx, &mut st);
    st.ensure_selection();
    st.drive_preview(ctx);
    st.publish_dirty(link);
}

fn toolbar(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let (dirty, can_undo, can_redo) = st
        .doc
        .as_ref()
        .map_or((false, false, false), |d| (d.dirty(), d.can_undo(), d.can_redo()));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let save = egui::Button::new(
            RichText::new(format!(
                "{}  {}{}",
                icons::FLOPPY_DISK,
                ctx.t("uied.save"),
                if dirty { " •" } else { "" }
            ))
            .size(12.5),
        )
        .min_size(vec2(0.0, 26.0));
        if ui
            .add_enabled(dirty, save)
            .on_hover_text(ctx.t("uied.save_tip"))
            .clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            match doc.save() {
                Ok(()) => ctx.info(ctx.tf("uied.saved", &[("dir", &doc.dir.display())])),
                Err(e) => ctx.error(ctx.tf("uied.save_failed", &[("error", &e)])),
            }
        }
        if ui
            .add_enabled_ui(can_undo, |ui| {
                icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, &ctx.t("uied.undo"))
            })
            .inner
            .clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            doc.undo();
        }
        if ui
            .add_enabled_ui(can_redo, |ui| {
                icon_button(ui, icons::ARROW_CLOCKWISE, &ctx.t("uied.redo"))
            })
            .inner
            .clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            doc.redo();
        }
        if icon_button(ui, icons::ARROWS_CLOCKWISE, &ctx.t("uied.reload")).clicked()
            && let Some(doc) = st.doc.as_mut()
            && let Err(e) = doc.reload()
        {
            ctx.error(e);
        }
        ui.separator();
        for (tab, key) in [
            (Tab::Screens, "uied.tab.screens"),
            (Tab::Theme, "uied.tab.theme"),
            (Tab::Splashes, "uied.tab.splashes"),
        ] {
            if ui
                .add(egui::Button::selectable(
                    st.tab == tab,
                    RichText::new(ctx.t(key)).size(12.5),
                ))
                .clicked()
            {
                st.tab = tab;
                if st.mode != Mode::Off {
                    st.mode = if tab == Tab::Splashes { Mode::Splashes } else { Mode::Screen };
                }
            }
        }
        ui.separator();
        let want = if st.tab == Tab::Splashes { Mode::Splashes } else { Mode::Screen };
        let on = st.mode == want;
        let label = if st.tab == Tab::Splashes {
            ctx.t("uied.preview_splashes")
        } else {
            ctx.t("uied.preview")
        };
        if ui
            .add(egui::Button::selectable(
                on,
                RichText::new(format!("{}  {label}", icons::EYE)).size(12.5),
            ))
            .on_hover_text(ctx.t("uied.preview_tip"))
            .clicked()
        {
            st.mode = if on { Mode::Off } else { want };
            if !on {
                ctx.request(EditorRequest::ShowPanel(panels::PREVIEW.to_owned()));
            }
        }
        // Language of the game texts (`ui.*`) in the preview: the story's
        // own by default, not the editor's.
        let lang_label = match st.preview_lang.as_str() {
            "" => ctx.t("uied.lang.auto"),
            "en" => ctx.t("uied.lang.en"),
            "fr" => ctx.t("uied.lang.fr"),
            other => other.to_owned(),
        };
        egui::ComboBox::from_id_salt("uied_preview_lang")
            .selected_text(lang_label)
            .width(110.0)
            .show_ui(ui, |ui| {
                for (code, key) in [
                    ("", "uied.lang.auto"),
                    ("en", "uied.lang.en"),
                    ("fr", "uied.lang.fr"),
                ] {
                    ui.selectable_value(&mut st.preview_lang, code.to_owned(), ctx.t(key));
                }
            })
            .response
            .on_hover_text(ctx.t("uied.lang_tip"));
        if icon_button(ui, icons::CORNERS_OUT, &ctx.t("uied.maximize")).clicked() {
            ctx.request(EditorRequest::ToggleMaximizePanel(panels::UI_EDITOR.to_owned()));
        }
    });
}

// ---------------------------------------------------------------- helpers

/// A full-width row that can be clicked and dragged. Returns it with its
/// rect, the background of a selected / hovered row painted.
fn item_row(ui: &mut Ui, selected: bool) -> (egui::Rect, egui::Response) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click_and_drag());
    if selected || response.hovered() {
        let fill = if selected {
            accent_soft(ui, 70)
        } else {
            ui.visuals().widgets.hovered.bg_fill
        };
        ui.painter()
            .rect_filled(rect.shrink2(vec2(1.0, 0.0)), CornerRadius::same(4), fill);
    }
    (rect, response)
}

fn row_text(ui: &Ui, rect: egui::Rect, x: f32, icon: &str, text: &str, color: Color32) {
    let painter = ui.painter_at(rect);
    let mid = rect.center().y;
    painter.text(
        egui::pos2(rect.left() + x + 8.0, mid),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(14.0),
        color,
    );
    painter.text(
        egui::pos2(rect.left() + x + 22.0, mid),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        ui.visuals().text_color(),
    );
}

fn images_of(ctx: &EditorContext<'_>) -> Vec<String> {
    ctx.story()
        .map(|s| {
            s.assets
                .assets
                .iter()
                .filter(|a| a.kind == AssetKind::Image)
                .map(|a| a.path.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// A short description of a widget for the tree.
fn widget_label(w: &WidgetDef) -> String {
    let name = if w.id.is_empty() {
        KindTag::of(&w.kind).name().to_owned()
    } else {
        w.id.clone()
    };
    let text = match &w.kind {
        WidgetKind::Button { text, .. }
        | WidgetKind::Label { text, .. }
        | WidgetKind::Toggle { text, .. } => match text {
            orbit_ui::TextRef::Key(k) => format!("  ({k})"),
            orbit_ui::TextRef::Literal(t) => format!("  “{t}”"),
            orbit_ui::TextRef::Bind(_) => String::new(),
        },
        _ => String::new(),
    };
    format!("{name}{text}")
}

/// The path of the widget with this id, when it has one.
fn find_widget(root: &WidgetDef, id: &str) -> Option<WidgetPath> {
    if id.is_empty() {
        return None;
    }
    model::all_paths(root)
        .into_iter()
        .find(|(p, _)| model::get(root, p).is_some_and(|w| w.id == id))
        .map(|(p, _)| p)
}

// ------------------------------------------------------------ screens tab

/// What the tree asked for this frame.
enum TreeOp {
    Add(KindTag),
    Delete,
    Duplicate,
    Move(i32),
    Drop { from: WidgetPath, into: WidgetPath, index: usize },
}

fn screens_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let tr = ctx.translator();
    let screens: Vec<String> = st
        .doc
        .as_ref()
        .map(|d| d.bundle.screens.iter().map(|s| s.id.clone()).collect())
        .unwrap_or_default();
    let styles: Vec<String> = st
        .doc
        .as_ref()
        .and_then(|d| d.bundle.theme.as_ref())
        .map(|t| t.styles.keys().cloned().collect())
        .unwrap_or_default();
    let images = images_of(ctx);
    let fonts = theme_font_names(st);
    let vars = ctx.story().map(orbit_editor_api::varwidgets::story_vars).unwrap_or_default();
    let scenes = story_scenes(ctx);
    let env = Env {
        tr: &tr,
        screens: &screens,
        styles: &styles,
        images: &images,
        fonts: &fonts,
        vars: &vars,
        scenes: &scenes,
    };
    let enable_card = st
        .doc
        .as_ref()
        .is_some_and(|d| d.bundle.screens.is_empty() && d.bundle.theme.is_none());
    if enable_card {
        enable_ui_card(ui, ctx, st);
    }
    ui.columns(2, |cols| {
        let (left, right) = cols.split_at_mut(1);
        screens_and_tree(&mut left[0], st, &tr);
        ScrollArea::vertical()
            .id_salt("uied_props")
            .auto_shrink([false, false])
            .show(&mut right[0], |ui| properties(ui, ctx, st, &env));
    });
}

fn enable_ui_card(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    super::ui_util::card(ui, |ui| {
        ui.label(RichText::new(ctx.t("uied.enable_title")).strong());
        ui.label(RichText::new(ctx.t("uied.enable_text")).small().color(weak(ui)));
        if ui
            .button(format!("{}  {}", icons::SPARKLE, ctx.t("uied.enable_button")))
            .clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            doc.enable_defaults();
            match doc.save() {
                Ok(()) => ctx.info(ctx.tf("uied.saved", &[("dir", &doc.dir.display())])),
                Err(e) => ctx.error(ctx.tf("uied.save_failed", &[("error", &e)])),
            }
        }
    });
}

fn screens_and_tree(ui: &mut Ui, st: &mut State, tr: &Translator) {
    // ---- the list of screens
    ui.horizontal(|ui| {
        section_title(ui, &tr.t("uied.screens"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let has = st.screen.is_some();
            if ui
                .add_enabled_ui(has, |ui| icon_button(ui, icons::TRASH, &tr.t("uied.delete_screen")))
                .inner
                .clicked()
                && let Some(id) = st.screen.clone()
                && let Some(doc) = st.doc.as_mut()
            {
                doc.edit(|b| model::delete_screen(b, &id));
            }
            if ui
                .add_enabled_ui(has, |ui| icon_button(ui, icons::COPY, &tr.t("uied.duplicate_screen")))
                .inner
                .clicked()
                && let Some(id) = st.screen.clone()
                && let Some(doc) = st.doc.as_mut()
            {
                if let Some(new) = doc.edit(|b| model::duplicate_screen(b, &id)) {
                    st.select_screen(&new);
                }
            }
            ui.menu_button(icons::PLUS, |ui| {
                ui.label(RichText::new(tr.t("uied.new_from")).small().color(weak(ui)));
                for template in Template::ALL {
                    if ui
                        .button(tr.t(&format!("uied.template.{}", template.name())))
                        .clicked()
                    {
                        if let Some(doc) = st.doc.as_mut() {
                            let screen = model::new_screen(&doc.bundle, template, template.default_id());
                            let id = screen.id.clone();
                            doc.edit(|b| b.screens.push(screen));
                            st.select_screen(&id);
                        }
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(tr.t("uied.new_screen"));
        });
    });
    let rows: Vec<(String, Option<&'static str>)> = st
        .doc
        .as_ref()
        .map(|d| {
            d.bundle
                .screens
                .iter()
                .map(|s| (s.id.clone(), model::system_role(&d.bundle, &s.id)))
                .collect()
        })
        .unwrap_or_default();
    ScrollArea::vertical()
        .id_salt("uied_screens")
        .max_height(130.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if rows.is_empty() {
                ui.label(RichText::new(tr.t("uied.no_screens")).small().color(weak(ui)));
            }
            for (id, role) in &rows {
                let selected = st.screen.as_deref() == Some(id);
                let (rect, resp) = item_row(ui, selected);
                row_text(ui, rect, 4.0, icons::BROWSER, id, if selected { accent(ui) } else { weak(ui) });
                if let Some(role) = role {
                    ui.painter_at(rect).text(
                        egui::pos2(rect.right() - 6.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        tr.t(&format!("uied.role.{role}")),
                        FontId::proportional(11.0),
                        faint(ui),
                    );
                }
                if resp.clicked() {
                    st.select_screen(id);
                }
            }
        });
    ui.add_space(6.0);
    ui.separator();

    // ---- the widget tree of the selected screen
    let mut op: Option<TreeOp> = None;
    ui.horizontal(|ui| {
        section_title(ui, &tr.t("uied.tree"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let has = st.screen.is_some();
            let root_selected = st.widget.is_empty();
            if ui
                .add_enabled_ui(has && !root_selected, |ui| {
                    icon_button(ui, icons::TRASH, &tr.t("uied.delete_widget"))
                })
                .inner
                .clicked()
            {
                op = Some(TreeOp::Delete);
            }
            if ui
                .add_enabled_ui(has && !root_selected, |ui| {
                    icon_button(ui, icons::ARROW_DOWN, &tr.t("uied.move_down"))
                })
                .inner
                .clicked()
            {
                op = Some(TreeOp::Move(1));
            }
            if ui
                .add_enabled_ui(has && !root_selected, |ui| {
                    icon_button(ui, icons::ARROW_UP, &tr.t("uied.move_up"))
                })
                .inner
                .clicked()
            {
                op = Some(TreeOp::Move(-1));
            }
            if ui
                .add_enabled_ui(has && !root_selected, |ui| {
                    icon_button(ui, icons::COPY, &tr.t("uied.duplicate_widget"))
                })
                .inner
                .clicked()
            {
                op = Some(TreeOp::Duplicate);
            }
            if ui
                .add_enabled_ui(has, |ui| icon_button(ui, icons::PLUS, &tr.t("uied.add_widget")))
                .inner
                .clicked()
            {
                op = Some(TreeOp::Add(st.add_kind));
            }
            let kind = st.add_kind;
            egui::ComboBox::from_id_salt("uied_add_kind")
                .selected_text(format!("{} {}", kind.icon(), tr.t(&format!("uied.kind.{}", kind.name()))))
                .width(110.0)
                .show_ui(ui, |ui| {
                    for k in KindTag::ALL {
                        ui.selectable_value(
                            &mut st.add_kind,
                            k,
                            format!("{} {}", k.icon(), tr.t(&format!("uied.kind.{}", k.name()))),
                        );
                    }
                });
        });
    });
    let tree: Vec<(WidgetPath, usize, String, KindTag)> = st
        .screen()
        .map(|s| {
            model::all_paths(&s.root)
                .into_iter()
                .filter_map(|(p, depth)| {
                    let w = model::get(&s.root, &p)?;
                    Some((p, depth, widget_label(w), KindTag::of(&w.kind)))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut clicked: Option<WidgetPath> = None;
    ScrollArea::vertical()
        .id_salt("uied_tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (path, depth, label, kind) in &tree {
                let selected = *path == st.widget;
                let (rect, resp) = item_row(ui, selected);
                row_text(
                    ui,
                    rect,
                    4.0 + *depth as f32 * 14.0,
                    kind.icon(),
                    label,
                    if selected { accent(ui) } else { weak(ui) },
                );
                if resp.clicked() {
                    clicked = Some(path.clone());
                }
                resp.dnd_set_drag_payload(path.clone());
                if let Some(from) = resp.dnd_hover_payload::<WidgetPath>() {
                    let ok = !path.starts_with(&from) && !from.is_empty();
                    let stroke = egui::Stroke::new(2.0, if ok { accent(ui) } else { danger(ui) });
                    ui.painter().rect_stroke(
                        rect.shrink(1.0),
                        CornerRadius::same(3),
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                if let Some(from) = resp.dnd_release_payload::<WidgetPath>() {
                    // Into a container, else right after the row.
                    let (into, index) = if kind.is_container() {
                        (path.clone(), usize::MAX)
                    } else {
                        match path.split_last() {
                            Some((&i, parent)) => (parent.to_vec(), i + 1),
                            None => (path.clone(), usize::MAX),
                        }
                    };
                    op = Some(TreeOp::Drop { from: (*from).clone(), into, index });
                }
            }
        });
    if let Some(path) = clicked {
        st.widget = path;
    }
    if let Some(op) = op {
        apply_tree_op(st, op);
    }
}

fn apply_tree_op(st: &mut State, op: TreeOp) {
    let (Some(id), Some(doc)) = (st.screen.clone(), st.doc.as_mut()) else {
        return;
    };
    let path = st.widget.clone();
    let new_path = doc.edit(|b| {
        let screen = b.screens.iter_mut().find(|s| s.id == id)?;
        match op {
            TreeOp::Add(kind) => model::add_child(screen, &path, kind.default_widget()),
            TreeOp::Delete => {
                model::remove(screen, &path)?;
                Some(path[..path.len() - 1].to_vec())
            }
            TreeOp::Duplicate => model::duplicate(screen, &path),
            TreeOp::Move(delta) => model::move_by(screen, &path, delta),
            TreeOp::Drop { from, into, index } => model::move_to(screen, &from, &into, index),
        }
    });
    if let Some(p) = new_path {
        st.widget = p;
    }
}

/// The screen's own properties, then the selected widget's.
fn properties(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State, env: &Env) {
    let Some(original) = st.screen().cloned() else {
        ui.label(RichText::new(env.tr.t("uied.select_screen")).color(weak(ui)));
        return;
    };
    // The rename buffer follows the selected screen.
    if st.rename_for != original.id {
        st.rename_for.clone_from(&original.id);
        st.rename.clone_from(&original.id);
    }
    section_title(ui, &env.tr.t("uied.screen_props"));
    let mut rename_to = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(env.tr.t("uied.id")).small().color(weak(ui)));
        let response = ui.add(egui::TextEdit::singleline(&mut st.rename).desired_width(150.0));
        let valid = model::valid_id(&st.rename);
        if !valid {
            ui.label(RichText::new(icons::WARNING).color(danger(ui)))
                .on_hover_text(env.tr.t("uied.invalid_id"));
        }
        if response.lost_focus() && st.rename != original.id {
            if valid {
                rename_to = Some(st.rename.clone());
            } else {
                st.rename.clone_from(&original.id);
            }
        }
    });
    if let Some(new) = rename_to {
        let old = original.id.clone();
        let result = st
            .doc
            .as_mut()
            .map(|d| d.edit(|b| model::rename_screen(b, &old, &new)));
        match result {
            Some(Ok(())) => st.select_screen(&new),
            Some(Err(_)) => {
                ctx.warn(ctx.tf("uied.rename_failed", &[("id", &new)]));
                st.rename.clone_from(&old);
            }
            None => {}
        }
        return;
    }

    // ---- the role of the screen
    let role = st
        .doc
        .as_ref()
        .and_then(|d| model::system_role(&d.bundle, &original.id));
    let mut new_role = role;
    ui.horizontal(|ui| {
        ui.label(RichText::new(env.tr.t("uied.stands_in")).small().color(weak(ui)));
        let label = |r: Option<&str>| match r {
            Some(r) => env.tr.t(&format!("uied.role.{r}")),
            None => env.tr.t("uied.none"),
        };
        egui::ComboBox::from_id_salt("uied_role")
            .selected_text(label(new_role))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut new_role, None, label(None));
                for r in SYSTEM_SCREENS {
                    ui.selectable_value(&mut new_role, Some(r), label(Some(r)));
                }
            });
    });
    if new_role != role
        && let Some(doc) = st.doc.as_mut()
    {
        let id = original.id.clone();
        doc.edit(|b| model::set_system_role(b, &id, new_role));
    }

    // ---- the screen flags, the widget: edit a copy, commit if it changed
    let mut screen = original.clone();
    ui.checkbox(&mut screen.modal, env.tr.t("uied.modal"));
    ui.checkbox(&mut screen.pauses_game, env.tr.t("uied.pauses_game"));
    ui.checkbox(&mut screen.dim_background, env.tr.t("uied.dim_background"));
    ui.label(RichText::new(env.tr.t("uied.on_cancel")).small().color(weak(ui)));
    props::actions_editor(ui, env, "cancel", &mut screen.on_cancel);
    let path = st.widget.clone();
    let is_root = path.is_empty();
    if let Some(w) = model::get_mut(&mut screen.root, &path) {
        props::widget_editor(ui, env, w, is_root);
    }
    if screen != original
        && let Some(doc) = st.doc.as_mut()
    {
        let key = format!("{}:{:?}", original.id, path);
        doc.edit_merged(&key, |b| {
            if let Some(slot) = b.screens.iter_mut().find(|s| s.id == original.id) {
                *slot = screen;
            }
        });
    }
}

// -------------------------------------------------------------- theme tab

fn theme_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let tr = ctx.translator();
    let has_theme = st.doc.as_ref().is_some_and(|d| d.bundle.theme.is_some());
    if !has_theme {
        super::ui_util::card(ui, |ui| {
            ui.label(RichText::new(tr.t("uied.theme_none_title")).strong());
            ui.label(RichText::new(tr.t("uied.theme_none_text")).small().color(weak(ui)));
            if ui
                .button(format!("{}  {}", icons::PALETTE, tr.t("uied.theme_create")))
                .clicked()
                && let Some(doc) = st.doc.as_mut()
            {
                doc.edit(|b| b.theme = UiBundle::builtin().theme);
            }
        });
        return;
    }
    let names: Vec<String> = st
        .doc
        .as_ref()
        .and_then(|d| d.bundle.theme.as_ref())
        .map(|t| t.styles.keys().cloned().collect())
        .unwrap_or_default();
    let images = images_of(ctx);
    let fonts = theme_font_names(st);
    let font_assets = font_assets_of(ctx);
    let root = ctx.project_info().map(|p| p.root.clone());
    let env = Env {
        tr: &tr,
        screens: &[],
        styles: &names,
        images: &images,
        fonts: &fonts,
        vars: &[],
        scenes: &[],
    };
    let contrast = st
        .doc
        .as_ref()
        .and_then(|d| d.bundle.theme.as_ref())
        .map(|t| t.contrast_issues())
        .unwrap_or_default();
    ui.columns(2, |cols| {
        let (left, right) = cols.split_at_mut(1);
        let ui = &mut left[0];
        contrast_section(ui, &tr, &contrast);
        fonts_section(ui, st, &tr, &font_assets, root.as_deref());
        section_title(ui, &tr.t("uied.styles"));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut st.new_style)
                    .hint_text(tr.t("uied.style_name_hint"))
                    .desired_width(ui.available_width() - 34.0),
            );
            if icon_button(ui, icons::PLUS, &tr.t("uied.add_style")).clicked()
                && let Some(doc) = st.doc.as_mut()
            {
                let base = if st.new_style.trim().is_empty() { "style" } else { st.new_style.trim() };
                let base = base.to_owned();
                let name = doc.edit(|b| b.theme.as_mut().map(|t| model::add_style(t, &base)));
                st.style = name;
                st.new_style.clear();
            }
        });
        ScrollArea::vertical()
            .id_salt("uied_styles")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for name in &names {
                    let selected = st.style.as_deref() == Some(name);
                    let (rect, resp) = item_row(ui, selected);
                    row_text(ui, rect, 4.0, icons::PALETTE, name, if selected { accent(ui) } else { weak(ui) });
                    if resp.clicked() {
                        st.style = Some(name.clone());
                    }
                }
            });

        let ui = &mut right[0];
        ScrollArea::vertical()
            .id_salt("uied_style_props")
            .auto_shrink([false, false])
            .show(ui, |ui| style_props(ui, ctx, st, &env));
    });
}

/// The color pairs of the theme under WCAG AA (M7-07), with the reminder
/// that the player's high-contrast mode replaces the colors anyway.
fn contrast_section(ui: &mut Ui, tr: &orbit_editor_api::Translator, issues: &[orbit_ui::ContrastIssue]) {
    if issues.is_empty() {
        return;
    }
    section_title(ui, &tr.t("uied.contrast_title"));
    ui.label(RichText::new(tr.t("uied.contrast_hint")).small().color(weak(ui)));
    for issue in issues {
        let key = format!("uied.contrast.{}", issue.id());
        let mut what = tr.t(&key);
        if what == key {
            what = issue.what.clone();
        }
        if let Some(subject) = &issue.subject {
            what = format!("{what} `{subject}`");
        }
        ui.label(
            RichText::new(format!(
                "{}  {what}: {:.1}:1 ({} {:.1}:1)",
                icons::WARNING,
                issue.ratio,
                tr.t("uied.contrast_needs"),
                issue.required
            ))
            .small()
            .color(warning(ui)),
        );
    }
    ui.add_space(6.0);
}

/// Names of the fonts declared by the theme.
fn theme_font_names(st: &State) -> Vec<String> {
    st.doc
        .as_ref()
        .and_then(|d| d.bundle.theme.as_ref())
        .map(|t| t.fonts.keys().cloned().collect())
        .unwrap_or_default()
}

/// Font assets (paths) of the project.
fn font_assets_of(ctx: &EditorContext<'_>) -> Vec<String> {
    ctx.story()
        .map(|s| {
            s.assets
                .assets
                .iter()
                .filter(|a| a.kind == AssetKind::Font)
                .map(|a| a.path.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Family name a font file provides (`None` when it cannot be read).
fn font_family_of(root: Option<&std::path::Path>, asset: &str) -> Option<String> {
    let bytes = std::fs::read(root?.join(asset)).ok()?;
    orbit_text::font_family(&bytes)
}

/// The fonts of the theme: the project font assets the styles can name.
fn fonts_section(
    ui: &mut Ui,
    st: &mut State,
    tr: &orbit_editor_api::Translator,
    font_assets: &[String],
    root: Option<&std::path::Path>,
) {
    let fonts: Vec<(String, orbit_ui::FontRef)> = st
        .doc
        .as_ref()
        .and_then(|d| d.bundle.theme.as_ref())
        .map(|t| t.fonts.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let mut remove = None;
    let mut add = None;
    egui::CollapsingHeader::new(tr.t("uied.fonts"))
        .id_salt("uied_fonts")
        .default_open(!fonts.is_empty())
        .show(ui, |ui| {
            if fonts.is_empty() {
                ui.label(RichText::new(tr.t("uied.fonts_none")).small().color(weak(ui)));
            }
            for (name, font) in &fonts {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).strong());
                    ui.label(RichText::new(&font.asset).small().color(weak(ui)));
                    if icon_button(ui, icons::TRASH, &tr.t("uied.font_remove")).clicked() {
                        remove = Some(name.clone());
                    }
                });
            }
            let unused: Vec<&String> = font_assets
                .iter()
                .filter(|a| !fonts.iter().any(|(_, f)| &f.asset == *a))
                .collect();
            if font_assets.is_empty() {
                ui.label(RichText::new(tr.t("uied.fonts_import")).small().color(weak(ui)));
            } else if !unused.is_empty() {
                egui::ComboBox::from_id_salt("uied_add_font")
                    .selected_text(format!("{}  {}", icons::PLUS, tr.t("uied.font_add")))
                    .show_ui(ui, |ui| {
                        for asset in unused {
                            if ui.selectable_label(false, asset).clicked() {
                                add = Some(asset.clone());
                            }
                        }
                    });
            }
        });
    let Some(doc) = st.doc.as_mut() else { return };
    if let Some(name) = remove {
        doc.edit(|b| {
            if let Some(t) = b.theme.as_mut() {
                t.fonts.remove(&name);
            }
        });
    }
    if let Some(asset) = add {
        let family = font_family_of(root, &asset).unwrap_or_else(|| {
            std::path::Path::new(&asset)
                .file_stem()
                .map_or_else(|| "font".to_owned(), |s| s.to_string_lossy().into_owned())
        });
        doc.edit(|b| {
            if let Some(t) = b.theme.as_mut() {
                let mut name = family.clone();
                let mut n = 2;
                while t.fonts.contains_key(&name) {
                    name = format!("{family} {n}");
                    n += 1;
                }
                t.fonts.insert(
                    name,
                    orbit_ui::FontRef {
                        asset,
                        family,
                    },
                );
            }
        });
    }
}

fn style_props(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State, env: &Env) {
    let Some(name) = st.style.clone() else {
        ui.label(RichText::new(env.tr.t("uied.select_style")).color(weak(ui)));
        return;
    };
    let Some(original) = st
        .doc
        .as_ref()
        .and_then(|d| d.bundle.theme.as_ref())
        .and_then(|t| t.styles.get(&name))
        .cloned()
    else {
        return;
    };
    if st.style_rename_for != name {
        st.style_rename_for.clone_from(&name);
        st.style_rename.clone_from(&name);
    }
    section_title(ui, &env.tr.t("uied.style_props"));
    let mut remove = false;
    let mut rename_to = None;
    ui.horizontal(|ui| {
        let response = ui.add(egui::TextEdit::singleline(&mut st.style_rename).desired_width(150.0));
        if response.lost_focus() && st.style_rename != name && !st.style_rename.trim().is_empty() {
            rename_to = Some(st.style_rename.trim().to_owned());
        }
        if icon_button(ui, icons::TRASH, &env.tr.t("uied.delete_style")).clicked() {
            remove = true;
        }
    });
    if let Some(new) = rename_to
        && let Some(doc) = st.doc.as_mut()
    {
        let old = name.clone();
        let result = doc.edit(|b| {
            let screens = &mut b.screens;
            match b.theme.as_mut() {
                Some(theme) => model::rename_style(theme, screens, &old, &new),
                None => Err(String::new()),
            }
        });
        match result {
            Ok(()) => st.style = Some(new),
            Err(_) => {
                ctx.warn(ctx.tf("uied.rename_failed", &[("id", &new)]));
                st.style_rename.clone_from(&name);
            }
        }
        return;
    }
    if remove && let Some(doc) = st.doc.as_mut() {
        doc.edit(|b| {
            let screens = &mut b.screens;
            if let Some(theme) = b.theme.as_mut() {
                model::delete_style(theme, screens, &name);
            }
        });
        return;
    }
    let mut style = original.clone();
    props::style_editor(ui, env, &mut style, Some(&name));
    if style != original
        && let Some(doc) = st.doc.as_mut()
    {
        let key = format!("style:{name}");
        doc.edit_merged(&key, |b| {
            if let Some(slot) = b.theme.as_mut().and_then(|t| t.styles.get_mut(&name)) {
                *slot = style;
            }
        });
    }
}

// ------------------------------------------------------------ splash tab

fn splash_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let tr = ctx.translator();
    let images = images_of(ctx);
    ui.columns(2, |cols| {
        let (left, right) = cols.split_at_mut(1);
        let ui = &mut left[0];
        ui.horizontal(|ui| {
            section_title(ui, &tr.t("uied.splashes"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let count = st.doc.as_ref().map_or(0, |d| d.bundle.splashes.len());
                let sel = st.splash;
                if ui
                    .add_enabled_ui(sel.is_some(), |ui| {
                        icon_button(ui, icons::TRASH, &tr.t("uied.delete_splash"))
                    })
                    .inner
                    .clicked()
                    && let (Some(i), Some(doc)) = (sel, st.doc.as_mut())
                {
                    doc.edit(|b| {
                        b.splashes.remove(i);
                    });
                }
                if ui
                    .add_enabled_ui(sel.is_some_and(|i| i + 1 < count), |ui| {
                        icon_button(ui, icons::ARROW_DOWN, &tr.t("uied.move_down"))
                    })
                    .inner
                    .clicked()
                    && let (Some(i), Some(doc)) = (sel, st.doc.as_mut())
                {
                    st.splash = doc.edit(|b| model::move_splash(&mut b.splashes, i, 1));
                }
                if ui
                    .add_enabled_ui(sel.is_some_and(|i| i > 0), |ui| {
                        icon_button(ui, icons::ARROW_UP, &tr.t("uied.move_up"))
                    })
                    .inner
                    .clicked()
                    && let (Some(i), Some(doc)) = (sel, st.doc.as_mut())
                {
                    st.splash = doc.edit(|b| model::move_splash(&mut b.splashes, i, -1));
                }
                ui.menu_button(icons::PLUS, |ui| {
                    ui.label(RichText::new(tr.t("uied.add_image_splash")).small().color(weak(ui)));
                    if images.is_empty() {
                        ui.label(RichText::new(tr.t("uied.no_images")).small().color(faint(ui)));
                    }
                    for img in &images {
                        if ui.button(img).clicked() {
                            if let Some(doc) = st.doc.as_mut() {
                                doc.edit(|b| b.splashes.push(model::new_splash(img)));
                                st.splash = doc.bundle.splashes.len().checked_sub(1);
                            }
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button(tr.t("uied.add_text_splash")).clicked() {
                        if let Some(doc) = st.doc.as_mut() {
                            doc.edit(|b| {
                                b.splashes.push(SplashDef {
                                    content: SplashContent::Text("Studio".into()),
                                    ..model::new_splash("")
                                });
                            });
                            st.splash = doc.bundle.splashes.len().checked_sub(1);
                        }
                        ui.close();
                    }
                })
                .response
                .on_hover_text(tr.t("uied.add_splash"));
            });
        });
        // The engine splash: first, locked.
        let (rect, resp) = item_row(ui, false);
        row_text(ui, rect, 4.0, icons::LOCK, &tr.t("uied.engine_splash"), faint(ui));
        resp.on_hover_text(tr.t("uied.engine_splash_tip"));
        ui.painter_at(rect).text(
            egui::pos2(rect.right() - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            "3 s",
            FontId::proportional(11.0),
            faint(ui),
        );
        let rows: Vec<String> = st
            .doc
            .as_ref()
            .map(|d| {
                d.bundle
                    .splashes
                    .iter()
                    .map(|s| match &s.content {
                        SplashContent::Image(p) => format!("{p}"),
                        SplashContent::Text(t) => format!("“{t}”"),
                        SplashContent::Video(p) => format!("{p} ({})", tr.t("uied.video")),
                    })
                    .collect()
            })
            .unwrap_or_default();
        ScrollArea::vertical()
            .id_salt("uied_splashes")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, label) in rows.iter().enumerate() {
                    let selected = st.splash == Some(i);
                    let (rect, resp) = item_row(ui, selected);
                    row_text(
                        ui,
                        rect,
                        4.0,
                        icons::IMAGE,
                        label,
                        if selected { accent(ui) } else { weak(ui) },
                    );
                    if resp.clicked() {
                        st.splash = Some(i);
                    }
                }
                if rows.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr.t("uied.no_splashes")).small().color(weak(ui)));
                }
            });

        let ui = &mut right[0];
        ScrollArea::vertical()
            .id_salt("uied_splash_props")
            .auto_shrink([false, false])
            .show(ui, |ui| splash_props(ui, ctx, st, &tr));
    });
}

/// The standard image chooser of the editor for a field that stores a project
/// path: thumbnail, the project's images, drop from the asset library and
/// import of a new file.
fn image_path_picker(ui: &mut Ui, ctx: &mut EditorContext<'_>, salt: &str, path: &mut String) {
    let mut imported = None;
    if let Some(story) = ctx.story() {
        let known = story
            .assets
            .assets
            .iter()
            .find(|a| a.kind == AssetKind::Image && a.path == *path);
        if let Some(asset) = known {
            super::thumbs::show(
                ui,
                ctx,
                story,
                asset.id,
                vec2(ui.available_width().min(240.0), 135.0),
            );
            ui.add_space(4.0);
        }
        let text = if path.is_empty() {
            RichText::new(ui.t("uied.splash_pick_image")).color(weak(ui))
        } else if known.is_none() {
            RichText::new(path.as_str()).color(danger(ui))
        } else {
            RichText::new(path.as_str())
        };
        let combo = egui::ComboBox::from_id_salt(salt)
            .selected_text(text)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                let mut any = false;
                for a in story.assets.assets.iter().filter(|a| a.kind == AssetKind::Image) {
                    any = true;
                    ui.selectable_value(path, a.path.clone(), &a.path);
                }
                if !any {
                    ui.label(RichText::new(ui.t("uied.no_images")).small().color(faint(ui)));
                }
            });
        if let Some(id) = orbit_editor_api::fields::asset_drop(ui, &combo.response, AssetKind::Image)
            && let Some(a) = story.assets.get(id)
        {
            path.clone_from(&a.path);
        }
    }
    if ui
        .small_button(format!("{}  {}", icons::FILE_PLUS, ui.t("inspector.asset.add_file")))
        .on_hover_text(ui.t("inspector.asset.add_file.tip"))
        .clicked()
    {
        imported = super::edit::import_asset_dialog(ctx, AssetKind::Image);
    }
    if let Some(id) = imported
        && let Some(a) = ctx.story().and_then(|s| s.assets.get(id))
    {
        path.clone_from(&a.path);
    }
}

fn splash_props(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State, tr: &Translator) {
    let Some(index) = st.splash else {
        ui.label(RichText::new(tr.t("uied.select_splash")).color(weak(ui)));
        return;
    };
    let Some(original) = st.doc.as_ref().and_then(|d| d.bundle.splashes.get(index)).cloned() else {
        return;
    };
    let mut s = original.clone();
    section_title(ui, &tr.t("uied.splash_props"));
    let tag = match &s.content {
        SplashContent::Image(_) => 0,
        SplashContent::Text(_) => 1,
        SplashContent::Video(_) => 2,
    };
    let mut new_tag = tag;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr.t("uied.splash_content")).small().color(weak(ui)));
        egui::ComboBox::from_id_salt("uied_splash_kind")
            .selected_text(tr.t(&format!("uied.splash_kind.{new_tag}")))
            .show_ui(ui, |ui| {
                for k in 0..3 {
                    ui.selectable_value(&mut new_tag, k, tr.t(&format!("uied.splash_kind.{k}")));
                }
            });
    });
    if new_tag != tag {
        let text = match &s.content {
            SplashContent::Image(t) | SplashContent::Text(t) | SplashContent::Video(t) => t.clone(),
        };
        s.content = match new_tag {
            0 => SplashContent::Image(text),
            1 => SplashContent::Text(text),
            _ => SplashContent::Video(text),
        };
    }
    match &mut s.content {
        SplashContent::Image(path) => image_path_picker(ui, ctx, "uied_splash_image", path),
        SplashContent::Video(path) => {
            ui.add(egui::TextEdit::singleline(path).desired_width(f32::INFINITY));
        }
        SplashContent::Text(text) => {
            ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
        }
    }
    egui::Grid::new("uied_splash_grid").num_columns(2).show(ui, |ui| {
        ui.label(RichText::new(tr.t("uied.duration")).small().color(weak(ui)));
        ui.add(egui::DragValue::new(&mut s.duration).speed(0.05).range(0.1..=60.0).suffix(" s"));
        ui.end_row();
        ui.label(RichText::new(tr.t("uied.fade_in")).small().color(weak(ui)));
        ui.add(egui::DragValue::new(&mut s.fade_in).speed(0.05).range(0.0..=30.0).suffix(" s"));
        ui.end_row();
        ui.label(RichText::new(tr.t("uied.fade_out")).small().color(weak(ui)));
        ui.add(egui::DragValue::new(&mut s.fade_out).speed(0.05).range(0.0..=30.0).suffix(" s"));
        ui.end_row();
        ui.label(RichText::new(tr.t("uied.background")).small().color(weak(ui)));
        ui.color_edit_button_srgba_unmultiplied(&mut s.background);
        ui.end_row();
    });
    ui.checkbox(&mut s.skippable, tr.t("uied.skippable"));
    if s != original
        && let Some(doc) = st.doc.as_mut()
    {
        let key = format!("splash:{index}");
        doc.edit_merged(&key, |b| {
            if let Some(slot) = b.splashes.get_mut(index) {
                *slot = s;
            }
        });
    }
}

/// Scenes of the open story as `(id, name)`, for `SceneGoto`.
fn story_scenes(ctx: &EditorContext<'_>) -> Vec<(String, String)> {
    ctx.story()
        .map(|s| s.scenes.iter().map(|sc| (sc.id.to_string(), sc.name.clone())).collect())
        .unwrap_or_default()
}

// ------------------------------------------------------------ diagnostics

fn diagnostics(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(doc) = &st.doc else { return };
    let vars = ctx.story().map(orbit_editor_api::varwidgets::story_vars);
    let ty_of = |n: &str| {
        vars.as_deref()
            .and_then(|v| orbit_editor_api::varwidgets::type_of(v, n))
    };
    let mut issues: Vec<Issue> = doc.diagnostics_with(&ty_of, vars.is_some());
    if ctx.story().is_some() {
        let scenes = story_scenes(ctx);
        let known = |key: &str| scenes.iter().any(|(id, name)| id == key || name == key);
        issues.extend(doc.bundle.scene_issues(&known));
    }
    ui.separator();
    let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
    let warnings = issues.len() - errors;
    ui.horizontal(|ui| {
        section_title(ui, &ctx.t("uied.diagnostics"));
        if issues.is_empty() {
            ui.label(
                RichText::new(format!("{}  {}", icons::CHECK_CIRCLE, ctx.t("uied.no_issues")))
                    .small()
                    .color(super::ui_util::success()),
            );
        } else {
            ui.label(
                RichText::new(ctx.tf(
                    "uied.issue_count",
                    &[("errors", &errors), ("warnings", &warnings)],
                ))
                .small()
                .color(if errors > 0 { danger(ui) } else { warning(ui) }),
            );
        }
    });
    let mut go: Option<(String, String)> = None;
    ScrollArea::vertical()
        .id_salt("uied_issues")
        .max_height(52.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for issue in &issues {
                let (icon, color) = match issue.severity {
                    Severity::Error => (icons::X_CIRCLE, danger(ui)),
                    Severity::Warning => (icons::WARNING, warning(ui)),
                };
                let place = if issue.widget.is_empty() {
                    issue.screen.clone()
                } else {
                    format!("{} › {}", issue.screen, issue.widget)
                };
                let text = format!("{icon}  {place}: {}", issue.message);
                if ui
                    .add(egui::Label::new(RichText::new(text).small().color(color)).sense(Sense::click()))
                    .clicked()
                {
                    go = Some((issue.screen.clone(), issue.widget.clone()));
                }
            }
        });
    if let Some((screen, widget)) = go {
        let path = st
            .doc
            .as_ref()
            .and_then(|d| d.bundle.screen(&screen))
            .and_then(|s| find_widget(&s.root, &widget));
        if st.doc.as_ref().is_some_and(|d| d.bundle.screen(&screen).is_some()) {
            st.tab = Tab::Screens;
            st.select_screen(&screen);
            if let Some(p) = path {
                st.widget = p;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_labels_name_the_widget_and_its_text() {
        let mut w = KindTag::Button.default_widget();
        assert_eq!(widget_label(&w), "button  “Button”");
        w.id = "play".into();
        w.kind = WidgetKind::Button {
            text: orbit_ui::TextRef::Key("ui.menu.new".into()),
            action: vec![],
        };
        assert_eq!(widget_label(&w), "play  (ui.menu.new)");
        assert_eq!(widget_label(&KindTag::Grid.default_widget()), "grid");
    }

    #[test]
    fn a_widget_is_found_by_its_id() {
        let mut screen = UiBundle::builtin().screen("pause").cloned().unwrap();
        let path = find_widget(&screen.root, "save").unwrap();
        assert_eq!(model::get(&screen.root, &path).unwrap().id, "save");
        assert!(find_widget(&screen.root, "").is_none());
        assert!(find_widget(&screen.root, "ghost").is_none());
        model::remove(&mut screen, &path);
        assert!(find_widget(&screen.root, "save").is_none());
    }

    #[test]
    fn the_panel_draws_without_a_project() {
        let ctx = egui::Context::default();
        let state = Rc::new(RefCell::new(State::default()));
        // `State::sync` needs an `EditorContext`; the pure parts are tested in
        // `model`. Here only the state's selection logic.
        let dir = std::env::temp_dir().join(format!("orbit-uied-sel-{}", std::process::id()));
        let mut doc = UiDoc::load(&dir).unwrap();
        doc.enable_defaults();
        let mut st = state.borrow_mut();
        st.doc = Some(doc);
        st.screen = Some("ghost".into());
        st.widget = vec![9, 9];
        st.ensure_selection();
        assert_eq!(st.screen.as_deref(), Some("main_menu"));
        assert!(st.widget.is_empty());
        assert!(st.style.is_some());
        st.splash = Some(4);
        st.ensure_selection();
        assert_eq!(st.splash, None);
        let _ = ctx;
    }
}
