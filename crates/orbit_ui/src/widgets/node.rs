//! Widget nodes: kind-specific data, layout properties and builders.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use orbit_render2d::TextureId;
use orbit_text::{TextAlign, TextBlock};

/// Identifies a widget to the screen that owns it (events carry it back).
/// `0` means "no tag".
pub type Tag = u32;
pub const NO_TAG: Tag = 0;

/// Index of a node inside its [`crate::Ui`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) usize);

/// How a node is sized along one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    /// Content size.
    Fit,
    /// Exact size in virtual pixels.
    Fixed(f32),
    /// Share of the room left in the parent, with this weight.
    Fill(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    /// Cross axis only: children take the full cross size.
    Stretch,
}

/// How a container places its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Vertical,
    Horizontal,
    /// All children overlap in the content box.
    Stack,
}

/// Where a root node sits on the canvas: `pivot` of the node is put on the
/// `canvas` fraction point, then moved by `offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub canvas: [f32; 2],
    pub pivot: [f32; 2],
    pub offset: [f32; 2],
}

impl Anchor {
    /// Top-left corner (a `Fill` node then covers the canvas).
    pub const FULL: Anchor = Anchor {
        canvas: [0.0, 0.0],
        pivot: [0.0, 0.0],
        offset: [0.0, 0.0],
    };

    pub fn new(canvas: [f32; 2], pivot: [f32; 2], offset: [f32; 2]) -> Self {
        Self {
            canvas,
            pivot,
            offset,
        }
    }

    pub fn center() -> Self {
        Self::new([0.5, 0.5], [0.5, 0.5], [0.0, 0.0])
    }

    pub fn top_center(dy: f32) -> Self {
        Self::new([0.5, 0.0], [0.5, 0.0], [0.0, dy])
    }

    /// Bottom edge centered, `dy` pixels above the bottom (use a negative
    /// value).
    pub fn bottom_center(dy: f32) -> Self {
        Self::new([0.5, 1.0], [0.5, 1.0], [0.0, dy])
    }

    pub fn top_left(x: f32, y: f32) -> Self {
        Self::new([0.0, 0.0], [0.0, 0.0], [x, y])
    }
}

/// Background of a [`Kind::Panel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    /// Themed panel with border.
    Normal,
    /// Translucent dimming layer (behind menus and modals).
    Dim,
    /// Opaque screen background color.
    Solid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Normal,
    Small,
}

#[derive(Clone, Debug)]
pub struct LabelData {
    pub text: String,
    /// Font size; `None` = theme text size.
    pub size: Option<f32>,
    /// `None` = theme text color.
    pub color: Option<[u8; 4]>,
    pub align: TextAlign,
    /// `false`: the text is shown literally (braces escaped).
    pub markup: bool,
    /// Lines of text the node reserves room for.
    pub lines: usize,
    pub dim: bool,
}

#[derive(Clone, Debug)]
pub struct ButtonData {
    pub label: String,
    /// Active tab / toggled entry.
    pub selected: bool,
    pub kind: ButtonKind,
}

/// What a node is and the state it keeps.
#[derive(Clone, Debug)]
pub enum Kind {
    Container,
    Panel(PanelKind),
    Label(LabelData),
    /// A button; its children (if any) are drawn on top (cards).
    Button(ButtonData),
    Slider {
        value: f32,
        step: Option<f32>,
    },
    Toggle {
        on: bool,
        label: String,
    },
    Selector {
        options: Vec<String>,
        index: usize,
    },
    /// Vertical list showing the children from `first` while they fit.
    ScrollList {
        first: usize,
        shown: usize,
    },
    Image {
        texture: Option<TextureId>,
        uv: [f32; 4],
    },
}

