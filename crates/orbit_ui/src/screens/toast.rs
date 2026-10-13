//! In-game notifications ("toasts"): the `Notify` node, scripts, UI actions,
//! achievement unlocks and the engine messages ("Saved", "Incompatible
//! save") all go through one [`NotificationStack`].
//!
//! Placement, timing, animation and looks come from
//! `Theme::notifications` ([`NotificationTheme`]). Toasts beyond
//! `max_visible` wait in a queue (their clock does not run). Everything is
//! driven by `update(dt)`: no wall clock, so it behaves the same on the web.
//! Nothing here is saved: the host clears the stack on load and rollback.
//! An empty stack costs nothing per frame.

use std::collections::HashMap;

use orbit_render2d::{Layer, Mesh2d, MeshVertex, TextureId};
use orbit_text::TextAlign;
use orbit_ui_data::{
    ACHIEVEMENT_NOTIFY_STYLE, BRIEF_SECONDS, CANVAS, NotificationStyle, NotificationTheme, NotifyAnimation,
    NotifySkip, SYSTEM_NOTIFY_STYLE, Style,
};

use crate::theme::{Border, Fill, StateColors, Theme, resolve_style, tint};
use crate::widgets::{Align, Anchor, Dir, DrawCtx, Node, NodeId, PanelKind, Size, Ui};

/// Layer of the toasts (above the stage, the dialogue box and the menus).
const LAYER: Layer = Layer(40);
/// Name of the per-toast style built from the notification style.
const TOAST_STYLE: &str = "__notify";
/// Longest text kept (a runaway variable cannot fill the screen).
const MAX_CHARS: usize = 600;
/// How fast a toast glides to its new place when another one leaves
/// (fraction of the way per second, frame-rate independent enough).
const GLIDE: f32 = 14.0;
/// Stock look (the toasts of earlier versions): padding and icon size.
const STOCK_PADDING: [f32; 2] = [40.0, 14.0];
const STOCK_ICON: f32 = 64.0;
/// Queued toasts kept at most (older queued ones are dropped).
const MAX_QUEUED: usize = 32;

/// One notification to show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notification {
    /// The text (markup when `markup` is set, else shown as is).
    pub text: String,
    pub markup: bool,
    /// A named style of `Theme::notifications` (`None`: `default`).
    pub style: Option<String>,
    /// The icon, already loaded by the host.
    pub icon: Option<TextureId>,
    /// The icon as a project path (looked up in the images given with
    /// [`NotificationStack::set_images`]) when `icon` is `None`.
    pub icon_path: Option<String>,
    /// Sound path played when it shows (else the style's, else the theme's).
    pub sound: Option<String>,
    /// Seconds on screen (else the style's, else the theme's).
    pub duration: Option<f32>,
}

impl Notification {
    /// A plain text in the `system` style (engine messages).
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: Some(SYSTEM_NOTIFY_STYLE.to_owned()),
            ..Self::default()
        }
    }
}

struct Toast {
    ui: Ui,
    root: NodeId,
    /// `[w, h]` once laid out.
    size: [f32; 2],
    /// Rounded background drawn under the widgets: fill, border, radius.
    shape: Option<([u8; 4], Option<Border>, f32)>,
    /// Seconds since it became visible.
    age: f32,
    duration: f32,
    /// Current top edge (glides to its target).
    y: Option<f32>,
    /// Text read by screen readers.
    plain: String,
}

/// The toasts on screen and those waiting their turn.
pub struct NotificationStack {
    theme: Theme,
    images: HashMap<String, TextureId>,
    visible: Vec<Toast>,
    queue: std::collections::VecDeque<Notification>,
    sounds: Vec<String>,
    skipping: bool,
    reduced_motion: bool,
    /// Every toast ever shown, for the screen-reader announcer.
    shown: u64,
}

/// Kept for the hosts written before the notification system.
pub type ToastStack = NotificationStack;

impl NotificationStack {
    pub fn new(theme: &Theme) -> Self {
        Self {
            theme: theme.clone(),
            images: HashMap::new(),
            visible: Vec::new(),
            queue: Default::default(),
            sounds: Vec::new(),
            skipping: false,
            reduced_motion: false,
            shown: 0,
        }
    }

