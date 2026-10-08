//! The `SaveSlots` widget of a data screen: which slots a page shows, and
//! the cards, tabs and page buttons that display them.

use super::screen::{Bound, Build, DataScreen, ItemCtx};
use crate::screens::{page_count, page_range};
use crate::texts::Texts;
use crate::view::{CategoryView, SlotCategory, SlotId, SlotState, SlotView};
use crate::widgets::{Align, Node, NodeId, Size, ellipsize};
use orbit_text::TextAlign;
use orbit_ui_data::{SlotField, SlotsMode, WidgetDef, WidgetKind};

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

pub(super) const CARD_H: f32 = 170.0;
pub(super) const THUMB: [f32; 2] = [224.0, 126.0];
const COLUMN_GAP: f32 = 24.0;
const ROW_GAP: f32 = 18.0;

pub(super) fn slot_title(texts: &Texts, id: SlotId) -> String {
    let n = (id.index + 1).to_string();
    match id.category {
        SlotCategory::Manual => texts.format("ui.save.slot", &[("n", &n)]),
        SlotCategory::Auto => texts.format("ui.save.auto_slot", &[("n", &n)]),
        SlotCategory::Quick => texts.get("ui.save.quick_slot"),
    }
}

/// A slot as the card template of a `SaveSlots` widget sees it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SlotCtx {
    pub view: SlotView,
    /// Room the text beside the thumbnail has in the stock card: the budget
    /// of the cut `Preview` text.
    pub text_w: f32,
}

impl SlotCtx {
    /// The value `Item` has inside a card: the state the `Cond::Slot*`
    /// conditions read.
    pub fn state_value(&self) -> orbit_core::Value {
        orbit_core::Value::Str(
            match self.view.state {
                SlotState::Empty => "empty",
                SlotState::Used => "used",
                SlotState::Incompatible => "locked",
            }
            .to_owned(),
        )
    }

    /// The text of a `BindPath::SlotField` (`Thumbnail` has none).
    pub fn field(&self, texts: &Texts, small_text_size: f32, field: SlotField) -> String {
        let v = &self.view;
        match field {
            SlotField::Title => slot_title(texts, v.id),
            SlotField::Number => (v.id.index + 1).to_string(),
            SlotField::Timestamp => v.timestamp.clone(),
            SlotField::Date => v.date.clone(),
            SlotField::PlayTime => v.play_time.clone(),
            SlotField::Chapter => v.chapter.clone(),
            SlotField::Preview => {
                if v.state == SlotState::Used {
                    let per_line = (self.text_w / (small_text_size * 0.6)).floor() as usize;
                    ellipsize(&v.preview, per_line * 2)
                } else {
                    String::new()
                }
            }
            SlotField::Status => match v.state {
                SlotState::Empty => texts.get("ui.save.empty"),
                SlotState::Incompatible => texts.get("ui.save.incompatible"),
                SlotState::Used => String::new(),
            },
            SlotField::Thumbnail => String::new(),
        }
    }
}

fn styled(node: Node, style: Option<&String>) -> Node {
    match style {
        Some(name) => node.style(name.clone()),
        None => node,
    }
}

