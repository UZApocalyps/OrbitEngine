//! Property editors of the UI editor: a widget, a style, the small enums
//! and lists they are made of. Every editor mutates the value it is given;
//! the caller compares it with the original to know whether something changed.

use egui::{ComboBox, DragValue, RichText, Ui};
use orbit_core::{Condition, Expr};
use orbit_editor_api::varwidgets::{self, VarInfo};
use orbit_editor_api::{Translator, icons};
use orbit_ui::{
    Align, Anchor, BindPath, Border, Cond, Dir, Fill, LayoutDef, PanelKind, SettingKey, Size,
    StateColors, Style, StyleSounds, TextRef, UiAction, WidgetDef, WidgetKind,
};

use super::model::{BUILTIN_SCREEN_IDS, KindTag};
use crate::builtin::ui_util::{faint, weak};

/// What the editors need to offer choices and speak the editor's language.
pub struct Env<'a> {
    pub tr: &'a Translator,
    /// Ids of the screens of the project.
    pub screens: &'a [String],
    /// Names of the styles of the theme.
    pub styles: &'a [String],
    /// Image assets of the project (paths).
    pub images: &'a [String],
    /// Names of the theme's fonts (`Theme::fonts`).
    pub fonts: &'a [String],
    /// Variables declared by the story (for conditions, operations, bindings).
    pub vars: &'a [VarInfo],
    /// Scenes of the story as `(id, name)` (targets of `SceneGoto`).
    pub scenes: &'a [(String, String)],
}

impl Env<'_> {
    /// Label of an enum variant (`uied.v.<name>`).
    fn v(&self, name: &str) -> String {
        self.tr.t(&format!("uied.v.{name}"))
    }

    fn t(&self, key: &str) -> String {
        self.tr.t(key)
    }
}

const SETTINGS: [(SettingKey, &str); 9] = [
    (SettingKey::MasterVolume, "master_volume"),
    (SettingKey::MusicVolume, "music_volume"),
    (SettingKey::SfxVolume, "sfx_volume"),
    (SettingKey::VoiceVolume, "voice_volume"),
    (SettingKey::TextSpeed, "text_speed"),
    (SettingKey::AutoDelay, "auto_delay"),
    (SettingKey::SkipMode, "skip_mode"),
    (SettingKey::Fullscreen, "fullscreen"),
    (SettingKey::Language, "language"),
];

fn setting_name(key: SettingKey) -> &'static str {
    SETTINGS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("master_volume", |(_, n)| n)
}

/// A caption on the left, the editor on the right.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [92.0, 20.0],
            egui::Label::new(RichText::new(label).small().color(weak(ui))).truncate(),
        );
        add(ui);
    });
}

/// The value a new dropdown option sets, by variable type.
fn default_value(ty: orbit_core::ValueType) -> orbit_core::Value {
    use orbit_core::{Value, ValueType};
    match ty {
        ValueType::Bool => Value::Bool(false),
        ValueType::Int => Value::Int(0),
        ValueType::Float => Value::Float(0.0),
        ValueType::Str => Value::Str(String::new()),
    }
}

/// A typed value: a check box, a number or a text.
fn value_editor(ui: &mut Ui, value: &mut orbit_core::Value) {
    use orbit_core::Value;
    match value {
        Value::Bool(b) => {
            ui.checkbox(b, "");
        }
        Value::Int(i) => {
            ui.add(DragValue::new(i).speed(0.2));
        }
        Value::Float(f) => {
            ui.add(DragValue::new(f).speed(0.05).max_decimals(3));
        }
        Value::Str(s) => {
            ui.add(egui::TextEdit::singleline(s).desired_width(80.0));
        }
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).strong());
    ui.separator();
}

fn num(ui: &mut Ui, value: &mut f32, speed: f32) {
    ui.add(DragValue::new(value).speed(speed).max_decimals(2));
}

fn color(ui: &mut Ui, c: &mut [u8; 4]) {
    ui.color_edit_button_srgba_unmultiplied(c);
}

/// An optional value: a checkbox that adds / removes it, then the editor.
fn optional<T: Clone>(
    ui: &mut Ui,
    value: &mut Option<T>,
    default: T,
    edit: impl FnOnce(&mut Ui, &mut T),
) {
    let mut on = value.is_some();
    if ui.checkbox(&mut on, "").changed() {
        *value = on.then(|| default.clone());
    }
    if let Some(v) = value {
        edit(ui, v);
    }
}

