//! The data-driven look of the in-game UI.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::layout::Anchor;

/// Look of the whole UI. Every field has a default, so a theme file only
/// needs to list what it overrides (`(textbox: (text_size: 36.0))`). All
/// lengths are in virtual pixels (a 1920x1080 canvas), colors are sRGBA bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Theme {
    pub textbox: TextBoxTheme,
    pub choice: ChoiceTheme,
    pub screens: ScreenTheme,
    /// Look of the widget toolkit and of the menu screens.
    pub widgets: WidgetTheme,
    /// Optional UI sounds, played by the host.
    pub sounds: UiSounds,
    /// Named styles that widgets pick with `Node::style` / `style:` in a
    /// screen asset.
    pub styles: BTreeMap<String, Style>,
    /// Named project fonts the styles pick with `Style::font`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fonts: BTreeMap<String, FontRef>,
    /// Per-language font fallback chains (M7-02): language code (`"ja"`) to
    /// font names, preferred first. Each name is an entry of
    /// [`Theme::fonts`] or a family name. Applied to the dialogue box,
    /// choices and every widget; a language without an entry uses the
    /// default font.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub language_fonts: BTreeMap<String, Vec<String>>,
    /// Placement and styles of the in-game quick menu (`ui_version` 8;
    /// absent = the stock bar).
    #[serde(skip_serializing_if = "QuickMenuTheme::is_default")]
    pub quick_menu: QuickMenuTheme,
    /// In-game notifications (toasts): placement, timing, animation and
    /// named looks (`ui_version` 11; absent = the stock toasts).
    #[serde(skip_serializing_if = "crate::notify::NotificationTheme::is_default")]
    pub notifications: crate::notify::NotificationTheme,
}

impl Theme {
    /// [`Theme::language_fonts`] with every name resolved to the family the
    /// text engine selects (see [`Theme::font_family`]).
    pub fn language_font_families(&self) -> BTreeMap<String, Vec<String>> {
        self.language_fonts
            .iter()
            .map(|(lang, names)| {
                let families = names
                    .iter()
                    .map(|n| self.font_family(n).to_owned())
                    .collect();
                (lang.clone(), families)
            })
            .collect()
    }

    /// The family name to draw a style's `font` with: the `family` of the
    /// [`Theme::fonts`] entry called `name`, else `name` itself.
    pub fn font_family<'a>(&'a self, name: &'a str) -> &'a str {
        self.fonts.get(name).map_or(name, |f| f.family.as_str())
    }

