//! The save / load screen: paged grid of slots, tabs per category,
//! confirmation before overwriting or deleting.

use std::ops::Range;

use super::{Confirm, Screen, forward, title};
use crate::input::UiInput;
use crate::texts::Texts;
use crate::theme::Theme;
use crate::view::{
    CategoryView, SLOTS_PER_PAGE, SaveLoadMode, SaveLoadView, SlotCategory, SlotState, SlotView,
    UiEvent,
};
use crate::widgets::{Align, Dir, DrawCtx, Node, PanelKind, Size, Tag, Ui, WidgetEvent, ellipsize};
use orbit_text::TextAlign;

const T_TAB: Tag = 10;
const T_CARD: Tag = 100;
const T_DELETE: Tag = 200;
const T_PREV: Tag = 300;
const T_NEXT: Tag = 301;
const T_BACK: Tag = 302;

const COLUMNS: usize = 3;
const CARD: [f32; 2] = [544.0, 170.0];
const THUMB: [f32; 2] = [224.0, 126.0];

/// Number of pages needed for `total` slots (at least 1).
pub fn page_count(total: usize, per_page: usize) -> usize {
    total.div_ceil(per_page.max(1)).max(1)
}

/// Slot indices shown on `page` (clamped to the last page).
pub fn page_range(total: usize, page: usize, per_page: usize) -> Range<usize> {
    let per_page = per_page.max(1);
    let page = page.min(page_count(total, per_page) - 1);
    let start = (page * per_page).min(total);
    start..(start + per_page).min(total)
}

/// Grid of slots. Emits `LoadSlot`, `SaveSlot`, `DeleteSlot`, `Toast`
/// (incompatible save) and `Back`.
pub struct SaveLoadScreen {
    ui: Ui,
    view: SaveLoadView,
    texts: Texts,
    /// Index into the visible categories.
    category: usize,
    page: usize,
    confirm: Confirm,
}

impl SaveLoadScreen {
    pub fn new(theme: &Theme, texts: &Texts, view: SaveLoadView) -> Self {
        let mut screen = Self {
            ui: Ui::new(theme.clone()),
            view,
            texts: texts.clone(),
            category: 0,
            page: 0,
            confirm: Confirm::default(),
        };
        screen.rebuild();
        screen
    }

