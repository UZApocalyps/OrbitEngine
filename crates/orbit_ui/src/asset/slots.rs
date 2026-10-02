//! The `SaveSlots` widget of a data screen: which slots a page shows, and
//! the cards, tabs and page buttons that display them.

use super::widget::SlotsMode;
use crate::screens::{page_count, page_range};
use crate::texts::Texts;
use crate::view::{CategoryView, SlotCategory, SlotId, SlotState, SlotView};
use crate::widgets::{Align, Node, NodeId, Size, Tag, Ui, ellipsize};
use orbit_text::TextAlign;

/// What one page of a `SaveSlots` widget shows.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotsPage {
    /// Tabs: the categories offered (only `Manual` when saving).
    pub categories: Vec<SlotCategory>,
    /// Index of the tab shown (clamped).
    pub category: usize,
    /// Page shown (clamped).
    pub page: usize,
    pub pages: usize,
    pub slots: Vec<SlotView>,
}

/// The slots of `category` / `page` out of the game's `saves`, with both
/// indices clamped to what exists.
pub fn slots_page(
    saves: &[CategoryView],
    mode: SlotsMode,
    category: usize,
    page: usize,
    per_page: usize,
) -> SlotsPage {
    let shown: Vec<&CategoryView> = saves
        .iter()
        .filter(|c| mode == SlotsMode::Load || c.category == SlotCategory::Manual)
        .collect();
    let category = category.min(shown.len().saturating_sub(1));
    let all: &[SlotView] = shown.get(category).map_or(&[], |c| c.slots.as_slice());
    let pages = page_count(all.len(), per_page);
    let page = page.min(pages - 1);
    let range = page_range(all.len(), page, per_page);
    SlotsPage {
        categories: shown.iter().map(|c| c.category).collect(),
        category,
        page,
        pages,
        slots: all[range].to_vec(),
    }
}

/// What a tagged widget of the slots does when activated.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum SlotAction {
    Card { id: SlotId, state: SlotState },
    Delete(SlotId),
    Tab(usize),
    Page(usize),
}

const CARD_H: f32 = 170.0;
const THUMB: [f32; 2] = [224.0, 126.0];

fn slot_title(texts: &Texts, id: SlotId) -> String {
    let n = (id.index + 1).to_string();
    match id.category {
        SlotCategory::Manual => texts.format("ui.save.slot", &[("n", &n)]),
        SlotCategory::Auto => texts.format("ui.save.auto_slot", &[("n", &n)]),
        SlotCategory::Quick => texts.get("ui.save.quick_slot"),
    }
}

