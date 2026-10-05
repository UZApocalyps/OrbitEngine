//! The document model of a data-driven screen.

use orbit_core::Condition;
use serde::{Deserialize, Serialize};

use crate::action::{BindPath, UiAction};
use crate::migrate;
use crate::theme::Style;
use crate::layout::{Align, Anchor, Dir, PanelKind, Size};
use crate::transition::ScreenTransition;

/// Current version of the screen document format.
pub const UI_VERSION: u32 = 8;

fn default_version() -> u32 {
    UI_VERSION
}

/// One screen: a tree of widgets plus how it behaves in the shell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenAsset {
    #[serde(default = "default_version")]
    pub ui_version: u32,
    /// Name other screens open it by (`Open("settings")`).
    pub id: String,
    /// Drawn above the screens below it, which keep showing.
    #[serde(default)]
    pub modal: bool,
    /// The game stops advancing while the screen is open.
    #[serde(default)]
    pub pauses_game: bool,
    /// A translucent dimming layer is drawn behind the root.
    #[serde(default)]
    pub dim_background: bool,
    /// Run when the player cancels (Escape / right click).
    #[serde(default)]
    pub on_cancel: Vec<UiAction>,
    pub root: WidgetDef,
    /// A heads-up display (`ui_version` 4, UI-14): opened by the story, it is
    /// drawn over the stage and *under* the textbox, never pauses the game,
    /// takes no keyboard focus and lets clicks outside its controls reach
    /// the game. Hidden while a menu (pause, save...) is open.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hud: bool,
    /// Played when the screen opens (`ui_version` 4, UI-16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<ScreenTransition>,
    /// Played when the screen closes (`ui_version` 4, UI-16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_out: Option<ScreenTransition>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl ScreenAsset {
    /// Whether the game keeps running while the screen is open: a HUD never
    /// pauses it, whatever `pauses_game` says.
    pub fn effective_pauses_game(&self) -> bool {
        self.pauses_game && !self.hud
    }
}

/// A text: a key of the interface table, a literal, or a bound value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TextRef {
    Key(String),
    Literal(String),
    Bind(BindPath),
}

/// A condition on the state the shell exposes (`DataContext`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Cond {
    #[default]
    Always,
    Never,
    /// A save exists to continue from.
    CanContinue,
    /// The game can be saved now.
    CanSave,
    /// The host can quit (hidden on the web).
    ShowQuit,
    /// A game is running (as opposed to the title).
    InGame,
    Not(Box<Cond>),
    /// A no-code condition on story variables (`gold >= 10`, all / any).
    Vars(Condition),
    /// Inside a `Repeat` template (`ui_version` 6): the element of the row
    /// is set (`true`, a non-zero number, a non-empty text or collection).
    /// A gallery row tells an unlocked image (its path) from a locked one
    /// (empty) with it. Always false outside a generated list.
    Item,
    /// Inside an `Achievements` row (`ui_version` 7): the achievement of the
    /// row is unlocked. Always false outside such a row.
    Unlocked,
    /// The story defines at least one achievement (`ui_version` 7).
    HasAchievements,
}

/// Size, spacing and placement of a widget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutDef {
    pub width: Size,
    pub height: Size,
    pub pad: [f32; 2],
    pub margin: [f32; 4],
    pub gap: f32,
    pub main: Align,
    pub cross: Align,
    /// Root widget only: where it sits on the canvas (default: top left).
    pub anchor: Option<Anchor>,
    /// Grid children: columns taken (0 counts as 1). `ui_version` 2.
    #[serde(skip_serializing_if = "is_one")]
    pub col_span: usize,
    /// Grid children: rows taken (0 counts as 1). `ui_version` 2.
    #[serde(skip_serializing_if = "is_one")]
    pub row_span: usize,
}

impl Default for LayoutDef {
    fn default() -> Self {
        Self {
            width: Size::Fit,
            height: Size::Fit,
            pad: [0.0; 2],
            margin: [0.0; 4],
            gap: 0.0,
            main: Align::Start,
            cross: Align::Start,
            anchor: None,
            col_span: 1,
            row_span: 1,
        }
    }
}

fn is_one(n: &usize) -> bool {
    *n == 1
}

fn vertical() -> Dir {
    Dir::Vertical
}

fn normal_panel() -> PanelKind {
    PanelKind::Normal
}

fn always() -> Cond {
    Cond::Always
}

/// Whether a [`WidgetKind::SaveSlots`] list saves into or loads from slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotsMode {
    /// Manual slots only; picking a used slot asks before overwriting.
    Save,
    /// Every category as a tab; picking a used slot loads it.
    Load,
}

fn three() -> usize {
    3
}

fn two() -> usize {
    2
}

