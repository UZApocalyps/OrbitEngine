//! [`DataScreen`]: a [`Screen`] built from a [`ScreenAsset`].

use std::collections::{BTreeMap, HashMap};

use orbit_core::{Value, VarOp};
use orbit_render2d::TextureId;

use super::action::{BindPath, SettingKey, UiAction};
use super::slots::{SlotAction, build_slots, slots_page};
use super::widget::{Cond, ScreenAsset, SlotsMode, TextRef, WidgetDef, WidgetKind};
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
}

impl DataContext {
    pub fn eval(&self, cond: &Cond) -> bool {
        match cond {
            Cond::Always => true,
            Cond::Never => false,
            Cond::CanContinue => self.can_continue,
            Cond::CanSave => self.can_save,
            Cond::ShowQuit => self.show_quit,
            Cond::InGame => self.in_game,
            Cond::Not(c) => !self.eval(c),
            Cond::Vars(c) => c.eval(&|n| self.vars.get(n).cloned()).unwrap_or(false),
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
    Toggle,
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
    /// Actions run that have no effect yet (`SceneGoto`, `VarSet`), for
    /// hosts that want to say so.
    unsupported: Vec<UiAction>,
    /// A variable changed locally: the tree is rebuilt after the event.
    vars_dirty: bool,
}

impl DataScreen {
    pub fn new(asset: ScreenAsset, theme: &Theme, texts: &Texts, ctx: DataContext) -> Self {
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
        };
        s.rebuild();
        s
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
        };
        if self.asset.dim_background {
            st.ui
                .add_root(Node::panel(PanelKind::Dim, Dir::Stack).fill());
        }
        let root = self.asset.root.clone();
        self.build(&root, None, &mut Vec::new(), &mut st);
        let mut theme = self.theme.clone();
        theme.styles.extend(std::mem::take(&mut st.styles));
        st.ui.set_theme(theme);
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
        match t {
            TextRef::Key(k) if k == "game.title" => self.ctx.title.clone(),
            TextRef::Key(k) => self.texts.get(k),
            TextRef::Literal(s) => s.clone(),
            TextRef::Bind(BindPath::Setting(key)) => self.format_setting(*key),
            TextRef::Bind(BindPath::Var(name)) => self.format_var(name),
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
        if !self.ctx.eval(&def.visible) {
            return;
        }
        let index = st.counter;
        st.counter += 1;
        let p = &self.ctx.prefs;
        let node = match &def.kind {
            WidgetKind::Panel { kind, dir } => Node::panel(*kind, *dir),
            WidgetKind::Stack { dir } => Node::container(*dir),
            WidgetKind::Grid { columns } => Node::grid(*columns),
            WidgetKind::Button { text, action } => {
                let tag = Self::new_tag(st);
                st.binds.insert(tag, Bound::Button(action.clone()));
                Node::button(tag, self.text(text))
            }
            WidgetKind::Label {
                text,
                size,
                center,
                dim,
            } => {
                let mut n = Node::label(self.text(text));
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
                    };
                    if let Some(readout) = readout {
                        let tag = Self::new_tag(st);
                        st.readouts.push((tag, readout));
                        n = n.tag(tag);
                    }
                }
                n
            }
            WidgetKind::Image { asset } => Node::image(match asset.as_deref() {
                Some(MENU_BACKGROUND) => self.ctx.background,
                _ => None,
            }),
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
                    let n = Node::toggle(tag, self.text(text), *on);
                    return self.finish(n, def, parent, path, index, st);
                }
                // Only the fullscreen toggle exists; the host may not offer it.
                if bind.setting() != Some(SettingKey::Fullscreen) {
                    return;
                }
                let Some(on) = p.fullscreen else { return };
                let tag = Self::new_tag(st);
                st.binds.insert(tag, Bound::Toggle);
                Node::toggle(tag, self.text(text), on)
            }
            WidgetKind::Dropdown { bind, options } => {
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
                Node::selector(tag, names, index)
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
        node.main = l.main;
        node.cross = l.cross;
        node.enabled = self.ctx.eval(&def.enabled);
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
            UiAction::SceneGoto(_) => {
                self.unsupported.push(action.clone());
                None
            }
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
        _ => return None,
    })
}

impl Screen for DataScreen {
    fn handle(&mut self, input: UiInput) -> Vec<UiEvent> {
        let mut out = Vec::new();
        let mut changed = false;
        for ev in self.ui.handle(input) {
            if let WidgetEvent::Sound(s) = ev {
                out.push(UiEvent::Sound(s));
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
                        Some(Bound::Toggle) => {
                            self.ctx.prefs.fullscreen = Some(on);
                            out.push(UiEvent::SetFullscreen(on));
                        }
                        Some(Bound::VarToggle(name)) => {
                            self.set_var(&name, Value::Bool(on), &mut out);
                            changed = true;
                        }
                        _ => {}
                    }
                }
                WidgetEvent::Selected(tag, index) => {
                    if let Some(Bound::Dropdown { key, codes }) = self.binds.get(&tag) {
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
    fn unsupported_actions_do_nothing() {
        let mut s = screen(ctx());
        // New, Save, Quit, Warp -> tag 4 once Continue is absent.
        assert!(click(&mut s, 4).is_empty());
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
        let crate::widgets::Kind::Selector { options, index } = &s.ui().node(id).kind else {
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
    fn unsupported_actions_are_reported_for_the_host() {
        let mut s = screen(ctx());
        assert!(click(&mut s, 4).is_empty());
        assert_eq!(s.take_unsupported(), vec![UiAction::SceneGoto("x".into())]);
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

}
