//! The document model of the UI editor: pure logic (no egui), unit tested.
//!
//! * widget tree operations by path (`Vec<usize>` of child indices);
//! * screen templates and the screen / style / splash lists of a bundle;
//! * [`UiDoc`]: the project's `ui/` folder as a [`UiBundle`] with a snapshot
//!   undo / redo stack, validation and save / load.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use orbit_ui::asset::{SYSTEM_SCREENS, to_ron};
use orbit_ui::{
    Align, Anchor, BindPath, Dir, Issue, LayoutDef, PanelKind, ScreenAsset,
    SettingKey, Severity, Size, SplashContent, SplashDef, Style, TextRef, Theme, UiAction,
    UiBundle, WidgetDef, WidgetKind,
};

/// Position of a widget in a screen: child indices from the root (empty =
/// the root itself).
pub type WidgetPath = Vec<usize>;

/// Screens the shell opens by id besides the ones of the bundle.
pub const BUILTIN_SCREEN_IDS: [&str; 5] = ["preferences", "save", "load", "history", "pause"];

// ------------------------------------------------------------ widget kinds

/// The kinds of widget one can add (a [`WidgetKind`] without its data).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindTag {
    Panel,
    Stack,
    Grid,
    Button,
    Label,
    Image,
    Slider,
    Toggle,
    Dropdown,
    ScrollList,
    SaveSlots,
}

impl KindTag {
    pub const ALL: [KindTag; 11] = [
        KindTag::Panel,
        KindTag::Stack,
        KindTag::Grid,
        KindTag::Button,
        KindTag::Label,
        KindTag::Image,
        KindTag::Slider,
        KindTag::Toggle,
        KindTag::Dropdown,
        KindTag::ScrollList,
        KindTag::SaveSlots,
    ];

    pub fn of(kind: &WidgetKind) -> Self {
        match kind {
            WidgetKind::Panel { .. } => Self::Panel,
            WidgetKind::Stack { .. } => Self::Stack,
            WidgetKind::Grid { .. } => Self::Grid,
            WidgetKind::Button { .. } => Self::Button,
            WidgetKind::Label { .. } => Self::Label,
            WidgetKind::Image { .. } => Self::Image,
            WidgetKind::Slider { .. } => Self::Slider,
            WidgetKind::Toggle { .. } => Self::Toggle,
            WidgetKind::Dropdown { .. } => Self::Dropdown,
            WidgetKind::ScrollList => Self::ScrollList,
            WidgetKind::SaveSlots { .. } => Self::SaveSlots,
        }
    }

    /// Name used in text keys (`uied.kind.<name>`) and as the default id.
    pub fn name(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Stack => "stack",
            Self::Grid => "grid",
            Self::Button => "button",
            Self::Label => "label",
            Self::Image => "image",
            Self::Slider => "slider",
            Self::Toggle => "toggle",
            Self::Dropdown => "dropdown",
            Self::ScrollList => "scroll_list",
            Self::SaveSlots => "save_slots",
        }
    }

    pub fn icon(self) -> &'static str {
        use orbit_editor_api::icons;
        match self {
            Self::Panel => icons::SQUARE,
            Self::Stack => icons::ROWS,
            Self::Grid => icons::GRID_FOUR,
            Self::Button => icons::CURSOR_CLICK,
            Self::Label => icons::TEXT_T,
            Self::Image => icons::IMAGE,
            Self::Slider => icons::SLIDERS_HORIZONTAL,
            Self::Toggle => icons::TOGGLE_LEFT,
            Self::Dropdown => icons::CARET_CIRCLE_DOWN,
            Self::ScrollList => icons::LIST,
            Self::SaveSlots => icons::FLOPPY_DISK,
        }
    }

    /// Whether widgets of this kind hold children.
    pub fn is_container(self) -> bool {
        matches!(
            self,
            Self::Panel | Self::Stack | Self::Grid | Self::ScrollList
        )
    }

    /// A widget of this kind with sensible defaults.
    pub fn default_widget(self) -> WidgetDef {
        let kind = match self {
            Self::Panel => WidgetKind::Panel {
                kind: PanelKind::Normal,
                dir: Dir::Vertical,
            },
            Self::Stack => WidgetKind::Stack { dir: Dir::Vertical },
            Self::Grid => WidgetKind::Grid { columns: 2 },
            Self::Button => WidgetKind::Button {
                text: TextRef::Literal("Button".into()),
                action: Vec::new(),
            },
            Self::Label => WidgetKind::Label {
                text: TextRef::Literal("Label".into()),
                size: None,
                center: false,
                dim: false,
            },
            Self::Image => WidgetKind::Image { asset: None },
            Self::Slider => WidgetKind::Slider {
                bind: BindPath::Setting(SettingKey::MasterVolume),
                min: 0.0,
                max: 1.0,
                step: None,
            },
            Self::Toggle => WidgetKind::Toggle {
                text: TextRef::Literal("Toggle".into()),
                bind: BindPath::Setting(SettingKey::Fullscreen),
            },
            Self::Dropdown => WidgetKind::Dropdown {
                bind: BindPath::Setting(SettingKey::Language),
                options: Vec::new(),
            },
            Self::ScrollList => WidgetKind::ScrollList,
            Self::SaveSlots => WidgetKind::SaveSlots {
                mode: orbit_ui::SlotsMode::Save,
                columns: 3,
                rows: 2,
            },
        };
        WidgetDef::new(kind)
    }
}