impl Kind {
    pub fn is_interactive(&self) -> bool {
        matches!(
            self,
            Kind::Button(_) | Kind::Slider { .. } | Kind::Toggle { .. } | Kind::Selector { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CacheKey {
    text: u64,
    color: [u8; 4],
    width: i32,
    size: i32,
    align: u8,
}

/// Laid-out text blocks of a node (rebuilt when text, size, color or width
/// change).
#[derive(Default)]
pub(crate) struct TextCache {
    entries: Vec<(CacheKey, TextBlock)>,
}

impl std::fmt::Debug for TextCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextCache({})", self.entries.len())
    }
}

impl TextCache {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn block(
        &mut self,
        markup: &str,
        color: [u8; 4],
        width: f32,
        size: f32,
        align: TextAlign,
    ) -> &mut TextBlock {
        let mut hasher = DefaultHasher::new();
        markup.hash(&mut hasher);
        let key = CacheKey {
            text: hasher.finish(),
            color,
            width: width.round() as i32,
            size: (size * 10.0).round() as i32,
            align: align as u8,
        };
        let index = match self.entries.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                if self.entries.len() >= 4 {
                    self.entries.remove(0);
                }
                let mut block = TextBlock::new(
                    markup,
                    orbit_text::TextStyle {
                        font_size: size,
                        color,
                        align,
                        ..Default::default()
                    },
                    Some(width.max(1.0)),
                    f32::INFINITY,
                );
                block.skip_to_end();
                self.entries.push((key, block));
                self.entries.len() - 1
            }
        };
        &mut self.entries[index].1
    }
}

/// One widget. Build with the constructors and chained setters, then add it
/// to a [`crate::Ui`].
#[derive(Debug)]
pub struct Node {
    pub kind: Kind,
    pub tag: Tag,
    pub width: Size,
    pub height: Size,
    pub dir: Dir,
    /// Horizontal and vertical inner margin.
    pub pad: [f32; 2],
    /// Gap between children.
    pub gap: f32,
    /// Placement along the container's axis (`Stretch` is treated as
    /// `Start`).
    pub main: Align,
    pub cross: Align,
    pub anchor: Anchor,
    pub enabled: bool,
    // --- computed by layout ---
    pub(crate) rect: [f32; 4],
    pub(crate) pref: [f32; 2],
    pub(crate) visible: bool,
    pub(crate) children: Vec<NodeId>,
    pub(crate) parent: Option<NodeId>,
    pub(crate) cache: TextCache,
}

impl Node {
    fn base(kind: Kind) -> Self {
        Self {
            kind,
            tag: NO_TAG,
            width: Size::Fit,
            height: Size::Fit,
            dir: Dir::Vertical,
            pad: [0.0, 0.0],
            gap: 0.0,
            main: Align::Start,
            cross: Align::Start,
            anchor: Anchor::FULL,
            enabled: true,
            rect: [0.0; 4],
            pref: [0.0; 2],
            visible: true,
            children: Vec::new(),
            parent: None,
            cache: TextCache::default(),
        }
    }

    pub fn container(dir: Dir) -> Self {
        let mut n = Self::base(Kind::Container);
        n.dir = dir;
        n
    }

    pub fn vbox() -> Self {
        Self::container(Dir::Vertical)
    }

    pub fn hbox() -> Self {
        Self::container(Dir::Horizontal)
    }

    pub fn stack() -> Self {
        Self::container(Dir::Stack)
    }

    /// An empty node taking `Fill(1)` room (pushes siblings apart).
    pub fn spacer() -> Self {
        Self::container(Dir::Stack).size(Size::Fill(1.0), Size::Fill(1.0))
    }

    pub fn panel(kind: PanelKind, dir: Dir) -> Self {
        let mut n = Self::base(Kind::Panel(kind));
        n.dir = dir;
        n
    }

    pub fn label(text: impl Into<String>) -> Self {
        Self::base(Kind::Label(LabelData {
            text: text.into(),
            size: None,
            color: None,
            align: TextAlign::Left,
            markup: false,
            lines: 1,
            dim: false,
        }))
    }

    pub fn button(tag: Tag, label: impl Into<String>) -> Self {
        let mut n = Self::base(Kind::Button(ButtonData {
            label: label.into(),
            selected: false,
            kind: ButtonKind::Normal,
        }));
        n.tag = tag;
        n.dir = Dir::Stack;
        n
    }

    pub fn slider(tag: Tag, value: f32) -> Self {
        let mut n = Self::base(Kind::Slider {
            value: value.clamp(0.0, 1.0),
            step: None,
        });
        n.tag = tag;
        n
    }

    pub fn toggle(tag: Tag, label: impl Into<String>, on: bool) -> Self {
        let mut n = Self::base(Kind::Toggle {
            on,
            label: label.into(),
        });
        n.tag = tag;
        n
    }

    pub fn selector(tag: Tag, options: Vec<String>, index: usize) -> Self {
        let index = if options.is_empty() {
            0
        } else {
            index.min(options.len() - 1)
        };
        let mut n = Self::base(Kind::Selector { options, index });
        n.tag = tag;
        n
    }