/// A combo over `(id, label)` options, setting `current` to the picked id.
fn pick<T: Copy + PartialEq>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    current: &mut T,
    options: &[(T, String)],
) {
    let text = options
        .iter()
        .find(|(v, _)| v == current)
        .map(|(_, l)| l.as_str())
        .unwrap_or("?");
    ComboBox::from_id_salt(salt)
        .selected_text(text)
        .show_ui(ui, |ui| {
            for (v, label) in options {
                ui.selectable_value(current, *v, label);
            }
        });
}

// ------------------------------------------------------------------ enums

fn align_editor(ui: &mut Ui, env: &Env, salt: &str, a: &mut Align) {
    let options = [
        (Align::Start, env.v("start")),
        (Align::Center, env.v("center")),
        (Align::End, env.v("end")),
        (Align::Stretch, env.v("stretch")),
    ];
    pick(ui, salt, a, &options);
}

fn size_editor(ui: &mut Ui, env: &Env, salt: &str, size: &mut Size) {
    let tag = match size {
        Size::Fit => 0,
        Size::Fixed(_) => 1,
        Size::Fill(_) => 2,
    };
    let mut new = tag;
    pick(
        ui,
        salt,
        &mut new,
        &[(0, env.v("fit")), (1, env.v("fixed")), (2, env.v("fill"))],
    );
    if new != tag {
        *size = match new {
            0 => Size::Fit,
            1 => Size::Fixed(200.0),
            _ => Size::Fill(1.0),
        };
    }
    match size {
        Size::Fixed(v) | Size::Fill(v) => num(ui, v, 1.0),
        Size::Fit => {}
    }
}

fn dir_editor(ui: &mut Ui, env: &Env, salt: &str, dir: &mut Dir) {
    let tag = match dir {
        Dir::Vertical => 0,
        Dir::Horizontal => 1,
        Dir::Stack => 2,
        Dir::Grid(_) => 3,
    };
    let mut new = tag;
    pick(
        ui,
        salt,
        &mut new,
        &[
            (0, env.v("vertical")),
            (1, env.v("horizontal")),
            (2, env.v("stack")),
            (3, env.v("grid")),
        ],
    );
    if new != tag {
        *dir = match new {
            0 => Dir::Vertical,
            1 => Dir::Horizontal,
            2 => Dir::Stack,
            _ => Dir::Grid(2),
        };
    }
    if let Dir::Grid(columns) = dir {
        ui.add(DragValue::new(columns).range(1..=12));
    }
}

fn cond_editor(ui: &mut Ui, env: &Env, salt: &str, cond: &mut Cond) {
    let (negated, base) = match &*cond {
        Cond::Not(inner) => (true, (**inner).clone()),
        other => (false, other.clone()),
    };
    let names = [
        (Cond::Always, "always"),
        (Cond::Never, "never"),
        (Cond::CanContinue, "can_continue"),
        (Cond::CanSave, "can_save"),
        (Cond::ShowQuit, "show_quit"),
        (Cond::InGame, "in_game"),
    ];
    let vars_index = names.len();
    let mut index = match &base {
        Cond::Vars(_) => vars_index,
        other => names.iter().position(|(c, _)| c == other).unwrap_or(0),
    };
    let mut neg = negated;
    let mut options: Vec<(usize, String)> = names
        .iter()
        .enumerate()
        .map(|(i, (_, n))| (i, env.v(n)))
        .collect();
    options.push((vars_index, env.v("vars")));
    pick(ui, salt, &mut index, &options);
    ui.checkbox(&mut neg, env.t("uied.not"));
    let mut base = if index == vars_index {
        match base {
            Cond::Vars(c) => Cond::Vars(c),
            _ => Cond::Vars(varwidgets::blank_row(env.vars)),
        }
    } else {
        names[index].0.clone()
    };
    if let Cond::Vars(c) = &mut base {
        ui.vertical(|ui| vars_condition_editor(ui, env, salt, c));
    }
    *cond = if neg { Cond::Not(Box::new(base)) } else { base };
}

/// The no-code condition builder of a `Cond::Vars`, with an "expression
/// mode" for the advanced.
fn vars_condition_editor(ui: &mut Ui, env: &Env, salt: &str, cond: &mut Condition) {
    if let Condition::Expr(e) = cond {
        if let Some(parsed) = expr_text(ui, (salt, "expr"), e) {
            *e = parsed;
        }
        if let Some(structured) = Some(Condition::from_expr(e)).filter(Condition::is_structured)
            && ui.small_button(env.t("varop.visual_mode")).clicked()
        {
            *cond = structured;
        }
    } else {
        if let Some(new) = varwidgets::condition_editor(ui, (salt, "vars"), env.vars, cond) {
            *cond = new;
        }
        if ui.small_button(env.t("varop.expr_mode")).clicked() {
            *cond = Condition::Expr(cond.to_expr());
        }
    }
    let ty_of = |n: &str| varwidgets::type_of(env.vars, n);
    if !env.vars.is_empty()
        && let Some(issue) = cond.issues(&ty_of).first()
    {
        ui.label(
            RichText::new(env.tr.format("varop.issue", &[("message", issue)]))
                .small()
                .color(ui.visuals().warn_fg_color),
        );
    }
}