// --------------------------------------------------------------- tree ops

pub fn get<'a>(root: &'a WidgetDef, path: &[usize]) -> Option<&'a WidgetDef> {
    let mut node = root;
    for &i in path {
        node = node.children.get(i)?;
    }
    Some(node)
}

pub fn get_mut<'a>(root: &'a mut WidgetDef, path: &[usize]) -> Option<&'a mut WidgetDef> {
    let mut node = root;
    for &i in path {
        node = node.children.get_mut(i)?;
    }
    Some(node)
}

/// An id (from `base`) no widget of `screen` has.
pub fn unique_widget_id(screen: &ScreenAsset, base: &str) -> String {
    let mut taken = std::collections::HashSet::new();
    screen.root.walk(&mut |w| {
        taken.insert(w.id.clone());
    });
    unique_name(base, |n| taken.contains(n))
}

fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|n| !taken(n))
        .expect("an unbounded range")
}

/// Adds `def` as the last child of the widget at `parent`; when that widget
/// cannot hold children, right after it instead. Returns the new widget's
/// path. `None` for an unknown path.
pub fn add_child(
    screen: &mut ScreenAsset,
    parent: &[usize],
    mut def: WidgetDef,
) -> Option<WidgetPath> {
    if def.id.is_empty() {
        let kind = KindTag::of(&def.kind);
        def.id = unique_widget_id(screen, kind.name());
    }
    let target = get(&screen.root, parent)?;
    if KindTag::of(&target.kind).is_container() {
        let node = get_mut(&mut screen.root, parent)?;
        node.children.push(def);
        let mut path = parent.to_vec();
        path.push(node.children.len() - 1);
        Some(path)
    } else {
        let (&index, grand) = parent.split_last()?;
        let node = get_mut(&mut screen.root, grand)?;
        node.children.insert(index + 1, def);
        let mut path = grand.to_vec();
        path.push(index + 1);
        Some(path)
    }
}

/// Removes the widget at `path` (never the root). Returns it.
pub fn remove(screen: &mut ScreenAsset, path: &[usize]) -> Option<WidgetDef> {
    let (&index, parent) = path.split_last()?;
    let node = get_mut(&mut screen.root, parent)?;
    (index < node.children.len()).then(|| node.children.remove(index))
}

/// Copies the widget at `path` right after it (ids made unique). Returns the
/// copy's path.
pub fn duplicate(screen: &mut ScreenAsset, path: &[usize]) -> Option<WidgetPath> {
    let (&index, parent) = path.split_last()?;
    let mut copy = get(&screen.root, path)?.clone();
    let mut taken = std::collections::HashSet::new();
    screen.root.walk(&mut |w| {
        taken.insert(w.id.clone());
    });
    fn renew(w: &mut WidgetDef, taken: &mut std::collections::HashSet<String>) {
        if !w.id.is_empty() {
            let id = unique_name(&w.id, |n| taken.contains(n));
            taken.insert(id.clone());
            w.id = id;
        }
        for c in &mut w.children {
            renew(c, taken);
        }
    }
    renew(&mut copy, &mut taken);
    let node = get_mut(&mut screen.root, parent)?;
    node.children.insert(index + 1, copy);
    let mut out = parent.to_vec();
    out.push(index + 1);
    Some(out)
}

/// Swaps the widget with its previous (`-1`) or next (`1`) sibling. Returns
/// the widget's new path, `None` when it cannot move that way.
pub fn move_by(screen: &mut ScreenAsset, path: &[usize], delta: i32) -> Option<WidgetPath> {
    let (&index, parent) = path.split_last()?;
    let node = get_mut(&mut screen.root, parent)?;
    let to = index as i64 + i64::from(delta);
    if to < 0 || to as usize >= node.children.len() || index >= node.children.len() {
        return None;
    }
    node.children.swap(index, to as usize);
    let mut out = parent.to_vec();
    out.push(to as usize);
    Some(out)
}