    pub fn scroll_list(tag: Tag) -> Self {
        let mut n = Self::base(Kind::ScrollList { first: 0, shown: 0 });
        n.tag = tag;
        n
    }

    pub fn image(texture: Option<TextureId>) -> Self {
        Self::base(Kind::Image {
            texture,
            uv: [0.0, 0.0, 1.0, 1.0],
        })
    }

    // ---- chained setters ----

    pub fn tag(mut self, tag: Tag) -> Self {
        self.tag = tag;
        self
    }

    pub fn w(mut self, size: Size) -> Self {
        self.width = size;
        self
    }

    pub fn h(mut self, size: Size) -> Self {
        self.height = size;
        self
    }

    pub fn size(mut self, w: Size, h: Size) -> Self {
        self.width = w;
        self.height = h;
        self
    }

    pub fn fixed(self, w: f32, h: f32) -> Self {
        self.size(Size::Fixed(w), Size::Fixed(h))
    }

    pub fn fill(self) -> Self {
        self.size(Size::Fill(1.0), Size::Fill(1.0))
    }

    pub fn pad(mut self, x: f32, y: f32) -> Self {
        self.pad = [x, y];
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn align(mut self, main: Align, cross: Align) -> Self {
        self.main = main;
        self.cross = cross;
        self
    }

    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn dir(mut self, dir: Dir) -> Self {
        self.dir = dir;
        self
    }

    // ---- kind-specific setters (no-ops on other kinds) ----

    pub fn font(mut self, size: f32) -> Self {
        if let Kind::Label(l) = &mut self.kind {
            l.size = Some(size);
        }
        self
    }

    pub fn color(mut self, color: [u8; 4]) -> Self {
        if let Kind::Label(l) = &mut self.kind {
            l.color = Some(color);
        }
        self
    }

    pub fn dimmed(mut self) -> Self {
        if let Kind::Label(l) = &mut self.kind {
            l.dim = true;
        }
        self
    }

    pub fn text_align(mut self, align: TextAlign) -> Self {
        if let Kind::Label(l) = &mut self.kind {
            l.align = align;
        }
        self
    }

    /// Interpret the label text as `orbit_text` markup.
    pub fn markup(mut self) -> Self {
        if let Kind::Label(l) = &mut self.kind {
            l.markup = true;
        }
        self
    }

    pub fn lines(mut self, lines: usize) -> Self {
        if let Kind::Label(l) = &mut self.kind {
            l.lines = lines.max(1);
        }
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        if let Kind::Button(b) = &mut self.kind {
            b.selected = selected;
        }
        self
    }

    pub fn small(mut self) -> Self {
        if let Kind::Button(b) = &mut self.kind {
            b.kind = ButtonKind::Small;
        }
        self
    }

    pub fn step(mut self, step: f32) -> Self {
        if let Kind::Slider { step: s, .. } = &mut self.kind {
            *s = Some(step);
        }
        self
    }

    pub fn uv(mut self, uv: [f32; 4]) -> Self {
        if let Kind::Image { uv: u, .. } = &mut self.kind {
            *u = uv;
        }
        self
    }

    /// Computed rectangle `[x, y, w, h]` (valid after layout).
    pub fn rect(&self) -> [f32; 4] {
        self.rect
    }

    /// `false` when the node is hidden (scrolled out of a list).
    pub fn is_visible(&self) -> bool {
        self.visible
    }
}

/// Rough text width: no font access in the layout (it must stay testable
/// without any GPU or font data). Non-ASCII text counts as full-width.
pub fn estimate_text_width(text: &str, size: f32) -> f32 {
    text.chars()
        .map(|c| if c.is_ascii() { 0.6 } else { 1.0 })
        .sum::<f32>()
        * size
}

/// Number of lines `text` needs in a box `width` wide (at least 1).
pub fn estimate_lines(text: &str, size: f32, width: f32) -> usize {
    if width <= 0.0 {
        return 1;
    }
    text.split('\n')
        .map(|line| ((estimate_text_width(line, size) / width).ceil() as usize).max(1))
        .sum::<usize>()
        .max(1)
}

/// Cuts `text` to `max_chars` characters, ending with an ellipsis when
/// shortened.
pub fn ellipsize(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('\u{2026}');
    out
}

/// Escapes the markup braces so `text` is displayed literally.
pub fn escape_markup(text: &str) -> String {
    text.replace('{', "{{").replace('}', "}}")
}