impl DataScreen {
    /// Adds the tabs, the grid and the pager of the `SaveSlots` widget `def`
    /// under `id`, and records what each tagged widget does. The children of
    /// `def`, when there are any, are the template of the inside of a card
    /// (built once per slot); without them the stock card is drawn.
    pub(super) fn build_slots(
        &self,
        def: &WidgetDef,
        id: NodeId,
        path: &mut Vec<usize>,
        st: &mut Build,
    ) {
        let WidgetKind::SaveSlots {
            mode,
            columns,
            rows,
            look,
        } = &def.kind
        else {
            return;
        };
        let columns = (*columns).max(1);
        let per_page = columns * (*rows).max(1);
        let page = slots_page(
            &self.ctx.saves,
            *mode,
            self.slots_category,
            self.slots_page,
            per_page,
        );
        let texts = &self.texts;
        let column_gap = look.column_gap.unwrap_or(COLUMN_GAP);
        let row_gap = look.row_gap.unwrap_or(ROW_GAP);
        let card_h = look.card_height.unwrap_or(CARD_H);
        let card_w =
            ((1680.0 - column_gap * (columns as f32 - 1.0)) / columns as f32).clamp(160.0, 544.0);
        let text_w = (card_w - 28.0 - THUMB[0] - 14.0).max(40.0);

        if page.categories.len() > 1 {
            let tabs = st.ui.add(id, Node::hbox().gap(12.0));
            for (i, cat) in page.categories.iter().enumerate() {
                let key = match cat {
                    SlotCategory::Manual => "ui.save.tab.manual",
                    SlotCategory::Auto => "ui.save.tab.auto",
                    SlotCategory::Quick => "ui.save.tab.quick",
                };
                let tag = Self::new_tag(st);
                st.binds.insert(tag, Bound::Slot(SlotAction::Tab(i)));
                st.ui.add(
                    tabs,
                    styled(
                        Node::button(tag, texts.get(key))
                            .small()
                            .w(Size::Fixed(260.0))
                            .selected(i == page.category),
                        look.tab_style.as_ref(),
                    ),
                );
            }
        }

        let grid = st.ui.add(id, Node::vbox().gap(row_gap));
        for row_slots in page.slots.chunks(columns) {
            let row = st.ui.add(grid, Node::hbox().gap(column_gap));
            for slot in row_slots {
                let cell = st.ui.add(row, Node::vbox().gap(8.0));
                let usable = match mode {
                    SlotsMode::Save => true,
                    SlotsMode::Load => slot.state != SlotState::Empty,
                };
                let tag = Self::new_tag(st);
                st.binds.insert(
                    tag,
                    Bound::Slot(SlotAction::Card {
                        id: slot.id,
                        state: slot.state,
                    }),
                );
                let style = match slot.state {
                    SlotState::Empty => look.empty_style.as_ref(),
                    SlotState::Incompatible => look.locked_style.as_ref(),
                    SlotState::Used => None,
                }
                .or(look.card_style.as_ref());
                let card = st.ui.add(
                    cell,
                    styled(
                        Node::button(tag, "").fixed(card_w, card_h).enabled(usable),
                        style,
                    ),
                );
                if def.children.is_empty() {
                    self.stock_card_body(st, card, slot, text_w);
                } else {
                    let ctx = SlotCtx {
                        view: slot.clone(),
                        text_w,
                    };
                    st.items.push(ItemCtx {
                        value: ctx.state_value(),
                        index: 0,
                        key: None,
                        slot: Some(ctx),
                    });
                    for (i, child) in def.children.iter().enumerate() {
                        path.push(i);
                        self.build(child, Some(card), path, st);
                        path.pop();
                    }
                    st.items.pop();
                }
                let tag = Self::new_tag(st);
                st.binds.insert(tag, Bound::Slot(SlotAction::Delete(slot.id)));
                st.ui.add(
                    cell,
                    styled(
                        Node::button(tag, texts.get("ui.save.delete"))
                            .small()
                            .w(Size::Fixed(180.0))
                            .enabled(slot.state != SlotState::Empty),
                        look.delete_style.as_ref(),
                    ),
                );
            }
        }

        if page.pages > 1 {
            let footer = st
                .ui
                .add(id, Node::hbox().gap(24.0).align(Align::Start, Align::Center));
            let prev = Self::new_tag(st);
            st.binds
                .insert(prev, Bound::Slot(SlotAction::Page(page.page.saturating_sub(1))));
            st.ui.add(
                footer,
                styled(
                    Node::button(prev, texts.get("ui.save.prev_page"))
                        .small()
                        .w(Size::Fixed(200.0))
                        .enabled(page.page > 0),
                    look.pager_style.as_ref(),
                ),
            );
            let label = texts.format(
                "ui.save.page",
                &[
                    ("page", &(page.page + 1).to_string()),
                    ("pages", &page.pages.to_string()),
                ],
            );
            st.ui.add(
                footer,
                Node::label(label)
                    .w(Size::Fixed(260.0))
                    .text_align(TextAlign::Center),
            );
            let next = Self::new_tag(st);
            st.binds
                .insert(next, Bound::Slot(SlotAction::Page(page.page + 1)));
            st.ui.add(
                footer,
                styled(
                    Node::button(next, texts.get("ui.save.next_page"))
                        .small()
                        .w(Size::Fixed(200.0))
                        .enabled(page.page + 1 < page.pages),
                    look.pager_style.as_ref(),
                ),
            );
        }

    }

    /// The inside of the stock card: thumbnail, title, and the empty /
    /// incompatible text or the date and the last line.
    fn stock_card_body(&self, st: &mut Build, card: NodeId, slot: &SlotView, text_w: f32) {
        let theme = &self.theme;
        let w = &theme.widgets;
        let texts = &self.texts;
        let ui = &mut st.ui;
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
    }
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
            ..SlotView::empty(SlotId { category, index })
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