/// Moves the widget at `from` into the widget at `into` (as its `index`th
/// child; clamped). Refuses to move a widget into itself or its descendants,
/// the root, and into a leaf widget. Returns the new path.
pub fn move_to(
    screen: &mut ScreenAsset,
    from: &[usize],
    into: &[usize],
    index: usize,
) -> Option<WidgetPath> {
    if from.is_empty() || into.starts_with(from) {
        return None;
    }
    if !KindTag::of(&get(&screen.root, into)?.kind).is_container() {
        return None;
    }
    let moved = remove(screen, from)?;
    let depth = from.len() - 1;
    // Dropped in the list it came from, below its old place: the removal
    // shifted the slots.
    let mut index = index;
    if into == &from[..depth] && from[depth] < index {
        index -= 1;
    }
    // Removing `from` shifts the later siblings of its parent: fix `into`.
    let mut into = into.to_vec();
    if into.len() > depth && into[..depth] == from[..depth] && into[depth] > from[depth] {
        into[depth] -= 1;
    }
    let node = get_mut(&mut screen.root, &into)?;
    let index = index.min(node.children.len());
    node.children.insert(index, moved);
    into.push(index);
    Some(into)
}

/// Every widget path of the screen with its depth, in tree order.
pub fn all_paths(root: &WidgetDef) -> Vec<(WidgetPath, usize)> {
    fn rec(w: &WidgetDef, path: &mut WidgetPath, out: &mut Vec<(WidgetPath, usize)>) {
        out.push((path.clone(), path.len()));
        for (i, c) in w.children.iter().enumerate() {
            path.push(i);
            rec(c, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    rec(root, &mut Vec::new(), &mut out);
    out
}

// --------------------------------------------------------------- screens

/// Where a new screen starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    Blank,
    MainMenu,
    Settings,
    Pause,
    Save,
    Load,
}

impl Template {
    pub const ALL: [Template; 6] = [
        Template::Blank,
        Template::MainMenu,
        Template::Settings,
        Template::Pause,
        Template::Save,
        Template::Load,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Blank => "blank",
            Self::MainMenu => "main_menu",
            Self::Settings => "settings",
            Self::Pause => "pause",
            Self::Save => "save",
            Self::Load => "load",
        }
    }

    /// The id the screen gets when it is free: the system names, so the new
    /// screen replaces the stock one right away.
    pub fn default_id(self) -> &'static str {
        match self {
            Self::Blank => "screen",
            Self::MainMenu => "main_menu",
            Self::Settings => "preferences",
            Self::Pause => "pause",
            Self::Save => "save",
            Self::Load => "load",
        }
    }
}

/// A fresh screen from `template` whose id is based on `base` and unique
/// among `bundle`'s screens.
pub fn new_screen(bundle: &UiBundle, template: Template, base: &str) -> ScreenAsset {
    let id = unique_name(base, |n| bundle.screen(n).is_some());
    let mut screen = match template {
        Template::Blank => blank_screen(),
        t => {
            let stock = UiBundle::builtin();
            let source = match t {
                Template::MainMenu => "main_menu",
                Template::Settings => "preferences",
                Template::Save => "save",
                Template::Load => "load",
                _ => "pause",
            };
            stock.screen(source).cloned().unwrap_or_else(blank_screen)
        }
    };
    screen.id = id;
    screen
}

fn blank_screen() -> ScreenAsset {
    let mut root = KindTag::Panel.default_widget();
    root.id = "root".into();
    root.layout = LayoutDef {
        width: Size::Fixed(640.0),
        pad: [60.0, 46.0],
        gap: 18.0,
        cross: Align::Stretch,
        anchor: Some(Anchor::new([0.5, 0.5], [0.5, 0.5], [0.0, 0.0])),
        ..LayoutDef::default()
    };
    let mut title = KindTag::Label.default_widget();
    title.id = "title".into();
    title.style = Some("heading".into());
    title.kind = WidgetKind::Label {
        text: TextRef::Literal("Title".into()),
        size: None,
        center: true,
        dim: false,
    };
    let mut back = KindTag::Button.default_widget();
    back.id = "back".into();
    back.kind = WidgetKind::Button {
        text: TextRef::Key("ui.menu.back".into()),
        action: vec![UiAction::Close],
    };
    root.children = vec![title, back];
    ScreenAsset {
        ui_version: orbit_ui::UI_VERSION,
        id: "screen".into(),
        modal: false,
        pauses_game: false,
        dim_background: true,
        on_cancel: vec![UiAction::Close],
        root,
    }
}

/// A screen id (and file name stem): letters, digits, `_` and `-`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn fix_actions(actions: &mut [UiAction], old: &str, new: &str) {
    for a in actions {
        if let UiAction::Open(id) | UiAction::Replace(id) = a
            && id == old
        {
            *id = new.to_owned();
        }
    }
}

