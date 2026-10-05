//! [`DataScreen`]: a [`Screen`] built from a [`ScreenAsset`].

use std::collections::{BTreeMap, BTreeSet, HashMap};

use orbit_core::{Value, VarOp};
use orbit_render2d::TextureId;

use orbit_ui_data::{BindPath, SettingKey, UiAction};
use super::slots::{SlotAction, build_slots, slots_page};
use orbit_ui_data::{Cond, ScreenAsset, SlotsMode, TextRef, WidgetDef, WidgetKind};
use crate::input::UiInput;
use crate::screens::{Confirm, Screen};
use crate::texts::Texts;
use crate::theme::{Style, Theme};
use crate::view::{AudioBus, CategoryView, PrefsView, SkipMode, SlotState, UiEvent};
use crate::widgets::{Dir, DrawCtx, Node, NodeId, PanelKind, Tag, Ui, WidgetEvent};
use orbit_text::TextAlign;

/// Name of the image asset standing for the title background of the game.
pub const MENU_BACKGROUND: &str = "@menu_background";

/// What the shell tells a data screen about the game: the data source of
/// its conditions and bindings.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DataContext {
    pub prefs: PrefsView,
    pub can_continue: bool,
    pub can_save: bool,
    pub show_quit: bool,
    pub in_game: bool,
    pub background: Option<TextureId>,
    pub title: String,
    /// The save slots, for the `SaveSlots` widgets.
    pub saves: Vec<CategoryView>,
    /// Story variables (both scopes) the conditions and bindings read.
    pub vars: BTreeMap<String, Value>,
    /// Textures of the project images the theme uses (nine-slice
    /// backgrounds) and of the gallery images, by asset path.
    pub images: HashMap<String, TextureId>,
    /// Gallery (M7-03): the images a `Gallery` source with no list shows
    /// (every background of the story, in story order), by asset path.
    pub gallery: Vec<String>,
    /// Asset paths of the images unlocked in a played game.
    pub unlocked: BTreeSet<String>,
    /// Scene replay (M7-04): every scene of the story in order, as
    /// (id, name, seen in a played game).
    pub scenes: Vec<(String, String, bool)>,
    /// Achievements (M7-05): every achievement of the story in order, as the
    /// player sees it (placeholders for the hidden locked ones).
    pub achievements: Vec<AchievementRow>,
}

/// One achievement as a data screen shows it (M7-05).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AchievementRow {
    pub id: String,
    /// Localized title (a placeholder while hidden and locked).
    pub title: String,
    /// Localized description (a placeholder while hidden and locked).
    pub description: String,
    /// Asset path of the icon; empty for none (or hidden and locked).
    pub icon: String,
    pub unlocked: bool,
}

impl DataContext {
    /// The rows of a `Gallery` source: the path of each unlocked image of
    /// `list` (or of [`DataContext::gallery`] when empty), `""` when locked.
    pub fn gallery_rows(&self, list: &[String]) -> Vec<Value> {
        let list = if list.is_empty() { &self.gallery } else { list };
        list.iter()
            .map(|p| Value::Str(if self.unlocked.contains(p) { p.clone() } else { String::new() }))
            .collect()
    }

    /// The rows of an `Achievements` source: the ids, in story order.
    pub fn achievement_rows(&self) -> Vec<Value> {
        self.achievements.iter().map(|a| Value::Str(a.id.clone())).collect()
    }

    fn achievement(&self, item: Option<&Value>) -> Option<&AchievementRow> {
        match item {
            Some(Value::Str(id)) => self.achievements.iter().find(|a| &a.id == id),
            _ => None,
        }
    }

    /// `3 / 10`: unlocked achievements out of all.
    pub fn achievement_progress(&self) -> String {
        let done = self.achievements.iter().filter(|a| a.unlocked).count();
        format!("{done} / {}", self.achievements.len())
    }

    /// The rows of a `ReplayScenes` source: the names of the seen scenes
    /// named (by id or name) in `list`, every seen scene when it is empty.
    pub fn replay_rows(&self, list: &[String]) -> Vec<Value> {
        self.scenes
            .iter()
            .filter(|(id, name, seen)| {
                *seen && (list.is_empty() || list.iter().any(|k| k.trim() == id || k.trim() == name))
            })
            .map(|(_, name, _)| Value::Str(name.clone()))
            .collect()
    }
}

impl DataContext {
    pub fn eval(&self, cond: &Cond) -> bool {
        self.eval_with(cond, None)
    }

    /// [`DataContext::eval`] inside a generated row whose element is `item`
    /// (read by `Cond::Item`).
    pub fn eval_with(&self, cond: &Cond, item: Option<&Value>) -> bool {
        match cond {
            Cond::Always => true,
            Cond::Never => false,
            Cond::CanContinue => self.can_continue,
            Cond::CanSave => self.can_save,
            Cond::ShowQuit => self.show_quit,
            Cond::InGame => self.in_game,
            Cond::Not(c) => !self.eval_with(c, item),
            Cond::Vars(c) => c.eval(&|n| self.vars.get(n).cloned()).unwrap_or(false),
            Cond::Unlocked => self.achievement(item).is_some_and(|a| a.unlocked),
            Cond::HasAchievements => !self.achievements.is_empty(),
            Cond::Item => item.is_some_and(|v| match v {
                Value::Bool(b) => *b,
                Value::Int(i) => *i != 0,
                Value::Float(f) => *f != 0.0,
                Value::Str(s) => !s.is_empty(),
                Value::List(_, l) => !l.is_empty(),
                Value::Map(_, m) => !m.is_empty(),
            }),
        }
    }

    /// Numeric value of a setting (booleans and choices have none).
    pub fn setting_f32(&self, key: SettingKey) -> Option<f32> {
        let p = &self.prefs;
        Some(match key {
            SettingKey::MasterVolume => p.master,
            SettingKey::MusicVolume => p.music,
            SettingKey::SfxVolume => p.sfx,
            SettingKey::VoiceVolume => p.voice,
            SettingKey::TextSpeed => p.text_speed,
            SettingKey::AutoDelay => p.auto_delay,
            SettingKey::TextScale => p.text_scale,
            _ => return None,
        })
    }

    fn set_setting_f32(&mut self, key: SettingKey, v: f32) {
        let p = &mut self.prefs;
        match key {
            SettingKey::MasterVolume => p.master = v,
            SettingKey::MusicVolume => p.music = v,
            SettingKey::SfxVolume => p.sfx = v,
            SettingKey::VoiceVolume => p.voice = v,
            SettingKey::TextSpeed => p.text_speed = v,
            SettingKey::AutoDelay => p.auto_delay = v,
            SettingKey::TextScale => p.text_scale = v,
            _ => {}
        }
    }
}

/// What a tagged widget does when it reports an event.
#[derive(Clone, Debug)]
enum Bound {
    Button(Vec<UiAction>),
    Slider {
        key: SettingKey,
        min: f32,
        max: f32,
        step: Option<f32>,
    },
    Toggle(SettingKey),
    VarSlider {
        name: String,
        min: f32,
        max: f32,
        step: Option<f32>,
        int: bool,
    },
    VarToggle(String),
    Dropdown {
        key: SettingKey,
        /// Language codes (language dropdown only).
        codes: Vec<String>,
    },
    /// A dropdown bound to a story variable: option `i` sets `values[i]`.
    VarDropdown {
        name: String,
        values: Vec<Value>,
    },
    Slot(SlotAction),
}

/// A label kept in sync with a setting or a story variable.
#[derive(Clone, Debug)]
enum Readout {
    Setting(SettingKey),
    Var(String),
}

struct Build {
    ui: Ui,
    next_tag: Tag,
    binds: HashMap<Tag, Bound>,
    readouts: Vec<(Tag, Readout)>,
    styles: BTreeMap<String, Style>,
    counter: usize,
    paths: Vec<(NodeId, Vec<usize>)>,
    /// The rows of the `Repeat` templates being built, innermost last.
    items: Vec<ItemCtx>,
}

/// One row of a generated list (`WidgetKind::Repeat`, UI-15).
#[derive(Clone, Debug, PartialEq)]
struct ItemCtx {
    value: Value,
    index: usize,
    /// The key of the entry, for a list generated from a map.
    key: Option<String>,
}

/// A screen defined by data. The shell keeps its [`DataContext`] current
/// ([`DataScreen::set_context`]); activating widgets emits [`UiEvent`]s.
pub struct DataScreen {
    asset: ScreenAsset,
    ui: Ui,
    theme: Theme,
    texts: Texts,
    ctx: DataContext,
    binds: HashMap<Tag, Bound>,
    readouts: Vec<(Tag, Readout)>,
    /// Widget nodes and where their definitions are (child indices from the
    /// root): what the editor picks with the pointer.
    node_paths: Vec<(NodeId, Vec<usize>)>,
    /// Tab and page of the `SaveSlots` widget.
    slots_category: usize,
    slots_page: usize,
    confirm: Confirm,
    /// Actions run that have no effect yet (none today; see
    /// `UiAction::is_supported`), for hosts that want to say so.
    unsupported: Vec<UiAction>,
    /// A variable changed locally: the tree is rebuilt after the event.
    vars_dirty: bool,
    /// Values the story gave the screen (`OpenScreen::params`), read by
    /// `BindPath::Param` and `Operand::Param`.
    params: Vec<(String, Value)>,
}

