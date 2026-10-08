//! Property editors of the UI editor: a widget, a style, the small enums
//! and lists they are made of. Every editor mutates the value it is given;
//! the caller compares it with the original to know whether something changed.

use egui::{ComboBox, DragValue, RichText, Ui};
use orbit_core::{Condition, Expr, Operand};
use orbit_editor_api::varwidgets::{self, VarInfo};
use orbit_editor_api::{Translator, icons};
use orbit_ui::{
    AchievementField, Align, Anchor, BindPath, Cond, Dir, LayoutDef, PanelKind, SettingKey, Size, SlotField, Style,
    TextRef, UiAction, WidgetDef, WidgetKind,
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
    /// The story's named curves (easings of the screen transitions).
    pub curves: &'a std::collections::BTreeMap<String, orbit_core::Curve>,
    /// The theme being edited (styles to resolve parents, widget defaults
    /// for the previews).
    pub theme: Option<&'a orbit_ui::Theme>,
    /// Audio assets of the project (paths), for style sounds.
    pub sounds: &'a [String],
    /// Music (and sound) files of the project, for screen music.
    pub musics: &'a [String],
    /// Thumbnails of the image assets.
    pub lookup: &'a super::style_ed::ImageLookup,
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

const SETTINGS: [(SettingKey, &str); 13] = [
    (SettingKey::MasterVolume, "master_volume"),
    (SettingKey::MusicVolume, "music_volume"),
    (SettingKey::SfxVolume, "sfx_volume"),
    (SettingKey::VoiceVolume, "voice_volume"),
    (SettingKey::TextSpeed, "text_speed"),
    (SettingKey::AutoDelay, "auto_delay"),
    (SettingKey::SkipMode, "skip_mode"),
    (SettingKey::Fullscreen, "fullscreen"),
    (SettingKey::Language, "language"),
    (SettingKey::ScreenReader, "screen_reader"),
    (SettingKey::TextScale, "text_scale"),
    (SettingKey::HighContrast, "high_contrast"),
    (SettingKey::ReducedMotion, "reduced_motion"),
];

fn setting_name(key: SettingKey) -> &'static str {
    SETTINGS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("master_volume", |(_, n)| n)
}

/// A caption on the left, the editor on the right.
pub(super) fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
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
    ty.default_value()
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
        Value::List(..) | Value::Map(..) => {
            if let Some(v) = orbit_editor_api::fields::value_editor(ui, value) {
                *value = v;
            }
        }
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).strong());
    ui.separator();
}

pub(super) fn num(ui: &mut Ui, value: &mut f32, speed: f32) {
    ui.add(DragValue::new(value).speed(speed).max_decimals(2));
}

pub(super) fn color(ui: &mut Ui, c: &mut [u8; 4]) {
    ui.color_edit_button_srgba_unmultiplied(c);
}