/// Renames a screen and everything that points to it (`Open` / `Replace`
/// actions, system screen links).
pub fn rename_screen(bundle: &mut UiBundle, old: &str, new: &str) -> Result<(), String> {
    if !valid_id(new) {
        return Err("invalid id".into());
    }
    if old == new {
        return Ok(());
    }
    if bundle.screen(new).is_some() {
        return Err("id taken".into());
    }
    if bundle.screen(old).is_none() {
        return Err("unknown screen".into());
    }
    for screen in &mut bundle.screens {
        if screen.id == old {
            screen.id = new.to_owned();
        }
        fix_actions(&mut screen.on_cancel, old, new);
        walk_mut(&mut screen.root, &mut |w| {
            if let WidgetKind::Button { action, .. } = &mut w.kind {
                fix_actions(action, old, new);
            }
        });
    }
    for target in bundle.system_screens.values_mut() {
        if target == old {
            *target = new.to_owned();
        }
    }
    Ok(())
}

fn walk_mut(w: &mut WidgetDef, f: &mut impl FnMut(&mut WidgetDef)) {
    f(w);
    for c in &mut w.children {
        walk_mut(c, f);
    }
}

/// Copies the screen `id` under a new unique id (`id_copy`).
pub fn duplicate_screen(bundle: &mut UiBundle, id: &str) -> Option<String> {
    let mut copy = bundle.screen(id)?.clone();
    copy.id = unique_name(&format!("{id}_copy"), |n| bundle.screen(n).is_some());
    let new_id = copy.id.clone();
    bundle.screens.push(copy);
    Some(new_id)
}

/// Removes the screen `id` and the system links to it.
pub fn delete_screen(bundle: &mut UiBundle, id: &str) -> bool {
    let before = bundle.screens.len();
    bundle.screens.retain(|s| s.id != id);
    bundle.system_screens.retain(|_, target| target != id);
    bundle.screens.len() != before
}

/// Which system screen (`main_menu`...) the screen stands in for: through the
/// `system_screens` table, or by carrying the system name itself.
pub fn system_role(bundle: &UiBundle, id: &str) -> Option<&'static str> {
    SYSTEM_SCREENS
        .iter()
        .find(|name| bundle.system_screen(name).is_some_and(|s| s.id == id))
        .copied()
}

/// Makes the screen `id` the one standing in for the system screen `role`
/// (`None`: stops standing in for any).
pub fn set_system_role(bundle: &mut UiBundle, id: &str, role: Option<&str>) {
    bundle.system_screens.retain(|_, target| target != id);
    if let Some(role) = role {
        bundle.system_screens.insert(role.to_owned(), id.to_owned());
    }
}

// ----------------------------------------------------------------- styles

/// Renames a style of the theme and the references to it.
pub fn rename_style(
    theme: &mut Theme,
    screens: &mut [ScreenAsset],
    old: &str,
    new: &str,
) -> Result<(), String> {
    if new.trim().is_empty() {
        return Err("empty name".into());
    }
    if theme.styles.contains_key(new) {
        return Err("name taken".into());
    }
    let style = theme.styles.remove(old).ok_or("unknown style")?;
    theme.styles.insert(new.to_owned(), style);
    retarget_style(theme, screens, old, Some(new));
    Ok(())
}

/// Removes a style; widgets that used it go back to the default look.
pub fn delete_style(theme: &mut Theme, screens: &mut [ScreenAsset], name: &str) -> bool {
    let existed = theme.styles.remove(name).is_some();
    retarget_style(theme, screens, name, None);
    existed
}

fn retarget_style(theme: &mut Theme, screens: &mut [ScreenAsset], old: &str, new: Option<&str>) {
    for style in theme.styles.values_mut() {
        if style.extends.as_deref() == Some(old) {
            style.extends = new.map(str::to_owned);
        }
    }
    for screen in screens {
        walk_mut(&mut screen.root, &mut |w| {
            if w.style.as_deref() == Some(old) {
                w.style = new.map(str::to_owned);
            }
            if w.overrides.extends.as_deref() == Some(old) {
                w.overrides.extends = new.map(str::to_owned);
            }
        });
    }
}

/// An empty style called `base` (made unique).
pub fn add_style(theme: &mut Theme, base: &str) -> String {
    let name = unique_name(base, |n| theme.styles.contains_key(n));
    theme.styles.insert(name.clone(), Style::default());
    name
}

// ---------------------------------------------------------------- splashes

/// A new splash showing the image `path`.
pub fn new_splash(path: &str) -> SplashDef {
    SplashDef {
        content: SplashContent::Image(path.to_owned()),
        duration: 3.0,
        fade_in: 0.5,
        fade_out: 0.5,
        skippable: true,
        background: [0, 0, 0, 255],
    }
}

/// Moves a splash one place up (`-1`) or down (`1`); its new index.
pub fn move_splash(splashes: &mut [SplashDef], index: usize, delta: i32) -> Option<usize> {
    let to = index as i64 + i64::from(delta);
    if index >= splashes.len() || to < 0 || to as usize >= splashes.len() {
        return None;
    }
    splashes.swap(index, to as usize);
    Some(to as usize)
}