/// An expression as text: returns the parsed expression after an edit that
/// parses (a text with an error is flagged and not applied).
fn expr_text(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, e: &Expr) -> Option<Expr> {
    let id = ui.make_persistent_id(("ui_expr_text", salt));
    let mut text = ui
        .data(|d| d.get_temp::<(String, Expr)>(id))
        .filter(|(_, source)| source == e)
        .map_or_else(|| e.to_string(), |(t, _)| t);
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(egui::TextStyle::Monospace)
            .desired_width(180.0),
    );
    let mut out = None;
    let mut source = e.clone();
    if response.changed() {
        match Expr::parse(&text) {
            Ok(parsed) => {
                source = parsed.clone();
                out = Some(parsed);
            }
            Err(err) => {
                ui.label(RichText::new(err.to_string()).small().color(ui.visuals().error_fg_color));
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(id, (text, source)));
    out
}

fn bind_editor(ui: &mut Ui, env: &Env, salt: &str, bind: &mut BindPath) {
    let is_setting = matches!(bind, BindPath::Setting(_));
    let mut new = is_setting;
    pick(
        ui,
        (salt, "kind"),
        &mut new,
        &[(true, env.v("setting")), (false, env.v("var"))],
    );
    if new != is_setting {
        *bind = if new {
            BindPath::Setting(SettingKey::MasterVolume)
        } else {
            BindPath::Var(String::new())
        };
    }
    match bind {
        BindPath::Setting(key) => {
            let options: Vec<(SettingKey, String)> = SETTINGS
                .iter()
                .map(|(k, n)| (*k, env.v(n)))
                .collect();
            let _ = setting_name(*key);
            pick(ui, (salt, "key"), key, &options);
        }
        BindPath::Var(name) => {
            if env.vars.is_empty() {
                ui.add(egui::TextEdit::singleline(name).desired_width(120.0));
            } else {
                let text = if name.is_empty() { env.t("varop.choose_var") } else { name.clone() };
                ComboBox::from_id_salt((salt, "var"))
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        for v in env.vars {
                            ui.selectable_value(name, v.name.clone(), &v.name);
                        }
                    });
            }
        }
    }
}

fn text_ref_editor(ui: &mut Ui, env: &Env, salt: &str, text: &mut TextRef) {
    let tag = match text {
        TextRef::Key(_) => 0,
        TextRef::Literal(_) => 1,
        TextRef::Bind(_) => 2,
    };
    let mut new = tag;
    pick(
        ui,
        (salt, "kind"),
        &mut new,
        &[(0, env.v("key")), (1, env.v("literal")), (2, env.v("bind"))],
    );
    if new != tag {
        *text = match new {
            0 => TextRef::Key(String::new()),
            1 => TextRef::Literal(String::new()),
            _ => TextRef::Bind(BindPath::Setting(SettingKey::MasterVolume)),
        };
    }
    match text {
        TextRef::Key(s) | TextRef::Literal(s) => {
            ui.add(egui::TextEdit::singleline(s).desired_width(f32::INFINITY));
        }
        TextRef::Bind(bind) => bind_editor(ui, env, &format!("{salt}b"), bind),
    }
}

/// Text field + a menu of known ids.
fn id_editor(ui: &mut Ui, env: &Env, salt: &str, id: &mut String, with_builtin: bool) {
    ui.add(egui::TextEdit::singleline(id).desired_width(110.0));
    ui.menu_button(icons::CARET_DOWN, |ui| {
        for s in env.screens {
            if ui.button(s).clicked() {
                id.clone_from(s);
                ui.close();
            }
        }
        if with_builtin {
            ui.separator();
            for s in BUILTIN_SCREEN_IDS {
                if !env.screens.iter().any(|x| x == s) && ui.button(s).clicked() {
                    *id = s.to_owned();
                    ui.close();
                }
            }
        }
    });
    let _ = salt;
}

