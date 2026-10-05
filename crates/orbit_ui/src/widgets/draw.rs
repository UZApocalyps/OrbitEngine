//! Drawing: turns the laid-out widgets into sprites and text on a `Scene`.

use std::collections::HashMap;

use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId};
use orbit_text::{DrawParams, TextAlign, TextEngine, TextRenderer};

use super::Ui;
use super::layout::LINE_HEIGHT;
use super::node::{ButtonKind, Kind, Node, NodeId, PanelKind, TextCache, escape_markup};
use crate::paint::Painter;
use crate::theme::{Fill, Style, TextAlignment, Theme, WidgetState, resolve_style, tint};

/// Everything drawing needs from the host.
pub struct DrawCtx<'a> {
    pub engine: &'a mut TextEngine,
    pub text_renderer: &'a mut TextRenderer,
    pub renderer: &'a Renderer2d,
    pub scene: &'a mut Scene,
    pub white: TextureId,
}

struct Drawer<'a, 'b, 'c> {
    theme: &'a Theme,
    images: &'a HashMap<String, TextureId>,
    family: Option<String>,
    nodes: &'a mut Vec<Node>,
    focus: Option<NodeId>,
    hover: Option<NodeId>,
    pressed: Option<NodeId>,
    focus_visible: bool,
    alpha: f32,
    ctx: &'c mut DrawCtx<'b>,
}

impl Ui {
    /// Appends every visible widget to the scene (layer `self.layer()`, the
    /// modal one layer above).
    pub fn draw(&mut self, ctx: &mut DrawCtx) {
        self.ensure_layout();
        let modal_root = self.modal.map(|(r, _, _)| r);
        let base = self.layer;
        let roots = self.roots.clone();
        let mut drawer = Drawer {
            theme: &self.theme,
            images: &self.images,
            family: None,
            nodes: &mut self.nodes,
            focus: self.focus,
            hover: self.hover,
            pressed: self.pressed,
            focus_visible: self.focus_visible,
            alpha: self.alpha,
            ctx,
        };
        for root in roots {
            let layer = if Some(root) == modal_root {
                Layer(base.0.saturating_add(1))
            } else {
                base
            };
            drawer.draw_node(root, layer);
        }
    }
}