// -------------------------------------------------------------- diagnostics

/// Problems the schema validation does not see: actions opening a screen
/// that does not exist.
pub fn dangling_actions(bundle: &UiBundle) -> Vec<Issue> {
    let mut out = Vec::new();
    let known = |id: &str| bundle.screen(id).is_some() || BUILTIN_SCREEN_IDS.contains(&id);
    for screen in &bundle.screens {
        let mut check = |actions: &[UiAction], widget: &str| {
            for a in actions {
                if let UiAction::Open(id) | UiAction::Replace(id) = a
                    && !known(id)
                {
                    out.push(Issue {
                        severity: Severity::Warning,
                        screen: screen.id.clone(),
                        widget: widget.to_owned(),
                        message: format!("opens `{id}`, which is not a screen of the project"),
                    });
                }
            }
        };
        check(&screen.on_cancel, "");
        screen.root.walk(&mut |w| {
            if let WidgetKind::Button { action, .. } = &w.kind {
                check(action, &w.id);
            }
        });
    }
    out
}

// ------------------------------------------------------------- the document

/// How long edits of the same field count as one undo step.
const MERGE_WINDOW: Duration = Duration::from_millis(900);
const MAX_HISTORY: usize = 100;

/// The project's `ui/` folder, being edited.
pub struct UiDoc {
    pub dir: PathBuf,
    pub bundle: UiBundle,
    saved: UiBundle,
    /// The folder has files (`false`: the project uses the stock screens).
    pub exists: bool,
    undo: Vec<UiBundle>,
    redo: Vec<UiBundle>,
    last_edit: Option<(String, Instant)>,
    /// Bumped by every change of `bundle` (the preview follows it).
    pub revision: u64,
}

impl UiDoc {
    /// Reads `dir` (a missing or empty folder gives an empty document).
    pub fn load(dir: &Path) -> Result<Self, String> {
        let has_files = dir.is_dir()
            && std::fs::read_dir(dir)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
        let bundle = if has_files {
            UiBundle::load_dir(dir)?
        } else {
            UiBundle::default()
        };
        Ok(Self {
            dir: dir.to_owned(),
            saved: bundle.clone(),
            bundle,
            exists: has_files,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            revision: 1,
        })
    }

    pub fn dirty(&self) -> bool {
        self.bundle != self.saved
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Edits the bundle as one undo step.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut UiBundle) -> R) -> R {
        self.edit_inner(None, f)
    }

    /// Like [`UiDoc::edit`], but edits with the same `key` in a short time
    /// (typing, dragging a value) share one undo step.
    pub fn edit_merged<R>(&mut self, key: &str, f: impl FnOnce(&mut UiBundle) -> R) -> R {
        self.edit_inner(Some(key), f)
    }

    fn edit_inner<R>(&mut self, key: Option<&str>, f: impl FnOnce(&mut UiBundle) -> R) -> R {
        let before = self.bundle.clone();
        let out = f(&mut self.bundle);
        if self.bundle == before {
            return out;
        }
        let now = Instant::now();
        let merge = match (&self.last_edit, key) {
            (Some((last, at)), Some(key)) => last == key && now.duration_since(*at) < MERGE_WINDOW,
            _ => false,
        };
        if !merge {
            self.undo.push(before);
            if self.undo.len() > MAX_HISTORY {
                self.undo.remove(0);
            }
        }
        self.last_edit = key.map(|k| (k.to_owned(), now));
        self.redo.clear();
        self.revision += 1;
        out
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.bundle, previous));
        self.last_edit = None;
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.bundle, next));
        self.last_edit = None;
        self.revision += 1;
        true
    }

    /// Starts a project from the stock theme and screens (kept as they are if
    /// the project already has some).
    pub fn enable_defaults(&mut self) {
        self.edit(|b| {
            let stock = UiBundle::builtin();
            if b.theme.is_none() {
                b.theme = stock.theme;
            }
            for screen in stock.screens {
                if b.screen(&screen.id).is_none() {
                    b.screens.push(screen);
                }
            }
        });
    }

    /// Problems of the bundle (errors first) and of the actions.
    pub fn diagnostics(&self) -> Vec<Issue> {
        let mut issues = self.bundle.validate();
        issues.extend(dangling_actions(&self.bundle));
        issues.sort_by_key(|i| i.severity != Severity::Error);
        issues
    }

    /// The bundle as the RON text the preview and the runtime read.
    pub fn bundle_ron(&self) -> String {
        self.bundle.to_ron().unwrap_or_default()
    }

    /// Writes the bundle to the folder (screens, theme, `ui.ron`) and
    /// removes the files of screens that no longer exist.
    pub fn save(&mut self) -> Result<(), String> {
        save_bundle(&self.dir, &self.bundle)?;
        self.saved = self.bundle.clone();
        self.exists = true;
        Ok(())
    }

    /// Reads the folder again, dropping unsaved edits and the history.
    pub fn reload(&mut self) -> Result<(), String> {
        let revision = self.revision;
        *self = Self::load(&self.dir.clone())?;
        self.revision = revision + 1;
        Ok(())
    }
}