    pub fn from_ron(source: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(source)
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new())
    }

    /// Every image path the theme draws (style fills of every state, the
    /// textbox, name plate and indicator images), each once, sorted: what a
    /// host loads before drawing the UI.
    pub fn image_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = self.styles.values().flat_map(Style::image_paths).collect();
        out.extend(self.textbox.indicator_image.iter().cloned());
        out.extend(self.notifications.image_paths());
        out.retain(|p| !p.trim().is_empty());
        out.sort();
        out.dedup();
        out
    }

    /// Every font file the theme names ([`Theme::fonts`]), each once, sorted.
    pub fn font_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = self.fonts.values().map(|f| f.asset.clone()).collect();
        out.retain(|p| !p.trim().is_empty());
        out.sort();
        out.dedup();
        out
    }

    /// Every sound file the theme plays (UI sounds and the sounds of the
    /// styles), each once, sorted.
    pub fn sound_paths(&self) -> Vec<String> {
        let s = &self.sounds;
        let mut out: Vec<String> = [&s.hover, &s.confirm, &s.cancel, &s.slide]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        for style in self.styles.values() {
            if let Some(sounds) = &style.sounds {
                out.extend(sounds.hover.iter().chain(&sounds.confirm).cloned());
            }
        }
        out.extend(self.notifications.sound_paths());
        out.retain(|p| !p.trim().is_empty());
        out.sort();
        out.dedup();
        out
    }

    /// Points every reference of the theme to the style `old` (parents of
    /// other styles, the dialogue box, choice and quick menu sections) to
    /// `new` (`None`: back to the default look).
    pub fn retarget_style(&mut self, old: &str, new: Option<&str>) {
        let fix = |slot: &mut Option<String>| {
            if slot.as_deref() == Some(old) {
                *slot = new.map(str::to_owned);
            }
        };
        for style in self.styles.values_mut() {
            fix(&mut style.extends);
        }
        fix(&mut self.textbox.style);
        fix(&mut self.textbox.name_style);
        if let Some(plate) = &mut self.textbox.name_plate {
            fix(&mut plate.style);
        }
        fix(&mut self.choice.button_style);
        fix(&mut self.quick_menu.button_style);
        for n in self.notifications.styles.values_mut() {
            fix(&mut n.style);
        }
    }

    /// Problems of the styles and of the dialogue box section: unknown
    /// style names, a style extending itself or a missing parent, image
    /// fills without an image, nine-slice insets that are negative.
    pub fn style_issues(&self) -> Vec<crate::validate::Issue> {
        use crate::validate::Issue;
        let mut out = Vec::new();
        for (at, name) in self.dialogue_style_refs() {
            if !self.styles.contains_key(name) {
                out.push(Issue::warning("theme", at, format!("unknown style `{name}`")));
            }
        }
        for (key, n) in &self.notifications.styles {
            if let Some(name) = &n.style
                && !self.styles.contains_key(name)
            {
                out.push(Issue::warning(
                    "theme",
                    &format!("notifications.styles.{key}.style"),
                    format!("unknown style `{name}`"),
                ));
            }
        }
        for (name, style) in &self.styles {
            let at = format!("style {name}");
            match style.extends.as_deref() {
                Some(p) if p == name => out.push(Issue::warning("theme", &at, "extends itself")),
                Some(p) if !self.styles.contains_key(p) => {
                    out.push(Issue::warning("theme", &at, format!("extends unknown style `{p}`")));
                }
                _ => {}
            }
            let fills = [
                style.fill.as_ref(),
                style.fills.normal.as_ref(),
                style.fills.hover.as_ref(),
                style.fills.pressed.as_ref(),
                style.fills.disabled.as_ref(),
                style.fills.focused.as_ref(),
            ];
            for fill in fills.into_iter().flatten() {
                match fill {
                    Fill::NineSlice { image, .. } | Fill::Image { image, .. } if image.trim().is_empty() => {
                        out.push(Issue::warning("theme", &at, "an image background has no image"));
                    }
                    Fill::NineSlice { insets, .. } if insets.iter().any(|v| *v < 0.0) => {
                        out.push(Issue::error("theme", &at, "nine-slice insets cannot be negative"));
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// The style names the dialogue box, choices and quick menu refer to,
    /// with where (`"textbox.style"`...): for validation and the editor's
    /// "used by" list.
    pub fn dialogue_style_refs(&self) -> Vec<(&'static str, &str)> {
        let refs: [(&'static str, Option<&String>); 5] = [
            ("textbox.style", self.textbox.style.as_ref()),
            ("textbox.name_style", self.textbox.name_style.as_ref()),
            (
                "textbox.name_plate.style",
                self.textbox.name_plate.as_ref().and_then(|p| p.style.as_ref()),
            ),
            ("choice.button_style", self.choice.button_style.as_ref()),
            ("quick_menu.button_style", self.quick_menu.button_style.as_ref()),
        ];
        refs.into_iter()
            .filter_map(|(at, name)| name.map(|n| (at, n.as_str())))
            .collect()
    }
}

/// Placement and look of the quick menu bar (Back, History, Skip...) shown
/// during play. Every field is optional: `None` keeps the stock value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuickMenuTheme {
    /// Where the bar sits (stock: bottom center, 8 px above the edge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// Style of the buttons (states, images, font...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button_style: Option<String>,
    /// Width of one button (stock: 150).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button_width: Option<f32>,
    /// Gap between buttons (stock: 8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<f32>,
    /// Hides the bar altogether.
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
}

impl QuickMenuTheme {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_true(b: &bool) -> bool {
    *b
}

fn yes() -> bool {
    true
}

/// Virtual canvas the dialogue box and the choices are placed on.
pub const CANVAS: [f32; 2] = [1920.0, 1080.0];

/// Top-left corner of a `size` box placed by `anchor` on the canvas.
pub fn anchored(anchor: &Anchor, size: [f32; 2]) -> [f32; 2] {
    [
        CANVAS[0] * anchor.canvas[0] - size[0] * anchor.pivot[0] + anchor.offset[0],
        CANVAS[1] * anchor.canvas[1] - size[1] * anchor.pivot[1] + anchor.offset[1],
    ]
}

/// A plate drawn behind the speaker name (`ui_version` 8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NamePlate {
    /// Top-left of the plate relative to the top-left of the textbox
    /// (negative `y`: above the box).
    pub offset: [f32; 2],
    /// `[width, height]`; a width of 0 fits the name.
    pub size: [f32; 2],
    /// Style of the plate (background, border, shadow, font, text color).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Inner margin between the plate edge and the name.
    pub padding: [f32; 2],
}

impl Default for NamePlate {
    fn default() -> Self {
        Self {
            offset: [30.0, -50.0],
            size: [0.0, 64.0],
            style: None,
            padding: [24.0, 6.0],
        }
    }
}

/// Where everything of the dialogue box goes, in canvas pixels: a pure
/// function of the theme so that the drawing code, the editor and the tests
/// agree (see [`TextBoxTheme::layout`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogueLayout {
    /// `[x, y, w, h]` of the panel.
    pub panel: [f32; 4],
    /// The name plate, when the theme has one.
    pub plate: Option<[f32; 4]>,
    /// Top-left of the speaker name and its wrapping width.
    pub name_origin: [f32; 2],
    pub name_width: f32,
    /// Top-left of the line and its wrapping width.
    pub text_origin: [f32; 2],
    pub text_width: f32,
    /// `[x, y, size, size]` of the "continue" indicator.
    pub indicator: [f32; 4],
}

/// The dialogue panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextBoxTheme {
    /// `[x, y, width, height]` of the panel (top-left origin).
    pub rect: [f32; 4],
    pub background: [u8; 4],
    /// Horizontal and vertical inner margin.
    pub padding: [f32; 2],
    pub name_size: f32,
    /// Distance between the top padding and the text (room for the name).
    pub text_offset: f32,
    pub text_size: f32,
    pub text_color: [u8; 4],
    /// Typewriter speed in characters per second.
    pub chars_per_second: f32,
    pub indicator_color: [u8; 4],
    /// Side of the diamond-shaped "continue" marker.
    pub indicator_size: f32,
    /// The marker pulses (`true`) or stays at full opacity. A host that
    /// draws on demand (the editor preview) turns it off so that a line
    /// waiting for the player is a still image.
    pub indicator_blink: bool,
    /// Places the panel by anchor (`ui_version` 8): `rect`'s `x` / `y` are
    /// then ignored, its width and height kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// Style of the panel (`ui_version` 8): background of any fill kind,
    /// border, shadow, font, bold, text color and size, alignment. A field
    /// the style sets wins over the plain fields above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Style of the speaker name (font, size, bold, color when
    /// `speaker_color` is off).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_style: Option<String>,
    /// A plate behind the speaker name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_plate: Option<NamePlate>,
    /// The name is drawn in the character's color (else the name style's
    /// text color).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub speaker_color: bool,
    /// An image drawn as the "continue" indicator instead of the diamond.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indicator_image: Option<String>,
    /// How lines are shown: the classic box (default) or comic speech
    /// bubbles next to the speaking character. Absent from the file when
    /// `Box`, so older themes and players are unchanged.
    #[serde(skip_serializing_if = "DialogueMode::is_box")]
    pub mode: DialogueMode,
    /// Look and placement of the speech bubbles (used in `Bubbles` mode).
    #[serde(skip_serializing_if = "BubbleTheme::is_default")]
    pub bubble: BubbleTheme,
}

/// How the dialogue lines are displayed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DialogueMode {
    /// One box at the bottom of the screen for every line.
    #[default]
    Box,
    /// Character lines in comic speech bubbles next to the speaker; the
    /// narrator keeps the box (or a caption, see [`BubbleTheme::narrator`]).
    Bubbles,
}

impl DialogueMode {
    pub fn is_box(&self) -> bool {
        *self == Self::Box
    }
}

/// The shape of one bubble, picked per line with a `{bubble=...}` tag
/// (`speech` when there is none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BubbleKind {
    #[default]
    Speech,
    /// A cloud with small circles leading to the character.
    Thought,
    /// A spiky burst.
    Shout,
}