impl DataScreen {
    pub fn new(asset: ScreenAsset, theme: &Theme, texts: &Texts, ctx: DataContext) -> Self {
        Self::with_params(asset, theme, texts, ctx, Vec::new())
    }

    /// A screen opened by the story with its parameters (UI-14).
    pub fn with_params(
        asset: ScreenAsset,
        theme: &Theme,
        texts: &Texts,
        ctx: DataContext,
        params: Vec<(String, Value)>,
    ) -> Self {
        let mut s = Self {
            asset,
            ui: Ui::new(theme.clone()),
            theme: theme.clone(),
            texts: texts.clone(),
            ctx,
            binds: HashMap::new(),
            readouts: Vec::new(),
            node_paths: Vec::new(),
            slots_category: 0,
            slots_page: 0,
            confirm: Confirm::default(),
            unsupported: Vec::new(),
            vars_dirty: false,
            params,
        };
        s.rebuild();
        s
    }

    /// The parameters the story gave the screen.
    pub fn params(&self) -> &[(String, Value)] {
        &self.params
    }

    /// Replaces the parameters (rebuilds when they changed).
    pub fn set_params(&mut self, params: Vec<(String, Value)>) {
        if params != self.params {
            self.params = params;
            self.rebuild();
        }
    }

    /// Whether the screen is a HUD (drawn under the textbox, never pausing).
    pub fn is_hud(&self) -> bool {
        self.asset.hud
    }

    /// Value of a variable, a screen parameter (`@param.name`) or, inside a
    /// row, the row's element (`@item`, `@index`, `@key`).
    fn lookup(&self, name: &str, item: Option<&ItemCtx>) -> Option<Value> {
        match name {
            orbit_core::ITEM_VAR => item.map(|i| i.value.clone()),
            orbit_core::ITEM_INDEX_VAR => item.map(|i| Value::Int(i.index as i64)),
            orbit_core::ITEM_KEY_VAR => item.and_then(|i| i.key.clone()).map(Value::Str),
            _ => match name.strip_prefix(orbit_core::PARAM_VAR_PREFIX) {
                Some(p) => self.param(p).cloned(),
                None => self.ctx.vars.get(name).cloned(),
            },
        }
    }