impl Drawer<'_, '_, '_> {
    fn quad(&mut self, rect: [f32; 4], color: [u8; 4], layer: Layer, z: f32) {
        if rect[2] <= 0.0 || rect[3] <= 0.0 || color[3] == 0 {
            return;
        }
        let mut t = tint(color);
        t[3] *= self.alpha;
        self.ctx.scene.push(
            Sprite2d::new(self.ctx.white, [rect[0], rect[1]], [rect[2], rect[3]])
                .anchor([0.0, 0.0])
                .layer(layer)
                .z(z)
                .tint(t),
        );
    }

    /// The background of a styled widget in `state`: its shadow, then the
    /// style's fill for the state (see [`Style::fill_for`]), else
    /// `default`. An image that is not loaded falls back to the style's
    /// colors, then `default`. Returns whether a nine-slice was drawn (no
    /// border goes on top of one).
    fn background(
        &mut self,
        style: Option<&Style>,
        state: WidgetState,
        rect: [f32; 4],
        layer: Layer,
        default: [u8; 4],
    ) -> bool {
        let painter = Painter {
            renderer: self.ctx.renderer,
            white: self.ctx.white,
            images: self.images,
            alpha: self.alpha,
        };
        if let Some(shadow) = style.and_then(|s| s.shadow.as_ref()) {
            painter.shadow(self.ctx.scene, shadow, rect, layer, 0.0);
        }
        let fill = style.and_then(|s| s.fill_for(state));
        if let Some(fill) = &fill
            && painter.fill(self.ctx.scene, fill, rect, layer, 0.0)
        {
            return matches!(fill, Fill::NineSlice { .. });
        }
        let color = style
            .and_then(|s| {
                s.colors.pick_state(state).or(match &s.fill {
                    Some(f) => f.flat_color(),
                    None => None,
                })
            })
            .unwrap_or(default);
        painter.quad(self.ctx.scene, rect, color, layer, 0.0);
        false
    }

    fn border(&mut self, rect: [f32; 4], width: f32, color: [u8; 4], layer: Layer, z: f32) {
        if width <= 0.0 {
            return;
        }
        let [x, y, w, h] = rect;
        self.quad([x, y, w, width], color, layer, z);
        self.quad([x, y + h - width, w, width], color, layer, z);
        self.quad([x, y + width, width, h - 2.0 * width], color, layer, z);
        self.quad(
            [x + w - width, y + width, width, h - 2.0 * width],
            color,
            layer,
            z,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        cache: &mut TextCache,
        markup: &str,
        color: [u8; 4],
        size: f32,
        align: TextAlign,
        rect: [f32; 4],
        center_v: bool,
        layer: Layer,
    ) {
        if markup.is_empty() || rect[2] <= 0.0 {
            return;
        }
        let block = cache.block(markup, color, rect[2], size, align, self.family.as_deref());
        let y = if center_v {
            rect[1] + (rect[3] - size * LINE_HEIGHT) * 0.5
        } else {
            rect[1]
        };
        let params = DrawParams {
            origin: [rect[0], y],
            layer,
            z: 1.0,
            alpha: self.alpha,
            ..Default::default()
        };
        block.draw(
            self.ctx.engine,
            self.ctx.text_renderer,
            self.ctx.renderer,
            self.ctx.scene,
            &params,
        );
    }

    fn draw_node(&mut self, id: NodeId, layer: Layer) {
        if !self.nodes[id.0].visible {
            return;
        }
        let rect = self.nodes[id.0].rect;
        let enabled = self.nodes[id.0].enabled;
        let hot = self.hover == Some(id) || (self.focus == Some(id) && self.focus_visible);
        let ring = self.focus == Some(id) && self.focus_visible && enabled;
        let pressed = self.pressed == Some(id) && self.hover == Some(id);
        let state = WidgetState::of(
            enabled,
            pressed,
            self.hover == Some(id),
            self.focus == Some(id) && self.focus_visible,
        );
        // Panels and labels only tell enabled from disabled.
        let still = if enabled { WidgetState::Normal } else { WidgetState::Disabled };
        let theme = self.theme;
        let w = &theme.widgets;
        let style = self.nodes[id.0]
            .style
            .as_deref()
            .and_then(|n| resolve_style(&theme.styles, n));

        self.family = style
            .as_ref()
            .and_then(|s| s.font.as_deref())
            .map(|f| theme.font_family(f).to_owned());
        // Take the node's cache out while drawing text (it is borrowed
        // from the arena otherwise).
        let mut cache = std::mem::take(&mut self.nodes[id.0].cache);
        let kind = self.nodes[id.0].kind.clone();
        match &kind {
            Kind::Container => {}
            Kind::Panel(PanelKind::Dim) => self.quad(rect, w.panel.dim, layer, 0.0),
            Kind::Panel(PanelKind::Solid) => self.quad(rect, theme.screens.background, layer, 0.0),
            Kind::Panel(PanelKind::Normal) => {
                let (bw, bc) = match style.as_ref().and_then(|s| s.border.as_ref()) {
                    Some(b) => (b.width, b.color),
                    None => (w.panel.border_width, w.panel.border_color),
                };
                if !self.background(style.as_ref(), still, rect, layer, w.panel.background) {
                    self.border(rect, bw, bc, layer, 0.2);
                }
            }
            Kind::Label(l) => {
                let size = l
                    .size
                    .or(style.as_ref().and_then(|s| s.text_size))
                    .unwrap_or(w.text_size);
                let color = l
                    .color
                    .or(style.as_ref().and_then(|s| s.text_color_for(still)))
                    .unwrap_or(if l.dim {
                        w.dim_text_color
                    } else {
                        w.text_color
                    });
                let markup = if l.markup {
                    l.text.clone()
                } else {
                    escape_markup(&l.text)
                };
                let markup = styled_markup(style.as_ref(), markup);
                let mut l = l.clone();
                if l.align == TextAlign::Left
                    && let Some(a) = style.as_ref().and_then(|s| s.text_align)
                {
                    l.align = text_align(a);
                }
                let l = &l;
                // The layout width of a label is only an estimate (no font
                // access): a single-line label gets some room so that wide
                // glyphs never wrap it ("Demo" must not become "Dem" / "o").
                let mut text_rect = rect;
                if l.lines == 1 {
                    let extra = rect[2] * 0.75 + size * 2.0;
                    match l.align {
                        TextAlign::Center => text_rect[0] -= extra * 0.5,
                        TextAlign::Right => text_rect[0] -= extra,
                        TextAlign::Left | TextAlign::Justified => {}
                    }
                    text_rect[2] += extra;
                }
                self.text(
                    &mut cache,
                    &markup,
                    color,
                    size,
                    l.align,
                    text_rect,
                    l.lines == 1,
                    layer,
                );
            }
            Kind::Button(b) => {
                let bt = &w.button;
                let bg = if !enabled {
                    bt.disabled
                } else if pressed {
                    bt.pressed
                } else if hot {
                    bt.hover
                } else if b.selected {
                    bt.selected
                } else {
                    bt.normal
                };
                let sliced = self.background(style.as_ref(), state, rect, layer, bg);
                if !sliced && let Some(border) = style.as_ref().and_then(|s| s.border.as_ref()) {
                    self.border(rect, border.width, border.color, layer, 0.2);
                }
                if b.selected && !hot && enabled {
                    self.border(rect, 2.0, w.accent_color, layer, 0.2);
                }
                if ring {
                    self.border(rect, bt.focus_width, bt.focus_color, layer, 0.4);
                }
                let color = if !enabled {
                    bt.disabled_text_color
                } else if hot {
                    bt.hover_text_color
                } else {
                    bt.text_color
                };
                let color = style
                    .as_ref()
                    .and_then(|s| s.text_color_for(state))
                    .unwrap_or(color);
                let size = match b.kind {
                    ButtonKind::Normal => style
                        .as_ref()
                        .and_then(|s| s.text_size)
                        .unwrap_or(w.text_size),
                    ButtonKind::Small => style
                        .as_ref()
                        .and_then(|s| s.text_size)
                        .unwrap_or(w.small_text_size),
                };
                // A style's horizontal padding is the room on each side;
                // the stock inset is half the theme's.
                let inset = style
                    .as_ref()
                    .and_then(|s| s.padding)
                    .map_or(bt.padding_x * 0.5, |p| p[0]);
                let text_rect = [
                    rect[0] + inset,
                    rect[1],
                    (rect[2] - 2.0 * inset).max(1.0),
                    rect[3],
                ];
                let label = if style.as_ref().is_none_or(Style::text_visible) {
                    styled_markup(style.as_ref(), escape_markup(&b.label))
                } else {
                    String::new()
                };
                let align = style
                    .as_ref()
                    .and_then(|s| s.text_align)
                    .map_or(TextAlign::Center, text_align);
                self.text(
                    &mut cache,
                    &label,
                    color,
                    size,
                    align,
                    text_rect,
                    true,
                    layer,
                );
            }
            Kind::Slider { value, .. } => {
                let c = &w.controls;
                let track = [
                    rect[0],
                    rect[1] + (rect[3] - c.track_height) * 0.5,
                    rect[2],
                    c.track_height,
                ];
                self.quad(track, c.track, layer, 0.0);
                let usable = (rect[2] - c.knob_size[0]).max(0.0);
                let knob_x = rect[0] + usable * value;
                self.quad(
                    [
                        track[0],
                        track[1],
                        knob_x - rect[0] + c.knob_size[0] * 0.5,
                        track[3],
                    ],
                    if enabled { c.fill } else { w.button.disabled },
                    layer,
                    0.1,
                );
                let knob = [
                    knob_x,
                    rect[1] + (rect[3] - c.knob_size[1]) * 0.5,
                    c.knob_size[0],
                    c.knob_size[1],
                ];
                self.quad(knob, c.knob, layer, 0.3);
                if ring {
                    self.border(rect, w.button.focus_width, w.button.focus_color, layer, 0.4);
                } else if hot {
                    self.border(knob, 2.0, w.accent_color, layer, 0.4);
                }
            }
            Kind::Toggle { on, label } => {
                let c = &w.controls;
                let bx = [
                    rect[0],
                    rect[1] + (rect[3] - c.toggle_size) * 0.5,
                    c.toggle_size,
                    c.toggle_size,
                ];
                self.quad(bx, c.toggle_off, layer, 0.0);
                self.border(
                    bx,
                    2.0,
                    if hot {
                        w.accent_color
                    } else {
                        w.panel.border_color
                    },
                    layer,
                    0.2,
                );
                if *on {
                    let inset = c.toggle_size * 0.22;
                    self.quad(
                        [
                            bx[0] + inset,
                            bx[1] + inset,
                            bx[2] - 2.0 * inset,
                            bx[3] - 2.0 * inset,
                        ],
                        c.toggle_on,
                        layer,
                        0.3,
                    );
                }
                if ring {
                    self.border(rect, w.button.focus_width, w.button.focus_color, layer, 0.4);
                }
                let tr = [
                    rect[0] + c.toggle_size + 20.0,
                    rect[1],
                    (rect[2] - c.toggle_size - 20.0).max(1.0),
                    rect[3],
                ];
                self.text(
                    &mut cache,
                    &escape_markup(label),
                    w.text_color,
                    w.text_size,
                    TextAlign::Left,
                    tr,
                    true,
                    layer,
                );
            }
            Kind::Selector {
                options,
                index,
                popup,
            } => {
                let bt = &w.button;
                let bg = if hot { bt.hover } else { bt.normal };
                self.background(style.as_ref(), state, rect, layer, bg);
                if ring {
                    self.border(rect, bt.focus_width, bt.focus_color, layer, 0.4);
                }
                let color = if hot {
                    bt.hover_text_color
                } else {
                    bt.text_color
                };
                let arrow = [rect[0], rect[1], 56.0, rect[3]];
                let arrow_r = [rect[0] + rect[2] - 56.0, rect[1], 56.0, rect[3]];
                let multi = options.len() > 1;
                if *popup {
                    self.text(
                        &mut cache,
                        "v",
                        w.accent_color,
                        w.text_size,
                        TextAlign::Center,
                        arrow_r,
                        true,
                        layer,
                    );
                } else if multi {
                    self.text(
                        &mut cache,
                        "<",
                        w.accent_color,
                        w.text_size,
                        TextAlign::Center,
                        arrow,
                        true,
                        layer,
                    );
                    self.text(
                        &mut cache,
                        ">",
                        w.accent_color,
                        w.text_size,
                        TextAlign::Center,
                        arrow_r,
                        true,
                        layer,
                    );
                }
                let label = options.get(*index).map(String::as_str).unwrap_or("");
                let tr = [rect[0] + 56.0, rect[1], (rect[2] - 112.0).max(1.0), rect[3]];
                self.text(
                    &mut cache,
                    &escape_markup(label),
                    color,
                    w.text_size,
                    TextAlign::Center,
                    tr,
                    true,
                    layer,
                );
            }
            Kind::Image { texture, uv, clickable } => {
                // A style gives an image a background per state (a
                // clickable image's hover / pressed look).
                if let Some(s) = style.as_ref()
                    && s.fill_for(state).is_some()
                {
                    self.background(Some(s), state, rect, layer, [0, 0, 0, 0]);
                }
                if *clickable && ring {
                    self.border(rect, w.button.focus_width, w.button.focus_color, layer, 0.4);
                }
                if let Some(tex) = texture {
                    let mut t = [1.0, 1.0, 1.0, self.alpha];
                    t[3] = self.alpha;
                    self.ctx.scene.push(
                        Sprite2d::new(*tex, [rect[0], rect[1]], [rect[2], rect[3]])
                            .anchor([0.0, 0.0])
                            .layer(layer)
                            .z(0.1)
                            .uv(*uv)
                            .tint(t),
                    );
                } else if style.as_ref().is_none_or(|s| s.fill_for(state).is_none()) {
                    self.quad(rect, w.button.disabled, layer, 0.0);
                }
            }
            Kind::ScrollList { first, shown } => {
                let total = self.nodes[id.0].children.len();
                if *shown < total {
                    let l = &w.list;
                    let pad = self.nodes[id.0].pad;
                    let track = [
                        rect[0] + rect[2] - pad[0] - l.bar_width,
                        rect[1] + pad[1],
                        l.bar_width,
                        (rect[3] - 2.0 * pad[1]).max(0.0),
                    ];
                    self.quad(track, l.bar_track, layer, 0.0);
                    let frac = *shown as f32 / total as f32;
                    let thumb_h = (track[3] * frac).max(24.0).min(track[3]);
                    let span = (total - shown).max(1) as f32;
                    let thumb_y = track[1] + (track[3] - thumb_h) * (*first as f32 / span).min(1.0);
                    self.quad(
                        [track[0], thumb_y, l.bar_width, thumb_h],
                        l.bar_thumb,
                        layer,
                        0.1,
                    );
                }
                if ring {
                    self.border(rect, w.button.focus_width, w.button.focus_color, layer, 0.4);
                }
            }
        }
        self.nodes[id.0].cache = cache;
        // Rows of a scroll list never paint outside the list (long text,
        // glows, borders).
        let clip_list = matches!(self.nodes[id.0].kind, Kind::ScrollList { .. });
        if clip_list {
            self.ctx.scene.push_clip(rect);
        }
        for i in 0..self.nodes[id.0].children.len() {
            let child = self.nodes[id.0].children[i];
            self.draw_node(child, layer);
        }
        if clip_list {
            self.ctx.scene.pop_clip();
        }
    }
}

/// The engine alignment of a style alignment.
pub(crate) fn text_align(a: TextAlignment) -> TextAlign {
    match a {
        TextAlignment::Left => TextAlign::Left,
        TextAlignment::Center => TextAlign::Center,
        TextAlignment::Right => TextAlign::Right,
    }
}

/// `markup` in bold when the style asks for it.
pub(crate) fn styled_markup(style: Option<&Style>, markup: String) -> String {
    if style.and_then(|s| s.bold) == Some(true) && !markup.is_empty() {
        format!("{{b}}{markup}{{/b}}")
    } else {
        markup
    }
}