impl BubbleKind {
    /// The kind a line asks for with a `{bubble=speech|thought|shout}` tag
    /// (the first valid one wins). The tag itself is invisible: `orbit_text`
    /// drops it.
    pub fn of_line(markup: &str) -> Self {
        let mut rest = markup;
        while let Some(at) = rest.find("{bubble=") {
            let tail = &rest[at + "{bubble=".len()..];
            let Some(end) = tail.find('}') else { break };
            if let Some(kind) = Self::from_name(tail[..end].trim()) {
                return kind;
            }
            rest = &tail[end..];
        }
        Self::Speech
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "speech" => Some(Self::Speech),
            "thought" => Some(Self::Thought),
            "shout" => Some(Self::Shout),
            _ => None,
        }
    }
}

/// How the narrator (lines without a speaker) is shown in `Bubbles` mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NarratorStyle {
    /// The classic dialogue box, unchanged.
    #[default]
    Box,
    /// A rectangular comic caption at the top of the screen.
    Caption,
}

/// What a bubble does when its speaker is not on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OffscreenStyle {
    /// A bubble without a tail at the bottom of the screen.
    #[default]
    Bubble,
    /// The classic dialogue box.
    Box,
}

/// Per-character overrides of the bubble look (keyed by character id).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BubbleCharacter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<[u8; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<[u8; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_color: Option<[u8; 4]>,
    /// Moves the head point (top-center of the sprite) the bubble points
    /// at, in canvas pixels (`y` > 0: lower).
    pub head_offset: [f32; 2],
}

impl BubbleCharacter {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Look and placement of the comic speech bubbles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BubbleTheme {
    pub fill: [u8; 4],
    pub border: [u8; 4],
    pub border_width: f32,
    pub corner_radius: f32,
    /// Inner margin `[x, y]` between the outline and the text.
    pub padding: [f32; 2],
    /// Widest bubble (outline included); the text wraps inside.
    pub max_width: f32,
    pub min_width: f32,
    pub text_size: f32,
    pub text_color: [u8; 4],
    /// Font of the text (a name of `Theme::fonts` or a family).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Shows the speaker name as a small label at the top of the bubble.
    pub show_name: bool,
    pub name_size: f32,
    /// Length and base width of the tail.
    pub tail_length: f32,
    pub tail_width: f32,
    /// Space left between the tail tip and the head.
    pub gap: f32,
    /// Distance kept from the edges of the screen.
    pub margin: f32,
    /// Duration of the pop-in animation in seconds (0: none).
    pub pop: f32,
    /// The outline takes the color of the speaking character.
    pub speaker_border: bool,
    pub narrator: NarratorStyle,
    pub caption_fill: [u8; 4],
    pub offscreen: OffscreenStyle,
    /// Overrides by character id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub characters: BTreeMap<String, BubbleCharacter>,
}

impl Default for BubbleTheme {
    fn default() -> Self {
        Self {
            fill: [255, 255, 255, 245],
            border: [20, 20, 28, 255],
            border_width: 4.0,
            corner_radius: 30.0,
            padding: [30.0, 20.0],
            max_width: 760.0,
            min_width: 140.0,
            text_size: 36.0,
            text_color: [20, 20, 28, 255],
            font: None,
            show_name: true,
            name_size: 26.0,
            tail_length: 48.0,
            tail_width: 38.0,
            gap: 14.0,
            margin: 24.0,
            pop: 0.18,
            speaker_border: false,
            narrator: NarratorStyle::Box,
            caption_fill: [255, 243, 196, 245],
            offscreen: OffscreenStyle::Bubble,
            characters: BTreeMap::new(),
        }
    }
}

impl BubbleTheme {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl TextBoxTheme {
    /// The panel rectangle (`rect`, or its size placed by `anchor`).
    pub fn panel_rect(&self) -> [f32; 4] {
        let [x, y, w, h] = self.rect;
        match &self.anchor {
            Some(a) => {
                let [ax, ay] = anchored(a, [w, h]);
                [ax, ay, w, h]
            }
            None => [x, y, w, h],
        }
    }

