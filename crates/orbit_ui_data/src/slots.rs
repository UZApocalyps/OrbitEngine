//! The stock card of a `SaveSlots` widget as a widget tree: what the editor
//! inserts when the author starts customizing the cards.

use crate::action::{BindPath, SlotField};
use crate::layout::{Align, Dir, Size};
use crate::theme::{Style, WidgetTheme};
use crate::widget::{Cond, LayoutDef, TextRef, WidgetDef, WidgetKind};

/// The children of a `SaveSlots` widget that draw exactly the stock card:
/// the thumbnail on the left; the slot title, then either the "empty" /
/// "incompatible" text or the date and the last line, on the right. The
/// sizes and colors are those of `widgets` (the theme in use), so the
/// layout is identical to the card drawn without a template.
pub fn default_slot_card(widgets: &WidgetTheme) -> Vec<WidgetDef> {
    let label = |text: TextRef, size: f32, id: &str| {
        let mut w = WidgetDef::new(WidgetKind::Label {
            text,
            size: Some(size),
            center: false,
            dim: false,
        });
        w.id = id.to_owned();
        w
    };
    let mut title = label(
        TextRef::Bind(BindPath::SlotField(SlotField::Title)),
        widgets.text_size - 4.0,
        "slot_title",
    );
    title.overrides = Style {
        text_colors: crate::theme::StateColors {
            normal: Some(widgets.accent_color),
            ..Default::default()
        },
        ..Style::default()
    };

    let mut empty = label(TextRef::Key("ui.save.empty".into()), widgets.small_text_size, "slot_empty");
    empty.visible = Cond::SlotEmpty;
    if let WidgetKind::Label { dim, .. } = &mut empty.kind {
        *dim = true;
    }

    let mut locked = label(
        TextRef::Key("ui.save.incompatible".into()),
        widgets.small_text_size,
        "slot_locked",
    );
    locked.visible = Cond::SlotLocked;
    locked.overrides = Style {
        text_colors: crate::theme::StateColors {
            normal: Some([255, 130, 130, 255]),
            ..Default::default()
        },
        ..Style::default()
    };

    let mut date = label(
        TextRef::Bind(BindPath::SlotField(SlotField::Timestamp)),
        widgets.small_text_size,
        "slot_date",
    );
    date.visible = Cond::SlotUsed;
    if let WidgetKind::Label { dim, .. } = &mut date.kind {
        *dim = true;
    }

    let mut preview = label(
        TextRef::Bind(BindPath::SlotField(SlotField::Preview)),
        widgets.small_text_size,
        "slot_preview",
    );
    preview.visible = Cond::SlotUsed;

    let mut info = WidgetDef::new(WidgetKind::Stack { dir: Dir::Vertical });
    info.id = "slot_info".into();
    info.layout = LayoutDef {
        width: Size::Fill(1.0),
        gap: 4.0,
        main: Align::Start,
        cross: Align::Stretch,
        ..LayoutDef::default()
    };
    info.children = vec![title, empty, locked, date, preview];

    let mut thumb = WidgetDef::new(WidgetKind::Image {
        asset: None,
        bind: Some(BindPath::SlotField(SlotField::Thumbnail)),
        action: Vec::new(),
    });
    thumb.id = "slot_thumbnail".into();
    thumb.layout = LayoutDef {
        width: Size::Fixed(224.0),
        height: Size::Fixed(126.0),
        ..LayoutDef::default()
    };

    let mut body = WidgetDef::new(WidgetKind::Stack {
        dir: Dir::Horizontal,
    });
    body.id = "slot_body".into();
    body.layout = LayoutDef {
        width: Size::Fill(1.0),
        height: Size::Fill(1.0),
        pad: [14.0, 14.0],
        gap: 14.0,
        ..LayoutDef::default()
    };
    body.children = vec![thumb, info];
    vec![body]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_card_is_one_body_with_a_thumbnail_and_five_texts() {
        let card = default_slot_card(&WidgetTheme::default());
        assert_eq!(card.len(), 1);
        assert_eq!(card[0].children.len(), 2);
        assert_eq!(card[0].children[1].children.len(), 5);
        let ron = ron::ser::to_string(&card).unwrap();
        let back: Vec<WidgetDef> = ron::from_str(&ron).unwrap();
        assert_eq!(back, card);
    }
}