/// An optional value: a checkbox that adds / removes it, then the editor.
pub(super) fn optional<T: Clone>(
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
pub(super) fn pick<T: Copy + PartialEq>(
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
        (Cond::Item, "item_set"),
        (Cond::Unlocked, "unlocked"),
        (Cond::HasAchievements, "has_achievements"),
        (Cond::SlotEmpty, "slot_empty"),
        (Cond::SlotUsed, "slot_used"),
        (Cond::SlotLocked, "slot_locked"),
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

/// Family of a bind path (the kind combo of [`bind_editor`]).
fn bind_tag(bind: &BindPath) -> usize {
    match bind {
        BindPath::Setting(_) => 0,
        BindPath::Var(_) => 1,
        BindPath::Item => 2,
        BindPath::ItemIndex => 3,
        BindPath::ItemKey => 4,
        BindPath::Param(_) => 5,
        BindPath::Gallery(_) => 6,
        BindPath::ReplayScenes(_) => 7,
        BindPath::Achievements => 8,
        BindPath::Achievement(_) => 9,
        BindPath::SlotField(_) => 10,
    }
}

/// `contextual`: also offer the values only a screen has (the item, index
/// or key of a generated row, a screen parameter): for texts.
/// The fields a save slot card template can show.
const SLOT_FIELDS: [(SlotField, &str); 9] = [
    (SlotField::Title, "slot_title"),
    (SlotField::Number, "slot_number"),
    (SlotField::Timestamp, "slot_timestamp"),
    (SlotField::Date, "slot_date"),
    (SlotField::PlayTime, "slot_play_time"),
    (SlotField::Chapter, "slot_chapter"),
    (SlotField::Preview, "slot_preview"),
    (SlotField::Status, "slot_status"),
    (SlotField::Thumbnail, "slot_thumbnail"),
];

fn bind_editor(ui: &mut Ui, env: &Env, salt: &str, bind: &mut BindPath, contextual: bool) {
    let tag = bind_tag(bind);
    let mut new = tag;
    let mut options = vec![(0, env.v("setting")), (1, env.v("var"))];
    if contextual || tag >= 2 {
        options.push((2, env.v("item")));
        options.push((3, env.v("item_index")));
        options.push((4, env.v("item_key")));
        options.push((5, env.v("param")));
        options.push((9, env.v("achievement")));
        options.push((10, env.v("slot_field")));
    }
    pick(ui, (salt, "kind"), &mut new, &options);
    if new != tag {
        *bind = match new {
            0 => BindPath::Setting(SettingKey::MasterVolume),
            1 => BindPath::Var(String::new()),
            2 => BindPath::Item,
            3 => BindPath::ItemIndex,
            4 => BindPath::ItemKey,
            9 => BindPath::Achievement(AchievementField::Title),
            10 => BindPath::SlotField(SlotField::Title),
            _ => BindPath::Param(String::new()),
        };
    }
    match bind {
        BindPath::Item
        | BindPath::ItemIndex
        | BindPath::ItemKey
        | BindPath::Gallery(_)
        | BindPath::ReplayScenes(_)
        | BindPath::Achievements => {}
        BindPath::Achievement(field) => {
            let options = [
                (AchievementField::Title, env.v("ach_title")),
                (AchievementField::Description, env.v("ach_description")),
                (AchievementField::Icon, env.v("ach_icon")),
                (AchievementField::Progress, env.v("ach_progress")),
            ];
            pick(ui, (salt, "ach_field"), field, &options);
        }
        BindPath::SlotField(field) => {
            let options: Vec<(SlotField, String)> = SLOT_FIELDS
                .iter()
                .map(|(f, name)| (*f, env.v(name)))
                .collect();
            pick(ui, (salt, "slot_field"), field, &options);
        }
        BindPath::Param(name) => {
            ui.add(
                egui::TextEdit::singleline(name)
                    .hint_text(env.t("uied.param_hint"))
                    .desired_width(120.0),
            );
        }
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
        TextRef::Bind(bind) => bind_editor(ui, env, &format!("{salt}b"), bind, true),
    }
}

/// The styles and sizes of a `SaveSlots` list (`ui_version` 9).
fn slots_look_editor(ui: &mut Ui, env: &Env, look: &mut orbit_ui::SlotsLook) {
    ui.collapsing(env.t("uied.slots_look"), |ui| {
        let styles: [(&str, &mut Option<String>); 8] = [
            ("uied.slots_card_style", &mut look.card_style),
            ("uied.slots_empty_style", &mut look.empty_style),
            ("uied.slots_locked_style", &mut look.locked_style),
            ("uied.slots_tab_style", &mut look.tab_style),
            ("uied.slots_pager_style", &mut look.pager_style),
            ("uied.slots_delete_style", &mut look.delete_style),
            ("uied.slots_confirm_style", &mut look.confirm_style),
            ("uied.slots_confirm_button_style", &mut look.confirm_button_style),
        ];
        for (key, slot) in styles {
            row(ui, &env.t(key), |ui| style_slot_combo(ui, env, key, slot));
        }
        for (key, value, default) in [
            ("uied.slots_card_height", &mut look.card_height, 170.0),
            ("uied.slots_column_gap", &mut look.column_gap, 24.0),
            ("uied.slots_row_gap", &mut look.row_gap, 18.0),
        ] {
            row(ui, &env.t(key), |ui| {
                optional(ui, value, default, |ui, v| {
                    ui.add(DragValue::new(v).range(0.0..=1080.0).speed(1.0));
                });
            });
        }
        ui.label(RichText::new(env.t("uied.slots_look_tip")).small().color(weak(ui)));
    });
}

/// A combo of the theme's styles for an optional style name.
fn style_slot_combo(ui: &mut Ui, env: &Env, salt: &str, slot: &mut Option<String>) {
    let mut name = slot.clone().unwrap_or_default();
    let text = if name.is_empty() {
        env.t("uied.none")
    } else {
        name.clone()
    };
    ComboBox::from_id_salt(("slot_style", salt))
        .selected_text(text)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut name, String::new(), env.t("uied.none"));
            for s in env.styles {
                ui.selectable_value(&mut name, s.clone(), s);
            }
            if !name.is_empty() && !env.styles.contains(&name) {
                ui.label(RichText::new(&name).color(ui.visuals().error_fg_color));
            }
        });
    *slot = (!name.is_empty()).then_some(name);
}