    /// Where the panel, the name, the line and the indicator go.
    /// `name_width` is the measured (or estimated) width of the speaker
    /// name, used by a plate whose width is 0 (fit).
    pub fn layout(&self, name_width: f32) -> DialogueLayout {
        let panel = self.panel_rect();
        let [x, y, w, h] = panel;
        let [px, py] = self.padding;
        let size = self.indicator_size;
        let (plate, name_origin, name_width) = match &self.name_plate {
            Some(p) => {
                let pw = if p.size[0] > 0.0 {
                    p.size[0]
                } else {
                    name_width + 2.0 * p.padding[0]
                };
                let rect = [x + p.offset[0], y + p.offset[1], pw, p.size[1]];
                let origin = [rect[0] + p.padding[0], rect[1] + p.padding[1]];
                (Some(rect), origin, (pw - 2.0 * p.padding[0]).max(1.0))
            }
            None => (None, [x + px, y + py], w - 2.0 * px),
        };
        DialogueLayout {
            panel,
            plate,
            name_origin,
            name_width,
            text_origin: [x + px, y + py + self.text_offset],
            text_width: w - 2.0 * px,
            indicator: [x + w - px - size, y + h - py - size, size, size],
        }
    }
}

impl Default for TextBoxTheme {
    fn default() -> Self {
        Self {
            rect: [90.0, 790.0, 1740.0, 260.0],
            background: [4, 6, 18, 190],
            padding: [50.0, 16.0],
            name_size: 44.0,
            text_offset: 64.0,
            text_size: 40.0,
            text_color: [255, 255, 255, 255],
            chars_per_second: 40.0,
            indicator_color: [255, 255, 255, 255],
            indicator_size: 22.0,
            indicator_blink: true,
            anchor: None,
            style: None,
            name_style: None,
            name_plate: None,
            speaker_color: true,
            indicator_image: None,
            mode: DialogueMode::Box,
            bubble: BubbleTheme::default(),
        }
    }
}

/// The choice screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChoiceTheme {
    /// Full-screen dimming drawn behind the buttons.
    pub overlay: [u8; 4],
    /// `[width, height]` of one button.
    pub button_size: [f32; 2],
    pub spacing: f32,
    pub text_size: f32,
    pub button_color: [u8; 4],
    /// Background of the hovered / keyboard-selected button.
    pub selected_color: [u8; 4],
    pub text_color: [u8; 4],
    pub selected_text_color: [u8; 4],
    /// Prefix the labels with their digit (`1.`, `2.`...), matching the
    /// number-key shortcuts.
    pub show_numbers: bool,
    /// Style of the buttons (`ui_version` 8): per-state fills (images,
    /// nine-slices...), text colors, font, size, border, shadow. A field the
    /// style sets wins over the plain colors above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button_style: Option<String>,
    /// Places the stack of buttons by anchor (stock: centered on the
    /// canvas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
}

impl Default for ChoiceTheme {
    fn default() -> Self {
        Self {
            overlay: [0, 0, 0, 110],
            button_size: [1200.0, 90.0],
            spacing: 26.0,
            text_size: 38.0,
            button_color: [14, 18, 40, 215],
            selected_color: [70, 110, 210, 235],
            text_color: [225, 230, 245, 255],
            selected_text_color: [255, 255, 255, 255],
            show_numbers: true,
            button_style: None,
            anchor: None,
        }
    }
}

/// Vertical room the stock choice stack keeps free above and below it.
const CHOICE_MARGIN_Y: f32 = 60.0;

impl ChoiceTheme {
    /// `[x, y, w, h]` of each of `count` buttons: centered on the canvas
    /// (or placed by `anchor`, the stack being one block). When the stack
    /// would not fit the screen, heights and spacing shrink.
    pub fn button_rects(&self, count: usize) -> Vec<[f32; 4]> {
        if count == 0 {
            return Vec::new();
        }
        let [w, h] = self.button_size;
        let n = count as f32;
        let full = n * h + (n - 1.0) * self.spacing;
        let avail = CANVAS[1] - 2.0 * CHOICE_MARGIN_Y;
        let k = if full > avail { avail / full } else { 1.0 };
        let (h, gap) = (h * k, self.spacing * k);
        let total = n * h + (n - 1.0) * gap;
        let [x, top] = match &self.anchor {
            Some(a) => {
                let [x, y] = anchored(a, [w, total]);
                [x, y.clamp(0.0, (CANVAS[1] - total).max(0.0))]
            }
            None => [(CANVAS[0] - w) * 0.5, (CANVAS[1] - total) * 0.5],
        };
        (0..count)
            .map(|i| [x, top + i as f32 * (h + gap), w, h])
            .collect()
    }
}

/// Loading, start, end and error screens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenTheme {
    pub background: [u8; 4],
    pub text_color: [u8; 4],
    pub error_color: [u8; 4],
    pub title_size: f32,
    pub body_size: f32,
    /// `[width, height]` of the loading bar.
    pub bar_size: [f32; 2],
    pub bar_background: [u8; 4],
    pub bar_fill: [u8; 4],
}

impl Default for ScreenTheme {
    fn default() -> Self {
        Self {
            background: [5, 7, 14, 255],
            text_color: [235, 238, 248, 255],
            error_color: [255, 130, 130, 255],
            title_size: 64.0,
            body_size: 30.0,
            bar_size: [700.0, 14.0],
            bar_background: [30, 36, 56, 255],
            bar_fill: [110, 160, 255, 255],
        }
    }
}

/// Paths (or asset ids, the host decides how to resolve them) of the optional
/// UI sounds. The UI never plays audio itself: it emits
/// [`crate::UiEvent::Sound`] and the host looks the sound up here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSounds {
    pub hover: Option<String>,
    pub confirm: Option<String>,
    pub cancel: Option<String>,
    pub slide: Option<String>,
}

impl UiSounds {
    /// The configured sound of `sound`, if any.
    pub fn get(&self, sound: crate::sound::UiSound) -> Option<&str> {
        let slot = match sound {
            crate::sound::UiSound::Hover => &self.hover,
            crate::sound::UiSound::Confirm => &self.confirm,
            crate::sound::UiSound::Cancel => &self.cancel,
            crate::sound::UiSound::Slide => &self.slide,
        };
        slot.as_deref()
    }
}

/// Panels (menu backgrounds, cards).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelTheme {
    pub background: [u8; 4],
    pub border_color: [u8; 4],
    /// Border thickness (0 = none).
    pub border_width: f32,
    /// Full-screen dimming behind pause menus and modal dialogs.
    pub dim: [u8; 4],
}

impl Default for PanelTheme {
    fn default() -> Self {
        Self {
            background: [10, 14, 34, 240],
            border_color: [90, 110, 190, 255],
            border_width: 2.0,
            dim: [0, 0, 0, 150],
        }
    }
}

/// Buttons (also tabs and quick-menu entries).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ButtonTheme {
    pub height: f32,
    /// Height of the small variant (quick menu, delete buttons).
    pub small_height: f32,
    pub padding_x: f32,
    pub normal: [u8; 4],
    pub hover: [u8; 4],
    pub pressed: [u8; 4],
    pub disabled: [u8; 4],
    /// Background of an active tab / toggled quick-menu entry.
    pub selected: [u8; 4],
    pub text_color: [u8; 4],
    pub hover_text_color: [u8; 4],
    pub disabled_text_color: [u8; 4],
    /// Ring drawn around the keyboard-focused widget.
    pub focus_color: [u8; 4],
    pub focus_width: f32,
}

impl Default for ButtonTheme {
    fn default() -> Self {
        Self {
            height: 70.0,
            small_height: 44.0,
            padding_x: 30.0,
            normal: [14, 18, 40, 220],
            hover: [70, 110, 210, 235],
            pressed: [50, 85, 170, 245],
            disabled: [20, 22, 30, 160],
            selected: [40, 70, 150, 235],
            text_color: [225, 230, 245, 255],
            hover_text_color: [255, 255, 255, 255],
            disabled_text_color: [110, 115, 130, 255],
            focus_color: [255, 214, 100, 255],
            focus_width: 3.0,
        }
    }
}

/// Sliders, toggles and selectors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlTheme {
    pub height: f32,
    pub track_height: f32,
    pub track: [u8; 4],
    pub fill: [u8; 4],
    /// `[width, height]` of the slider knob.
    pub knob_size: [f32; 2],
    pub knob: [u8; 4],
    /// Side of the toggle box.
    pub toggle_size: f32,
    pub toggle_off: [u8; 4],
    pub toggle_on: [u8; 4],
    /// Keyboard step of a slider (fraction of its range).
    pub slider_step: f32,
}