/// Target of a `SceneGoto`: a list of the story's scenes (stores the scene
/// id, shows its name). Without a story, a plain text field.
fn scene_editor(ui: &mut Ui, env: &Env, salt: &str, scene: &mut String) {
    if env.scenes.is_empty() {
        ui.add(egui::TextEdit::singleline(scene).desired_width(140.0));
        return;
    }
    let current = scene_label(env.scenes, scene);
    let text = match &current {
        Some(name) => name.clone(),
        None if scene.trim().is_empty() => env.t("uied.scene_pick"),
        None => format!("{} ({scene})", env.t("uied.scene_unknown")),
    };
    ComboBox::from_id_salt((salt, "scene"))
        .selected_text(text)
        .show_ui(ui, |ui| {
            for (id, name) in env.scenes {
                if ui.selectable_label(scene == id, name).clicked() {
                    scene.clone_from(id);
                }
            }
        });
}

/// Name of the scene `key` designates (by id, or by name), if any.
fn scene_label(scenes: &[(String, String)], key: &str) -> Option<String> {
    let key = key.trim();
    scenes
        .iter()
        .find(|(id, _)| id == key)
        .or_else(|| scenes.iter().find(|(_, name)| name == key))
        .map(|(_, name)| name.clone())
}

// ---------------------------------------------------------------- actions

const ACTION_NAMES: [&str; 17] = [
    "game_new",
    "game_continue",
    "game_quit",
    "return_to_title",
    "open",
    "replace",
    "close",
    "close_all",
    "quick_save",
    "quick_load",
    "open_save",
    "open_load",
    "reset_preferences",
    "play_sound",
    "custom",
    "scene_goto",
    "var_set",
];

fn action_index(a: &UiAction) -> usize {
    match a {
        UiAction::GameNew => 0,
        UiAction::GameContinue => 1,
        UiAction::GameQuit => 2,
        UiAction::ReturnToTitle => 3,
        UiAction::Open(_) => 4,
        UiAction::Replace(_) => 5,
        UiAction::Close => 6,
        UiAction::CloseAll => 7,
        UiAction::QuickSave => 8,
        UiAction::QuickLoad => 9,
        UiAction::OpenSave => 10,
        UiAction::OpenLoad => 11,
        UiAction::ResetPreferences => 12,
        UiAction::PlaySound(_) => 13,
        UiAction::Custom { .. } => 14,
        UiAction::SceneGoto(_) => 15,
        UiAction::VarSet(_) => 16,
    }
}

fn action_from_index(i: usize, vars: &[VarInfo]) -> UiAction {
    match i {
        0 => UiAction::GameNew,
        1 => UiAction::GameContinue,
        2 => UiAction::GameQuit,
        3 => UiAction::ReturnToTitle,
        4 => UiAction::Open(String::new()),
        5 => UiAction::Replace(String::new()),
        6 => UiAction::Close,
        7 => UiAction::CloseAll,
        8 => UiAction::QuickSave,
        9 => UiAction::QuickLoad,
        10 => UiAction::OpenSave,
        11 => UiAction::OpenLoad,
        12 => UiAction::ResetPreferences,
        13 => UiAction::PlaySound(String::new()),
        14 => UiAction::Custom {
            name: String::new(),
            args: Vec::new(),
        },
        15 => UiAction::SceneGoto(String::new()),
        _ => UiAction::VarSet(varwidgets::blank_op(vars)),
    }
}