/// `<id>.screen.ron`, the id made safe for a file name.
pub fn screen_file_name(id: &str) -> String {
    let stem: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{stem}.screen.ron")
}

/// Writes `bundle` into `dir`: one `<id>.screen.ron` per screen, `theme.ron`,
/// and `ui.ron` for the system links and splashes. Files that no longer
/// match anything are removed.
pub fn save_bundle(dir: &Path, bundle: &UiBundle) -> Result<(), String> {
    let write = |name: &str, text: String| {
        std::fs::write(dir.join(name), text).map_err(|e| format!("{name}: {e}"))
    };
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut kept = std::collections::BTreeSet::new();
    for screen in &bundle.screens {
        let name = screen_file_name(&screen.id);
        if !kept.insert(name.clone()) {
            return Err(format!("two screens would be saved as {name}"));
        }
        write(&name, to_ron(screen)?)?;
    }
    for entry in std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".screen.ron") && !kept.contains(&name) {
            std::fs::remove_file(entry.path()).map_err(|e| format!("{name}: {e}"))?;
        }
    }
    match &bundle.theme {
        Some(theme) => write("theme.ron", theme.to_ron().map_err(|e| e.to_string())?)?,
        None => remove_if_present(&dir.join("theme.ron"))?,
    }
    if bundle.system_screens.is_empty() && bundle.splashes.is_empty() {
        remove_if_present(&dir.join("ui.ron"))?;
    } else {
        let rest = UiBundle {
            system_screens: bundle.system_screens.clone(),
            splashes: bundle.splashes.clone(),
            ..UiBundle::default()
        };
        write("ui.ron", rest.to_ron()?)?;
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}


