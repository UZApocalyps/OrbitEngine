//! Widget nodes: kind-specific data, layout properties and builders.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub use orbit_ui_data::{Align, Anchor, Dir, PanelKind, Size};

use orbit_render2d::TextureId;
use orbit_text::{TextAlign, TextBlock};

/// Identifies a widget to the screen that owns it (events carry it back).
/// `0` means "no tag".
pub type Tag = u32;
pub const NO_TAG: Tag = 0;

/// Index of a node inside its [`crate::Ui`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) usize);

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
        /// A dropdown: click / Confirm opens a popup list of the options
        /// (Left / Right still step through them).
        popup: bool,
    },
    /// A list showing its rows (units: one per child of a vertical list,
    /// one per row of a grid) from `first` while they fit. Layout keeps
    /// `shown` (units in view) and `total` (units) current.
    ScrollList {
        first: usize,
        shown: usize,
        total: usize,
        mode: ListMode,
    },
    Image {
        texture: Option<TextureId>,
        uv: [f32; 4],
        /// Clickable and focusable like a button (`ui_version` 8 image
        /// actions): activating it emits `Activated(tag)`.
        clickable: bool,
    },
}

/// How a [`Kind::ScrollList`] handles rows that do not fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMode {
    /// The `ScrollList` widget: a vertical list, scrollbar always reserved.
    Legacy,
    /// A generated list (`Repeat`): scrolls by row, laid out like a plain
    /// container while everything fits.
    Scroll,
    /// Whole pages of rows, with a pager footer as last child.
    Pages,
}

impl Kind {
    pub fn is_interactive(&self) -> bool {
        matches!(
            self,
            Kind::Button(_)
                | Kind::Slider { .. }
                | Kind::Toggle { .. }
                | Kind::Selector { .. }
                | Kind::Image { clickable: true, .. }
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
    family: u64,
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
        family: Option<&str>,
    ) -> &mut TextBlock {
        let mut hasher = DefaultHasher::new();
        markup.hash(&mut hasher);
        let mut family_hasher = DefaultHasher::new();
        family.hash(&mut family_hasher);
        let key = CacheKey {
            text: hasher.finish(),
            color,
            width: width.round() as i32,
            size: (size * 10.0).round() as i32,
            align: align as u8,
            family: family_hasher.finish(),
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
                        family: family.map(str::to_owned),
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
    /// Outer margin `[left, top, right, bottom]` kept around the node.
    pub margin: [f32; 4],
    /// Name of a theme style (`Theme::styles`) applied when drawing.
    pub style: Option<String>,
    /// Gap between children.
    pub gap: f32,
    /// Placement along the container's axis (`Stretch` is treated as
    /// `Start`).
    pub main: Align,
    pub cross: Align,
    pub anchor: Anchor,
    /// Columns / rows a child of a grid covers (1 = a single cell).
    pub col_span: usize,
    pub row_span: usize,
    pub enabled: bool,
    /// Pager footer of a paged list (its last child): the page label text,
    /// `{page}` and `{pages}` filled in by layout. Holds `[previous button,
    /// label, next button]`.
    pub footer: Option<String>,
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
            margin: [0.0; 4],
            style: None,
            gap: 0.0,
            main: Align::Start,
            cross: Align::Start,
            anchor: Anchor::FULL,
            col_span: 1,
            row_span: 1,
            enabled: true,
            footer: None,
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
        let mut n = Self::base(Kind::Selector {
            options,
            index,
            popup: false,
        });
        n.tag = tag;
        n
    }

    /// A selector that opens a popup list (a dropdown).
    pub fn dropdown(tag: Tag, options: Vec<String>, index: usize) -> Self {
        let mut n = Self::selector(tag, options, index);
        if let Kind::Selector { popup, .. } = &mut n.kind {
            *popup = true;
        }
        n
    }

    pub fn scroll_list(tag: Tag) -> Self {
        let mut n = Self::base(Kind::ScrollList {
            first: 0,
            shown: 0,
            total: 0,
            mode: ListMode::Legacy,
        });
        n.tag = tag;
        n
    }

    /// A generated list: rows in `dir` (`Vertical` or `Grid`) that scroll,
    /// or page when `paged` (then add a [`Node::pager`] last).
    pub fn collection(tag: Tag, dir: Dir, paged: bool) -> Self {
        let mut n = Self::base(Kind::ScrollList {
            first: 0,
            shown: 0,
            total: 0,
            mode: if paged { ListMode::Pages } else { ListMode::Scroll },
        });
        n.tag = tag;
        n.dir = dir;
        n
    }

    /// The footer of a paged list; `template` is the page label (`{page}`,
    /// `{pages}`). Children: previous button, label, next button.
    pub fn pager(template: impl Into<String>) -> Self {
        let mut n = Self::hbox();
        n.footer = Some(template.into());
        n.width = Size::Fill(1.0);
        n.main = Align::Center;
        n.cross = Align::Center;
        n.gap = 24.0;
        n
    }

    pub fn image(texture: Option<TextureId>) -> Self {
        Self::base(Kind::Image {
            texture,
            uv: [0.0, 0.0, 1.0, 1.0],
            clickable: false,
        })
    }

    /// An image that is clicked like a button (emits `Activated(tag)`).
    pub fn clickable_image(tag: Tag, texture: Option<TextureId>) -> Self {
        let mut n = Self::image(texture);
        if let Kind::Image { clickable, .. } = &mut n.kind {
            *clickable = true;
        }
        n.tag = tag;
        n
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

    /// Outer margin `[left, top, right, bottom]`.
    pub fn margin(mut self, margin: [f32; 4]) -> Self {
        self.margin = margin;
        self
    }

    /// Applies the theme style called `name`.
    pub fn style(mut self, name: impl Into<String>) -> Self {
        self.style = Some(name.into());
        self
    }

    /// A grid container `columns` wide.
    pub fn grid(columns: usize) -> Self {
        Self::container(Dir::Grid(columns))
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

    /// Columns and rows this node covers when it is a child of a grid.
    pub fn span(mut self, cols: usize, rows: usize) -> Self {
        self.col_span = cols.max(1);
        self.row_span = rows.max(1);
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