/// A combo of the project's audio files for a music path.
pub(super) fn music_file_picker(
    ui: &mut Ui,
    env: &Env,
    salt: impl std::hash::Hash + std::fmt::Debug,
    path: &mut String,
) {
    let text = if path.is_empty() {
        env.t("uied.none")
    } else {
        path.clone()
    };
    ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(160.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(path, String::new(), env.t("uied.none"));
            for s in env.musics {
                ui.selectable_value(path, s.clone(), s);
            }
            // A file the project no longer lists stays visible.
            if !path.is_empty() && !env.musics.contains(path) {
                ui.label(RichText::new(path.as_str()).color(ui.visuals().error_fg_color));
            }
        });
}

/// The music of a screen (`ui_version` 9): a track, fades, volume, what
/// happens over a running game.
pub(super) fn music_editor(ui: &mut Ui, env: &Env, value: &mut Option<orbit_ui::ScreenMusic>) {
    use orbit_ui::{OverGame, ScreenMusic};
    ui.horizontal(|ui| {
        let mut on = value.is_some();
        if ui.checkbox(&mut on, env.t("uied.music")).changed() {
            *value = on.then(ScreenMusic::default);
        }
    });
    let Some(m) = value else { return };
    row(ui, &env.t("uied.music_file"), |ui| music_file_picker(ui, env, "screen_music", &mut m.asset));
    if !m.asset.is_empty() && !env.musics.contains(&m.asset) {
        ui.label(
            RichText::new(format!("{}  {}", icons::WARNING, env.t("uied.music_missing")))
                .small()
                .color(ui.visuals().warn_fg_color),
        );
    }
    row(ui, &env.t("uied.music_volume"), |ui| {
        ui.add(egui::Slider::new(&mut m.volume, 0.0..=1.0).max_decimals(2));
    });
    row(ui, &env.t("uied.music_fades"), |ui| {
        ui.add(DragValue::new(&mut m.fade_in_ms).range(0..=10_000).speed(10.0).suffix(" ms"))
            .on_hover_text(env.t("uied.music_fade_in"));
        ui.add(DragValue::new(&mut m.fade_out_ms).range(0..=10_000).speed(10.0).suffix(" ms"))
            .on_hover_text(env.t("uied.music_fade_out"));
    });
    ui.checkbox(&mut m.looping, env.t("uied.music_loop"));
    ui.checkbox(&mut m.keep_if_same, env.t("uied.music_keep_same"))
        .on_hover_text(env.t("uied.music_keep_same_tip"));
    row(ui, &env.t("uied.music_over_game"), |ui| {
        pick(
            ui,
            "music_over_game",
            &mut m.over_game,
            &[
                (OverGame::Replace, env.v("music_replace")),
                (OverGame::Duck, env.v("music_duck")),
            ],
        );
    });
    match m.over_game {
        OverGame::Replace => {
            ui.checkbox(&mut m.resume_game_music, env.t("uied.music_resume"))
                .on_hover_text(env.t("uied.music_resume_tip"));
        }
        OverGame::Duck => {
            row(ui, &env.t("uied.music_duck_level"), |ui| {
                ui.add(egui::Slider::new(&mut m.duck_volume, 0.0..=1.0).max_decimals(2));
            });
        }
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

/// Target of a `ReplayScene`: the scene of the row (in a
/// `ReplayScenes` list) or a fixed scene.
fn replay_target_editor(ui: &mut Ui, env: &Env, salt: &str, o: &mut Operand) {
    let mut row = matches!(o, Operand::Item);
    if ui
        .checkbox(&mut row, env.t("uied.replay_row"))
        .on_hover_text(env.t("uied.replay_row_tip"))
        .changed()
    {
        *o = if row { Operand::Item } else { Operand::Lit(orbit_core::Value::Str(String::new())) };
    }
    if let Operand::Lit(orbit_core::Value::Str(scene)) = o {
        scene_editor(ui, env, salt, scene);
    }
}

/// The eligible entries of a gallery / replay list: a checkbox per
/// candidate `(key, label)`; none checked = all of them.
fn eligible_editor(ui: &mut Ui, env: &Env, list: &mut Vec<String>, candidates: &[(String, String)]) {
    ui.vertical(|ui| {
        ui.label(RichText::new(env.t("uied.eligible_tip")).small().color(weak(ui)));
        for (key, label) in candidates {
            let mut on = list.contains(key);
            if ui.checkbox(&mut on, label).changed() {
                if on {
                    // Keep the story order.
                    list.push(key.clone());
                    let order = |k: &String| candidates.iter().position(|(c, _)| c == k);
                    list.sort_by_key(order);
                } else {
                    list.retain(|k| k != key);
                }
            }
        }
        // Entries that are no longer candidates (renamed, deleted).
        let stale: Vec<String> = list
            .iter()
            .filter(|k| !candidates.iter().any(|(c, _)| c == *k))
            .cloned()
            .collect();
        for key in stale {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{key} ({})", env.t("uied.scene_unknown"))).color(ui.visuals().warn_fg_color));
                if ui.small_button(icons::X).clicked() {
                    list.retain(|k| *k != key);
                }
            });
        }
    });
}