    /// A new theme applies to the toasts shown from now on.
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = theme.clone();
    }

    /// The project images the toasts may draw (icons, style backgrounds).
    pub fn set_images(&mut self, images: HashMap<String, TextureId>) {
        self.images = images;
    }

    /// While skipping, new toasts follow `notifications.skip`.
    pub fn set_skipping(&mut self, skipping: bool) {
        self.skipping = skipping;
    }

    /// Reduced motion: toasts fade instead of sliding.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }

    fn config(&self) -> &NotificationTheme {
        &self.theme.notifications
    }

    /// An engine message (`system` style).
    pub fn push(&mut self, text: impl Into<String>) {
        self.notify(Notification::system(text));
    }

    /// An achievement unlock (`achievement` style) with its icon.
    pub fn push_with_icon(&mut self, text: impl Into<String>, icon: Option<TextureId>) {
        self.notify(Notification {
            text: text.into(),
            style: Some(ACHIEVEMENT_NOTIFY_STYLE.to_owned()),
            icon,
            ..Notification::default()
        });
    }

    /// Shows `n` (or queues it when `max_visible` toasts are on screen).
    /// Returns `false` when it is dropped (skip mode `Suppress`).
    pub fn notify(&mut self, mut n: Notification) -> bool {
        if self.skipping {
            match self.config().skip {
                NotifySkip::Suppress => return false,
                NotifySkip::Brief => {
                    let d = self.duration_of(&n).min(BRIEF_SECONDS);
                    n.duration = Some(d);
                }
                NotifySkip::Show => {}
            }
        }
        if n.text.chars().count() > MAX_CHARS {
            n.text = n.text.chars().take(MAX_CHARS).collect();
        }
        if self.queue.len() >= MAX_QUEUED {
            self.queue.pop_front();
        }
        self.queue.push_back(n);
        self.promote();
        true
    }

    /// Removes every toast at once (load, rollback, back to the title).
    pub fn clear(&mut self) {
        self.visible.clear();
        self.queue.clear();
        self.sounds.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty() && self.queue.is_empty()
    }

    /// Toasts on screen plus those waiting.
    pub fn len(&self) -> usize {
        self.visible.len() + self.queue.len()
    }

    /// Toasts on screen.
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// How many toasts were shown so far (screen-reader announcements).
    pub fn shown_count(&self) -> u64 {
        self.shown
    }

    /// Sounds of the toasts that appeared since the last call.
    pub fn take_sounds(&mut self) -> Vec<String> {
        std::mem::take(&mut self.sounds)
    }

    /// The plain texts on screen, oldest first (screen readers, tests).
    pub fn texts(&self) -> Vec<&str> {
        self.visible.iter().map(|t| t.plain.as_str()).collect()
    }

    /// `[x, y, w, h]` of each toast on screen, oldest first.
    pub fn rects(&self) -> Vec<[f32; 4]> {
        self.visible.iter().map(|t| self.rect_of(t, 0.0)).collect()
    }

    fn duration_of(&self, n: &Notification) -> f32 {
        let style = self.config().style(n.style.as_deref());
        let d = n
            .duration
            .or(style.duration)
            .or(self.config().duration)
            .unwrap_or(self.theme.widgets.list.toast_seconds);
        if d.is_finite() { d.max(0.1) } else { self.theme.widgets.list.toast_seconds }
    }

    fn anim(&self) -> (NotifyAnimation, f32) {
        let c = self.config();
        let secs = c.animation_seconds.max(0.0);
        match c.animation {
            NotifyAnimation::Slide if self.reduced_motion => (NotifyAnimation::Fade, secs.min(0.15)),
            a => (a, secs),
        }
    }

    /// Moves queued toasts on screen while there is room.
    fn promote(&mut self) {
        let max = self.config().max_visible.max(1);
        while self.visible.len() < max {
            let Some(n) = self.queue.pop_front() else { break };
            let toast = self.build(&n);
            let style = self.config().style(n.style.as_deref());
            if let Some(sound) = n
                .sound
                .clone()
                .or(style.sound)
                .or_else(|| self.config().sound.clone())
                .filter(|s| !s.trim().is_empty())
            {
                self.sounds.push(sound);
            }
            self.shown += 1;
            self.visible.push(toast);
        }
    }

    /// Advances the clocks: toasts expire, the queue moves up, the others
    /// glide to their place.
    pub fn update(&mut self, dt: f32) {
        if self.is_empty() {
            return;
        }
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        for t in &mut self.visible {
            t.age += dt;
        }
        self.visible.retain(|t| t.age < t.duration);
        self.promote();
        let targets = self.targets();
        let k = (dt * GLIDE).min(1.0);
        for (t, target) in self.visible.iter_mut().zip(targets) {
            t.y = Some(match t.y {
                Some(y) if (y - target).abs() > 0.5 => y + (target - y) * k,
                _ => target,
            });
        }
    }

    /// Dismisses the toast under `pos` (canvas pixels): it starts leaving.
    /// Returns `true` when a toast was hit (the click is consumed).
    pub fn click(&mut self, pos: [f32; 2]) -> bool {
        if self.visible.is_empty() || !self.config().click_to_dismiss {
            return false;
        }
        let (_, secs) = self.anim();
        let rects = self.rects();
        for (t, r) in self.visible.iter_mut().zip(rects) {
            if pos[0] >= r[0] && pos[0] <= r[0] + r[2] && pos[1] >= r[1] && pos[1] <= r[1] + r[3] {
                let leave = (t.duration - secs).max(0.0);
                t.age = t.age.max(leave);
                return true;
            }
        }
        false
    }

    /// The top edge each visible toast goes to, oldest first.
    fn targets(&self) -> Vec<f32> {
        let heights: Vec<f32> = self.visible.iter().map(|t| t.size[1]).collect();
        stack_tops(&heights, self.config())
    }

    /// `[x, y, w, h]` of a toast, moved by `slide` (0: in place, 1: fully
    /// out).
    fn rect_of(&self, t: &Toast, slide: f32) -> [f32; 4] {
        let c = self.config();
        let [w, h] = t.size;
        let [fx, fy] = c.anchor.fraction();
        let x = match fx {
            f if f < 0.25 => c.offset[0],
            f if f > 0.75 => CANVAS[0] - c.offset[0] - w,
            _ => (CANVAS[0] - w) * 0.5,
        };
        let y = t.y.unwrap_or_else(|| {
            let i = self.visible.iter().position(|o| std::ptr::eq(o, t)).unwrap_or(0);
            self.targets().get(i).copied().unwrap_or(c.offset[1])
        });
        if slide <= 0.0 {
            return [x, y, w, h];
        }
        // Out through the nearest edge.
        let (dx, dy) = match fx {
            f if f < 0.25 => (-(w + c.offset[0]), 0.0),
            f if f > 0.75 => (w + c.offset[0], 0.0),
            _ if fy < 0.5 => (0.0, -(h + c.offset[1])),
            _ => (0.0, h + c.offset[1]),
        };
        [x + dx * slide, y + dy * slide, w, h]
    }

    /// Builds the widgets of one toast.
    fn build(&self, n: &Notification) -> Toast {
        let c = self.config();
        let ns = c.style(n.style.as_deref());
        let stock = is_stock(&ns);
        let width = ns.width.unwrap_or(c.width);
        let padding = ns.padding.unwrap_or(STOCK_PADDING);
        let icon = n.icon.or_else(|| {
            n.icon_path
                .as_deref()
                .or(ns.icon.as_deref())
                .and_then(|p| self.images.get(p).copied())
        });
        let radius = ns.corner_radius.unwrap_or(0.0).max(0.0);
        let mut theme = self.theme.clone();
        let mut shape = None;
        if !stock {
            // The toast's own style: the named theme style, then the fields
            // of the notification style on top.
            let base = ns
                .style
                .as_deref()
                .and_then(|name| resolve_style(&theme.styles, name))
                .unwrap_or_default();
            let mut own = Style {
                font: ns.font.clone(),
                text_size: ns.text_size,
                text_colors: StateColors { normal: ns.text_color, ..StateColors::default() },
                fill: ns.background.map(Fill::Color),
                border: ns.border.clone(),
                padding: ns.padding,
                ..Style::default()
            }
            .over(&base);
            if radius > 0.5 {
                // Rounded corners are drawn as a mesh under the widgets.
                let fill = ns
                    .background
                    .or_else(|| own.fill.as_ref().and_then(Fill::flat_color))
                    .unwrap_or(theme.widgets.panel.background);
                shape = Some((fill, own.border.clone(), radius));
                own.fill = Some(Fill::Color([0, 0, 0, 0]));
                own.fills = Default::default();
                own.colors = StateColors::default();
                own.border = Some(Border { color: [0, 0, 0, 0], width: 0.0 });
                own.shadow = None;
            }
            theme.styles.insert(TOAST_STYLE.to_owned(), own);
        }
        let mut ui = Ui::new(theme);
        ui.set_layer(LAYER);
        ui.set_images(self.images.clone());
        let mut panel = Node::panel(PanelKind::Normal, Dir::Stack).pad(padding[0], padding[1]);
        if width > 0.0 {
            panel = panel.w(Size::Fixed(width.min(CANVAS[0])));
        }
        if !stock {
            panel = panel.style(TOAST_STYLE);
        }
        let root = ui.add_root(panel.anchor(Anchor::FULL));
        let parent = match icon {
            Some(icon) => {
                let row = ui.add(root, Node::hbox().gap(16.0).align(Align::Start, Align::Center));
                let side = ns.icon_size.unwrap_or(STOCK_ICON).max(1.0);
                ui.add(row, Node::image(Some(icon)).fixed(side, side));
                row
            }
            None => root,
        };
        let align = if icon.is_some() || width > 0.0 && !stock { TextAlign::Left } else { TextAlign::Center };
        let mut label = Node::label(n.text.clone()).text_align(align);
        if n.markup {
            label = label.markup();
        }
        if !stock {
            label = label.style(TOAST_STYLE);
        }
        ui.add(parent, label);
        ui.layout();
        let r = ui.node_rect(root);
        let plain = if n.markup {
            orbit_text::parse_markup(&n.text).plain()
        } else {
            n.text.clone()
        };
        Toast {
            ui,
            root,
            size: [r[2], r[3]],
            shape,
            age: 0.0,
            duration: self.duration_of(n),
            y: None,
            plain,
        }
    }

    pub fn draw(&mut self, ctx: &mut DrawCtx) {
        if self.visible.is_empty() {
            return;
        }
        let (anim, secs) = self.anim();
        let rects: Vec<([f32; 4], f32)> = self
            .visible
            .iter()
            .map(|t| {
                let p = phase(t.age, t.duration, secs);
                let slide = if anim == NotifyAnimation::Slide { 1.0 - ease_out(p) } else { 0.0 };
                let alpha = if anim == NotifyAnimation::None { 1.0 } else { p };
                (self.rect_of(t, slide), alpha)
            })
            .collect();
        for (t, (rect, alpha)) in self.visible.iter_mut().zip(rects) {
            if alpha <= 0.0 {
                continue;
            }
            if let Some((fill, border, radius)) = &t.shape {
                push_rounded(ctx, rect, *fill, border.as_ref(), *radius, alpha);
            }
            if t.ui.node(t.root).anchor.offset != [rect[0], rect[1]] {
                t.ui.node_mut(t.root).anchor = Anchor::new([0.0, 0.0], [0.0, 0.0], [rect[0], rect[1]]);
            }
            t.ui.set_alpha(alpha);
            t.ui.draw(ctx);
        }
    }
}