impl Default for ControlTheme {
    fn default() -> Self {
        Self {
            height: 56.0,
            track_height: 10.0,
            track: [40, 46, 72, 255],
            fill: [110, 160, 255, 255],
            knob_size: [18.0, 38.0],
            knob: [240, 244, 255, 255],
            toggle_size: 38.0,
            toggle_off: [24, 28, 52, 255],
            toggle_on: [110, 160, 255, 255],
            slider_step: 0.05,
        }
    }
}

/// Scroll lists and toast notifications.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListTheme {
    pub bar_width: f32,
    pub bar_track: [u8; 4],
    pub bar_thumb: [u8; 4],
    /// Items scrolled by one mouse-wheel notch.
    pub wheel_items: usize,
    /// Seconds a toast stays visible.
    pub toast_seconds: f32,
    pub toast_background: [u8; 4],
}

impl Default for ListTheme {
    fn default() -> Self {
        Self {
            bar_width: 10.0,
            bar_track: [30, 34, 56, 200],
            bar_thumb: [120, 150, 230, 255],
            wheel_items: 1,
            toast_seconds: 2.5,
            toast_background: [12, 16, 40, 235],
        }
    }
}

/// Everything the widget toolkit and the menu screens need.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetTheme {
    pub title_size: f32,
    pub heading_size: f32,
    pub text_size: f32,
    pub small_text_size: f32,
    pub text_color: [u8; 4],
    pub dim_text_color: [u8; 4],
    pub accent_color: [u8; 4],
    /// Gap between siblings in the stock screens.
    pub spacing: f32,
    pub panel: PanelTheme,
    pub button: ButtonTheme,
    pub controls: ControlTheme,
    pub list: ListTheme,
}

impl Default for WidgetTheme {
    fn default() -> Self {
        Self {
            title_size: 72.0,
            heading_size: 48.0,
            text_size: 30.0,
            small_text_size: 22.0,
            text_color: [235, 238, 248, 255],
            dim_text_color: [150, 158, 185, 255],
            accent_color: [255, 214, 100, 255],
            spacing: 18.0,
            panel: PanelTheme::default(),
            button: ButtonTheme::default(),
            controls: ControlTheme::default(),
            list: ListTheme::default(),
        }
    }
}

/// What a widget background is made of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Fill {
    Color([u8; 4]),
    /// A project image cut in nine: the corners keep their size, the edges
    /// and the center stretch. `image` is the asset path (`assets/ui/x.png`),
    /// `insets` the `[left, top, right, bottom]` border widths in pixels of
    /// the image (`ui_version` 2).
    NineSlice { image: String, insets: [f32; 4] },
    /// A project image drawn whole (`ui_version` 8), scaled as `fit` says.
    Image {
        image: String,
        #[serde(default)]
        fit: ImageFit,
    },
    /// A vertical gradient from `top` to `bottom` (`ui_version` 8).
    Gradient { top: [u8; 4], bottom: [u8; 4] },
}

impl Fill {
    /// The image this fill draws, if any.
    pub fn image(&self) -> Option<&str> {
        match self {
            Fill::NineSlice { image, .. } | Fill::Image { image, .. } => Some(image.as_str()),
            Fill::Color(_) | Fill::Gradient { .. } => None,
        }
    }

    /// A color standing in for the fill when its image is not loaded (or
    /// for a quick swatch): the color, the gradient's middle, else `None`.
    pub fn flat_color(&self) -> Option<[u8; 4]> {
        match self {
            Fill::Color(c) => Some(*c),
            Fill::Gradient { top, bottom } => {
                let mut c = [0u8; 4];
                for i in 0..4 {
                    c[i] = ((top[i] as u16 + bottom[i] as u16) / 2) as u8;
                }
                Some(c)
            }
            Fill::NineSlice { .. } | Fill::Image { .. } => None,
        }
    }
}

/// How a [`Fill::Image`] covers its rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFit {
    /// Stretched to the rectangle (aspect ratio lost).
    #[default]
    Stretch,
    /// Scaled to fit inside, centered (aspect ratio kept, may leave bands).
    Fit,
    /// Scaled to cover, centered and cropped (aspect ratio kept).
    Cover,
}

impl ImageFit {
    /// `([x, y, w, h], [u0, v0, u1, v1])` to draw an image of `size` pixels
    /// in `rect`.
    pub fn place(self, rect: [f32; 4], size: [f32; 2]) -> ([f32; 4], [f32; 4]) {
        let full = [0.0, 0.0, 1.0, 1.0];
        if size[0] <= 0.0 || size[1] <= 0.0 || rect[2] <= 0.0 || rect[3] <= 0.0 {
            return (rect, full);
        }
        let sx = rect[2] / size[0];
        let sy = rect[3] / size[1];
        match self {
            ImageFit::Stretch => (rect, full),
            ImageFit::Fit => {
                let s = sx.min(sy);
                let (w, h) = (size[0] * s, size[1] * s);
                (
                    [rect[0] + (rect[2] - w) * 0.5, rect[1] + (rect[3] - h) * 0.5, w, h],
                    full,
                )
            }
            ImageFit::Cover => {
                let s = sx.max(sy);
                // Fraction of the image that shows on each axis.
                let fu = (rect[2] / (size[0] * s)).min(1.0);
                let fv = (rect[3] / (size[1] * s)).min(1.0);
                let (u0, v0) = ((1.0 - fu) * 0.5, (1.0 - fv) * 0.5);
                (rect, [u0, v0, u0 + fu, v0 + fv])
            }
        }
    }
}

/// The state a widget is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WidgetState {
    Normal,
    Hover,
    Pressed,
    Disabled,
    /// Keyboard focus (without the pointer over it).
    Focused,
}

impl WidgetState {
    pub const ALL: [WidgetState; 5] = [
        WidgetState::Normal,
        WidgetState::Hover,
        WidgetState::Pressed,
        WidgetState::Disabled,
        WidgetState::Focused,
    ];

    /// The state from the flags the widget code tracks: disabled wins, then
    /// pressed, hover, focus.
    pub fn of(enabled: bool, pressed: bool, hover: bool, focused: bool) -> Self {
        if !enabled {
            WidgetState::Disabled
        } else if pressed {
            WidgetState::Pressed
        } else if hover {
            WidgetState::Hover
        } else if focused {
            WidgetState::Focused
        } else {
            WidgetState::Normal
        }
    }