    fn param(&self, name: &str) -> Option<&Value> {
        self.params.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Value of a binding that is not a setting.
    fn bind_value(&self, bind: &BindPath, item: Option<&ItemCtx>) -> Option<Value> {
        let name = match bind {
            BindPath::Setting(_) => return None,
            BindPath::Var(n) => n.clone(),
            BindPath::Item => orbit_core::ITEM_VAR.to_owned(),
            BindPath::ItemIndex => orbit_core::ITEM_INDEX_VAR.to_owned(),
            BindPath::ItemKey => orbit_core::ITEM_KEY_VAR.to_owned(),
            BindPath::Param(p) => format!("{}{p}", orbit_core::PARAM_VAR_PREFIX),
            BindPath::Gallery(list) => {
                return Some(Value::list(orbit_core::ScalarType::Str, self.ctx.gallery_rows(list)));
            }
            BindPath::ReplayScenes(list) => {
                return Some(Value::list(orbit_core::ScalarType::Str, self.ctx.replay_rows(list)));
            }
            BindPath::Achievements => {
                return Some(Value::list(orbit_core::ScalarType::Str, self.ctx.achievement_rows()));
            }
            BindPath::Achievement(field) => {
                use orbit_ui_data::AchievementField as F;
                let row = self.ctx.achievement(item.map(|i| &i.value));
                return Some(Value::Str(match field {
                    F::Progress => self.ctx.achievement_progress(),
                    F::Title => row.map(|a| a.title.clone()).unwrap_or_default(),
                    F::Description => row.map(|a| a.description.clone()).unwrap_or_default(),
                    F::Icon => row.map(|a| a.icon.clone()).unwrap_or_default(),
                }));
            }
        };
        self.lookup(&name, item)
    }

    /// `operand` with the row and parameter values written in (they are not
    /// known any more when the button is pressed).
    fn resolve_operand(&self, operand: &orbit_core::Operand, item: Option<&ItemCtx>) -> orbit_core::Operand {
        if operand.is_contextual()
            && let Some(v) = operand.eval(&|n| self.lookup(n, item))
        {
            return orbit_core::Operand::Lit(v);
        }
        operand.clone()
    }

    fn resolve_actions(&self, actions: &[UiAction], item: Option<&ItemCtx>) -> Vec<UiAction> {
        actions
            .iter()
            .map(|a| match a {
                UiAction::VarSet(op) => {
                    let mut op = op.clone();
                    op.value = self.resolve_operand(&op.value, item);
                    op.key = op.key.as_ref().map(|k| self.resolve_operand(k, item));
                    UiAction::VarSet(op)
                }
                UiAction::CloseWith(o) => UiAction::CloseWith(self.resolve_operand(o, item)),
                UiAction::ReplayScene(o) => UiAction::ReplayScene(self.resolve_operand(o, item)),
                other => other.clone(),
            })
            .collect()
    }

    /// The rows a `Repeat` builds from `source` (at most `MAX_REPEAT_ITEMS`).
    fn repeat_items(&self, source: &BindPath, item: Option<&ItemCtx>) -> Vec<ItemCtx> {
        let max = orbit_ui_data::MAX_REPEAT_ITEMS;
        match self.bind_value(source, item) {
            Some(Value::List(_, list)) => list
                .iter()
                .take(max)
                .enumerate()
                .map(|(index, value)| ItemCtx {
                    value: value.clone(),
                    index,
                    key: None,
                })
                .collect(),
            Some(Value::Map(_, map)) => map
                .iter()
                .take(max)
                .enumerate()
                .map(|(index, (k, value))| ItemCtx {
                    value: value.clone(),
                    index,
                    key: Some(k.clone()),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn id(&self) -> &str {
        &self.asset.id
    }

    pub fn asset(&self) -> &ScreenAsset {
        &self.asset
    }

    /// Whether the screen is drawn over the one below it.
    pub fn is_modal(&self) -> bool {
        self.asset.modal
    }

    pub fn pauses_game(&self) -> bool {
        self.asset.pauses_game
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    /// Replaces the data the screen reads; rebuilds only when it changed.
    pub fn set_context(&mut self, ctx: DataContext) {
        if ctx != self.ctx {
            self.ctx = ctx;
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        let focus = self.ui.focused_tag();
        let mut st = Build {
            ui: Ui::new(self.theme.clone()),
            next_tag: 1,
            binds: HashMap::new(),
            readouts: Vec::new(),
            styles: BTreeMap::new(),
            counter: 0,
            paths: Vec::new(),
            items: Vec::new(),
        };
        if self.asset.hud {
            // Under the textbox (`Layer::UI`), above the stage.
            st.ui.set_layer(orbit_render2d::Layer(orbit_render2d::Layer::UI.0 - 1));
        }
        if self.asset.dim_background {
            st.ui
                .add_root(Node::panel(PanelKind::Dim, Dir::Stack).fill());
        }
        let root = self.asset.root.clone();
        self.build(&root, None, &mut Vec::new(), &mut st);
        let mut theme = self.theme.clone();
        theme.styles.extend(std::mem::take(&mut st.styles));
        st.ui.set_theme(theme);
        st.ui.set_images(self.ctx.images.clone());
        st.ui.layout();
        st.ui.focus_first();
        if let Some(tag) = focus {
            st.ui.set_focus_tag(tag);
        }
        self.ui = st.ui;
        self.binds = st.binds;
        self.readouts = st.readouts;
        self.node_paths = st.paths;
        self.confirm.reset();
    }

    fn text(&self, t: &TextRef) -> String {
        self.text_in(t, None)
    }

    /// A text inside the row `item` of a generated list (if any).
    fn text_in(&self, t: &TextRef, item: Option<&ItemCtx>) -> String {
        match t {
            TextRef::Key(k) if k == "game.title" => self.ctx.title.clone(),
            TextRef::Key(k) => self.texts.get(k),
            TextRef::Literal(s) => s.clone(),
            TextRef::Bind(BindPath::Setting(key)) => self.format_setting(*key),
            TextRef::Bind(BindPath::Var(name)) => self.format_var(name),
            TextRef::Bind(b) => self
                .bind_value(b, item)
                .map(|v| v.to_string())
                .unwrap_or_default(),
        }
    }

    fn format_var(&self, name: &str) -> String {
        self.ctx.vars.get(name).map(ToString::to_string).unwrap_or_default()
    }

    fn format_setting(&self, key: SettingKey) -> String {
        let p = &self.ctx.prefs;
        match key {
            SettingKey::MasterVolume
            | SettingKey::MusicVolume
            | SettingKey::SfxVolume
            | SettingKey::VoiceVolume => {
                format!("{:.0}%", self.ctx.setting_f32(key).unwrap_or(0.0) * 100.0)
            }
            SettingKey::TextSpeed => self
                .texts
                .format("ui.prefs.chars_per_second", &[("v", &format!("{:.0}", p.text_speed))]),
            SettingKey::AutoDelay => self
                .texts
                .format("ui.prefs.seconds", &[("v", &format!("{:.1}", p.auto_delay))]),
            SettingKey::SkipMode => self.texts.get(match p.skip_mode {
                SkipMode::ReadOnly => "ui.prefs.skip_read",
                SkipMode::All => "ui.prefs.skip_all",
            }),
            SettingKey::Fullscreen => {
                if p.fullscreen == Some(true) { "On" } else { "Off" }.to_owned()
            }
            SettingKey::ScreenReader => {
                if p.screen_reader == Some(true) { "On" } else { "Off" }.to_owned()
            }
            SettingKey::HighContrast => if p.high_contrast { "On" } else { "Off" }.to_owned(),
            SettingKey::ReducedMotion => if p.reduced_motion { "On" } else { "Off" }.to_owned(),
            SettingKey::TextScale => format!("{:.0}%", p.text_scale * 100.0),
            SettingKey::Language => p
                .languages
                .iter()
                .find(|l| l.code == p.language)
                .map(|l| l.name.clone())
                .unwrap_or_else(|| p.language.clone()),
        }
    }

    fn new_tag(st: &mut Build) -> Tag {
        let tag = st.next_tag;
        st.next_tag += 1;
        tag
    }

    /// Adds `def` (and its descendants) under `parent`; returns nothing when
    /// the widget is left out (condition, unsupported binding).
    fn build(
        &self,
        def: &WidgetDef,
        parent: Option<NodeId>,
        path: &mut Vec<usize>,
        st: &mut Build,
    ) {
        if !self.ctx.eval_with(&def.visible, st.items.last().map(|i| &i.value)) {
            return;
        }
        let index = st.counter;
        st.counter += 1;
        let p = &self.ctx.prefs;
        let item = st.items.last().cloned();
        let item = item.as_ref();
        let node = match &def.kind {
            WidgetKind::Panel { kind, dir } => Node::panel(*kind, *dir),
            WidgetKind::Stack { dir } | WidgetKind::Repeat { dir, .. } => Node::container(*dir),
            WidgetKind::Grid { columns } => Node::grid(*columns),
            WidgetKind::Button { text, action } => {
                let tag = Self::new_tag(st);
                st.binds
                    .insert(tag, Bound::Button(self.resolve_actions(action, item)));
                Node::button(tag, self.text_in(text, item))
            }
            WidgetKind::Label {
                text,
                size,
                center,
                dim,
            } => {
                let mut n = Node::label(self.text_in(text, item));
                if let Some(size) = size {
                    n = n.font(*size);
                }
                if *center {
                    n = n.text_align(TextAlign::Center);
                }
                if *dim {
                    n = n.dimmed();
                }
                if let TextRef::Bind(bind) = text {
                    let readout = match bind {
                        BindPath::Setting(key) => Some(Readout::Setting(*key)),
                        BindPath::Var(name) => Some(Readout::Var(name.clone())),
                        // Rows and parameters are fixed until a rebuild.
                        _ => None,
                    };
                    if let Some(readout) = readout {
                        let tag = Self::new_tag(st);
                        st.readouts.push((tag, readout));
                        n = n.tag(tag);
                    }
                }
                n
            }
            WidgetKind::Image { asset, bind, action } => {
                let bound = bind.as_ref().map(|b| match self.bind_value(b, item) {
                    Some(Value::Str(path)) => path,
                    _ => String::new(),
                });
                let texture = match bound.as_deref().or(asset.as_deref()) {
                    Some(MENU_BACKGROUND) => self.ctx.background,
                    // A gallery or project image the host loaded, or
                    // nothing.
                    Some(path) => self.ctx.images.get(path).copied(),
                    None => None,
                };
                if action.is_empty() {
                    Node::image(texture)
                } else {
                    let tag = Self::new_tag(st);
                    st.binds
                        .insert(tag, Bound::Button(self.resolve_actions(action, item)));
                    Node::clickable_image(tag, texture)
                }
            }
            WidgetKind::Slider {
                bind,
                min,
                max,
                step,
            } => {
                if max <= min {
                    return;
                }
                if let Some(name) = bind.var() {
                    let (value, int) = match self.ctx.vars.get(name) {
                        Some(Value::Int(i)) => (*i as f32, true),
                        Some(Value::Float(f)) => (*f as f32, false),
                        _ => return,
                    };
                    let tag = Self::new_tag(st);
                    st.binds.insert(
                        tag,
                        Bound::VarSlider {
                            name: name.to_owned(),
                            min: *min,
                            max: *max,
                            step: *step,
                            int,
                        },
                    );
                    let mut n = Node::slider(tag, ((value - min) / (max - min)).clamp(0.0, 1.0));
                    if let Some(step) = step {
                        n = n.step(step / (max - min));
                    }
                    return self.finish(n, def, parent, path, index, st);
                }
                let Some(key) = bind.setting() else { return };
                let Some(value) = self.ctx.setting_f32(key) else {
                    return;
                };
                let tag = Self::new_tag(st);
                st.binds.insert(
                    tag,
                    Bound::Slider {
                        key,
                        min: *min,
                        max: *max,
                        step: *step,
                    },
                );
                let mut n = Node::slider(tag, (value - min) / (max - min));
                if let Some(step) = step {
                    n = n.step(step / (max - min));
                }
                n
            }
            WidgetKind::Toggle { text, bind } => {
                if let Some(name) = bind.var() {
                    let Some(Value::Bool(on)) = self.ctx.vars.get(name) else {
                        return;
                    };
                    let tag = Self::new_tag(st);
                    st.binds.insert(tag, Bound::VarToggle(name.to_owned()));
                    let n = Node::toggle(tag, self.text_in(text, item), *on);
                    return self.finish(n, def, parent, path, index, st);
                }
                // The host may not offer fullscreen (or the screen reader
                // option): the toggle is then left out.
                let Some(key) = bind.setting() else { return };
                let on = match key {
                    SettingKey::Fullscreen => p.fullscreen,
                    SettingKey::ScreenReader => p.screen_reader,
                    SettingKey::HighContrast => Some(p.high_contrast),
                    SettingKey::ReducedMotion => Some(p.reduced_motion),
                    _ => None,
                };
                let Some(on) = on else { return };
                let tag = Self::new_tag(st);
                st.binds.insert(tag, Bound::Toggle(key));
                let n = Node::toggle(tag, self.text_in(text, item), on);
                return self.finish(n, def, parent, path, index, st);
            }
            WidgetKind::Dropdown {
                bind,
                options,
                values,
            } => {
                if let Some(name) = bind.var() {
                    let Some(current) = self.ctx.vars.get(name) else {
                        return;
                    };
                    if options.is_empty()
                        || values.len() != options.len()
                        || values
                            .iter()
                            .any(|v| v.value_type() != current.value_type())
                    {
                        return;
                    }
                    let selected = values.iter().position(|v| v == current).unwrap_or(0);
                    let tag = Self::new_tag(st);
                    st.binds.insert(
                        tag,
                        Bound::VarDropdown {
                            name: name.to_owned(),
                            values: values.clone(),
                        },
                    );
                    let names = options.iter().map(|o| self.text_in(o, item)).collect();
                    let n = Node::dropdown(tag, names, selected);
                    return self.finish(n, def, parent, path, index, st);
                }
                let Some(key) = bind.setting() else { return };
                let (names, index, codes) = match key {
                    SettingKey::Language => (
                        p.languages.iter().map(|l| l.name.clone()).collect(),
                        p.languages
                            .iter()
                            .position(|l| l.code == p.language)
                            .unwrap_or(0),
                        p.languages.iter().map(|l| l.code.clone()).collect(),
                    ),
                    SettingKey::SkipMode => (
                        options.iter().map(|o| self.text(o)).collect::<Vec<_>>(),
                        usize::from(p.skip_mode == SkipMode::All),
                        Vec::new(),
                    ),
                    _ => return,
                };
                let tag = Self::new_tag(st);
                st.binds.insert(tag, Bound::Dropdown { key, codes });
                Node::dropdown(tag, names, index)
            }
            WidgetKind::ScrollList => Node::scroll_list(Self::new_tag(st)),
            WidgetKind::SaveSlots { .. } => Node::vbox().gap(14.0),
        };
        self.finish(node, def, parent, path, index, st);
    }

    /// Applies layout, style and conditions to `node`, adds it to the tree
    /// and builds the children of `def`.
    fn finish(
        &self,
        mut node: Node,
        def: &WidgetDef,
        parent: Option<NodeId>,
        path: &mut Vec<usize>,
        index: usize,
        st: &mut Build,
    ) {
        let l = &def.layout;
        node.width = l.width;
        node.height = l.height;
        node.pad = l.pad;
        node.margin = l.margin;
        node.gap = l.gap;
        node.col_span = l.col_span.max(1);
        node.row_span = l.row_span.max(1);
        node.main = l.main;
        node.cross = l.cross;
        node.enabled = self.ctx.eval_with(&def.enabled, st.items.last().map(|i| &i.value));
        if parent.is_none() {
            node.anchor = l.anchor.unwrap_or(crate::widgets::Anchor::FULL);
        }
        node.style = if def.overrides == Style::default() {
            def.style.clone()
        } else {
            let name = format!("@{index}");
            let mut local = def.overrides.clone();
            local.extends = def.style.clone();
            st.styles.insert(name.clone(), local);
            Some(name)
        };

        let id = match parent {
            None => st.ui.add_root(node),
            Some(parent) => st.ui.add(parent, node),
        };
        st.paths.push((id, path.clone()));
        if let WidgetKind::SaveSlots {
            mode,
            columns,
            rows,
        } = &def.kind
        {
            let columns = (*columns).max(1);
            let per_page = columns * (*rows).max(1);
            let page = slots_page(
                &self.ctx.saves,
                *mode,
                self.slots_category,
                self.slots_page,
                per_page,
            );
            let mut next = st.next_tag;
            let bound = build_slots(&mut st.ui, id, &self.texts, *mode, columns, &page, &mut || {
                next += 1;
                next - 1
            });
            st.next_tag = next;
            for (tag, action) in bound {
                st.binds.insert(tag, Bound::Slot(action));
            }
        }
        if let WidgetKind::Repeat { source, empty, .. } = &def.kind {
            // The children are the template of one row, built per element.
            let outer = st.items.last().cloned();
            let rows = self.repeat_items(source, outer.as_ref());
            if rows.is_empty() {
                if let Some(text) = empty {
                    let label = Node::label(self.text_in(text, outer.as_ref())).dimmed();
                    st.ui.add(id, label);
                }
                return;
            }
            for row in rows {
                st.items.push(row);
                for (i, child) in def.children.iter().enumerate() {
                    path.push(i);
                    self.build(child, Some(id), path, st);
                    path.pop();
                }
                st.items.pop();
            }
            return;
        }
        for (i, child) in def.children.iter().enumerate() {
            path.push(i);
            self.build(child, Some(id), path, st);
            path.pop();
        }
    }

    /// Path of the deepest widget whose rectangle holds `point` (virtual
    /// pixels), labels and panels included. Among equally deep widgets the
    /// later one wins, as it is drawn on top.
    pub fn widget_at(&mut self, point: [f32; 2]) -> Option<Vec<usize>> {
        let mut best: Option<&Vec<usize>> = None;
        for (id, path) in &self.node_paths {
            let rect = self.ui.node_rect(*id);
            let inside = point[0] >= rect[0]
                && point[0] < rect[0] + rect[2]
                && point[1] >= rect[1]
                && point[1] < rect[1] + rect[3];
            if inside && best.is_none_or(|b| path.len() >= b.len()) {
                best = Some(path);
            }
        }
        best.cloned()
    }

    /// Rectangle of the widget at `path` (`None` when it is not built:
    /// hidden by its condition, or no such widget).
    pub fn rect_of_path(&mut self, path: &[usize]) -> Option<[f32; 4]> {
        let id = self.node_paths.iter().find(|(_, p)| p == path)?.0;
        Some(self.ui.node_rect(id))
    }

    /// Draws a selection outline around the widget at `path`.
    pub fn draw_outline(&mut self, ctx: &mut DrawCtx, path: &[usize]) {
        let Some([x, y, w, h]) = self.rect_of_path(path) else {
            return;
        };
        let t = 3.0;
        for r in [
            [x, y, w, t],
            [x, y + h - t, w, t],
            [x, y + t, t, h - 2.0 * t],
            [x + w - t, y + t, t, h - 2.0 * t],
        ] {
            if r[2] > 0.0 && r[3] > 0.0 {
                ctx.scene.push(
                    orbit_render2d::Sprite2d::new(ctx.white, [r[0], r[1]], [r[2], r[3]])
                        .anchor([0.0, 0.0])
                        .layer(orbit_render2d::Layer(60))
                        .z(0.0)
                        .tint([0.2, 0.7, 1.0, 1.0]),
                );
            }
        }
    }

    /// Actions run since the last call that have no effect yet.
    pub fn take_unsupported(&mut self) -> Vec<UiAction> {
        std::mem::take(&mut self.unsupported)
    }

    fn run_slot(&mut self, tag: Tag, action: SlotAction, out: &mut Vec<UiEvent>) {
        let mode = self.slots_mode();
        match action {
            SlotAction::Tab(i) => {
                self.slots_category = i;
                self.slots_page = 0;
                self.rebuild();
                self.ui.set_focus_tag(tag);
            }
            SlotAction::Page(p) => {
                self.slots_page = p;
                self.rebuild();
                self.ui.set_focus_tag(tag);
            }
            SlotAction::Delete(id) => self.ask("ui.save.confirm_delete", UiEvent::DeleteSlot(id)),
            SlotAction::Card { id, state } => match (mode, state) {
                (Some(SlotsMode::Load), SlotState::Used) => out.push(UiEvent::LoadSlot(id)),
                (Some(SlotsMode::Load), SlotState::Incompatible) => {
                    out.push(UiEvent::Toast(self.texts.get("ui.toast.incompatible")));
                }
                (Some(SlotsMode::Save), SlotState::Empty) => out.push(UiEvent::SaveSlot(id)),
                (Some(SlotsMode::Save), _) => {
                    self.ask("ui.save.confirm_overwrite", UiEvent::SaveSlot(id));
                }
                _ => {}
            },
        }
    }

    /// Mode of the (first) `SaveSlots` widget of the screen.
    fn slots_mode(&self) -> Option<SlotsMode> {
        let mut mode = None;
        self.asset.root.walk(&mut |w| {
            if let WidgetKind::SaveSlots { mode: m, .. } = &w.kind {
                mode.get_or_insert(*m);
            }
        });
        mode
    }

    fn ask(&mut self, key: &str, on_yes: UiEvent) {
        let message = self.texts.get(key);
        let texts = self.texts.clone();
        self.confirm.open(&mut self.ui, &texts, &message, on_yes);
    }

    fn refresh_readouts(&mut self) {
        let updates: Vec<(Tag, String)> = self
            .readouts
            .iter()
            .map(|(tag, readout)| {
                (
                    *tag,
                    match readout {
                        Readout::Setting(key) => self.format_setting(*key),
                        Readout::Var(name) => self.format_var(name),
                    },
                )
            })
            .collect();
        for (tag, text) in updates {
            self.ui.set_text(tag, text);
        }
    }

    /// A bound control changed a story variable.
    fn set_var(&mut self, name: &str, value: Value, out: &mut Vec<UiEvent>) {
        self.ctx.vars.insert(name.to_owned(), value.clone());
        out.push(UiEvent::VarOp(VarOp::new(
            name,
            orbit_core::OpKind::Set,
            orbit_core::Operand::Lit(value),
        )));
        // A condition on the variable may show or hide widgets.
        if self.conditions_use(name) {
            self.vars_dirty = true;
        }
    }

    /// Whether a visible / enabled condition of the screen reads `name`.
    fn conditions_use(&self, name: &str) -> bool {
        fn uses(cond: &Cond, name: &str) -> bool {
            match cond {
                Cond::Vars(c) => c.variables().iter().any(|v| v == name),
                Cond::Not(c) => uses(c, name),
                _ => false,
            }
        }
        let mut found = false;
        self.asset.root.walk(&mut |w| {
            found |= uses(&w.visible, name) || uses(&w.enabled, name);
        });
        found
    }

    fn run(&mut self, action: &UiAction, out: &mut Vec<UiEvent>) {
        out.extend(match action {
            UiAction::GameNew => Some(UiEvent::StartGame),
            UiAction::GameContinue => Some(UiEvent::Continue),
            UiAction::GameQuit => Some(UiEvent::Quit),
            UiAction::ReturnToTitle => Some(UiEvent::ReturnToMainMenu),
            UiAction::Open(id) => Some(UiEvent::OpenScreen(id.clone())),
            UiAction::Replace(id) => Some(UiEvent::ReplaceScreen(id.clone())),
            UiAction::Close => Some(UiEvent::Back),
            UiAction::CloseAll => Some(UiEvent::CloseAll),
            UiAction::QuickSave => Some(UiEvent::QuickSave),
            UiAction::QuickLoad => Some(UiEvent::QuickLoad),
            UiAction::OpenSave => Some(UiEvent::OpenSave),
            UiAction::OpenLoad => Some(UiEvent::OpenLoad),
            UiAction::ResetPreferences => Some(UiEvent::ResetPreferences),
            UiAction::PlaySound(path) => Some(UiEvent::PlaySound(path.clone())),
            UiAction::Custom { name, args } => Some(UiEvent::Custom {
                name: name.clone(),
                args: args.clone(),
            }),
            UiAction::VarSet(op) => {
                // Show the effect at once; the shell applies the same
                // operation to the game and sends the real values back.
                if let Ok(value) = op.apply(&|n| self.ctx.vars.get(n).cloned()) {
                    self.ctx.vars.insert(op.var.clone(), value);
                    self.vars_dirty = true;
                }
                Some(UiEvent::VarOp(op.clone()))
            }
            UiAction::SceneGoto(scene) => Some(UiEvent::SceneGoto(scene.clone())),
            // The row was written in when the button was built.
            UiAction::ReplayScene(operand) => match operand.eval(&|n| self.lookup(n, None)) {
                Some(Value::Str(scene)) if !scene.trim().is_empty() => Some(UiEvent::ReplayScene(scene)),
                _ => None,
            },
            // Row values were written in when the button was built.
            UiAction::CloseWith(operand) => Some(match operand.eval(&|n| self.lookup(n, None)) {
                Some(value) => UiEvent::BackWith(value),
                // No value (e.g. an unknown variable): a plain close.
                None => UiEvent::Back,
            }),
        });
    }
}

fn setting_event(key: SettingKey, v: f32) -> Option<UiEvent> {
    Some(match key {
        SettingKey::MasterVolume => UiEvent::SetVolume(AudioBus::Master, v),
        SettingKey::MusicVolume => UiEvent::SetVolume(AudioBus::Music, v),
        SettingKey::SfxVolume => UiEvent::SetVolume(AudioBus::Sfx, v),
        SettingKey::VoiceVolume => UiEvent::SetVolume(AudioBus::Voice, v),
        SettingKey::TextSpeed => UiEvent::SetTextSpeed(v),
        SettingKey::AutoDelay => UiEvent::SetAutoDelay(v),
        SettingKey::TextScale => UiEvent::SetTextScale(v),
        _ => return None,
    })
}

impl Screen for DataScreen {
    fn widgets(&mut self) -> Option<&mut Ui> {
        Some(&mut self.ui)
    }

    fn handle(&mut self, input: UiInput) -> Vec<UiEvent> {
        let mut out = Vec::new();
        let mut changed = false;
        for ev in self.ui.handle(input) {
            if let WidgetEvent::Sound(s) = ev {
                out.push(UiEvent::Sound(s));
                continue;
            }
            if let WidgetEvent::SoundFile(path) = ev {
                out.push(UiEvent::PlaySound(path));
                continue;
            }
            if self.confirm.route(&mut self.ui, &ev, &mut out) {
                continue;
            }
            match ev {
                WidgetEvent::Sound(_) => {}
                WidgetEvent::Activated(tag) => match self.binds.get(&tag).cloned() {
                    Some(Bound::Button(actions)) => {
                        for action in &actions {
                            self.run(action, &mut out);
                        }
                    }
                    Some(Bound::Slot(action)) => self.run_slot(tag, action, &mut out),
                    _ => {}
                },
                WidgetEvent::SliderChanged(tag, unit) => {
                    if let Some(&Bound::Slider {
                        key,
                        min,
                        max,
                        step,
                    }) = self.binds.get(&tag)
                    {
                        let mut v = min + unit.clamp(0.0, 1.0) * (max - min);
                        if let Some(step) = step {
                            v = (min + ((v - min) / step).round() * step).clamp(min, max);
                        }
                        self.ctx.set_setting_f32(key, v);
                        out.extend(setting_event(key, v));
                        changed = true;
                    } else if let Some(Bound::VarSlider {
                        name,
                        min,
                        max,
                        step,
                        int,
                    }) = self.binds.get(&tag).cloned()
                    {
                        let mut v = min + unit.clamp(0.0, 1.0) * (max - min);
                        if let Some(step) = step {
                            v = (min + ((v - min) / step).round() * step).clamp(min, max);
                        }
                        let value = if int {
                            Value::Int(v.round() as i64)
                        } else {
                            Value::Float(f64::from(v))
                        };
                        self.set_var(&name, value, &mut out);
                        changed = true;
                    }
                }
                WidgetEvent::Toggled(tag, on) => {
                    match self.binds.get(&tag).cloned() {
                        Some(Bound::Toggle(key)) => {
                            let prefs = &mut self.ctx.prefs;
                            match key {
                                SettingKey::Fullscreen => {
                                    prefs.fullscreen = Some(on);
                                    out.push(UiEvent::SetFullscreen(on));
                                }
                                SettingKey::ScreenReader => {
                                    prefs.screen_reader = Some(on);
                                    out.push(UiEvent::SetScreenReader(on));
                                }
                                SettingKey::HighContrast => {
                                    prefs.high_contrast = on;
                                    out.push(UiEvent::SetHighContrast(on));
                                }
                                SettingKey::ReducedMotion => {
                                    prefs.reduced_motion = on;
                                    out.push(UiEvent::SetReducedMotion(on));
                                }
                                _ => {}
                            }
                            changed = true;
                        }
                        Some(Bound::VarToggle(name)) => {
                            self.set_var(&name, Value::Bool(on), &mut out);
                            changed = true;
                        }
                        _ => {}
                    }
                }
                WidgetEvent::SoundFile(_) => {}
                WidgetEvent::Selected(tag, index) => {
                    if let Some(Bound::VarDropdown { name, values }) = self.binds.get(&tag).cloned()
                    {
                        if let Some(value) = values.get(index) {
                            self.set_var(&name, value.clone(), &mut out);
                            changed = true;
                        }
                    } else if let Some(Bound::Dropdown { key, codes }) = self.binds.get(&tag) {
                        match key {
                            SettingKey::Language => {
                                if let Some(code) = codes.get(index) {
                                    self.ctx.prefs.language = code.clone();
                                    out.push(UiEvent::SetLanguage(code.clone()));
                                }
                            }
                            SettingKey::SkipMode => {
                                let mode = if index == 0 {
                                    SkipMode::ReadOnly
                                } else {
                                    SkipMode::All
                                };
                                self.ctx.prefs.skip_mode = mode;
                                out.push(UiEvent::SetSkipMode(mode));
                                changed = true;
                            }
                            _ => {}
                        }
                    }
                }
                WidgetEvent::Cancelled => {
                    for action in &self.asset.on_cancel.clone() {
                        self.run(action, &mut out);
                    }
                }
            }
        }
        if std::mem::take(&mut self.vars_dirty) {
            self.rebuild();
        } else if changed {
            self.refresh_readouts();
        }
        out
    }

    fn update(&mut self, dt: f32) {
        self.ui.update(dt);
    }

    fn draw(&mut self, ctx: &mut DrawCtx) {
        self.ui.draw(ctx);
    }

    fn set_theme(&mut self, theme: &Theme) {
        self.theme = theme.clone();
        self.rebuild();
    }

    fn set_texts(&mut self, texts: &Texts) {
        self.texts = texts.clone();
        self.rebuild();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::from_ron;
    use crate::view::LanguageView;

    const SCREEN: &str = r#"(
        id: "demo",
        on_cancel: [Close],
        root: (kind: Stack(dir: Vertical), children: [
            (kind: Button(text: Literal("New"), action: [GameNew])),
            (kind: Button(text: Literal("Continue"), action: [GameContinue]), visible: CanContinue),
            (kind: Button(text: Literal("Save"), action: [OpenSave, Open("prefs2"), Custom(name: "hi", args: ["a"])]), enabled: CanSave),
            (kind: Button(text: Literal("Quit"), action: [GameQuit]), visible: ShowQuit),
            (kind: Button(text: Literal("Warp"), action: [SceneGoto("x")])),
            (kind: Slider(bind: Setting(MusicVolume), min: 0.0, max: 1.0, step: Some(0.1))),
            (kind: Label(text: Bind(Setting(MusicVolume)))),
            (kind: Toggle(text: Literal("Full"), bind: Setting(Fullscreen))),
            (kind: Dropdown(bind: Setting(Language))),
        ]),
    )"#;

    fn ctx() -> DataContext {
        DataContext {
            prefs: PrefsView {
                languages: vec![
                    LanguageView {
                        code: "en".into(),
                        name: "English".into(),
                    },
                    LanguageView {
                        code: "fr".into(),
                        name: "Francais".into(),
                    },
                ],
                language: "en".into(),
                ..PrefsView::default()
            },
            can_continue: false,
            can_save: false,
            show_quit: true,
            ..DataContext::default()
        }
    }

    fn screen(ctx: DataContext) -> DataScreen {
        DataScreen::new(
            from_ron(SCREEN).unwrap(),
            &Theme::default(),
            &Texts::builtin("en"),
            ctx,
        )
    }

    fn no_sound(events: Vec<UiEvent>) -> Vec<UiEvent> {
        events
            .into_iter()
            .filter(|e| !matches!(e, UiEvent::Sound(_)))
            .collect()
    }

    fn center_of(s: &mut DataScreen, tag: Tag) -> [f32; 2] {
        let r = s.ui().rect_of(tag).unwrap();
        [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0]
    }

    fn click(s: &mut DataScreen, tag: Tag) -> Vec<UiEvent> {
        let p = center_of(s, tag);
        s.handle(UiInput::PointerMove(p));
        s.handle(UiInput::PointerDown(p));
        no_sound(s.handle(UiInput::PointerUp(p)))
    }

    #[test]
    fn conditions_hide_widgets_and_disable_buttons() {
        let mut s = screen(ctx());
        // New (1), Save (2), Quit (3) ...: Continue is left out.
        assert!(s.ui().find(1).is_some());
        let tags: Vec<Tag> = (1..8).filter(|t| s.ui().find(*t).is_some()).collect();
        assert_eq!(tags.len(), 7, "{tags:?}");
        let id = s.ui().find(2).unwrap();
        assert!(!s.ui().node(id).enabled, "Save disabled");
        let mut with = ctx();
        with.can_continue = true;
        with.can_save = true;
        s.set_context(with);
        let id = s.ui().find(3).unwrap();
        assert!(s.ui().node(id).enabled);
    }

    #[test]
    fn buttons_map_their_actions_to_events() {
        let mut s = screen(ctx());
        assert_eq!(click(&mut s, 1), vec![UiEvent::StartGame]);
        let mut c = ctx();
        c.can_save = true;
        s.set_context(c);
        assert_eq!(
            click(&mut s, 2),
            vec![
                UiEvent::OpenSave,
                UiEvent::OpenScreen("prefs2".into()),
                UiEvent::Custom {
                    name: "hi".into(),
                    args: vec!["a".into()]
                }
            ]
        );
    }

    #[test]
    fn scene_goto_is_reported_to_the_host() {
        let mut s = screen(ctx());
        // New, Save, Quit, Warp -> tag 4 once Continue is absent.
        assert_eq!(click(&mut s, 4), vec![UiEvent::SceneGoto("x".into())]);
    }

    #[test]
    fn cancel_runs_on_cancel() {
        let mut s = screen(ctx());
        assert_eq!(no_sound(s.handle(UiInput::Cancel)), vec![UiEvent::Back]);
    }

    #[test]
    fn a_bound_slider_edits_the_setting_and_its_readout() {
        let mut s = screen(ctx());
        let slider = 5;
        let r = s.ui().rect_of(slider).unwrap();
        let p = [r[0] + r[2] * 0.5, r[1] + r[3] / 2.0];
        s.handle(UiInput::PointerMove(p));
        let mut ev = s.handle(UiInput::PointerDown(p));
        ev.extend(s.handle(UiInput::PointerUp(p)));
        let ev = no_sound(ev);
        let UiEvent::SetVolume(AudioBus::Music, v) = ev[0] else {
            panic!("{ev:?}")
        };
        assert!((v - 0.5).abs() < 0.11, "{v}");
        assert_eq!((v * 10.0).round() / 10.0, v, "stepped by 0.1");
        let label = s.ui().find(6).unwrap();
        let crate::widgets::Kind::Label(l) = &s.ui().node(label).kind else {
            panic!()
        };
        assert_eq!(l.text, format!("{:.0}%", v * 100.0));
    }

    #[test]
    fn fullscreen_toggle_follows_the_host_support() {
        let mut c = ctx();
        c.prefs.fullscreen = None;
        let mut s = screen(c);
        let toggles = (1..9)
            .filter(|t| s.ui().find(*t).is_some_and(|id| matches!(s.ui().node(id).kind, crate::widgets::Kind::Toggle { .. })))
            .count();
        assert_eq!(toggles, 0);
        let mut c = ctx();
        c.prefs.fullscreen = Some(false);
        let mut s = screen(c);
        assert_eq!(click(&mut s, 7), vec![UiEvent::SetFullscreen(true)]);
    }

    #[test]
    fn the_language_dropdown_lists_the_game_languages() {
        let mut s = screen(ctx());
        let tag = 8;
        let id = s.ui().find(tag).unwrap();
        let crate::widgets::Kind::Selector { options, index, .. } = &s.ui().node(id).kind else {
            panic!()
        };
        assert_eq!(options, &["English", "Francais"]);
        assert_eq!(*index, 0);
        s.ui().set_focus_tag(tag);
        assert_eq!(
            no_sound(s.handle(UiInput::Right)),
            vec![UiEvent::SetLanguage("fr".into())]
        );
    }

    #[test]
    fn overrides_become_a_style_extending_the_named_one() {
        let asset = from_ron(
            "(id: \"s\", root: (kind: Button(text: Literal(\"x\")), style: Some(\"base\"), \
             overrides: (text_size: Some(50.0))))",
        )
        .unwrap();
        let mut theme = Theme::default();
        theme.styles.insert(
            "base".into(),
            Style {
                padding: Some([1.0, 2.0]),
                ..Style::default()
            },
        );
        let mut s = DataScreen::new(asset, &theme, &Texts::builtin("en"), ctx());
        let id = s.ui().find(1).unwrap();
        let name = s.ui().node(id).style.clone().unwrap();
        let resolved = crate::theme::resolve_style(&s.ui().theme().styles, &name).unwrap();
        assert_eq!(resolved.text_size, Some(50.0));
        assert_eq!(resolved.padding, Some([1.0, 2.0]));
    }

    /// `ui_version` 8: an image with actions is clicked like a button (and
    /// focused by the keyboard); one without stays inert.
    #[test]
    fn an_image_with_actions_is_a_clickable_control() {
        let asset = from_ron(
            "(id: \"s\", root: (kind: Stack(dir: Vertical), children: [\
             (kind: Image(asset: Some(\"assets/logo.png\"), action: [Open(\"extras\")]), \
              layout: (width: Fixed(200.0), height: Fixed(100.0))),\
             (kind: Image(asset: Some(\"assets/logo.png\")), layout: (width: Fixed(200.0), height: Fixed(100.0))),\
             ]))",
        )
        .unwrap();
        let mut s = DataScreen::new(asset, &Theme::default(), &Texts::builtin("en"), ctx());
        let tag = 1;
        let id = s.ui().find(tag).unwrap();
        assert!(matches!(s.ui().node(id).kind, crate::widgets::Kind::Image { clickable: true, .. }));
        assert_eq!(click(&mut s, tag), vec![UiEvent::OpenScreen("extras".into())]);
        let inert = s
            .ui()
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, crate::widgets::Kind::Image { clickable: false, .. }))
            .count();
        assert_eq!(inert, 1);
    }

    #[test]
    fn dim_background_adds_a_dim_layer_first() {
        let asset = from_ron(
            "(id: \"s\", dim_background: true, root: (kind: Stack(dir: Vertical)))",
        )
        .unwrap();
        let s = DataScreen::new(asset, &Theme::default(), &Texts::builtin("en"), ctx());
        assert_eq!(s.ui.roots.len(), 2);
    }
    // ------------------------------------------------------ picking, slots

    #[test]
    fn widget_at_picks_the_deepest_widget_and_paths_skip_hidden_ones() {
        let mut s = screen(ctx());
        // New (path [0]), Continue hidden, Save [2].
        let new = center_of(&mut s, 1);
        assert_eq!(s.widget_at(new), Some(vec![0]));
        let save = center_of(&mut s, 2);
        assert_eq!(s.widget_at(save), Some(vec![2]), "indices count hidden siblings");
        // A label has no interaction but can be picked.
        let label = s.ui().find(6).unwrap();
        let r = s.ui().node(label).rect();
        let at = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
        assert_eq!(s.widget_at(at), Some(vec![6]));
        assert_eq!(s.widget_at([-5.0, -5.0]), None);
        assert_eq!(s.rect_of_path(&[0]), s.ui().rect_of(1));
        assert_eq!(s.rect_of_path(&[1]), None, "Continue is not built");
        assert_eq!(s.rect_of_path(&[99]), None);
    }

    #[test]
    fn nested_widgets_win_over_their_container() {
        let asset = from_ron(
            "(id: \"n\", root: (kind: Panel(), layout: (pad: (20.0, 20.0)), children: [\
             (kind: Stack(dir: Horizontal), children: [(kind: Label(text: Literal(\"deep\")))])]))",
        )
        .unwrap();
        let mut s = DataScreen::new(asset, &Theme::default(), &Texts::builtin("en"), ctx());
        let (_, path) = s.node_paths.last().unwrap().clone();
        assert_eq!(path, vec![0, 0]);
        let r = s.rect_of_path(&path).unwrap();
        assert_eq!(s.widget_at([r[0] + 1.0, r[1] + 1.0]), Some(vec![0, 0]));
        let panel = s.rect_of_path(&[]).unwrap();
        assert_eq!(s.widget_at([panel[0] + 1.0, panel[1] + 1.0]), Some(vec![]));
    }

    #[test]
    fn supported_actions_are_not_reported_as_unsupported() {
        let mut s = screen(ctx());
        assert!(!click(&mut s, 4).is_empty());
        assert!(s.take_unsupported().is_empty());
    }

    fn slots_screen(mode: &str, saves: bool) -> DataScreen {
        let asset = from_ron(&format!(
            "(id: \"slots\", on_cancel: [Close], root: (kind: Stack(dir: Vertical), children: [\
             (kind: SaveSlots(mode: {mode}))]))"
        ))
        .unwrap();
        let mut c = ctx();
        if saves {
            c.saves = crate::asset::sample_saves();
        }
        DataScreen::new(asset, &Theme::default(), &Texts::builtin("en"), c)
    }

    fn manual(i: u32) -> crate::view::SlotId {
        crate::view::SlotId {
            category: crate::view::SlotCategory::Manual,
            index: i,
        }
    }

    #[test]
    fn saving_into_a_used_slot_asks_first_and_an_empty_one_does_not() {
        let mut s = slots_screen("Save", true);
        // Cards and delete buttons alternate: card 0 = tag 1, delete 0 = tag 2...
        assert!(click(&mut s, 1).is_empty(), "slot 1 is used: confirm opens");
        assert!(s.ui().has_modal());
        assert!(click(&mut s, 9002).is_empty(), "No");
        assert!(!s.ui().has_modal());
        click(&mut s, 1);
        assert_eq!(click(&mut s, 9001), vec![UiEvent::SaveSlot(manual(0))]);
        assert_eq!(click(&mut s, 3), vec![UiEvent::SaveSlot(manual(1))], "empty slot");
    }

    #[test]
    fn deleting_always_confirms_and_cancel_closes_the_dialog_first() {
        let mut s = slots_screen("Save", true);
        assert!(click(&mut s, 2).is_empty());
        assert!(s.ui().has_modal());
        // Escape closes the question, it does not run on_cancel (Close).
        assert!(no_sound(s.handle(UiInput::Cancel)).is_empty());
        assert!(!s.ui().has_modal());
        click(&mut s, 2);
        assert_eq!(click(&mut s, 9001), vec![UiEvent::DeleteSlot(manual(0))]);
        // An empty slot has nothing to delete.
        assert!(click(&mut s, 4).is_empty());
        assert!(!s.ui().has_modal());
    }

    #[test]
    fn loading_uses_category_tabs_and_reports_incompatible_saves() {
        let mut s = slots_screen("Load", true);
        // Tabs 1..=3, then cards from tag 4.
        assert_eq!(click(&mut s, 4), vec![UiEvent::LoadSlot(manual(0))]);
        assert!(click(&mut s, 6).is_empty(), "empty slot: disabled");
        let UiEvent::Toast(t) = click(&mut s, 10).remove(0) else {
            panic!("incompatible save")
        };
        assert_eq!(t, Texts::builtin("en").get("ui.toast.incompatible"));
        // The autosaves tab.
        assert!(click(&mut s, 2).is_empty());
        let auto = crate::view::SlotId {
            category: crate::view::SlotCategory::Auto,
            index: 0,
        };
        assert_eq!(click(&mut s, 4), vec![UiEvent::LoadSlot(auto)]);
    }

    #[test]
    fn the_slot_pager_changes_pages_and_follows_new_saves() {
        let mut s = slots_screen("Save", true);
        // 8 manual slots, 6 per page: 12 card/delete tags, then prev 13, next 14.
        assert!(click(&mut s, 14).is_empty());
        assert_eq!(click(&mut s, 3), vec![UiEvent::SaveSlot(manual(7))], "page 2: slot 8");
        // A fresh context (a save happened) keeps the page.
        let mut c = ctx();
        c.saves = crate::asset::sample_saves();
        c.saves[0].slots[7].state = crate::view::SlotState::Used;
        s.set_context(c);
        assert!(click(&mut s, 3).is_empty(), "now used: asks");
        assert!(s.ui().has_modal());
    }

    #[test]
    fn slots_are_reachable_and_loadable_without_pointer() {
        let mut s = slots_screen("Load", true);
        // Gamepad d-pad / keyboard only: Next walks tabs then cards.
        let mut steps = 0;
        while s.ui().focused_tag() != Some(4) {
            s.handle(UiInput::Next);
            steps += 1;
            assert!(steps < 20, "the first card is reachable with Next");
        }
        // Spatial navigation never traps focus.
        s.handle(UiInput::Right);
        assert_ne!(s.ui().focused_tag(), Some(4));
        assert_ne!(s.ui().focused_tag(), Some(2), "stays in the row of cards");
        s.handle(UiInput::Left);
        assert_eq!(s.ui().focused_tag(), Some(4));
        let events = no_sound(s.handle(UiInput::Confirm));
        assert_eq!(events, vec![UiEvent::LoadSlot(manual(0))]);
    }

    #[test]
    fn without_saves_the_slots_widget_is_empty_and_safe() {
        let mut s = slots_screen("Load", false);
        assert!(s.ui().find(1).is_none());
        assert!(no_sound(s.handle(UiInput::Confirm)).is_empty());
    }
    // ------------------------------------------------- story variables

    const VARS_SCREEN: &str = r#"(
        id: "vars",
        root: (kind: Stack(dir: Vertical), children: [
            (kind: Button(text: Literal("+5"), action: [VarSet((var: "gold", op: Add, value: Lit(Int(5))))])),
            (kind: Button(text: Literal("Shop"), action: [GameNew]), visible: Vars(Cmp(var: "gold", cmp: Ge, value: Lit(Int(10))))),
            (kind: Label(text: Bind(Var("gold")))),
            (kind: Toggle(text: Literal("T"), bind: Var("flag"))),
            (kind: Slider(bind: Var("gold"), min: 0.0, max: 20.0, step: Some(1.0))),
        ]),
    )"#;

    fn vars_ctx(gold: i64) -> DataContext {
        let mut c = ctx();
        c.vars.insert("gold".into(), Value::Int(gold));
        c.vars.insert("flag".into(), Value::Bool(false));
        c
    }

    fn vars_screen(c: DataContext) -> DataScreen {
        DataScreen::new(
            from_ron(VARS_SCREEN).unwrap(),
            &Theme::default(),
            &Texts::builtin("en"),
            c,
        )
    }

    fn label_text(s: &mut DataScreen, tag: Tag) -> String {
        let id = s.ui().find(tag).unwrap();
        let crate::widgets::Kind::Label(l) = &s.ui().node(id).kind else {
            panic!()
        };
        l.text.clone()
    }

    #[test]
    fn a_var_button_emits_the_operation_and_updates_conditions_and_labels() {
        let mut s = vars_screen(vars_ctx(0));
        // Shop is hidden: tags are +5 (1), label (2), toggle (3), slider (4).
        assert_eq!(label_text(&mut s, 2), "0");
        let op = VarOp::new("gold", orbit_core::OpKind::Add, orbit_core::Operand::Lit(Value::Int(5)));
        assert_eq!(click(&mut s, 1), vec![UiEvent::VarOp(op.clone())]);
        assert_eq!(label_text(&mut s, 2), "5");
        assert_eq!(click(&mut s, 1), vec![UiEvent::VarOp(op)]);
        // gold == 10: the Shop button appeared (tags shift by one).
        assert!(s.ui().find(2).is_some());
        assert_eq!(label_text(&mut s, 3), "10");
        // The shell then sends the real value: no visible change.
        s.set_context(vars_ctx(10));
        assert_eq!(label_text(&mut s, 3), "10");
    }

    #[test]
    fn var_bound_toggle_and_slider_edit_the_variable() {
        let mut s = vars_screen(vars_ctx(0));
        let set = |v: Value| UiEvent::VarOp(VarOp::new("flag", orbit_core::OpKind::Set, orbit_core::Operand::Lit(v)));
        assert_eq!(click(&mut s, 3), vec![set(Value::Bool(true))]);
        let r = s.ui().rect_of(4).unwrap();
        let p = [r[0] + r[2] * 0.5, r[1] + r[3] / 2.0];
        s.handle(UiInput::PointerMove(p));
        let mut ev = s.handle(UiInput::PointerDown(p));
        ev.extend(s.handle(UiInput::PointerUp(p)));
        let ev = no_sound(ev);
        let UiEvent::VarOp(op) = &ev[0] else {
            panic!("{ev:?}")
        };
        let orbit_core::Operand::Lit(Value::Int(v)) = op.value else {
            panic!("an integer variable gets an integer: {op:?}")
        };
        assert!((v - 10).abs() <= 2, "{v}");
        // The screen already shows the new value (the Shop condition may have
        // rebuilt it, so look the label up by content).
        assert_eq!(s.ctx.vars.get("gold"), Some(&Value::Int(v)));
    }

    const DROPDOWN_SCREEN: &str = "(id: \"d\", root: (kind: Stack(dir: Vertical), children: [\
        (kind: Dropdown(bind: Var(\"mode\"), options: [Literal(\"Easy\"), Literal(\"Normal\"), Literal(\"Hard\")], \
         values: [Str(\"easy\"), Str(\"normal\"), Str(\"hard\")]), style: Some(\"fancy\"), layout: (width: Fixed(400.0)))]))";

    fn dropdown_screen(mode: &str) -> DataScreen {
        let mut c = ctx();
        c.vars.insert("mode".into(), Value::Str(mode.into()));
        let mut theme = Theme::default();
        theme.styles.insert(
            "fancy".into(),
            Style {
                sounds: Some(crate::theme::StyleSounds {
                    hover: Some("sfx/h.ogg".into()),
                    confirm: Some("sfx/c.ogg".into()),
                }),
                ..Style::default()
            },
        );
        DataScreen::new(from_ron(DROPDOWN_SCREEN).unwrap(), &theme, &Texts::builtin("en"), c)
    }

    #[test]
    fn a_dropdown_bound_to_a_variable_shows_it_and_sets_it_from_the_popup() {
        let mut s = dropdown_screen("normal");
        let id = s.ui().find(1).unwrap();
        let crate::widgets::Kind::Selector { options, index, popup } = &s.ui().node(id).kind else {
            panic!()
        };
        assert_eq!((options.clone(), *index, *popup), (vec!["Easy".into(), "Normal".into(), "Hard".into()], 1, true));
        // Open the popup, pick the last row.
        click(&mut s, 1);
        assert!(s.ui().popup_open());
        let row = crate::widgets::POPUP_TAG + 2;
        let ev = click(&mut s, row);
        assert_eq!(
            ev,
            vec![UiEvent::VarOp(VarOp::new(
                "mode",
                orbit_core::OpKind::Set,
                orbit_core::Operand::Lit(Value::Str("hard".into()))
            ))]
        );
        assert_eq!(s.ctx.vars.get("mode"), Some(&Value::Str("hard".into())));
        assert!(!s.ui().popup_open());
    }

    #[test]
    fn style_sounds_play_the_named_file_through_a_data_screen() {
        let mut s = dropdown_screen("easy");
        let p = center_of(&mut s, 1);
        assert!(s.handle(UiInput::PointerMove(p)).contains(&UiEvent::PlaySound("sfx/h.ogg".into())));
        // Opening the dropdown is its "confirm".
        let down = s.handle(UiInput::PointerDown(p));
        assert!(down.contains(&UiEvent::PlaySound("sfx/c.ogg".into())), "{down:?}");
    }

    #[test]
    fn a_variable_dropdown_is_left_out_without_a_matching_variable() {
        let mut c = ctx();
        c.vars.insert("mode".into(), Value::Int(1));
        let mut s = DataScreen::new(from_ron(DROPDOWN_SCREEN).unwrap(), &Theme::default(), &Texts::builtin("en"), c);
        assert!(s.ui().find(1).is_none(), "wrong type");
        let mut s = DataScreen::new(from_ron(DROPDOWN_SCREEN).unwrap(), &Theme::default(), &Texts::builtin("en"), ctx());
        assert!(s.ui().find(1).is_none(), "unknown variable");
    }

    #[test]
    fn var_widgets_are_left_out_when_the_variable_has_the_wrong_type() {
        let mut c = ctx();
        c.vars.insert("gold".into(), Value::Str("x".into()));
        let mut s = vars_screen(c);
        // Only the button, the (empty-ish) label remain: no toggle, no slider.
        let kinds = (1..8)
            .filter(|t| s.ui().find(*t).is_some_and(|id| matches!(s.ui().node(id).kind, crate::widgets::Kind::Slider { .. } | crate::widgets::Kind::Toggle { .. })))
            .count();
        assert_eq!(kinds, 0);
    }

    #[test]
    fn var_conditions_validate_against_declared_types() {
        let asset = from_ron(VARS_SCREEN).unwrap();
        let ty = |n: &str| match n {
            "gold" => Some(orbit_core::ValueType::Int),
            "flag" => Some(orbit_core::ValueType::Bool),
            _ => None,
        };
        assert!(crate::asset::validate_with_vars(&asset, None, &ty).is_empty());
        let none = |_: &str| None;
        assert_eq!(crate::asset::validate_with_vars(&asset, None, &none).len(), 5);
    }

    const A11Y_SCREEN: &str = r#"(
        id: "a11y",
        root: (kind: Stack(dir: Vertical), children: [
            (kind: Slider(bind: Setting(TextScale), min: 1.0, max: 1.6, step: Some(0.1))),
            (kind: Label(text: Bind(Setting(TextScale)))),
            (kind: Toggle(text: Literal("HC"), bind: Setting(HighContrast))),
            (kind: Toggle(text: Literal("RM"), bind: Setting(ReducedMotion))),
            (kind: Toggle(text: Literal("SR"), bind: Setting(ScreenReader))),
        ]),
    )"#;

    #[test]
    fn accessibility_settings_drive_toggles_and_the_text_scale_slider() {
        let mut c = ctx();
        c.prefs.text_scale = 1.2;
        let mut s = DataScreen::new(from_ron(A11Y_SCREEN).unwrap(), &Theme::default(), &Texts::builtin("en"), c);
        // Slider 1, label readout 2, toggles 3..5.
        assert_eq!(click(&mut s, 3), vec![UiEvent::SetHighContrast(true)]);
        assert_eq!(click(&mut s, 4), vec![UiEvent::SetReducedMotion(true)]);
        assert_eq!(click(&mut s, 5), vec![UiEvent::SetScreenReader(false)]);
        // A click in the middle of the slider is 1.3 (1.0 + 0.5 * 0.6).
        let p = center_of(&mut s, 1);
        s.handle(UiInput::PointerMove(p));
        let mut ev = s.handle(UiInput::PointerDown(p));
        ev.extend(s.handle(UiInput::PointerUp(p)));
        match no_sound(ev).as_slice() {
            [UiEvent::SetTextScale(v)] => assert!((v - 1.3).abs() < 0.051, "{v}"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(s.ctx.prefs.high_contrast && s.ctx.prefs.reduced_motion);
        assert_eq!(s.ctx.prefs.screen_reader, Some(false));
    }

    #[test]
    fn the_screen_reader_toggle_is_left_out_when_the_host_hides_the_option() {
        let mut c = ctx();
        c.prefs.screen_reader = None;
        let mut s = DataScreen::new(from_ron(A11Y_SCREEN).unwrap(), &Theme::default(), &Texts::builtin("en"), c);
        assert!(s.ui().find(5).is_none());
        assert!(s.ui().find(4).is_some());
    }

    fn stock(id: &str) -> crate::ScreenAsset {
        orbit_ui_data::UiBundle::builtin().screen(id).unwrap().clone()
    }

    #[test]
    fn the_stock_gallery_shows_unlocked_images_and_locked_cards() {
        let mut c = ctx();
        c.gallery = vec!["bg/a.png".into(), "bg/b.png".into(), "bg/c.png".into()];
        c.unlocked.insert("bg/b.png".into());
        let mut s = DataScreen::new(stock("gallery"), &Theme::default(), &Texts::builtin("en"), c);
        let nodes = &s.ui().nodes;
        let images = nodes.iter().filter(|n| matches!(n.kind, crate::widgets::Kind::Image { .. })).count();
        let locked = nodes
            .iter()
            .filter(|n| matches!(&n.kind, crate::widgets::Kind::Label(l) if l.text == "Locked"))
            .count();
        assert_eq!((images, locked), (1, 2));
        // Nothing unlocked and no image at all: the empty text.
        let mut c = ctx();
        c.gallery.clear();
        let mut s = DataScreen::new(stock("gallery"), &Theme::default(), &Texts::builtin("en"), c);
        assert!(s.ui().nodes.iter().any(
            |n| matches!(&n.kind, crate::widgets::Kind::Label(l) if l.text == "Nothing to see yet.")
        ));
    }

    #[test]
    fn the_stock_replay_screen_lists_seen_scenes_and_replays_them() {
        let mut c = ctx();
        c.scenes = vec![
            ("01A".into(), "intro".into(), true),
            ("01B".into(), "secret".into(), false),
            ("01C".into(), "ending".into(), true),
        ];
        let mut s = DataScreen::new(stock("replay"), &Theme::default(), &Texts::builtin("en"), c);
        // Tags: one button per seen scene, then Back.
        assert_eq!(click(&mut s, 2), vec![UiEvent::ReplayScene("ending".into())]);
        assert_eq!(click(&mut s, 1), vec![UiEvent::ReplayScene("intro".into())]);
        assert_eq!(click(&mut s, 3), vec![UiEvent::Back]);
    }

    const BAG_SCREEN: &str = r#"(id: "bag", root: (kind: Stack(dir: Vertical), children: [
        (kind: Repeat(source: Var("bag"), empty: Some(Literal("Nothing"))), children: [
            (kind: Button(text: Bind(Item), action: [
                VarSet((var: "bag", op: Remove, value: Item)),
                CloseWith(Item),
            ])),
            (kind: Label(text: Bind(ItemIndex))),
        ]),
        (kind: Button(text: Bind(Param("who")), action: [CloseWith(Param("who"))])),
    ]))"#;

    fn bag_screen(items: &[&str]) -> DataScreen {
        let mut c = ctx();
        c.vars.insert(
            "bag".into(),
            Value::list(orbit_core::ScalarType::Str, items.iter().map(|s| Value::from(*s))),
        );
        DataScreen::with_params(
            from_ron(BAG_SCREEN).unwrap(),
            &Theme::default(),
            &Texts::builtin("en"),
            c,
            vec![("who".into(), Value::from("Aiko"))],
        )
    }

    #[test]
    fn a_repeat_builds_one_row_per_element_with_item_actions() {
        let mut s = bag_screen(&["key", "map"]);
        // Tags: row 0 button (1), row 1 button (2), the param button (3).
        let remove = |v: &str| {
            UiEvent::VarOp(VarOp::new(
                "bag",
                orbit_core::OpKind::Remove,
                orbit_core::Operand::Lit(Value::from(v)),
            ))
        };
        assert_eq!(
            click(&mut s, 2),
            vec![remove("map"), UiEvent::BackWith(Value::from("map"))]
        );
        // The local change rebuilt the list: one row left.
        assert!(s.ui().find(3).is_none());
        assert_eq!(
            click(&mut s, 2),
            vec![UiEvent::BackWith(Value::from("Aiko"))],
            "parameters are read by bindings and close values"
        );
        let mut s = bag_screen(&["key", "map"]);
        assert_eq!(
            click(&mut s, 1),
            vec![remove("key"), UiEvent::BackWith(Value::from("key"))]
        );
    }

    #[test]
    fn an_empty_repeat_shows_its_empty_text() {
        let mut s = bag_screen(&[]);
        // Only the parameter button remains.
        assert!(s.ui().find(1).is_some() && s.ui().find(2).is_none());
        assert_eq!(s.params(), &[("who".to_owned(), Value::from("Aiko"))]);
        // A bigger list from the shell rebuilds the rows.
        let mut c = ctx();
        c.vars.insert(
            "bag".into(),
            Value::list(orbit_core::ScalarType::Str, [Value::from("x")]),
        );
        s.set_context(c);
        assert!(s.ui().find(2).is_some());
    }
}