/// Whether a resolved notification style asks for nothing: the stock toast
/// (the theme's panel and widget text).
fn is_stock(s: &NotificationStyle) -> bool {
    s.style.is_none()
        && s.background.is_none()
        && s.border.is_none()
        && s.corner_radius.is_none_or(|r| r <= 0.5)
        && s.text_size.is_none()
        && s.text_color.is_none()
        && s.font.is_none()
        && s.padding.is_none()
}

/// 0 -> 1 while entering, 1 on screen, 1 -> 0 while leaving.
pub fn phase(age: f32, duration: f32, anim: f32) -> f32 {
    if anim <= 0.0 {
        return if age < duration { 1.0 } else { 0.0 };
    }
    let enter = (age / anim).clamp(0.0, 1.0);
    let leave = ((duration - age) / anim).clamp(0.0, 1.0);
    enter.min(leave)
}

fn ease_out(p: f32) -> f32 {
    let q = 1.0 - p.clamp(0.0, 1.0);
    1.0 - q * q * q
}

/// The top edge of each toast (oldest first) of `heights`, stacked from the
/// anchored edge as `config` says.
pub fn stack_tops(heights: &[f32], config: &NotificationTheme) -> Vec<f32> {
    let n = heights.len();
    if n == 0 {
        return Vec::new();
    }
    let gap = config.gap.max(0.0);
    let total: f32 = heights.iter().sum::<f32>() + gap * (n - 1) as f32;
    let top_anchor = config.anchor.is_top();
    let block_top = if top_anchor {
        config.offset[1]
    } else {
        CANVAS[1] - config.offset[1] - total
    };
    // Order from top to bottom: oldest first when the stack grows down.
    let order: Vec<usize> = if config.grows_down() {
        (0..n).collect()
    } else {
        (0..n).rev().collect()
    };
    let mut tops = vec![0.0; n];
    let mut y = block_top;
    for i in order {
        tops[i] = y;
        y += heights[i] + gap;
    }
    tops
}