    /// The states looked at, in order, before the normal one: a pressed
    /// widget without a pressed look takes the hover one, and so does a
    /// focused one (the focus used to count as hover).
    fn chain(self) -> &'static [WidgetState] {
        match self {
            WidgetState::Normal => &[],
            WidgetState::Hover => &[WidgetState::Hover],
            WidgetState::Pressed => &[WidgetState::Pressed, WidgetState::Hover],
            WidgetState::Disabled => &[WidgetState::Disabled],
            WidgetState::Focused => &[WidgetState::Focused, WidgetState::Hover],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            WidgetState::Normal => "normal",
            WidgetState::Hover => "hover",
            WidgetState::Pressed => "pressed",
            WidgetState::Disabled => "disabled",
            WidgetState::Focused => "focused",
        }
    }
}

/// One optional background per widget state (`ui_version` 8): a button
/// can show a different image (or nine-slice, gradient, color) when
/// hovered, pressed, disabled or focused.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StateFills {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal: Option<Fill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover: Option<Fill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pressed: Option<Fill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<Fill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused: Option<Fill>,
}

impl StateFills {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn get(&self, state: WidgetState) -> Option<&Fill> {
        match state {
            WidgetState::Normal => self.normal.as_ref(),
            WidgetState::Hover => self.hover.as_ref(),
            WidgetState::Pressed => self.pressed.as_ref(),
            WidgetState::Disabled => self.disabled.as_ref(),
            WidgetState::Focused => self.focused.as_ref(),
        }
    }

    pub fn get_mut(&mut self, state: WidgetState) -> &mut Option<Fill> {
        match state {
            WidgetState::Normal => &mut self.normal,
            WidgetState::Hover => &mut self.hover,
            WidgetState::Pressed => &mut self.pressed,
            WidgetState::Disabled => &mut self.disabled,
            WidgetState::Focused => &mut self.focused,
        }
    }

    fn over(&self, base: &StateFills) -> StateFills {
        StateFills {
            normal: self.normal.clone().or_else(|| base.normal.clone()),
            hover: self.hover.clone().or_else(|| base.hover.clone()),
            pressed: self.pressed.clone().or_else(|| base.pressed.clone()),
            disabled: self.disabled.clone().or_else(|| base.disabled.clone()),
            focused: self.focused.clone().or_else(|| base.focused.clone()),
        }
    }
}

/// A drop shadow: the widget's rectangle in `color`, moved by `offset`
/// (`ui_version` 8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub color: [u8; 4],
    pub offset: [f32; 2],
}

/// Horizontal alignment of a widget's text (`ui_version` 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
}

/// A font of the project, by name: `Style::font` may name an entry of
/// [`Theme::fonts`] (else it is taken as a family name).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FontRef {
    /// Path of the font asset (`assets/fonts/x.ttf`, `AssetKind::Font`).
    pub asset: String,
    /// Family name the font file provides (what the text engine selects).
    pub family: String,
}

/// One optional color per widget state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StateColors {
    pub normal: Option<[u8; 4]>,
    pub hover: Option<[u8; 4]>,
    pub pressed: Option<[u8; 4]>,
    pub disabled: Option<[u8; 4]>,
    /// Keyboard focus (`ui_version` 8; falls back to `hover`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused: Option<[u8; 4]>,
}

impl StateColors {
    pub fn get(&self, state: WidgetState) -> Option<[u8; 4]> {
        match state {
            WidgetState::Normal => self.normal,
            WidgetState::Hover => self.hover,
            WidgetState::Pressed => self.pressed,
            WidgetState::Disabled => self.disabled,
            WidgetState::Focused => self.focused,
        }
    }

    pub fn get_mut(&mut self, state: WidgetState) -> &mut Option<[u8; 4]> {
        match state {
            WidgetState::Normal => &mut self.normal,
            WidgetState::Hover => &mut self.hover,
            WidgetState::Pressed => &mut self.pressed,
            WidgetState::Disabled => &mut self.disabled,
            WidgetState::Focused => &mut self.focused,
        }
    }

    /// The color of `state`, following [`WidgetState`]'s fallbacks, then
    /// `normal`.
    pub fn pick_state(&self, state: WidgetState) -> Option<[u8; 4]> {
        state
            .chain()
            .iter()
            .find_map(|s| self.get(*s))
            .or(self.normal)
    }

    /// The color of a state: `disabled` wins, then `pressed`, then `hover`;
    /// a missing state color falls back to `normal`.
    pub fn pick(&self, enabled: bool, pressed: bool, hot: bool) -> Option<[u8; 4]> {
        let state = if !enabled {
            self.disabled
        } else if pressed {
            self.pressed.or(self.hover)
        } else if hot {
            self.hover
        } else {
            None
        };
        state.or(self.normal)
    }

    fn over(&self, base: &StateColors) -> StateColors {
        StateColors {
            normal: self.normal.or(base.normal),
            hover: self.hover.or(base.hover),
            pressed: self.pressed.or(base.pressed),
            disabled: self.disabled.or(base.disabled),
            focused: self.focused.or(base.focused),
        }
    }
}

/// Border of a styled widget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Border {
    pub color: [u8; 4],
    pub width: f32,
}

/// Sound files of a styled widget (resolved by the host like
/// [`UiSounds`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleSounds {
    pub hover: Option<String>,
    pub confirm: Option<String>,
}

/// A named look: every field is optional so that a style only lists what it
/// changes. `extends` names another style whose fields fill the gaps (one
/// level: the parent's own `extends` is ignored).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub extends: Option<String>,
    /// Font family name (`None` = the default font).
    pub font: Option<String>,
    pub text_size: Option<f32>,
    pub text_colors: StateColors,
    /// Background when no per-state color is given.
    pub fill: Option<Fill>,
    /// Background per state (overrides `fill`).
    pub colors: StateColors,
    pub border: Option<Border>,
    pub padding: Option<[f32; 2]>,
    pub sounds: Option<StyleSounds>,
    /// Background per state, of any fill kind (`ui_version` 8): wins over
    /// `colors` and `fill` for the states it sets.
    #[serde(skip_serializing_if = "StateFills::is_empty")]
    pub fills: StateFills,
    /// Drop shadow under the background (`ui_version` 8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    /// Horizontal alignment of the text (`ui_version` 8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_align: Option<TextAlignment>,
    /// Bold text (`ui_version` 8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// `Some(false)`: a button shows only its background (image buttons);
    /// the text stays for screen readers (`ui_version` 8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_text: Option<bool>,
}