// -------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use orbit_ui::asset::from_ron;
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "orbit-uied-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn two_buttons() -> ScreenAsset {
        let mut s = blank_screen();
        let b = add_child(&mut s, &[], KindTag::Button.default_widget()).unwrap();
        assert_eq!(b, vec![2]);
        s
    }

    #[test]
    fn every_template_is_a_valid_screen() {
        let bundle = UiBundle::default();
        for t in Template::ALL {
            let s = new_screen(&bundle, t, t.default_id());
            assert_eq!(s.id, t.default_id());
            let mut b = UiBundle::builtin();
            b.screens.retain(|x| x.id != s.id);
            b.screens.push(s);
            assert!(b.is_valid(), "{t:?}: {:?}", b.validate());
        }
        let mut taken = UiBundle::default();
        taken.screens.push(blank_screen());
        assert_eq!(new_screen(&taken, Template::Blank, "screen").id, "screen_2");
    }

    #[test]
    fn adding_into_a_leaf_puts_the_widget_after_it() {
        let mut s = blank_screen();
        // `title` (path [0]) is a label: the new widget lands after it.
        let p = add_child(&mut s, &[0], KindTag::Slider.default_widget()).unwrap();
        assert_eq!(p, vec![1]);
        assert_eq!(s.root.children.len(), 3);
        assert!(matches!(s.root.children[1].kind, WidgetKind::Slider { .. }));
        // Into a container: last child, with a generated unique id.
        let p = add_child(&mut s, &[], KindTag::Button.default_widget()).unwrap();
        assert_eq!(p, vec![3]);
        let p2 = add_child(&mut s, &[], KindTag::Button.default_widget()).unwrap();
        assert_ne!(get(&s.root, &p).unwrap().id, get(&s.root, &p2).unwrap().id);
        assert!(add_child(&mut s, &[9], KindTag::Label.default_widget()).is_none());
    }

    #[test]
    fn remove_duplicate_and_move() {
        let mut s = two_buttons();
        assert!(remove(&mut s, &[]).is_none(), "the root stays");
        let dup = duplicate(&mut s, &[1]).unwrap();
        assert_eq!(dup, vec![2]);
        let (a, b) = (s.root.children[1].id.clone(), s.root.children[2].id.clone());
        assert_ne!(a, b);
        assert_eq!(move_by(&mut s, &[2], -1), Some(vec![1]));
        assert_eq!(s.root.children[1].id, b);
        assert_eq!(move_by(&mut s, &[0], -1), None);
        assert_eq!(move_by(&mut s, &[3], 1), None);
        assert!(remove(&mut s, &[1]).is_some());
        assert_eq!(s.root.children.len(), 3);
        assert!(remove(&mut s, &[7]).is_none());
        let paths: Vec<_> = all_paths(&s.root).into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, vec![vec![], vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn moving_into_a_container_fixes_the_path() {
        let mut s = blank_screen();
        let group = add_child(&mut s, &[], KindTag::Stack.default_widget()).unwrap(); // [2]
        // Move `title` ([0]) into the group: the group slides to [1].
        let p = move_to(&mut s, &[0], &group, 0).unwrap();
        assert_eq!(p, vec![1, 0]);
        assert_eq!(s.root.children[1].children[0].id, "title");
        // Not into itself / a descendant / a leaf.
        assert!(move_to(&mut s, &[1], &[1, 0], 0).is_none());
        assert!(move_to(&mut s, &[1], &[1], 0).is_none());
        assert!(move_to(&mut s, &[0], &[1, 0], 0).is_none(), "a label is a leaf");
        assert!(move_to(&mut s, &[], &[0], 0).is_none());
    }

    #[test]
    fn dropping_inside_the_same_list_lands_where_it_was_dropped() {
        let mut s = two_buttons(); // title, back, button
        let ids = |s: &ScreenAsset| s.root.children.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        // Dropped "after the last one" (slot 3): the title ends up last.
        assert_eq!(move_to(&mut s, &[0], &[], 3), Some(vec![2]));
        assert_eq!(ids(&s), ["back", "button", "title"]);
        // Dropped before its own place: no shift.
        assert_eq!(move_to(&mut s, &[2], &[], 0), Some(vec![0]));
        assert_eq!(ids(&s), ["title", "back", "button"]);
    }

    #[test]
    fn renaming_a_screen_updates_what_points_to_it() {
        let mut b = UiBundle::builtin();
        let opens = |b: &UiBundle, id: &str| {
            b.screen("pause").unwrap().root.children.iter().any(|w| {
                matches!(&w.kind, WidgetKind::Button { action, .. }
                    if action.contains(&UiAction::Open(id.into())))
            })
        };
        assert!(opens(&b, "preferences"));
        rename_screen(&mut b, "preferences", "settings").unwrap();
        assert!(b.screen("settings").is_some() && b.screen("preferences").is_none());
        assert!(opens(&b, "settings"));
        assert!(rename_screen(&mut b, "settings", "pause").is_err(), "taken");
        assert!(rename_screen(&mut b, "settings", "a b").is_err(), "invalid");
        assert!(rename_screen(&mut b, "ghost", "x").is_err());
    }

    #[test]
    fn system_roles_follow_the_links() {
        let mut b = UiBundle::builtin();
        assert_eq!(system_role(&b, "pause"), Some("pause"));
        let copy = duplicate_screen(&mut b, "pause").unwrap();
        assert_eq!(copy, "pause_copy");
        assert_eq!(system_role(&b, &copy), None);
        set_system_role(&mut b, &copy, Some("pause"));
        assert_eq!(system_role(&b, &copy), Some("pause"));
        assert_eq!(system_role(&b, "pause"), None);
        assert!(delete_screen(&mut b, &copy));
        assert_eq!(system_role(&b, "pause"), Some("pause"), "back to the screen named pause");
        assert!(!delete_screen(&mut b, "ghost"));
    }

    #[test]
    fn styles_rename_and_delete_keep_references_right() {
        let mut b = UiBundle::builtin();
        let mut theme = b.theme.take().unwrap();
        let mine = add_style(&mut theme, "mine");
        add_style(&mut theme, "mine");
        assert_eq!(theme.styles.keys().filter(|k| k.starts_with("mine")).count(), 2);
        theme.styles.get_mut("mine_2").unwrap().extends = Some(mine.clone());
        let mut screens = std::mem::take(&mut b.screens);
        screens[0].root.children[0].style = Some(mine);
        rename_style(&mut theme, &mut screens, "mine", "fancy").unwrap();
        assert_eq!(screens[0].root.children[0].style.as_deref(), Some("fancy"));
        assert_eq!(theme.styles["mine_2"].extends.as_deref(), Some("fancy"));
        assert!(rename_style(&mut theme, &mut screens, "fancy", "mine_2").is_err());
        assert!(delete_style(&mut theme, &mut screens, "fancy"));
        assert_eq!(screens[0].root.children[0].style, None);
        assert_eq!(theme.styles["mine_2"].extends, None);
    }

    #[test]
    fn splashes_reorder() {
        let mut v = vec![new_splash("a.png"), new_splash("b.png"), new_splash("c.png")];
        assert_eq!(move_splash(&mut v, 2, -1), Some(1));
        assert_eq!(v[1].content, SplashContent::Image("c.png".into()));
        assert_eq!(move_splash(&mut v, 0, -1), None);
        assert_eq!(move_splash(&mut v, 2, 1), None);
    }

    #[test]
    fn undo_redo_and_merged_edits() {
        let t = TempDir::new("undo");
        let mut doc = UiDoc::load(&t.0).unwrap();
        assert!(!doc.exists && !doc.dirty());
        doc.enable_defaults();
        assert_eq!(doc.bundle.screens.len(), 5);
        assert!(doc.dirty());
        // Quick edits of one field are one step; another key is another.
        for n in 0..3 {
            doc.edit_merged("splash0", |b| {
                if b.splashes.is_empty() {
                    b.splashes.push(new_splash("a.png"));
                }
                b.splashes[0].duration = 4.0 + n as f32;
            });
        }
        doc.edit(|b| b.splashes.push(new_splash("b.png")));
        let rev = doc.revision;
        doc.edit(|_| {});
        assert_eq!(doc.revision, rev, "a no-op edit changes nothing");
        assert!(doc.undo());
        assert_eq!(doc.bundle.splashes.len(), 1);
        assert!(doc.undo());
        assert!(doc.bundle.splashes.is_empty(), "the merged edits go back together");
        assert!(doc.redo() && doc.redo());
        assert_eq!(doc.bundle.splashes.len(), 2);
        assert!(!doc.redo());
        assert!(doc.undo());
        doc.edit(|b| b.splashes.clear());
        assert!(!doc.can_redo(), "a new edit drops the redo stack");
    }

    #[test]
    fn a_saved_folder_loads_back_the_same_and_drops_deleted_screens() {
        let t = TempDir::new("roundtrip");
        let mut doc = UiDoc::load(&t.0).unwrap();
        doc.enable_defaults();
        doc.edit(|b| {
            b.splashes.push(new_splash("logo.png"));
            b.splashes.push(SplashDef {
                content: SplashContent::Text("Studio".into()),
                ..new_splash("")
            });
            b.system_screens.insert("pause".into(), "pause".into());
        });
        doc.save().unwrap();
        assert!(!doc.dirty() && doc.exists);
        let back = UiDoc::load(&t.0).unwrap();
        // `load_dir` sorts screens by file name: compare as sets.
        let mut a = doc.bundle.clone();
        let mut b = back.bundle.clone();
        a.screens.sort_by(|x, y| x.id.cmp(&y.id));
        b.screens.sort_by(|x, y| x.id.cmp(&y.id));
        assert_eq!(a, b);
        assert!(back.bundle.is_valid());
        // Deleting a screen removes its file; clearing links and splashes
        // removes ui.ron; a missing theme removes theme.ron.
        doc.edit(|b| {
            delete_screen(b, "pause");
            b.splashes.clear();
            b.system_screens.clear();
            b.theme = None;
        });
        doc.save().unwrap();
        assert!(!t.0.join("pause.screen.ron").exists());
        assert!(!t.0.join("ui.ron").exists());
        assert!(!t.0.join("theme.ron").exists());
        assert!(t.0.join("main_menu.screen.ron").exists());
        let again = UiDoc::load(&t.0).unwrap();
        assert_eq!(again.bundle.screens.len(), 4);
        assert!(again.bundle.theme.is_none());
    }

    #[test]
    fn reload_drops_unsaved_edits_and_bad_files_are_reported() {
        let t = TempDir::new("reload");
        let mut doc = UiDoc::load(&t.0).unwrap();
        doc.enable_defaults();
        doc.save().unwrap();
        doc.edit(|b| b.screens.clear());
        doc.reload().unwrap();
        assert_eq!(doc.bundle.screens.len(), 5);
        assert!(!doc.can_undo());
        std::fs::write(t.0.join("broken.screen.ron"), "(id: ").unwrap();
        assert!(UiDoc::load(&t.0).is_err());
    }

    #[test]
    fn diagnostics_list_errors_first_and_dangling_actions() {
        let t = TempDir::new("diag");
        let mut doc = UiDoc::load(&t.0).unwrap();
        doc.enable_defaults();
        doc.edit(|b| {
            let s = &mut b.screens[0];
            let w = add_child(s, &[], KindTag::Button.default_widget()).unwrap();
            if let WidgetKind::Button { action, .. } = &mut get_mut(&mut s.root, &w).unwrap().kind {
                action.push(UiAction::Open("nowhere".into()));
            }
            b.splashes.push(SplashDef {
                duration: 0.0,
                ..new_splash("a.png")
            });
        });
        let issues = doc.diagnostics();
        assert!(issues.iter().any(|i| i.message.contains("nowhere")));
        assert!(issues.iter().any(|i| i.message.contains("duration")));
        assert_eq!(issues[0].severity, Severity::Error, "errors come first");
    }

    #[test]
    fn ids_and_file_names_are_made_safe() {
        assert!(valid_id("main_menu-2") && !valid_id("") && !valid_id("a/b") && !valid_id("é"));
        assert_eq!(screen_file_name("a b/c"), "a_b_c.screen.ron");
        assert!(from_ron(&to_ron(&blank_screen()).unwrap()).is_ok());
    }
}