/// A filled rounded rectangle with an optional outline, faded by `alpha`.
fn push_rounded(ctx: &mut DrawCtx, rect: [f32; 4], fill: [u8; 4], border: Option<&Border>, radius: f32, alpha: f32) {
    let mut polys = Vec::new();
    if let Some(b) = border.filter(|b| b.width > 0.0 && b.color[3] > 0) {
        let g = b.width;
        let outer = [rect[0] - g, rect[1] - g, rect[2] + 2.0 * g, rect[3] + 2.0 * g];
        polys.push((crate::bubble::rounded_rect(outer, radius + g), b.color, -0.6));
    }
    polys.push((crate::bubble::rounded_rect(rect, radius), fill, -0.5));
    for (poly, color, z) in polys {
        if color[3] == 0 || poly.len() < 3 {
            continue;
        }
        let mut c = tint(color);
        c[3] *= alpha;
        let center = poly.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
        let center = [center[0] / poly.len() as f32, center[1] / poly.len() as f32];
        let at = |p: [f32; 2]| MeshVertex { position: p, uv: [0.5, 0.5], color: c };
        let mut vertices = Vec::with_capacity(poly.len() + 1);
        vertices.push(at(center));
        vertices.extend(poly.iter().map(|p| at(*p)));
        let k = poly.len() as u32;
        let indices = (0..k).flat_map(|i| [0, 1 + i, 1 + (i + 1) % k]).collect();
        ctx.scene.push_mesh(Mesh2d::new(ctx.white, vertices, indices).layer(LAYER).z(z));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_ui_data::{NotifyAnchor, StackDirection};

    fn stack() -> NotificationStack {
        NotificationStack::new(&Theme::default())
    }

    #[test]
    fn toasts_expire_after_their_duration() {
        let mut t = stack();
        assert!(t.is_empty());
        t.push("a");
        t.update(1.0);
        assert_eq!(t.len(), 1);
        t.update(2.0); // 3 s > 2.5 s (widgets.list.toast_seconds)
        assert!(t.is_empty());
    }

    #[test]
    fn extra_toasts_wait_their_turn() {
        let mut theme = Theme::default();
        theme.notifications.max_visible = 2;
        let mut t = NotificationStack::new(&theme);
        for s in ["a", "b", "c"] {
            t.notify(Notification { text: s.into(), duration: Some(1.0), ..Default::default() });
        }
        assert_eq!(t.visible_len(), 2);
        assert_eq!(t.len(), 3);
        assert_eq!(t.texts(), ["a", "b"]);
        // The queued one's clock does not run: it gets its full second.
        t.update(1.05);
        assert_eq!(t.texts(), ["c"]);
        t.update(0.9);
        assert_eq!(t.texts(), ["c"]);
        t.update(0.2);
        assert!(t.is_empty());
    }

    #[test]
    fn a_click_dismisses_the_toast_under_the_pointer() {
        let mut t = stack();
        t.notify(Notification { text: "hello".into(), duration: Some(10.0), ..Default::default() });
        t.update(0.5);
        let r = t.rects()[0];
        assert!(!t.click([r[0] - 5.0, r[1] - 5.0]), "outside");
        assert!(t.click([r[0] + 2.0, r[1] + 2.0]));
        t.update(0.3); // the exit animation (0.25 s)
        assert!(t.is_empty());
    }

    #[test]
    fn stacking_follows_the_anchor_and_direction() {
        let mut c = NotificationTheme::default();
        c.offset = [10.0, 20.0];
        c.gap = 5.0;
        // Top: oldest on top, the next ones below.
        assert_eq!(stack_tops(&[30.0, 40.0], &c), vec![20.0, 55.0]);
        c.direction = StackDirection::Up;
        assert_eq!(stack_tops(&[30.0, 40.0], &c), vec![65.0, 20.0]);
        // Bottom: the block ends `offset` above the bottom edge; it grows up
        // (newest on top) by default.
        c.anchor = NotifyAnchor::BottomLeft;
        c.direction = StackDirection::Auto;
        let tops = stack_tops(&[30.0, 40.0], &c);
        assert_eq!(tops[0] + 30.0, CANVAS[1] - 20.0);
        assert_eq!(tops[1], CANVAS[1] - 20.0 - 75.0);
    }

    #[test]
    fn toasts_sit_in_the_anchored_corner() {
        let mut t = stack();
        t.push("a");
        t.update(0.0);
        let r = t.rects()[0];
        assert!((r[0] + r[2] - (CANVAS[0] - 32.0)).abs() < 0.5, "{r:?}");
        assert_eq!(r[1], 32.0);
    }

    #[test]
    fn skipping_shortens_or_drops_toasts() {
        let mut t = stack();
        t.set_skipping(true);
        t.notify(Notification { text: "a".into(), duration: Some(5.0), ..Default::default() });
        t.update(1.01);
        assert!(t.is_empty(), "brief while skipping");
        let mut theme = Theme::default();
        theme.notifications.skip = NotifySkip::Suppress;
        let mut t = NotificationStack::new(&theme);
        t.set_skipping(true);
        assert!(!t.notify(Notification::system("a")));
        assert!(t.is_empty());
    }

    #[test]
    fn enter_and_exit_phases() {
        assert_eq!(phase(0.0, 3.0, 0.25), 0.0);
        assert_eq!(phase(0.25, 3.0, 0.25), 1.0);
        assert_eq!(phase(1.0, 3.0, 0.25), 1.0);
        assert!((phase(2.9, 3.0, 0.25) - 0.4).abs() < 1e-4);
        assert_eq!(phase(0.0, 3.0, 0.0), 1.0, "no animation");
    }

    #[test]
    fn sounds_play_when_a_toast_shows() {
        let mut theme = Theme::default();
        theme.notifications.sound = Some("assets/ding.ogg".into());
        theme.notifications.max_visible = 1;
        let mut t = NotificationStack::new(&theme);
        t.push("a");
        t.notify(Notification { text: "b".into(), sound: Some("assets/b.ogg".into()), ..Default::default() });
        assert_eq!(t.take_sounds(), ["assets/ding.ogg"], "b waits");
        t.update(3.0);
        assert_eq!(t.take_sounds(), ["assets/b.ogg"]);
    }

    #[test]
    fn styled_toasts_build_and_clear() {
        let mut theme = Theme::default();
        theme.notifications.styles.insert(
            "quest".into(),
            NotificationStyle { corner_radius: Some(12.0), background: Some([1, 2, 3, 255]), ..Default::default() },
        );
        let mut t = NotificationStack::new(&theme);
        t.notify(Notification { text: "{b}Quest{/b}".into(), markup: true, style: Some("quest".into()), ..Default::default() });
        assert_eq!(t.texts(), ["Quest"]);
        assert!(t.visible[0].shape.is_some());
        t.clear();
        assert!(t.is_empty());
    }
}
