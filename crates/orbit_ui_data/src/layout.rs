//! Serializable layout vocabulary shared by widgets and screen documents.

use serde::{Deserialize, Serialize};

/// How a node is sized along one axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Size {
    /// Content size.
    Fit,
    /// Exact size in virtual pixels.
    Fixed(f32),
    /// Share of the room left in the parent, with this weight.
    Fill(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    /// Cross axis only: children take the full cross size.
    Stretch,
}

/// How a container places its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    Vertical,
    Horizontal,
    /// All children overlap in the content box.
    Stack,
    /// Children fill a grid row by row, this many columns wide (0 counts as
    /// one). Column widths and row heights follow the widest / tallest cell;
    /// spare room goes to the columns / rows holding a `Fill` child. `gap`
    /// separates rows and columns.
    Grid(usize),
}

/// Where a root node sits on the canvas: `pivot` of the node is put on the
/// `canvas` fraction point, then moved by `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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

/// Background of a panel widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelKind {
    /// Themed panel with border.
    Normal,
    /// Translucent dimming layer (behind menus and modals).
    Dim,
    /// Opaque screen background color.
    Solid,
}