    /// Replaces the view-model (after a save, a delete...). Keeps the tab,
    /// the page and the focus.
    pub fn set_view(&mut self, view: SaveLoadView) {
        self.view = view;
        self.rebuild();
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn mode(&self) -> SaveLoadMode {
        self.view.mode
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn category_index(&self) -> usize {
        self.category
    }

    /// Categories shown as tabs: save mode only offers manual slots.
    fn categories(&self) -> Vec<&CategoryView> {
        self.view
            .categories
            .iter()
            .filter(|c| self.view.mode == SaveLoadMode::Load || c.category == SlotCategory::Manual)
            .collect()
    }

    fn current_slots(&self) -> &[SlotView] {
        self.categories()
            .get(self.category)
            .map_or(&[], |c| c.slots.as_slice())
    }

    fn slot_title(&self, slot: &SlotView) -> String {
        let n = (slot.id.index + 1).to_string();
        match slot.id.category {
            SlotCategory::Manual => self.texts.format("ui.save.slot", &[("n", &n)]),
            SlotCategory::Auto => self.texts.format("ui.save.auto_slot", &[("n", &n)]),
            SlotCategory::Quick => self.texts.get("ui.save.quick_slot"),
        }
    }

    fn rebuild(&mut self) {
        let focus = self.ui.focused_tag();
        let theme = self.ui.theme().clone();
        let w = &theme.widgets;
        let cats = self.categories().len();
        self.category = self.category.min(cats.saturating_sub(1));
        let total = self.current_slots().len();
        self.page = self.page.min(page_count(total, SLOTS_PER_PAGE) - 1);
        let range = page_range(total, self.page, SLOTS_PER_PAGE);
        let slots: Vec<SlotView> = self.current_slots()[range.clone()].to_vec();
        let mode = self.view.mode;

        let mut ui = Ui::new(theme.clone());
        let root = ui.add_root(Node::stack().fill().align(Align::Center, Align::Center));
        ui.add(root, Node::panel(PanelKind::Dim, Dir::Stack).fill());
        let panel = ui.add(
            root,
            Node::panel(PanelKind::Normal, Dir::Vertical)
                .fixed(1760.0, 980.0)
                .pad(40.0, 28.0)
                .gap(14.0)
                .align(Align::Start, Align::Stretch),
        );
        let heading = match mode {
            SaveLoadMode::Save => self.texts.get("ui.save.title"),
            SaveLoadMode::Load => self.texts.get("ui.load.title"),
        };
        ui.add(panel, title(&theme, &heading));

        // Tabs (only when there is a choice).
        if cats > 1 {
            let tabs = ui.add(panel, Node::hbox().gap(12.0));
            for (i, cat) in self.categories().iter().enumerate() {
                let key = match cat.category {
                    SlotCategory::Manual => "ui.save.tab.manual",
                    SlotCategory::Auto => "ui.save.tab.auto",
                    SlotCategory::Quick => "ui.save.tab.quick",
                };
                ui.add(
                    tabs,
                    Node::button(T_TAB + i as Tag, self.texts.get(key))
                        .small()
                        .w(Size::Fixed(260.0))
                        .selected(i == self.category),
                );
            }
        }

        // Grid.
        let grid = ui.add(panel, Node::vbox().gap(18.0));
        for (row_index, row_slots) in slots.chunks(COLUMNS).enumerate() {
            let row = ui.add(grid, Node::hbox().gap(24.0));
            for (col, slot) in row_slots.iter().enumerate() {
                let i = row_index * COLUMNS + col;
                self.build_card(&mut ui, row, i, slot, mode);
            }
        }

        // Footer.
        ui.add(panel, Node::spacer());
        let footer = ui.add(
            panel,
            Node::hbox().gap(24.0).align(Align::Start, Align::Center),
        );
        let pages = page_count(total, SLOTS_PER_PAGE);
        if pages > 1 {
            ui.add(
                footer,
                Node::button(T_PREV, self.texts.get("ui.save.prev_page"))
                    .small()
                    .w(Size::Fixed(200.0))
                    .enabled(self.page > 0),
            );
            let label = self.texts.format(
                "ui.save.page",
                &[
                    ("page", &(self.page + 1).to_string()),
                    ("pages", &pages.to_string()),
                ],
            );
            ui.add(
                footer,
                Node::label(label)
                    .w(Size::Fixed(260.0))
                    .text_align(TextAlign::Center),
            );
            ui.add(
                footer,
                Node::button(T_NEXT, self.texts.get("ui.save.next_page"))
                    .small()
                    .w(Size::Fixed(200.0))
                    .enabled(self.page + 1 < pages),
            );
        }
        ui.add(footer, Node::spacer());
        ui.add(
            footer,
            Node::button(T_BACK, self.texts.get("ui.menu.back")).w(Size::Fixed(300.0)),
        );
        let _ = w;
        ui.layout();
        ui.focus_first();
        if let Some(tag) = focus {
            ui.set_focus_tag(tag);
        }
        self.ui = ui;
        self.confirm.reset();
    }

    fn build_card(
        &self,
        ui: &mut Ui,
        row: crate::widgets::NodeId,
        i: usize,
        slot: &SlotView,
        mode: SaveLoadMode,
    ) {
        let theme = ui.theme().clone();
        let w = &theme.widgets;
        let cell = ui.add(row, Node::vbox().gap(8.0));
        let usable = match mode {
            SaveLoadMode::Save => true,
            SaveLoadMode::Load => slot.state != SlotState::Empty,
        };
        let card = ui.add(
            cell,
            Node::button(T_CARD + i as Tag, "")
                .fixed(CARD[0], CARD[1])
                .enabled(usable),
        );
        let body = ui.add(card, Node::hbox().fill().pad(14.0, 14.0).gap(14.0));
        ui.add(
            body,
            Node::image(slot.thumbnail)
                .fixed(THUMB[0], THUMB[1])
                .uv([0.0, 0.0, 1.0, 1.0]),
        );
        let text_w = CARD[0] - 28.0 - THUMB[0] - 14.0;
        let info = ui.add(
            body,
            Node::vbox()
                .w(Size::Fixed(text_w))
                .gap(4.0)
                .align(Align::Start, Align::Stretch),
        );
        ui.add(
            info,
            Node::label(self.slot_title(slot))
                .font(w.text_size - 4.0)
                .color(w.accent_color),
        );
        match slot.state {
            SlotState::Empty => {
                ui.add(
                    info,
                    Node::label(self.texts.get("ui.save.empty"))
                        .font(w.small_text_size)
                        .dimmed(),
                );
            }
            SlotState::Incompatible => {
                ui.add(
                    info,
                    Node::label(self.texts.get("ui.save.incompatible"))
                        .font(w.small_text_size)
                        .color([255, 130, 130, 255]),
                );
                if !slot.timestamp.is_empty() {
                    ui.add(
                        info,
                        Node::label(slot.timestamp.clone())
                            .font(w.small_text_size)
                            .dimmed(),
                    );
                }
            }
            SlotState::Used => {
                ui.add(
                    info,
                    Node::label(slot.timestamp.clone())
                        .font(w.small_text_size)
                        .dimmed(),
                );
                let per_line = (text_w / (w.small_text_size * 0.6)).floor() as usize;
                ui.add(
                    info,
                    Node::label(ellipsize(&slot.preview, per_line * 2))
                        .font(w.small_text_size)
                        .lines(2),
                );
            }
        }
        ui.add(
            cell,
            Node::button(T_DELETE + i as Tag, self.texts.get("ui.save.delete"))
                .small()
                .w(Size::Fixed(180.0))
                .enabled(slot.state != SlotState::Empty),
        );
    }

    fn open_confirm(&mut self, key: &str, event: UiEvent) {
        let message = self.texts.get(key);
        self.confirm
            .open(&mut self.ui, &self.texts.clone(), &message, event);
    }

    fn activated(&mut self, tag: Tag, out: &mut Vec<UiEvent>) {
        let total = self.current_slots().len();
        let range = page_range(total, self.page, SLOTS_PER_PAGE);
        let slot_at = |this: &Self, i: usize| this.current_slots().get(range.start + i).cloned();
        match tag {
            T_BACK => out.push(UiEvent::Back),
            T_PREV => self.set_page(self.page.saturating_sub(1)),
            T_NEXT => self.set_page(self.page + 1),
            t if (T_TAB..T_TAB + 8).contains(&t) => {
                let i = (t - T_TAB) as usize;
                if i != self.category && i < self.categories().len() {
                    self.category = i;
                    self.page = 0;
                    self.rebuild();
                    self.ui.set_focus_tag(t);
                }
            }
            t if (T_CARD..T_CARD + SLOTS_PER_PAGE as Tag).contains(&t) => {
                let Some(slot) = slot_at(self, (t - T_CARD) as usize) else {
                    return;
                };
                match (self.view.mode, slot.state) {
                    (SaveLoadMode::Load, SlotState::Used) => out.push(UiEvent::LoadSlot(slot.id)),
                    (SaveLoadMode::Load, SlotState::Incompatible) => {
                        out.push(UiEvent::Toast(self.texts.get("ui.toast.incompatible")));
                    }
                    (SaveLoadMode::Load, SlotState::Empty) => {}
                    (SaveLoadMode::Save, SlotState::Empty) => out.push(UiEvent::SaveSlot(slot.id)),
                    (SaveLoadMode::Save, _) => {
                        self.open_confirm("ui.save.confirm_overwrite", UiEvent::SaveSlot(slot.id));
                    }
                }
            }
            t if (T_DELETE..T_DELETE + SLOTS_PER_PAGE as Tag).contains(&t) => {
                if let Some(slot) = slot_at(self, (t - T_DELETE) as usize)
                    && slot.state != SlotState::Empty
                {
                    self.open_confirm("ui.save.confirm_delete", UiEvent::DeleteSlot(slot.id));
                }
            }
            _ => {}
        }
    }

    fn set_page(&mut self, page: usize) {
        let pages = page_count(self.current_slots().len(), SLOTS_PER_PAGE);
        let page = page.min(pages - 1);
        if page != self.page {
            self.page = page;
            self.rebuild();
        }
    }
}

impl Screen for SaveLoadScreen {
    fn widgets(&mut self) -> Option<&mut Ui> {
        Some(&mut self.ui)
    }

    fn handle(&mut self, input: UiInput) -> Vec<UiEvent> {
        let mut out = Vec::new();
        if !self.confirm.is_open() {
            match input {
                UiInput::PageUp => {
                    self.set_page(self.page.saturating_sub(1));
                    return out;
                }
                UiInput::PageDown => {
                    self.set_page(self.page + 1);
                    return out;
                }
                _ => {}
            }
        }
        let events = self.ui.handle(input);
        let mut pending = Vec::new();
        forward(events, &mut out, |ev, _| pending.push(ev));
        for ev in pending {
            let mut consumed = Vec::new();
            if self.confirm.route(&mut self.ui, &ev, &mut consumed) {
                out.extend(consumed);
                continue;
            }
            match ev {
                WidgetEvent::Cancelled => out.push(UiEvent::Back),
                WidgetEvent::Activated(tag) => self.activated(tag, &mut out),
                _ => {}
            }
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
        self.ui.set_theme(theme.clone());
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
    use crate::view::SlotId;

    fn slot(category: SlotCategory, index: u32, state: SlotState) -> SlotView {
        SlotView {
            id: SlotId { category, index },
            state,
            thumbnail: None,
            timestamp: "2026-01-02 10:00".into(),
            preview: "Hello there".into(),
            ..SlotView::empty(SlotId { category, index })
        }
    }

    fn view(mode: SaveLoadMode, manual: usize) -> SaveLoadView {
        let manual_slots = (0..manual)
            .map(|i| {
                let state = match i % 3 {
                    0 => SlotState::Used,
                    1 => SlotState::Empty,
                    _ => SlotState::Incompatible,
                };
                slot(SlotCategory::Manual, i as u32, state)
            })
            .collect();
        SaveLoadView {
            mode,
            categories: vec![
                CategoryView {
                    category: SlotCategory::Manual,
                    slots: manual_slots,
                },
                CategoryView {
                    category: SlotCategory::Auto,
                    slots: vec![slot(SlotCategory::Auto, 0, SlotState::Used)],
                },
            ],
        }
    }

    fn screen(mode: SaveLoadMode, manual: usize) -> SaveLoadScreen {
        SaveLoadScreen::new(&Theme::default(), &Texts::builtin("en"), view(mode, manual))
    }

    fn real(v: Vec<UiEvent>) -> Vec<UiEvent> {
        v.into_iter()
            .filter(|e| !matches!(e, UiEvent::Sound(_)))
            .collect()
    }

    fn click(s: &mut SaveLoadScreen, tag: Tag) -> Vec<UiEvent> {
        let r =
            s.ui.rect_of(tag)
                .unwrap_or_else(|| panic!("no widget {tag}"));
        let p = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
        s.handle(UiInput::PointerMove(p));
        s.handle(UiInput::PointerDown(p));
        real(s.handle(UiInput::PointerUp(p)))
    }

    #[test]
    fn pagination_math() {
        assert_eq!(page_count(0, 6), 1);
        assert_eq!(page_count(6, 6), 1);
        assert_eq!(page_count(7, 6), 2);
        assert_eq!(page_count(30, 6), 5);
        assert_eq!(page_range(30, 0, 6), 0..6);
        assert_eq!(page_range(30, 4, 6), 24..30);
        assert_eq!(page_range(30, 99, 6), 24..30, "clamped to the last page");
        assert_eq!(page_range(7, 1, 6), 6..7);
        assert_eq!(page_range(0, 0, 6), 0..0);
    }

    #[test]
    fn grid_is_three_columns_by_two_rows_and_cards_do_not_overlap() {
        let mut s = screen(SaveLoadMode::Load, 30);
        let rects: Vec<[f32; 4]> = (0..6).map(|i| s.ui.rect_of(T_CARD + i).unwrap()).collect();
        assert_eq!(rects[0][1], rects[2][1], "same row");
        assert!(rects[3][1] > rects[0][1] + rects[0][3], "second row below");
        assert!(rects[1][0] > rects[0][0] + rects[0][2] - 1.0);
        // Everything fits the canvas.
        for r in &rects {
            assert!(r[0] >= 0.0 && r[0] + r[2] <= 1920.0 && r[1] + r[3] <= 1080.0);
        }
        let back = s.ui.rect_of(T_BACK).unwrap();
        assert!(back[1] + back[3] <= 1080.0);
    }

    #[test]
    fn pages_navigate_with_buttons_and_page_keys() {
        let mut s = screen(SaveLoadMode::Load, 14);
        assert_eq!(s.page(), 0);
        click(&mut s, T_NEXT);
        assert_eq!(s.page(), 1);
        s.handle(UiInput::PageDown);
        assert_eq!(s.page(), 2);
        s.handle(UiInput::PageDown);
        assert_eq!(s.page(), 2, "stays on the last page");
        s.handle(UiInput::PageUp);
        assert_eq!(s.page(), 1);
        // Last page has two slots only (14 = 6 + 6 + 2).
        s.handle(UiInput::PageDown);
        assert!(s.ui.find(T_CARD + 2).is_none());
        assert!(s.ui.find(T_CARD + 1).is_some());
    }

    #[test]
    fn load_mode_loads_used_refuses_incompatible_and_ignores_empty() {
        let mut s = screen(SaveLoadMode::Load, 3);
        // Slot 0 used, 1 empty (disabled), 2 incompatible.
        assert_eq!(
            click(&mut s, T_CARD),
            vec![UiEvent::LoadSlot(SlotId {
                category: SlotCategory::Manual,
                index: 0
            })]
        );
        assert!(click(&mut s, T_CARD + 1).is_empty());
        let ev = click(&mut s, T_CARD + 2);
        assert!(matches!(ev.as_slice(), [UiEvent::Toast(t)] if t == "Incompatible save"));
    }

    #[test]
    fn save_mode_asks_before_overwriting_and_only_shows_manual() {
        let mut s = screen(SaveLoadMode::Save, 3);
        assert!(s.ui.find(T_TAB + 1).is_none(), "no tabs: only manual slots");
        // Empty slot: saves at once.
        assert_eq!(
            click(&mut s, T_CARD + 1),
            vec![UiEvent::SaveSlot(SlotId {
                category: SlotCategory::Manual,
                index: 1
            })]
        );
        // Used slot: modal first.
        assert!(click(&mut s, T_CARD).is_empty());
        assert!(s.ui.has_modal());
        // Escape answers "no".
        assert!(real(s.handle(UiInput::Cancel)).is_empty());
        assert!(!s.ui.has_modal());
        // Again, then confirm with the keyboard (focus starts on "No").
        click(&mut s, T_CARD);
        s.handle(UiInput::Left);
        let ev = real(s.handle(UiInput::Confirm));
        assert_eq!(
            ev,
            vec![UiEvent::SaveSlot(SlotId {
                category: SlotCategory::Manual,
                index: 0
            })]
        );
        assert!(!s.ui.has_modal());
    }

    #[test]
    fn delete_requires_confirmation_and_is_disabled_on_empty_slots() {
        let mut s = screen(SaveLoadMode::Load, 3);
        assert!(
            click(&mut s, T_DELETE + 1).is_empty(),
            "empty slot: nothing"
        );
        assert!(!s.ui.has_modal());
        assert!(click(&mut s, T_DELETE).is_empty());
        assert!(s.ui.has_modal());
        s.handle(UiInput::Left);
        let ev = real(s.handle(UiInput::Confirm));
        assert_eq!(
            ev,
            vec![UiEvent::DeleteSlot(SlotId {
                category: SlotCategory::Manual,
                index: 0
            })]
        );
    }

    #[test]
    fn tabs_switch_category_and_reset_the_page() {
        let mut s = screen(SaveLoadMode::Load, 14);
        click(&mut s, T_NEXT);
        assert_eq!(s.page(), 1);
        click(&mut s, T_TAB + 1);
        assert_eq!(s.category_index(), 1);
        assert_eq!(s.page(), 0);
        assert!(s.ui.find(T_CARD).is_some());
        assert!(s.ui.find(T_CARD + 1).is_none(), "autosave tab has one slot");
    }

    #[test]
    fn back_and_cancel_close_the_screen() {
        let mut s = screen(SaveLoadMode::Load, 3);
        assert_eq!(click(&mut s, T_BACK), vec![UiEvent::Back]);
        assert_eq!(real(s.handle(UiInput::Cancel)), vec![UiEvent::Back]);
    }

    #[test]
    fn modal_captures_focus_and_clicks_behind_it_do_nothing() {
        let mut s = screen(SaveLoadMode::Save, 3);
        click(&mut s, T_CARD);
        assert!(s.ui.has_modal());
        assert_eq!(
            s.ui.focus_order(),
            vec![super::super::T_YES, super::super::T_NO]
        );
        // Clicking the Back button behind the modal is ignored.
        assert!(click(&mut s, T_BACK).is_empty());
        assert!(s.ui.has_modal());
    }
}