/// The fields of a [`Style`], by name: what the style editor lists, marks
/// as overridden and resets to the parent.
pub const STYLE_FIELDS: [&str; 14] = [
    "font",
    "text_size",
    "text_colors",
    "fill",
    "colors",
    "fills",
    "border",
    "padding",
    "sounds",
    "shadow",
    "text_align",
    "bold",
    "show_text",
    "extends",
];

impl Style {
    /// `self` on top of `base`.
    pub fn over(&self, base: &Style) -> Style {
        Style {
            extends: None,
            font: self.font.clone().or_else(|| base.font.clone()),
            text_size: self.text_size.or(base.text_size),
            text_colors: self.text_colors.over(&base.text_colors),
            fill: self.fill.clone().or_else(|| base.fill.clone()),
            colors: self.colors.over(&base.colors),
            border: self.border.clone().or_else(|| base.border.clone()),
            padding: self.padding.or(base.padding),
            sounds: self.sounds.clone().or_else(|| base.sounds.clone()),
            fills: self.fills.over(&base.fills),
            shadow: self.shadow.clone().or_else(|| base.shadow.clone()),
            text_align: self.text_align.or(base.text_align),
            bold: self.bold.or(base.bold),
            show_text: self.show_text.or(base.show_text),
        }
    }

    /// Whether the style itself sets `field` (one of [`STYLE_FIELDS`]),
    /// as opposed to inheriting it.
    pub fn sets(&self, field: &str) -> bool {
        match field {
            "font" => self.font.is_some(),
            "text_size" => self.text_size.is_some(),
            "text_colors" => self.text_colors != StateColors::default(),
            "fill" => self.fill.is_some(),
            "colors" => self.colors != StateColors::default(),
            "fills" => !self.fills.is_empty(),
            "border" => self.border.is_some(),
            "padding" => self.padding.is_some(),
            "sounds" => self.sounds.is_some(),
            "shadow" => self.shadow.is_some(),
            "text_align" => self.text_align.is_some(),
            "bold" => self.bold.is_some(),
            "show_text" => self.show_text.is_some(),
            "extends" => self.extends.is_some(),
            _ => false,
        }
    }

    /// Clears `field` so that it is inherited again ("reset to parent").
    pub fn reset(&mut self, field: &str) {
        match field {
            "font" => self.font = None,
            "text_size" => self.text_size = None,
            "text_colors" => self.text_colors = StateColors::default(),
            "fill" => self.fill = None,
            "colors" => self.colors = StateColors::default(),
            "fills" => self.fills = StateFills::default(),
            "border" => self.border = None,
            "padding" => self.padding = None,
            "sounds" => self.sounds = None,
            "shadow" => self.shadow = None,
            "text_align" => self.text_align = None,
            "bold" => self.bold = None,
            "show_text" => self.show_text = None,
            "extends" => self.extends = None,
            _ => {}
        }
    }

    /// The background of `state`: the state's own fill (or its fallback,
    /// see [`WidgetState`]), the normal fill, a non-color `fill` (a
    /// nine-slice always won over the colors), the state colors, then a
    /// color `fill`. `None`: the theme's default look. A style without
    /// `fills` resolves exactly as before `ui_version` 8.
    pub fn fill_for(&self, state: WidgetState) -> Option<Fill> {
        if let Some(f) = state.chain().iter().find_map(|s| self.fills.get(*s)) {
            return Some(f.clone());
        }
        if let Some(f) = &self.fills.normal {
            return Some(f.clone());
        }
        if let Some(f) = self.fill.as_ref().filter(|f| !matches!(f, Fill::Color(_))) {
            return Some(f.clone());
        }
        self.colors
            .pick_state(state)
            .map(Fill::Color)
            .or_else(|| self.fill.clone())
    }

    /// Moves the pre-`ui_version` 8 background (`fill` + per-state
    /// `colors`) into per-state `fills`, keeping the look of every state:
    /// a non-color `fill` (nine-slice) always won over the colors, so it
    /// becomes the normal fill alone; otherwise each state color becomes
    /// that state's fill and the normal one (or the color fill) the normal
    /// fill. States `fills` already sets are kept. Returns whether
    /// anything moved.
    pub fn modernize_background(&mut self) -> bool {
        if self.fill.is_none() && self.colors == StateColors::default() {
            return false;
        }
        let fill = self.fill.take();
        let colors = std::mem::take(&mut self.colors);
        match fill {
            Some(f) if !matches!(f, Fill::Color(_)) => {
                self.fills.normal.get_or_insert(f);
            }
            fill => {
                for state in [WidgetState::Hover, WidgetState::Pressed, WidgetState::Disabled, WidgetState::Focused] {
                    if let Some(c) = colors.get(state) {
                        self.fills.get_mut(state).get_or_insert(Fill::Color(c));
                    }
                }
                if let Some(f) = colors.normal.map(Fill::Color).or(fill) {
                    self.fills.normal.get_or_insert(f);
                }
            }
        }
        true
    }

    /// The text color of `state` (`None`: the theme's).
    pub fn text_color_for(&self, state: WidgetState) -> Option<[u8; 4]> {
        self.text_colors.pick_state(state)
    }

    /// Whether the text is drawn (`show_text` is not `Some(false)`).
    pub fn text_visible(&self) -> bool {
        self.show_text != Some(false)
    }

    /// Every image path the style draws (all states).
    pub fn image_paths(&self) -> Vec<String> {
        let fills = [
            self.fill.as_ref(),
            self.fills.normal.as_ref(),
            self.fills.hover.as_ref(),
            self.fills.pressed.as_ref(),
            self.fills.disabled.as_ref(),
            self.fills.focused.as_ref(),
        ];
        fills
            .into_iter()
            .flatten()
            .filter_map(Fill::image)
            .map(str::to_owned)
            .collect()
    }

    /// Background color for a state (`colors`, then `fill`).
    pub fn background(&self, enabled: bool, pressed: bool, hot: bool) -> Option<[u8; 4]> {
        self.colors.pick(enabled, pressed, hot).or(match &self.fill {
            Some(Fill::Color(c)) => Some(*c),
            _ => None,
        })
    }