/// What a widget is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WidgetKind {
    /// A (themed) box: `Normal` has a background and border, `Dim` dims,
    /// `Solid` is the screen background.
    Panel {
        #[serde(default = "normal_panel")]
        kind: PanelKind,
        #[serde(default = "vertical")]
        dir: Dir,
    },
    /// An invisible container (`Vertical`, `Horizontal` or overlapping
    /// `Stack`).
    Stack {
        #[serde(default = "vertical")]
        dir: Dir,
    },
    Grid {
        columns: usize,
    },
    Button {
        text: TextRef,
        #[serde(default)]
        action: Vec<UiAction>,
    },
    Label {
        text: TextRef,
        #[serde(default)]
        size: Option<f32>,
        #[serde(default)]
        center: bool,
        #[serde(default)]
        dim: bool,
    },
    /// An image: `"@menu_background"` is the title background of the game.
    Image {
        #[serde(default)]
        asset: Option<String>,
        /// Shows the image whose asset path this binding holds (`ui_version`
        /// 6), e.g. `Item` in a gallery row; nothing when it is empty or not
        /// loaded. Takes over `asset` when set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind: Option<BindPath>,
        /// Run when the image is clicked (`ui_version` 8): a non-empty list
        /// makes the image a clickable, focusable control (its style can
        /// give it hover / pressed backgrounds).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        action: Vec<UiAction>,
    },
    Slider {
        bind: BindPath,
        min: f32,
        max: f32,
        #[serde(default)]
        step: Option<f32>,
    },
    Toggle {
        text: TextRef,
        bind: BindPath,
    },
    /// A dropdown: it shows the current option and opens a popup list.
    /// `Language` lists the languages of the game; other settings list
    /// `options`. Bound to a story variable (`BindPath::Var`), option `i`
    /// sets the variable to `values[i]` (all of the variable's type) and
    /// the option equal to the variable's value is the selected one.
    Dropdown {
        bind: BindPath,
        #[serde(default)]
        options: Vec<TextRef>,
        /// Variable binding only: the value of each option.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        values: Vec<orbit_core::Value>,
    },
    ScrollList,
    /// The save slots of the game as a paged grid of cards (thumbnail, date,
    /// last line) with category tabs (`Load`), page buttons and a delete
    /// button per card. Picking a card saves or loads (after a confirmation
    /// when it would overwrite); delete always confirms.
    SaveSlots {
        mode: SlotsMode,
        #[serde(default = "three")]
        columns: usize,
        #[serde(default = "two")]
        rows: usize,
    },
    /// A list generated from a story variable or a screen parameter
    /// (`ui_version` 4, UI-15, ADR 0016 §4): the children of the widget are
    /// the template of one row, built once per element of the list (or
    /// entry of the map, in key order) — at most [`MAX_REPEAT_ITEMS`] rows.
    /// Inside the template, `BindPath::Item` / `ItemIndex` / `ItemKey` show
    /// the element, its position (from 0) and its key, and the actions can
    /// use `Operand::Item` & co. ("use this object", "call this contact").
    /// `empty` is shown instead when there is no element.
    Repeat {
        source: BindPath,
        /// How the rows are laid out (`Grid(n)` for a grid of `n` columns).
        #[serde(default = "vertical")]
        dir: Dir,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty: Option<TextRef>,
    },
}

impl WidgetKind {
    /// The actions the widget runs when activated (a button, a clickable
    /// image).
    pub fn actions(&self) -> Option<&Vec<UiAction>> {
        match self {
            WidgetKind::Button { action, .. } | WidgetKind::Image { action, .. } => Some(action),
            _ => None,
        }
    }

    pub fn actions_mut(&mut self) -> Option<&mut Vec<UiAction>> {
        match self {
            WidgetKind::Button { action, .. } | WidgetKind::Image { action, .. } => Some(action),
            _ => None,
        }
    }
}

/// Rows a [`WidgetKind::Repeat`] builds at most (the rest is left out).
pub const MAX_REPEAT_ITEMS: usize = 500;

/// One widget of a screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidgetDef {
    /// Optional, unique inside the screen.
    #[serde(default)]
    pub id: String,
    pub kind: WidgetKind,
    /// Name of a style of the theme.
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub layout: LayoutDef,
    /// The widget is left out while false.
    #[serde(default)]
    pub visible: Cond,
    /// The widget is shown greyed out and ignores input while false.
    #[serde(default = "always")]
    pub enabled: Cond,
    #[serde(default)]
    pub children: Vec<WidgetDef>,
    /// Local changes on top of `style`.
    #[serde(default)]
    pub overrides: Style,
}

impl WidgetDef {
    pub fn new(kind: WidgetKind) -> Self {
        Self {
            id: String::new(),
            kind,
            style: None,
            layout: LayoutDef::default(),
            visible: Cond::Always,
            enabled: Cond::Always,
            children: Vec::new(),
            overrides: Style::default(),
        }
    }

    /// Depth-first visit of this widget and its descendants.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a WidgetDef)) {
        f(self);
        for child in &self.children {
            child.walk(f);
        }
    }
}

/// Parses a screen document, migrating older versions first.
pub fn from_ron(source: &str) -> Result<ScreenAsset, String> {
    let version = migrate::read_version(source)?;
    let migrated = migrate::migrate(version, source)?;
    ron::from_str(&migrated).map_err(|e| e.to_string())
}

pub fn to_ron(asset: &ScreenAsset) -> Result<String, String> {
    ron::ser::to_string_pretty(asset, ron::ser::PrettyConfig::new()).map_err(|e| e.to_string())
}