fn action_editor(ui: &mut Ui, env: &Env, salt: &str, action: &mut UiAction) {
    let index = action_index(action);
    let mut new = index;
    let options: Vec<(usize, String)> = ACTION_NAMES
        .iter()
        .enumerate()
        .map(|(i, n)| (i, env.v(n)))
        .collect();
    pick(ui, (salt, "kind"), &mut new, &options);
    if new != index {
        *action = action_from_index(new, env.vars);
    }
    match action {
        UiAction::Open(id) | UiAction::Replace(id) => id_editor(ui, env, salt, id, true),
        UiAction::PlaySound(path) => {
            ui.add(egui::TextEdit::singleline(path).desired_width(140.0));
        }
        UiAction::SceneGoto(scene) => scene_editor(ui, env, salt, scene),
        UiAction::Custom { name, args } => {
            ui.add(egui::TextEdit::singleline(name).desired_width(90.0));
            let mut joined = args.join(", ");
            if ui
                .add(
                    egui::TextEdit::singleline(&mut joined)
                        .hint_text(env.t("uied.args_hint"))
                        .desired_width(110.0),
                )
                .changed()
            {
                *args = joined
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
        UiAction::VarSet(op) => {
            ui.vertical(|ui| {
                if let Some(new) = varwidgets::varop_editor(ui, (salt, "op"), env.vars, op, true) {
                    *op = new;
                }
            });
        }
        _ => {}
    }
    if !action.is_supported() {
        ui.label(RichText::new(icons::WARNING).color(ui.visuals().warn_fg_color))
            .on_hover_text(env.t("uied.unsupported"));
    }
}

/// The ordered list of actions of a button / the screen's cancel.
pub fn actions_editor(ui: &mut Ui, env: &Env, salt: &str, actions: &mut Vec<UiAction>) {
    let mut remove = None;
    let mut swap = None;
    let count = actions.len();
    for (i, action) in actions.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            action_editor(ui, env, &format!("{salt}{i}"), action);
            if ui.small_button(icons::ARROW_UP).clicked() && i > 0 {
                swap = Some((i, i - 1));
            }
            if ui.small_button(icons::ARROW_DOWN).clicked() && i + 1 < count {
                swap = Some((i, i + 1));
            }
            if ui.small_button(icons::X).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some((a, b)) = swap {
        actions.swap(a, b);
    }
    if let Some(i) = remove {
        actions.remove(i);
    }
    if ui
        .button(format!("{}  {}", icons::PLUS, env.t("uied.add_action")))
        .clicked()
    {
        actions.push(UiAction::Close);
    }
}

// ----------------------------------------------------------------- widget

/// The properties of one widget: identity, kind data, layout, style.
pub fn widget_editor(ui: &mut Ui, env: &Env, w: &mut WidgetDef, is_root: bool) {
    let tag = KindTag::of(&w.kind);
    heading(
        ui,
        &format!("{}  {}", tag.icon(), env.t(&format!("uied.kind.{}", tag.name()))),
    );
    row(ui, &env.t("uied.id"), |ui| {
        ui.add(egui::TextEdit::singleline(&mut w.id).desired_width(f32::INFINITY));
    });

    // ---- kind data
    match &mut w.kind {
        WidgetKind::Panel { kind, dir } => {
            row(ui, &env.t("uied.panel_kind"), |ui| {
                pick(
                    ui,
                    "panel_kind",
                    kind,
                    &[
                        (PanelKind::Normal, env.v("normal")),
                        (PanelKind::Dim, env.v("dim")),
                        (PanelKind::Solid, env.v("solid")),
                    ],
                );
            });
            row(ui, &env.t("uied.dir"), |ui| dir_editor(ui, env, "dir", dir));
        }
        WidgetKind::Stack { dir } => {
            row(ui, &env.t("uied.dir"), |ui| dir_editor(ui, env, "dir", dir));
        }
        WidgetKind::Grid { columns } => {
            row(ui, &env.t("uied.columns"), |ui| {
                ui.add(DragValue::new(columns).range(1..=12));
            });
        }
        WidgetKind::Button { text, action } => {
            row(ui, &env.t("uied.text"), |ui| {
                text_ref_editor(ui, env, "text", text);
            });
            ui.add_space(4.0);
            ui.label(RichText::new(env.t("uied.actions")).small().color(weak(ui)));
            actions_editor(ui, env, "act", action);
        }
        WidgetKind::Label {
            text,
            size,
            center,
            dim,
        } => {
            row(ui, &env.t("uied.text"), |ui| {
                text_ref_editor(ui, env, "text", text);
            });
            row(ui, &env.t("uied.font_size"), |ui| {
                optional(ui, size, 30.0, |ui, v| num(ui, v, 0.5));
            });
            row(ui, "", |ui| {
                ui.checkbox(center, env.t("uied.center"));
                ui.checkbox(dim, env.t("uied.dim_text"));
            });
        }
        WidgetKind::Image { asset } => {
            row(ui, &env.t("uied.image"), |ui| {
                let mut path = asset.clone().unwrap_or_default();
                ui.add(egui::TextEdit::singleline(&mut path).desired_width(140.0));
                ui.menu_button(icons::CARET_DOWN, |ui| {
                    if ui.button("@menu_background").clicked() {
                        path = "@menu_background".into();
                        ui.close();
                    }
                    for img in env.images {
                        if ui.button(img).clicked() {
                            path.clone_from(img);
                            ui.close();
                        }
                    }
                });
                *asset = (!path.is_empty()).then_some(path);
            });
        }
        WidgetKind::Slider {
            bind,
            min,
            max,
            step,
        } => {
            row(ui, &env.t("uied.bind"), |ui| bind_editor(ui, env, "bind", bind));
            row(ui, &env.t("uied.range"), |ui| {
                num(ui, min, 0.05);
                ui.label("→");
                num(ui, max, 0.05);
            });
            row(ui, &env.t("uied.step"), |ui| {
                optional(ui, step, 0.1, |ui, v| num(ui, v, 0.01));
            });
        }
        WidgetKind::Toggle { text, bind } => {
            row(ui, &env.t("uied.text"), |ui| {
                text_ref_editor(ui, env, "text", text);
            });
            row(ui, &env.t("uied.bind"), |ui| bind_editor(ui, env, "bind", bind));
        }
        WidgetKind::Dropdown {
            bind,
            options,
            values,
        } => {
            row(ui, &env.t("uied.bind"), |ui| bind_editor(ui, env, "bind", bind));
            // A variable dropdown gives each option the value it sets.
            let var_ty = bind
                .var()
                .and_then(|name| env.vars.iter().find(|v| v.name == name))
                .map(|v| v.ty);
            let by_var = bind.var().is_some();
            if by_var {
                let ty = var_ty.unwrap_or(orbit_core::ValueType::Str);
                values.resize_with(options.len(), || default_value(ty));
                values.truncate(options.len());
            } else {
                values.clear();
            }
            ui.label(RichText::new(env.t("uied.options")).small().color(weak(ui)));
            let mut remove = None;
            for (i, option) in options.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    text_ref_editor(ui, env, &format!("opt{i}"), option);
                    if by_var && let Some(value) = values.get_mut(i) {
                        ui.label("→");
                        value_editor(ui, value);
                    }
                    if ui.small_button(icons::X).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                options.remove(i);
                if by_var && i < values.len() {
                    values.remove(i);
                }
            }
            if ui
                .button(format!("{}  {}", icons::PLUS, env.t("uied.add_option")))
                .clicked()
            {
                options.push(TextRef::Literal(String::new()));
                if by_var {
                    values.push(default_value(var_ty.unwrap_or(orbit_core::ValueType::Str)));
                }
            }
            if by_var && var_ty.is_none() {
                ui.label(
                    RichText::new(env.t("uied.dropdown_pick_var"))
                        .small()
                        .color(weak(ui)),
                );
            }
        }
        WidgetKind::ScrollList => {}
        WidgetKind::SaveSlots { mode, columns, rows } => {
            row(ui, &env.t("uied.slots_mode"), |ui| {
                pick(
                    ui,
                    "slots_mode",
                    mode,
                    &[
                        (orbit_ui::SlotsMode::Save, env.v("slots_save")),
                        (orbit_ui::SlotsMode::Load, env.v("slots_load")),
                    ],
                );
            });
            row(ui, &env.t("uied.columns"), |ui| {
                ui.add(DragValue::new(columns).range(1..=8));
            });
            row(ui, &env.t("uied.rows"), |ui| {
                ui.add(DragValue::new(rows).range(1..=8));
            });
        }
    }

    // ---- conditions
    heading(ui, &env.t("uied.conditions"));
    row(ui, &env.t("uied.visible"), |ui| {
        cond_editor(ui, env, "visible", &mut w.visible);
    });
    row(ui, &env.t("uied.enabled"), |ui| {
        cond_editor(ui, env, "enabled", &mut w.enabled);
    });

    // ---- layout
    heading(ui, &env.t("uied.layout"));
    layout_editor(ui, env, &mut w.layout, is_root);

    // ---- style
    heading(ui, &env.t("uied.style"));
    row(ui, &env.t("uied.style_name"), |ui| {
        let mut name = w.style.clone().unwrap_or_default();
        let text = if name.is_empty() {
            env.t("uied.none")
        } else {
            name.clone()
        };
        if env.styles.is_empty() {
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0));
        } else {
            ComboBox::from_id_salt("style_name")
                .selected_text(text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut name, String::new(), env.t("uied.none"));
                    for s in env.styles {
                        ui.selectable_value(&mut name, s.clone(), s);
                    }
                    // A name the theme does not have stays selectable.
                    if !name.is_empty() && !env.styles.contains(&name) {
                        ui.label(RichText::new(&name).color(ui.visuals().error_fg_color));
                    }
                });
        }
        w.style = (!name.is_empty()).then_some(name);
    });
    ui.collapsing(env.t("uied.overrides"), |ui| {
        style_editor(ui, env, &mut w.overrides, None);
    });
}

