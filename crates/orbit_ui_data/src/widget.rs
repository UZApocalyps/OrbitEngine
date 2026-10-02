//! The document model of a data-driven screen.

use orbit_core::Condition;
use serde::{Deserialize, Serialize};

use crate::action::{BindPath, UiAction};
use crate::migrate;
use crate::theme::Style;
use crate::layout::{Align, Anchor, Dir, PanelKind, Size};

/// Current version of the screen document format.
pub const UI_VERSION: u32 = 1;

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
        }
    }
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
    /// A selector (a real dropdown list comes later). `Language` lists the
    /// languages of the game; other settings list `options`.
    Dropdown {
        bind: BindPath,
        #[serde(default)]
        options: Vec<TextRef>,
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
}

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