// ---------------------------------------------------------------- actions

const ACTION_NAMES: [&str; 20] = [
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
    "replay_scene",
    "play_music",
    "stop_music",
];

fn action_index(a: &UiAction) -> usize {
    match a {
        UiAction::GameNew => 0,
        UiAction::GameContinue => 1,
        UiAction::GameQuit => 2,
        UiAction::ReturnToTitle => 3,
        UiAction::Open(_) => 4,
        UiAction::Replace(_) => 5,
        UiAction::Close | UiAction::CloseWith(_) => 6,
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
        UiAction::ReplayScene(_) => 17,
        UiAction::PlayMusic(_) => 18,
        UiAction::StopMusic => 19,
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
        17 => UiAction::ReplayScene(Operand::Item),
        18 => UiAction::PlayMusic(String::new()),
        19 => UiAction::StopMusic,
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
        UiAction::PlayMusic(path) => music_file_picker(ui, env, (salt, "music"), path),
        UiAction::SceneGoto(scene) => scene_editor(ui, env, salt, scene),
        UiAction::ReplayScene(o) => replay_target_editor(ui, env, salt, o),
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
                // Inside a screen the value can also be the item / index /
                // key of a generated row or a screen parameter.
                if let Some(new) = contextual_value_editor(ui, env, (salt, "opctx"), op) {
                    *op = new;
                }
            });
        }
        UiAction::Close | UiAction::CloseWith(_) => {
            let mut result = match action {
                UiAction::CloseWith(o) => Some(o.clone()),
                _ => None,
            };
            let mut on = result.is_some();
            if ui
                .checkbox(&mut on, env.t("uied.return_value"))
                .on_hover_text(env.t("uied.return_value_tip"))
                .changed()
            {
                result = on.then(|| Operand::Lit(orbit_core::Value::Bool(true)));
            }
            if let Some(o) = &result
                && let Some(new) = varwidgets::screen_operand_editor(
                    ui,
                    (salt, "ret"),
                    env.vars,
                    orbit_core::ValueType::Str,
                    o,
                )
            {
                result = Some(new);
            }
            *action = match result {
                Some(o) => UiAction::CloseWith(o),
                None => UiAction::Close,
            };
        }
        _ => {}
    }
    if !action.is_supported() {
        ui.label(RichText::new(icons::WARNING).color(ui.visuals().warn_fg_color))
            .on_hover_text(env.t("uied.unsupported"));
    }
}

/// The contextual operands (item / index / key / parameter) as a value of a
/// `VarSet`: a menu to use one, and the full picker once one is used.
fn contextual_value_editor(
    ui: &mut Ui,
    env: &Env,
    salt: impl orbit_editor_api::fields::Salt,
    op: &orbit_core::VarOp,
) -> Option<orbit_core::VarOp> {
    let ty = varwidgets::type_of(env.vars, &op.var)?;
    let elem = match ty {
        orbit_core::ValueType::List(s) | orbit_core::ValueType::Map(s) => s.value_type(),
        other => other,
    };
    if op.value.is_contextual() {
        let new = varwidgets::screen_operand_editor(ui, salt, env.vars, elem, &op.value)?;
        return Some(orbit_core::VarOp {
            value: new,
            ..op.clone()
        });
    }
    let mut picked = None;
    ui.menu_button(env.t("uied.value_from_screen"), |ui| {
        for (key, operand) in [
            ("item", Operand::Item),
            ("item_index", Operand::ItemIndex),
            ("item_key", Operand::ItemKey),
            ("param", Operand::Param(String::new())),
        ] {
            if ui.button(env.v(key)).clicked() {
                picked = Some(operand);
                ui.close();
            }
        }
    });
    picked.map(|value| orbit_core::VarOp {
        value,
        ..op.clone()
    })
}