fn layout_editor(ui: &mut Ui, env: &Env, l: &mut LayoutDef, is_root: bool) {
    row(ui, &env.t("uied.width"), |ui| size_editor(ui, env, "w", &mut l.width));
    row(ui, &env.t("uied.height"), |ui| size_editor(ui, env, "h", &mut l.height));
    row(ui, &env.t("uied.padding"), |ui| {
        num(ui, &mut l.pad[0], 1.0);
        num(ui, &mut l.pad[1], 1.0);
    });
    row(ui, &env.t("uied.margin"), |ui| {
        for m in &mut l.margin {
            num(ui, m, 1.0);
        }
    });
    row(ui, &env.t("uied.gap"), |ui| num(ui, &mut l.gap, 1.0));
    row(ui, &env.t("uied.span"), |ui| {
        // Columns / rows taken inside a grid (1 = one cell).
        ui.add(DragValue::new(&mut l.col_span).range(1..=12));
        ui.label("×");
        ui.add(DragValue::new(&mut l.row_span).range(1..=12));
    });
    row(ui, &env.t("uied.main_align"), |ui| align_editor(ui, env, "main", &mut l.main));
    row(ui, &env.t("uied.cross_align"), |ui| {
        align_editor(ui, env, "cross", &mut l.cross);
    });
    if is_root {
        row(ui, &env.t("uied.anchor"), |ui| {
            optional(ui, &mut l.anchor, Anchor::FULL, |ui, a| {
                ui.vertical(|ui| {
                    // Presets: the canvas point and the pivot go together.
                    ui.horizontal(|ui| {
                        for (label, p) in [
                            (icons::ARROW_UP_LEFT, [0.0, 0.0]),
                            (icons::ARROW_UP, [0.5, 0.0]),
                            (icons::ARROW_UP_RIGHT, [1.0, 0.0]),
                            (icons::ARROW_LEFT, [0.0, 0.5]),
                            (icons::DOT_OUTLINE, [0.5, 0.5]),
                            (icons::ARROW_RIGHT, [1.0, 0.5]),
                            (icons::ARROW_DOWN_LEFT, [0.0, 1.0]),
                            (icons::ARROW_DOWN, [0.5, 1.0]),
                            (icons::ARROW_DOWN_RIGHT, [1.0, 1.0]),
                        ] {
                            if ui.small_button(label).clicked() {
                                a.canvas = p;
                                a.pivot = p;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(env.t("uied.canvas")).small().color(faint(ui)));
                        num(ui, &mut a.canvas[0], 0.01);
                        num(ui, &mut a.canvas[1], 0.01);
                        ui.label(RichText::new(env.t("uied.pivot")).small().color(faint(ui)));
                        num(ui, &mut a.pivot[0], 0.01);
                        num(ui, &mut a.pivot[1], 0.01);
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(env.t("uied.offset")).small().color(faint(ui)));
                        num(ui, &mut a.offset[0], 1.0);
                        num(ui, &mut a.offset[1], 1.0);
                    });
                });
            });
        });
    }
}