/// Adds the tabs, the grid and the pager under `parent`; returns what each
/// tagged widget does. `new_tag` hands out unique tags.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_slots(
    ui: &mut Ui,
    parent: NodeId,
    texts: &Texts,
    mode: SlotsMode,
    columns: usize,
    page: &SlotsPage,
    new_tag: &mut dyn FnMut() -> Tag,
) -> Vec<(Tag, SlotAction)> {
    let theme = ui.theme().clone();
    let w = &theme.widgets;
    let mut out = Vec::new();
    let columns = columns.max(1);
    let card_w = ((1680.0 - 24.0 * (columns as f32 - 1.0)) / columns as f32).clamp(160.0, 544.0);
    let text_w = (card_w - 28.0 - THUMB[0] - 14.0).max(40.0);

    if page.categories.len() > 1 {
        let tabs = ui.add(parent, Node::hbox().gap(12.0));
        for (i, cat) in page.categories.iter().enumerate() {
            let key = match cat {
                SlotCategory::Manual => "ui.save.tab.manual",
                SlotCategory::Auto => "ui.save.tab.auto",
                SlotCategory::Quick => "ui.save.tab.quick",
            };
            let tag = new_tag();
            out.push((tag, SlotAction::Tab(i)));
            ui.add(
                tabs,
                Node::button(tag, texts.get(key))
                    .small()
                    .w(Size::Fixed(260.0))
                    .selected(i == page.category),
            );
        }
    }

    let grid = ui.add(parent, Node::vbox().gap(18.0));
    for row_slots in page.slots.chunks(columns) {
        let row = ui.add(grid, Node::hbox().gap(24.0));
        for slot in row_slots {
            let cell = ui.add(row, Node::vbox().gap(8.0));
            let usable = match mode {
                SlotsMode::Save => true,
                SlotsMode::Load => slot.state != SlotState::Empty,
            };
            let tag = new_tag();
            out.push((
                tag,
                SlotAction::Card {
                    id: slot.id,
                    state: slot.state,
                },
            ));
            let card = ui.add(
                cell,
                Node::button(tag, "").fixed(card_w, CARD_H).enabled(usable),
            );
            let body = ui.add(card, Node::hbox().fill().pad(14.0, 14.0).gap(14.0));
            ui.add(
                body,
                Node::image(slot.thumbnail)
                    .fixed(THUMB[0], THUMB[1])
                    .uv([0.0, 0.0, 1.0, 1.0]),
            );
            let info = ui.add(
                body,
                Node::vbox()
                    .w(Size::Fixed(text_w))
                    .gap(4.0)
                    .align(Align::Start, Align::Stretch),
            );
            ui.add(
                info,
                Node::label(slot_title(texts, slot.id))
                    .font(w.text_size - 4.0)
                    .color(w.accent_color),
            );
            match slot.state {
                SlotState::Empty => {
                    ui.add(
                        info,
                        Node::label(texts.get("ui.save.empty"))
                            .font(w.small_text_size)
                            .dimmed(),
                    );
                }
                SlotState::Incompatible => {
                    ui.add(
                        info,
                        Node::label(texts.get("ui.save.incompatible"))
                            .font(w.small_text_size)
                            .color([255, 130, 130, 255]),
                    );
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
            let tag = new_tag();
            out.push((tag, SlotAction::Delete(slot.id)));
            ui.add(
                cell,
                Node::button(tag, texts.get("ui.save.delete"))
                    .small()
                    .w(Size::Fixed(180.0))
                    .enabled(slot.state != SlotState::Empty),
            );
        }
    }

    if page.pages > 1 {
        let footer = ui.add(parent, Node::hbox().gap(24.0).align(Align::Start, Align::Center));
        let prev = new_tag();
        out.push((prev, SlotAction::Page(page.page.saturating_sub(1))));
        ui.add(
            footer,
            Node::button(prev, texts.get("ui.save.prev_page"))
                .small()
                .w(Size::Fixed(200.0))
                .enabled(page.page > 0),
        );
        let label = texts.format(
            "ui.save.page",
            &[
                ("page", &(page.page + 1).to_string()),
                ("pages", &page.pages.to_string()),
            ],
        );
        ui.add(
            footer,
            Node::label(label)
                .w(Size::Fixed(260.0))
                .text_align(TextAlign::Center),
        );
        let next = new_tag();
        out.push((next, SlotAction::Page(page.page + 1)));
        ui.add(
            footer,
            Node::button(next, texts.get("ui.save.next_page"))
                .small()
                .w(Size::Fixed(200.0))
                .enabled(page.page + 1 < page.pages),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(category: SlotCategory, index: u32, state: SlotState) -> SlotView {
        SlotView {
            id: SlotId { category, index },
            state,
            thumbnail: None,
            timestamp: "2026-01-02 10:00".into(),
            preview: "Hello".into(),
        }
    }

    fn saves(manual: usize) -> Vec<CategoryView> {
        vec![
            CategoryView {
                category: SlotCategory::Manual,
                slots: (0..manual as u32)
                    .map(|i| slot(SlotCategory::Manual, i, SlotState::Used))
                    .collect(),
            },
            CategoryView {
                category: SlotCategory::Auto,
                slots: vec![slot(SlotCategory::Auto, 0, SlotState::Used)],
            },
        ]
    }

    #[test]
    fn save_mode_only_offers_manual_slots() {
        let p = slots_page(&saves(10), SlotsMode::Save, 1, 0, 6);
        assert_eq!(p.categories, vec![SlotCategory::Manual]);
        assert_eq!(p.category, 0, "the tab is clamped");
        assert_eq!(p.slots.len(), 6);
        assert_eq!(p.pages, 2);
    }

    #[test]
    fn load_mode_has_a_tab_per_category_and_clamps_the_page() {
        let p = slots_page(&saves(10), SlotsMode::Load, 0, 9, 6);
        assert_eq!(p.categories.len(), 2);
        assert_eq!((p.page, p.slots.len()), (1, 4));
        let auto = slots_page(&saves(10), SlotsMode::Load, 1, 5, 6);
        assert_eq!((auto.page, auto.pages, auto.slots.len()), (0, 1, 1));
        let none = slots_page(&[], SlotsMode::Load, 0, 0, 6);
        assert_eq!((none.pages, none.slots.len()), (1, 0));
    }
}