/// A screen's opening / closing transition: none, or kind + duration +
/// easing curve.
pub fn screen_transition_editor(
    ui: &mut Ui,
    env: &Env,
    salt: &str,
    caption: &str,
    value: &mut Option<orbit_ui::ScreenTransition>,
) {
    use orbit_ui::{ScreenTransition, ScreenTransitionKind};
    ui.horizontal(|ui| {
        let mut on = value.is_some();
        if ui.checkbox(&mut on, caption).changed() {
            *value = on.then(ScreenTransition::default);
        }
        if let Some(t) = value {
            let options: Vec<(ScreenTransitionKind, String)> = ScreenTransitionKind::ALL
                .iter()
                .map(|k| (*k, env.t(&format!("uied.transition.{}", k.key()))))
                .collect();
            pick(ui, (salt, "tkind"), &mut t.kind, &options);
            ui.add(
                DragValue::new(&mut t.duration)
                    .range(0.0..=5.0)
                    .speed(0.01)
                    .max_decimals(2)
                    .suffix(" s"),
            );
        }
    });
    if let Some(t) = value
        && t.duration > 0.0
    {
        let ease = t.ease.clone().unwrap_or_default();
        let header = format!(
            "{}: {}",
            env.t("curve.easing"),
            orbit_editor_api::curve_editor::curve_label(&ease)
        );
        egui::CollapsingHeader::new(header)
            .id_salt((salt, "ease"))
            .show(ui, |ui| {
                let options = orbit_editor_api::curve_editor::CurveEditorOptions {
                    graph_size: 110.0,
                    ..Default::default()
                };
                if let Some(c) = orbit_editor_api::curve_editor::curve_editor(
                    ui,
                    (salt, "curve"),
                    &ease,
                    env.curves,
                    options,
                ) {
                    t.ease = (c != orbit_core::Curve::LINEAR).then_some(c);
                }
            });
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
        WidgetKind::Image { asset, bind, action } => {
            row(ui, "", |ui| {
                let mut from_row = bind.is_some();
                if ui
                    .checkbox(&mut from_row, env.t("uied.image_from_row"))
                    .on_hover_text(env.t("uied.image_from_row_tip"))
                    .changed()
                {
                    *bind = from_row.then_some(BindPath::Item);
                }
                if let Some(b) = bind {
                    bind_editor(ui, env, "image_bind", b, true);
                }
            });
            // A row image shows the element of the row instead of `asset`.
            if bind.is_none() {
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
            ui.add_space(4.0);
            ui.label(RichText::new(env.t("uied.image_actions")).small().color(weak(ui)))
                .on_hover_text(env.t("uied.image_actions_tip"));
            actions_editor(ui, env, "img_act", action);
        }
        WidgetKind::Slider {
            bind,
            min,
            max,
            step,
        } => {
            row(ui, &env.t("uied.bind"), |ui| bind_editor(ui, env, "bind", bind, false));
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
            row(ui, &env.t("uied.bind"), |ui| bind_editor(ui, env, "bind", bind, false));
        }
        WidgetKind::Dropdown {
            bind,
            options,
            values,
        } => {
            row(ui, &env.t("uied.bind"), |ui| bind_editor(ui, env, "bind", bind, false));
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
        WidgetKind::Repeat { source, dir, empty } => {
            row(ui, &env.t("uied.repeat_source"), |ui| {
                // 0 variable, 1 parameter, 2 gallery, 3 scene replay.
                let kind = match source {
                    BindPath::Param(_) => 1,
                    BindPath::Gallery(_) => 2,
                    BindPath::ReplayScenes(_) => 3,
                    BindPath::Achievements => 4,
                    _ => 0,
                };
                let mut want = kind;
                pick(
                    ui,
                    "repeat_src_kind",
                    &mut want,
                    &[
                        (0, env.v("var")),
                        (1, env.v("param")),
                        (2, env.v("gallery")),
                        (3, env.v("replay_scenes")),
                        (4, env.v("achievements")),
                    ],
                );
                if want != kind {
                    *source = match want {
                        1 => BindPath::Param(String::new()),
                        2 => BindPath::Gallery(Vec::new()),
                        3 => BindPath::ReplayScenes(Vec::new()),
                        4 => BindPath::Achievements,
                        _ => BindPath::Var(String::new()),
                    };
                }
                match source {
                    BindPath::Param(name) => {
                        ui.add(
                            egui::TextEdit::singleline(name)
                                .hint_text(env.t("uied.param_hint"))
                                .desired_width(120.0),
                        );
                    }
                    BindPath::Var(name) => {
                        // Only list and map variables can be repeated.
                        let lists: Vec<&VarInfo> = env
                            .vars
                            .iter()
                            .filter(|v| {
                                matches!(
                                    v.ty,
                                    orbit_core::ValueType::List(_) | orbit_core::ValueType::Map(_)
                                )
                            })
                            .collect();
                        if lists.is_empty() && env.vars.is_empty() {
                            ui.add(egui::TextEdit::singleline(name).desired_width(120.0));
                        } else {
                            let text = if name.is_empty() {
                                env.t("uied.repeat_pick_list")
                            } else {
                                name.clone()
                            };
                            ComboBox::from_id_salt("repeat_src_var")
                                .selected_text(text)
                                .show_ui(ui, |ui| {
                                    for v in &lists {
                                        ui.selectable_value(name, v.name.clone(), &v.name);
                                    }
                                });
                        }
                    }
                    _ => {}
                }
            });
            // Which images / scenes the gallery and the replay menu offer
            // (M7-03, M7-04): stored in the screen, not in the story.
            match source {
                BindPath::Gallery(list) => {
                    let candidates: Vec<(String, String)> =
                        env.images.iter().map(|p| (p.clone(), p.clone())).collect();
                    row(ui, &env.t("uied.gallery_images"), |ui| {
                        eligible_editor(ui, env, list, &candidates);
                    });
                }
                BindPath::ReplayScenes(list) => {
                    row(ui, &env.t("uied.replay_scenes"), |ui| {
                        eligible_editor(ui, env, list, env.scenes);
                    });
                }
                _ => {}
            }
            row(ui, &env.t("uied.dir"), |ui| dir_editor(ui, env, "dir", dir));
            row(ui, &env.t("uied.repeat_empty"), |ui| {
                ui.vertical(|ui| {
                    optional(ui, empty, TextRef::Literal(String::new()), |ui, t| {
                        text_ref_editor(ui, env, "empty", t);
                    });
                });
            });
            ui.label(
                RichText::new(env.t("uied.repeat_tip"))
                    .small()
                    .color(weak(ui)),
            );
        }
        WidgetKind::SaveSlots { mode, columns, rows, look } => {
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
            slots_look_editor(ui, env, look);
            // The card template is the widget's children: a button writes
            // the stock card as a tree to edit, another goes back to it.
            ui.add_space(4.0);
            if w.children.is_empty() {
                ui.label(RichText::new(env.t("uied.slots_stock_card")).small().color(weak(ui)));
                if ui
                    .button(env.t("uied.slots_customize"))
                    .on_hover_text(env.t("uied.slots_customize_tip"))
                    .clicked()
                {
                    let widgets = env.theme.map(|t| t.widgets.clone()).unwrap_or_default();
                    w.children = orbit_ui::default_slot_card(&widgets);
                }
            } else {
                ui.label(RichText::new(env.t("uied.slots_template")).small().color(weak(ui)));
                if ui
                    .button(env.t("uied.slots_reset"))
                    .on_hover_text(env.t("uied.slots_reset_tip"))
                    .clicked()
                {
                    w.children.clear();
                }
            }
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
        let parent = w.style.clone();
        ui.label(
            RichText::new(env.t("uied.overrides_hint"))
                .small()
                .color(weak(ui)),
        );
        style_editor(ui, env, &mut w.overrides, parent.as_deref());
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

/// The local overrides of a widget, on top of its named style (the
/// visual style editor, see [`super::style_ed`]).
pub fn style_editor(ui: &mut Ui, env: &Env, s: &mut Style, parent_name: Option<&str>) {
    let parent = parent_name
        .zip(env.theme)
        .and_then(|(name, theme)| orbit_ui::resolve_style(&theme.styles, name));
    super::style_ed::style_editor(ui, env, env.lookup, s, parent.as_ref(), None, "overrides", false);
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
        assert_eq!(ACTION_NAMES.len(), 20);
        for (key, name) in SETTINGS {
            assert_eq!(setting_name(key), name);
        }
    }
}