    /// The nine-slice background `(image path, insets)`, when the fill is one.
    pub fn nine_slice(&self) -> Option<(&str, [f32; 4])> {
        match &self.fill {
            Some(Fill::NineSlice { image, insets }) => Some((image.as_str(), *insets)),
            _ => None,
        }
    }
}

/// The style called `name` with its `extends` parent merged in (the result
/// has no `extends`). `None` when `name` is unknown; an unknown parent is
/// ignored.
pub fn resolve_style(styles: &BTreeMap<String, Style>, name: &str) -> Option<Style> {
    let style = styles.get(name)?;
    Some(match style.extends.as_deref().and_then(|p| styles.get(p)) {
        Some(parent) => style.over(parent),
        None => Style {
            extends: None,
            ..style.clone()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogue_mode_defaults_to_the_box_and_is_not_written() {
        let old = Theme::from_ron("(textbox: (text_size: 30.0))").unwrap();
        assert_eq!(old.textbox.mode, DialogueMode::Box);
        assert!(old.textbox.bubble.is_default());
        let ron = Theme::default().to_ron().unwrap();
        assert!(!ron.contains("mode") && !ron.contains("bubble"), "{ron}");
    }

    #[test]
    fn bubble_settings_round_trip() {
        let mut theme = Theme::default();
        theme.textbox.mode = DialogueMode::Bubbles;
        theme.textbox.bubble.narrator = NarratorStyle::Caption;
        theme.textbox.bubble.offscreen = OffscreenStyle::Box;
        theme.textbox.bubble.corner_radius = 12.0;
        theme.textbox.bubble.characters.insert(
            "01HZZZZZZZZZZZZZZZZZZZZZZZ".into(),
            BubbleCharacter {
                fill: Some([255, 200, 200, 255]),
                head_offset: [10.0, 30.0],
                ..Default::default()
            },
        );
        let back = Theme::from_ron(&theme.to_ron().unwrap()).unwrap();
        assert_eq!(back, theme);
        // A partial bubble section keeps the other defaults.
        let partial = Theme::from_ron("(textbox: (mode: Bubbles, bubble: (tail_length: 10.0)))").unwrap();
        assert_eq!(partial.textbox.bubble.tail_length, 10.0);
        assert_eq!(partial.textbox.bubble.fill, BubbleTheme::default().fill);
    }

    #[test]
    fn partial_widget_theme_keeps_the_other_defaults() {
        let theme = Theme::from_ron(
            "(widgets: (button: (height: 90.0), accent_color: (1, 2, 3, 4)), \
             sounds: (confirm: Some(\"ui/click.ogg\")))",
        )
        .unwrap();
        assert_eq!(theme.widgets.button.height, 90.0);
        assert_eq!(
            theme.widgets.button.hover,
            ButtonTheme::default().hover,
            "sibling fields keep their defaults"
        );
        assert_eq!(theme.widgets.accent_color, [1, 2, 3, 4]);
        assert_eq!(theme.widgets.panel, PanelTheme::default());
        assert_eq!(
            theme.sounds.get(crate::sound::UiSound::Confirm),
            Some("ui/click.ogg")
        );
        assert_eq!(theme.sounds.get(crate::sound::UiSound::Hover), None);
        assert_eq!(theme.textbox, TextBoxTheme::default());
    }

    #[test]
    fn default_theme_round_trips_through_ron() {
        let theme = Theme::default();
        let text = theme.to_ron().unwrap();
        assert_eq!(Theme::from_ron(&text).unwrap(), theme);
    }

    #[test]
    fn partial_theme_keeps_defaults() {
        let theme =
            Theme::from_ron("(textbox: (text_size: 30.0), choice: (show_numbers: false))").unwrap();
        assert_eq!(theme.textbox.text_size, 30.0);
        assert_eq!(theme.textbox.name_size, TextBoxTheme::default().name_size);
        assert!(!theme.choice.show_numbers);
        assert_eq!(theme.screens, ScreenTheme::default());
        assert_eq!(Theme::from_ron("()").unwrap(), Theme::default());
    }

    #[test]
    fn invalid_theme_is_an_error_not_a_panic() {
        assert!(Theme::from_ron("(textbox: 3)").is_err());
    }

#[test]    fn styles_extend_one_level_and_unknown_names_resolve_to_none() {        let theme = Theme::from_ron(            "(styles: {\"base\": (text_size: Some(20.0), fill: Some(Color((1, 2, 3, 4))),              colors: (hover: Some((9, 9, 9, 9)))), \"child\": (extends: Some(\"base\"),              text_size: Some(30.0), colors: (normal: Some((5, 5, 5, 5))))})",        )        .unwrap();        let s = resolve_style(&theme.styles, "child").unwrap();        assert_eq!(s.text_size, Some(30.0));        assert_eq!(s.fill, Some(Fill::Color([1, 2, 3, 4])));        assert_eq!(s.extends, None);        assert_eq!(s.background(true, false, false), Some([5, 5, 5, 5]));        assert_eq!(s.background(true, false, true), Some([9, 9, 9, 9]));        assert_eq!(s.background(true, true, true), Some([9, 9, 9, 9]), "pressed falls back to hover");        assert_eq!(s.background(false, false, false), Some([5, 5, 5, 5]), "disabled falls back to normal");        assert!(resolve_style(&theme.styles, "nope").is_none());    }    #[test]    fn a_grandparent_is_ignored_and_a_missing_parent_too() {        let mut styles = BTreeMap::new();        styles.insert("a".to_owned(), Style { text_size: Some(1.0), padding: Some([1.0, 1.0]), ..Default::default() });        styles.insert("b".to_owned(), Style { extends: Some("a".into()), ..Default::default() });        styles.insert("c".to_owned(), Style { extends: Some("b".into()), ..Default::default() });        styles.insert("d".to_owned(), Style { extends: Some("ghost".into()), text_size: Some(7.0), ..Default::default() });        assert_eq!(resolve_style(&styles, "c").unwrap().text_size, None);        assert_eq!(resolve_style(&styles, "b").unwrap().text_size, Some(1.0));        assert_eq!(resolve_style(&styles, "d").unwrap().text_size, Some(7.0));    }
}