// ------------------------------------------------------------------ style

fn state_colors(ui: &mut Ui, env: &Env, colors: &mut StateColors) {
    for (label, slot) in [
        ("uied.state.normal", &mut colors.normal),
        ("uied.state.hover", &mut colors.hover),
        ("uied.state.pressed", &mut colors.pressed),
        ("uied.state.disabled", &mut colors.disabled),
    ] {
        row(ui, &env.t(label), |ui| {
            optional(ui, slot, [60, 66, 90, 255], |ui, c| color(ui, c));
        });
    }
}

/// A style (theme entry or widget override). `exclude` is the name of the
/// style itself, which cannot extend itself.
pub fn style_editor(ui: &mut Ui, env: &Env, s: &mut Style, exclude: Option<&str>) {
    row(ui, &env.t("uied.extends"), |ui| {
        let mut name = s.extends.clone().unwrap_or_default();
        let text = if name.is_empty() {
            env.t("uied.none")
        } else {
            name.clone()
        };
        ComboBox::from_id_salt("extends")
            .selected_text(text)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut name, String::new(), env.t("uied.none"));
                for other in env.styles.iter().filter(|o| Some(o.as_str()) != exclude) {
                    ui.selectable_value(&mut name, other.clone(), other);
                }
            });
        s.extends = (!name.is_empty()).then_some(name);
    });
    row(ui, &env.t("uied.font"), |ui| {
        let mut font = s.font.clone().unwrap_or_default();
        // The project fonts of the theme, or a family name typed by hand.
        if !env.fonts.is_empty() {
            ComboBox::from_id_salt("style_font")
                .selected_text(if font.is_empty() {
                    env.t("uied.font_default")
                } else {
                    font.clone()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut font, String::new(), env.t("uied.font_default"));
                    for name in env.fonts {
                        ui.selectable_value(&mut font, name.clone(), name);
                    }
                });
        }
        ui.add(egui::TextEdit::singleline(&mut font).desired_width(100.0));
        s.font = (!font.is_empty()).then_some(font);
    });
    row(ui, &env.t("uied.font_size"), |ui| {
        optional(ui, &mut s.text_size, 30.0, |ui, v| num(ui, v, 0.5));
    });
    row(ui, &env.t("uied.padding"), |ui| {
        optional(ui, &mut s.padding, [20.0, 12.0], |ui, p| {
            num(ui, &mut p[0], 1.0);
            num(ui, &mut p[1], 1.0);
        });
    });
    fill_editor(ui, env, &mut s.fill);
    row(ui, &env.t("uied.border"), |ui| {
        optional(
            ui,
            &mut s.border,
            Border {
                color: [255, 255, 255, 80],
                width: 2.0,
            },
            |ui, b| {
                color(ui, &mut b.color);
                num(ui, &mut b.width, 0.1);
            },
        );
    });
    ui.label(RichText::new(env.t("uied.bg_colors")).small().color(weak(ui)));
    state_colors(ui, env, &mut s.colors);
    ui.label(RichText::new(env.t("uied.text_colors")).small().color(weak(ui)));
    state_colors(ui, env, &mut s.text_colors);
    row(ui, &env.t("uied.sound_hover"), |ui| {
        sound_field(ui, s, true);
    });
    row(ui, &env.t("uied.sound_confirm"), |ui| {
        sound_field(ui, s, false);
    });
}

/// Kind of a style background, for the picker.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FillKind {
    None,
    Color,
    NineSlice,
}

fn fill_kind(fill: &Option<Fill>) -> FillKind {
    match fill {
        None => FillKind::None,
        Some(Fill::Color(_)) => FillKind::Color,
        Some(Fill::NineSlice { .. }) => FillKind::NineSlice,
    }
}

/// The background of a style: nothing, a color, or a 9-slice image with
/// its four insets (pixels of the image kept as borders).
fn fill_editor(ui: &mut Ui, env: &Env, fill: &mut Option<Fill>) {
    row(ui, &env.t("uied.fill"), |ui| {
        let mut kind = fill_kind(fill);
        let before = kind;
        pick(
            ui,
            "fill_kind",
            &mut kind,
            &[
                (FillKind::None, env.t("uied.none")),
                (FillKind::Color, env.v("fill_color")),
                (FillKind::NineSlice, env.v("fill_nine_slice")),
            ],
        );
        if kind != before {
            *fill = match kind {
                FillKind::None => None,
                FillKind::Color => Some(Fill::Color([40, 44, 66, 255])),
                FillKind::NineSlice => Some(Fill::NineSlice {
                    image: env.images.first().cloned().unwrap_or_default(),
                    insets: [16.0; 4],
                }),
            };
        }
        if let Some(Fill::Color(c)) = fill {
            color(ui, c);
        }
    });
    if let Some(Fill::NineSlice { image, insets }) = fill {
        row(ui, &env.t("uied.nine_image"), |ui| {
            let text = if image.is_empty() {
                env.t("uied.none")
            } else {
                image.clone()
            };
            ComboBox::from_id_salt("nine_image")
                .selected_text(text)
                .width(160.0)
                .show_ui(ui, |ui| {
                    for img in env.images {
                        ui.selectable_value(image, img.clone(), img);
                    }
                });
        });
        for (label, slot) in [
            ("uied.inset_left", 0),
            ("uied.inset_top", 1),
            ("uied.inset_right", 2),
            ("uied.inset_bottom", 3),
        ] {
            row(ui, &env.t(label), |ui| {
                ui.add(DragValue::new(&mut insets[slot]).speed(0.5).range(0.0..=4096.0));
                ui.label(RichText::new("px").small().color(weak(ui)));
            });
        }
    }
}

fn sound_field(ui: &mut Ui, s: &mut Style, hover: bool) {
    let sounds = s.sounds.get_or_insert_with(StyleSounds::default);
    let slot = if hover {
        &mut sounds.hover
    } else {
        &mut sounds.confirm
    };
    let mut path = slot.clone().unwrap_or_default();
    ui.add(egui::TextEdit::singleline(&mut path).desired_width(f32::INFINITY));
    *slot = (!path.is_empty()).then_some(path);
    if sounds.hover.is_none() && sounds.confirm.is_none() {
        s.sounds = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_targets_show_the_scene_name() {
        let scenes = vec![
            ("01JBX0S0000000000000000001".to_owned(), "prologue".to_owned()),
            ("01JBX0S0000000000000000002".to_owned(), "epilogue".to_owned()),
        ];
        assert_eq!(scene_label(&scenes, "01JBX0S0000000000000000002").as_deref(), Some("epilogue"));
        assert_eq!(scene_label(&scenes, "prologue").as_deref(), Some("prologue"));
        assert_eq!(scene_label(&scenes, "nope"), None);
    }

    #[test]
    fn action_indices_round_trip_for_every_variant() {
        for i in 0..ACTION_NAMES.len() {
            assert_eq!(action_index(&action_from_index(i, &[])), i);
        }
        assert_eq!(ACTION_NAMES.len(), 17);
        for (key, name) in SETTINGS {
            assert_eq!(setting_name(key), name);
        }
    }
}
