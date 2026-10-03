use super::*;
use crate::input::UiInput;
use crate::theme::Theme;

fn ui() -> Ui {
    Ui::new(Theme::default())
}

fn centre(r: [f32; 4]) -> [f32; 2] {
    [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0]
}

fn sounds_stripped(v: Vec<WidgetEvent>) -> Vec<WidgetEvent> {
    v.into_iter()
        .filter(|e| !matches!(e, WidgetEvent::Sound(_)))
        .collect()
}

// ------------------------------------------------------------- layout

#[test]
fn vertical_box_stacks_children_with_padding_and_spacing() {
    let mut ui = ui();
    let root = ui.add_root(Node::vbox().pad(10.0, 20.0).gap(5.0).fixed(300.0, 400.0));
    ui.add(root, Node::vbox().fixed(100.0, 50.0).tag(1));
    ui.add(root, Node::vbox().fixed(80.0, 60.0).tag(2));
    ui.layout();
    assert_eq!(ui.rect_of(1).unwrap(), [10.0, 20.0, 100.0, 50.0]);
    assert_eq!(ui.rect_of(2).unwrap(), [10.0, 75.0, 80.0, 60.0]);
}

#[test]
fn horizontal_fill_shares_the_remaining_room_by_weight() {
    let mut ui = ui();
    let root = ui.add_root(Node::hbox().gap(10.0).fixed(500.0, 100.0));
    ui.add(root, Node::vbox().fixed(100.0, 10.0).tag(1));
    ui.add(root, Node::vbox().size(Size::Fill(1.0), Size::Fit).tag(2));
    ui.add(root, Node::vbox().size(Size::Fill(3.0), Size::Fit).tag(3));
    ui.layout();
    // 500 - 100 - 2 gaps (20) = 380 shared 1:3.
    assert_eq!(ui.rect_of(1).unwrap()[2], 100.0);
    assert!((ui.rect_of(2).unwrap()[2] - 95.0).abs() < 1e-3);
    assert!((ui.rect_of(3).unwrap()[2] - 285.0).abs() < 1e-3);
    assert!((ui.rect_of(3).unwrap()[0] - (100.0 + 10.0 + 95.0 + 10.0)).abs() < 1e-3);
}

#[test]
fn cross_axis_alignment_and_stretch() {
    let mut ui = ui();
    let root = ui.add_root(
        Node::vbox()
            .fixed(400.0, 300.0)
            .align(Align::Start, Align::Center),
    );
    ui.add(root, Node::vbox().fixed(100.0, 40.0).tag(1));
    ui.layout();
    assert_eq!(ui.rect_of(1).unwrap()[0], 150.0);
    ui.node_mut(root).cross = Align::End;
    ui.layout();
    assert_eq!(ui.rect_of(1).unwrap()[0], 300.0);
    ui.node_mut(root).cross = Align::Stretch;
    let child = ui.add(root, Node::vbox().h(Size::Fixed(10.0)).tag(2));
    let _ = child;
    ui.layout();
    assert_eq!(ui.rect_of(2).unwrap()[2], 400.0, "Fit child stretched");
    assert_eq!(
        ui.rect_of(1).unwrap()[2],
        100.0,
        "Fixed child keeps its width"
    );
}

#[test]
fn main_axis_centering_without_fill_children() {
    let mut ui = ui();
    let root = ui.add_root(
        Node::vbox()
            .fixed(200.0, 300.0)
            .align(Align::Center, Align::Start),
    );
    ui.add(root, Node::vbox().fixed(50.0, 100.0).tag(1));
    ui.layout();
    assert_eq!(ui.rect_of(1).unwrap()[1], 100.0);
}

#[test]
fn anchors_place_roots_on_the_virtual_canvas() {
    let mut ui = ui();
    ui.add_root(
        Node::vbox()
            .fixed(200.0, 100.0)
            .anchor(Anchor::center())
            .tag(1),
    );
    ui.add_root(
        Node::vbox()
            .fixed(200.0, 100.0)
            .anchor(Anchor::bottom_center(-20.0))
            .tag(2),
    );
    ui.add_root(
        Node::vbox()
            .fixed(200.0, 100.0)
            .anchor(Anchor::top_left(30.0, 40.0))
            .tag(3),
    );
    ui.add_root(Node::vbox().fill().tag(4));
    ui.layout();
    assert_eq!(ui.rect_of(1).unwrap(), [860.0, 490.0, 200.0, 100.0]);
    assert_eq!(ui.rect_of(2).unwrap(), [860.0, 960.0, 200.0, 100.0]);
    assert_eq!(ui.rect_of(3).unwrap(), [30.0, 40.0, 200.0, 100.0]);
    assert_eq!(ui.rect_of(4).unwrap(), [0.0, 0.0, 1920.0, 1080.0]);
}

#[test]
fn fit_container_wraps_its_children() {
    let mut ui = ui();
    let root = ui.add_root(Node::hbox().pad(7.0, 3.0).gap(4.0).tag(1));
    ui.add(root, Node::vbox().fixed(40.0, 10.0));
    ui.add(root, Node::vbox().fixed(60.0, 30.0));
    ui.layout();
    let r = ui.rect_of(1).unwrap();
    assert_eq!((r[2], r[3]), (40.0 + 60.0 + 4.0 + 14.0, 30.0 + 6.0));
}

#[test]
fn scroll_list_shows_only_whole_items_that_fit() {
    let mut ui = ui();
    let list = ui.add_root(Node::scroll_list(1).fixed(400.0, 250.0).gap(10.0));
    for i in 0..10 {
        ui.add(
            list,
            Node::vbox()
                .w(Size::Fill(1.0))
                .h(Size::Fixed(100.0))
                .tag(100 + i),
        );
    }
    ui.layout();
    // 100 + 10 + 100 = 210 fits, a third would need 320.
    assert!(ui.node(ui.find(101).unwrap()).is_visible());
    assert!(!ui.node(ui.find(102).unwrap()).is_visible());
    assert!(matches!(
        ui.node(list).kind,
        Kind::ScrollList { shown: 2, .. }
    ));
    let before = ui.rect_of(101).unwrap();
    ui.handle(UiInput::PointerMove([10.0, 10.0]));
    ui.handle(UiInput::Scroll(-1.0));
    assert_eq!(ui.scroll_first(1), Some(1));
    assert!(!ui.node(ui.find(100).unwrap()).is_visible());
    assert_eq!(ui.rect_of(101).unwrap()[1], before[1] - 110.0);
}

#[test]
fn dragging_a_scroll_list_scrolls_it_without_clicking_rows() {
    let mut ui = ui();
    let list = ui.add_root(Node::scroll_list(1).fixed(400.0, 250.0).gap(10.0));
    for i in 0..10 {
        ui.add(list, Node::button(100 + i, "x").w(Size::Fill(1.0)).h(Size::Fixed(100.0)));
    }
    ui.layout();
    let y = ui.rect_of(100).unwrap()[1] + 20.0;
    ui.handle(UiInput::PointerMove([20.0, y]));
    ui.handle(UiInput::PointerDown([20.0, y]));
    // Drag up by two row pitches: two rows scroll away.
    ui.handle(UiInput::PointerMove([20.0, y - 220.0]));
    assert_eq!(ui.scroll_first(1), Some(2));
    let events = ui.handle(UiInput::PointerUp([20.0, y - 220.0]));
    assert!(!events.iter().any(|e| matches!(e, WidgetEvent::Activated(_))));
    // A plain click (no movement) still activates a row.
    let y = ui.rect_of(102).unwrap()[1] + 20.0;
    ui.handle(UiInput::PointerMove([20.0, y]));
    ui.handle(UiInput::PointerDown([20.0, y]));
    let events = ui.handle(UiInput::PointerUp([20.0, y]));
    assert!(events.iter().any(|e| matches!(e, WidgetEvent::Activated(_))));
    assert_eq!(ui.scroll_first(1), Some(2));
}

// ------------------------------------------------------- hit testing

#[test]
fn hit_test_returns_the_topmost_enabled_interactive_node() {
    let mut ui = ui();
    let root = ui.add_root(Node::vbox().fixed(400.0, 300.0).gap(20.0));
    ui.add(root, Node::button(1, "A").fixed(200.0, 60.0));
    ui.add(root, Node::button(2, "B").fixed(200.0, 60.0).enabled(false));
    ui.layout();
    let a = ui.rect_of(1).unwrap();
    assert_eq!(ui.hit_test(centre(a)), ui.find(1));
    assert_eq!(ui.hit_test([a[0] + a[2] + 1.0, a[1] + 5.0]), None);
    // The gap between two buttons belongs to none.
    assert_eq!(ui.hit_test([a[0] + 5.0, a[1] + a[3] + 5.0]), None);
    // Disabled buttons are not hit.
    let b = ui.rect_of(2).unwrap();
    assert_eq!(ui.hit_test(centre(b)), None);
}

// ------------------------------------------------------------- focus

fn column(n: usize) -> Ui {
    let mut ui = ui();
    let root = ui.add_root(
        Node::vbox()
            .gap(10.0)
            .anchor(Anchor::top_left(100.0, 100.0)),
    );
    for i in 0..n {
        ui.add(
            root,
            Node::button(1 + i as Tag, format!("B{i}")).w(Size::Fixed(300.0)),
        );
    }
    ui.layout();
    ui
}

#[test]
fn arrows_move_focus_and_wrap_around() {
    let mut ui = column(3);
    assert_eq!(ui.focused_tag(), None);
    ui.handle(UiInput::Down); // first input just focuses the first widget
    assert_eq!(ui.focused_tag(), Some(1));
    ui.handle(UiInput::Down);
    ui.handle(UiInput::Down);
    assert_eq!(ui.focused_tag(), Some(3));
    ui.handle(UiInput::Down);
    assert_eq!(ui.focused_tag(), Some(1), "wraps to the top");
    ui.handle(UiInput::Up);
    assert_eq!(ui.focused_tag(), Some(3), "wraps to the bottom");
}

#[test]
fn tab_cycles_in_tree_order_and_shift_tab_goes_back() {
    let mut ui = column(3);
    ui.handle(UiInput::Next);
    assert_eq!(ui.focused_tag(), Some(1));
    ui.handle(UiInput::Next);
    ui.handle(UiInput::Next);
    ui.handle(UiInput::Next);
    assert_eq!(ui.focused_tag(), Some(1));
    ui.handle(UiInput::Prev);
    assert_eq!(ui.focused_tag(), Some(3));
}

#[test]
fn spatial_navigation_in_a_grid() {
    let mut ui = ui();
    let root = ui.add_root(Node::vbox().gap(10.0));
    for r in 0..2 {
        let row = ui.add(root, Node::hbox().gap(10.0));
        for c in 0..3 {
            ui.add(row, Node::button(1 + r * 3 + c, "x").fixed(100.0, 50.0));
        }
    }
    ui.layout();
    ui.set_focus_tag(2);
    ui.handle(UiInput::Right);
    assert_eq!(ui.focused_tag(), Some(3));
    ui.handle(UiInput::Down);
    assert_eq!(ui.focused_tag(), Some(6));
    ui.handle(UiInput::Right);
    assert_eq!(ui.focused_tag(), Some(4), "wraps to the start of the row");
    ui.handle(UiInput::Up);
    assert_eq!(ui.focused_tag(), Some(1));
    ui.handle(UiInput::Up);
    assert_eq!(
        ui.focused_tag(),
        Some(4),
        "wraps vertically in the same column"
    );
}

#[test]
fn disabled_widgets_are_skipped() {
    let mut ui = ui();
    let root = ui.add_root(Node::vbox().gap(10.0));
    ui.add(root, Node::button(1, "a"));
    ui.add(root, Node::button(2, "b").enabled(false));
    ui.add(root, Node::button(3, "c"));
    ui.layout();
    assert_eq!(ui.focus_order(), vec![1, 3]);
    ui.handle(UiInput::Down);
    ui.handle(UiInput::Down);
    assert_eq!(ui.focused_tag(), Some(3));
}

#[test]
fn a_modal_captures_focus_and_input() {
    let mut ui = column(2);
    ui.set_focus_tag(2);
    ui.open_modal(|ui| {
        let root = ui.add_root(Node::vbox().anchor(Anchor::center()).gap(10.0));
        ui.add(root, Node::button(50, "yes"));
        ui.add(root, Node::button(51, "no"));
        root
    });
    assert!(ui.has_modal());
    assert_eq!(ui.focus_order(), vec![50, 51]);
    assert_eq!(ui.focused_tag(), Some(50));
    // Keyboard never leaves the modal.
    for _ in 0..5 {
        ui.handle(UiInput::Down);
        assert!([50, 51].contains(&ui.focused_tag().unwrap()));
    }
    // A click on a widget behind the modal does nothing.
    let behind = ui.rect_of(1).unwrap();
    ui.handle(UiInput::PointerDown(centre(behind)));
    let ev = ui.handle(UiInput::PointerUp(centre(behind)));
    assert!(sounds_stripped(ev).is_empty());
    // Closing restores the previous focus and the base widgets.
    ui.close_modal();
    assert!(!ui.has_modal());
    assert_eq!(ui.focused_tag(), Some(2));
    assert_eq!(ui.focus_order(), vec![1, 2]);
    assert!(ui.find(50).is_none());
}

#[test]
fn mouse_hover_moves_focus_and_a_click_activates_on_release() {
    let mut ui = column(2);
    let b = ui.rect_of(2).unwrap();
    let p = centre(b);
    let ev = ui.handle(UiInput::PointerMove(p));
    assert!(ev.contains(&WidgetEvent::Sound(crate::UiSound::Hover)));
    assert_eq!(ui.focused_tag(), Some(2));
    assert!(sounds_stripped(ui.handle(UiInput::PointerDown(p))).is_empty());
    assert_eq!(
        sounds_stripped(ui.handle(UiInput::PointerUp(p))),
        vec![WidgetEvent::Activated(2)]
    );
    // Press, drag off, release: no activation.
    ui.handle(UiInput::PointerDown(p));
    let off = [b[0] - 50.0, b[1]];
    ui.handle(UiInput::PointerMove(off));
    assert!(sounds_stripped(ui.handle(UiInput::PointerUp(off))).is_empty());
}

#[test]
fn enter_activates_the_focused_button_and_escape_cancels() {
    let mut ui = column(2);
    ui.handle(UiInput::Down);
    assert_eq!(
        sounds_stripped(ui.handle(UiInput::Confirm)),
        vec![WidgetEvent::Activated(1)]
    );
    assert_eq!(
        sounds_stripped(ui.handle(UiInput::Cancel)),
        vec![WidgetEvent::Cancelled]
    );
}

// ----------------------------------------------------- slider & co

#[test]
fn slider_keyboard_steps_and_clamps() {
    let mut ui = ui();
    ui.add_root(Node::slider(1, 0.5).fixed(400.0, 56.0));
    ui.layout();
    ui.handle(UiInput::Right);
    assert_eq!(ui.focused_tag(), Some(1));
    let ev = sounds_stripped(ui.handle(UiInput::Right));
    assert_eq!(ev, vec![WidgetEvent::SliderChanged(1, 0.55)]);
    for _ in 0..30 {
        ui.handle(UiInput::Right);
    }
    assert_eq!(ui.slider_value(1), Some(1.0));
    assert!(
        sounds_stripped(ui.handle(UiInput::Right)).is_empty(),
        "no event at the bound"
    );
    for _ in 0..40 {
        ui.handle(UiInput::Left);
    }
    assert_eq!(ui.slider_value(1), Some(0.0));
}

#[test]
fn slider_custom_step_is_used() {
    let mut ui = ui();
    ui.add_root(Node::slider(1, 0.0).fixed(400.0, 56.0).step(0.25));
    ui.layout();
    ui.handle(UiInput::Right);
    ui.handle(UiInput::Right);
    assert_eq!(ui.slider_value(1), Some(0.25));
}

#[test]
fn slider_mouse_maps_the_track_to_the_value_and_drags() {
    let mut ui = ui();
    ui.add_root(
        Node::slider(1, 0.0)
            .fixed(418.0, 56.0)
            .anchor(Anchor::top_left(100.0, 100.0)),
    );
    ui.layout();
    let r = ui.rect_of(1).unwrap();
    let y = r[1] + 28.0;
    let knob = ui.theme().widgets.controls.knob_size[0];
    let usable = r[2] - knob;
    let ev =
        sounds_stripped(ui.handle(UiInput::PointerDown([r[0] + knob / 2.0 + usable * 0.25, y])));
    match ev.as_slice() {
        [WidgetEvent::SliderChanged(1, v)] => assert!((v - 0.25).abs() < 1e-4),
        other => panic!("{other:?}"),
    }
    // Dragging outside the widget keeps tracking, clamped to 0..1.
    ui.handle(UiInput::PointerMove([r[0] - 500.0, y + 300.0]));
    assert_eq!(ui.slider_value(1), Some(0.0));
    ui.handle(UiInput::PointerMove([r[0] + 5000.0, y]));
    assert_eq!(ui.slider_value(1), Some(1.0));
    ui.handle(UiInput::PointerUp([0.0, 0.0]));
    ui.handle(UiInput::PointerMove([r[0], y]));
    assert_eq!(ui.slider_value(1), Some(1.0), "released: no more tracking");
}

#[test]
fn toggle_and_selector_react_to_keys_and_clicks() {
    let mut ui = ui();
    let root = ui.add_root(Node::vbox().gap(10.0));
    ui.add(root, Node::toggle(1, "Full", false).w(Size::Fixed(300.0)));
    ui.add(
        root,
        Node::selector(2, vec!["a".into(), "b".into(), "c".into()], 0).w(Size::Fixed(300.0)),
    );
    ui.layout();
    ui.handle(UiInput::Down);
    assert_eq!(
        sounds_stripped(ui.handle(UiInput::Confirm)),
        vec![WidgetEvent::Toggled(1, true)]
    );
    ui.handle(UiInput::Down);
    assert_eq!(
        sounds_stripped(ui.handle(UiInput::Left)),
        vec![WidgetEvent::Selected(2, 2)],
        "wraps"
    );
    assert_eq!(
        sounds_stripped(ui.handle(UiInput::Right)),
        vec![WidgetEvent::Selected(2, 0)]
    );
    // Clicking the right part goes forward, the left third back.
    let r = ui.rect_of(2).unwrap();
    let ev = sounds_stripped(ui.handle(UiInput::PointerDown([r[0] + r[2] - 5.0, r[1] + 5.0])));
    assert_eq!(ev, vec![WidgetEvent::Selected(2, 1)]);
    ui.handle(UiInput::PointerUp([0.0, 0.0]));
    let ev = sounds_stripped(ui.handle(UiInput::PointerDown([r[0] + 5.0, r[1] + 5.0])));
    assert_eq!(ev, vec![WidgetEvent::Selected(2, 0)]);
}

#[test]
fn text_helpers() {
    assert_eq!(estimate_lines("", 30.0, 100.0), 1);
    assert_eq!(estimate_lines("a\nb\nc", 30.0, 1000.0), 3);
    assert!(estimate_lines(&"x".repeat(200), 30.0, 600.0) > 5);
    assert_eq!(ellipsize("short", 10), "short");
    assert_eq!(ellipsize("abcdefghij", 5), "abcd\u{2026}");
    assert_eq!(escape_markup("a{b}"), "a{{b}}");
}

#[test]
fn margin_keeps_room_around_a_node() {
    let mut ui = ui();
    let root = ui.add_root(Node::vbox().fixed(300.0, 400.0));
    ui.add(
        root,
        Node::vbox()
            .fixed(100.0, 50.0)
            .margin([10.0, 20.0, 10.0, 20.0])
            .tag(1),
    );
    ui.add(root, Node::vbox().fixed(80.0, 60.0).tag(2));
    ui.layout();
    assert_eq!(ui.rect_of(1).unwrap(), [10.0, 20.0, 100.0, 50.0]);
    assert_eq!(ui.rect_of(2).unwrap(), [0.0, 90.0, 80.0, 60.0]);
}

#[test]
fn a_fit_container_includes_the_margin_of_its_children() {
    let mut ui = ui();
    let root = ui.add_root(Node::vbox().tag(9));
    ui.add(root, Node::vbox().fixed(100.0, 50.0).margin([5.0, 5.0, 15.0, 5.0]));
    ui.layout();
    let r = ui.rect_of(9).unwrap();
    assert_eq!((r[2], r[3]), (120.0, 60.0));
}

#[test]
fn grid_places_cells_row_by_row_with_the_widest_column() {
    let mut ui = ui();
    let root = ui.add_root(Node::grid(2).gap(10.0).fixed(1000.0, 500.0));
    ui.add(root, Node::vbox().fixed(100.0, 40.0).tag(1));
    ui.add(root, Node::vbox().fixed(60.0, 40.0).tag(2));
    ui.add(root, Node::vbox().fixed(80.0, 30.0).tag(3));
    ui.add(root, Node::vbox().fixed(60.0, 30.0).tag(4));
    ui.add(root, Node::vbox().fixed(10.0, 10.0).tag(5));
    ui.layout();
    assert_eq!(ui.rect_of(1).unwrap(), [0.0, 0.0, 100.0, 40.0]);
    assert_eq!(ui.rect_of(2).unwrap(), [110.0, 0.0, 60.0, 40.0]);
    assert_eq!(ui.rect_of(3).unwrap(), [0.0, 50.0, 80.0, 30.0]);
    assert_eq!(ui.rect_of(4).unwrap(), [110.0, 50.0, 60.0, 30.0]);
    assert_eq!(ui.rect_of(5).unwrap(), [0.0, 90.0, 10.0, 10.0], "odd cell count");
}

#[test]
fn grid_gives_spare_width_to_fill_columns() {
    let mut ui = ui();
    let root = ui.add_root(Node::grid(2).gap(10.0).fixed(400.0, 100.0));
    ui.add(root, Node::vbox().fixed(100.0, 40.0).tag(1));
    ui.add(root, Node::vbox().w(Size::Fill(1.0)).h(Size::Fixed(40.0)).tag(2));
    ui.layout();
    assert_eq!(ui.rect_of(2).unwrap(), [110.0, 0.0, 290.0, 40.0]);
}

fn dropdown_ui(options: usize, selected: usize) -> Ui {
    let mut ui = ui();
    let root = ui.add_root(Node::vbox().pad(20.0, 20.0));
    let names = (0..options).map(|i| format!("opt{i}")).collect();
    ui.add(root, Node::dropdown(7, names, selected).w(Size::Fixed(300.0)));
    ui.layout();
    ui
}

fn click(ui: &mut Ui, p: [f32; 2]) -> Vec<WidgetEvent> {
    ui.handle(UiInput::PointerMove(p));
    let mut out = ui.handle(UiInput::PointerDown(p));
    out.extend(ui.handle(UiInput::PointerUp(p)));
    sounds_stripped(out)
}

#[test]
fn a_dropdown_opens_a_popup_and_a_row_click_selects_and_closes() {
    let mut ui = dropdown_ui(3, 0);
    let r = ui.rect_of(7).unwrap();
    assert!(click(&mut ui, centre(r)).is_empty(), "opening selects nothing");
    assert!(ui.popup_open() && ui.has_modal());
    assert_eq!(ui.selector_index(7), Some(0));
    // Rows are below the dropdown, in order.
    let rows: Vec<[f32; 4]> = (0..3)
        .map(|i| ui.rect_of(crate::widgets::interact::POPUP_TAG + i).unwrap())
        .collect();
    assert!(rows[0][1] >= r[1] + r[3], "popup opens below: {rows:?}");
    assert!(rows[0][1] < rows[1][1] && rows[1][1] < rows[2][1]);
    assert_eq!(click(&mut ui, centre(rows[2])), vec![WidgetEvent::Selected(7, 2)]);
    assert!(!ui.popup_open() && !ui.has_modal());
    assert_eq!(ui.selector_index(7), Some(2));
    // Picking the current option again changes nothing.
    click(&mut ui, centre(r));
    let again = ui.rect_of(crate::widgets::interact::POPUP_TAG + 2).unwrap();
    assert!(click(&mut ui, centre(again)).is_empty());
}

#[test]
fn a_dropdown_popup_closes_on_cancel_and_on_an_outside_click_without_selecting() {
    let mut ui = dropdown_ui(3, 1);
    let r = ui.rect_of(7).unwrap();
    click(&mut ui, centre(r));
    assert!(ui.popup_open());
    assert!(ui.handle(UiInput::Cancel).iter().all(|e| *e != WidgetEvent::Cancelled));
    assert!(!ui.popup_open());
    click(&mut ui, centre(r));
    assert!(ui.popup_open());
    assert!(click(&mut ui, [1500.0, 900.0]).is_empty());
    assert!(!ui.popup_open());
    assert_eq!(ui.selector_index(7), Some(1));
}

#[test]
fn a_dropdown_popup_is_keyboard_driven_and_scrolls_long_lists() {
    let mut ui = dropdown_ui(10, 0);
    ui.set_focus_tag(7);
    ui.handle(UiInput::Confirm);
    assert!(ui.popup_open());
    assert_eq!(ui.focused_tag(), Some(crate::widgets::interact::POPUP_TAG));
    ui.handle(UiInput::Down);
    ui.handle(UiInput::Down);
    let ev = sounds_stripped(ui.handle(UiInput::Confirm));
    assert_eq!(ev, vec![WidgetEvent::Selected(7, 2)]);
    assert_eq!(ui.focused_tag(), Some(7), "focus returns to the dropdown");
    // Ten options: only six rows are shown, the popup scrolls.
    ui.handle(UiInput::Confirm);
    let hidden = ui
        .find(crate::widgets::interact::POPUP_TAG + 9)
        .map(|id| ui.node(id).is_visible());
    assert_eq!(hidden, Some(false));
}

#[test]
fn style_sounds_replace_the_generic_cue() {
    let mut theme = Theme::default();
    theme.styles.insert(
        "fancy".into(),
        crate::theme::Style {
            sounds: Some(crate::theme::StyleSounds {
                hover: Some("sfx/hover.ogg".into()),
                confirm: Some("sfx/click.ogg".into()),
            }),
            ..Default::default()
        },
    );
    let mut ui = Ui::new(theme);
    let root = ui.add_root(Node::vbox());
    ui.add(root, Node::button(1, "A").style("fancy").w(Size::Fixed(200.0)));
    ui.add(root, Node::button(2, "B").w(Size::Fixed(200.0)));
    ui.layout();
    let a = centre(ui.rect_of(1).unwrap());
    let b = centre(ui.rect_of(2).unwrap());
    assert!(ui
        .handle(UiInput::PointerMove(a))
        .contains(&WidgetEvent::SoundFile("sfx/hover.ogg".into())));
    ui.handle(UiInput::PointerDown(a));
    let up = ui.handle(UiInput::PointerUp(a));
    assert!(up.contains(&WidgetEvent::SoundFile("sfx/click.ogg".into())), "{up:?}");
    // An unstyled button keeps the generic cues.
    assert!(ui
        .handle(UiInput::PointerMove(b))
        .contains(&WidgetEvent::Sound(crate::input::UiSound::Hover)));
}

#[test]
fn grid_spans_cover_several_columns_and_rows() {
    let mut ui = ui();
    let root = ui.add_root(Node::grid(3).gap(10.0).fixed(1000.0, 500.0));
    // Row 0: a header spanning the three columns.
    ui.add(root, Node::vbox().fixed(200.0, 20.0).span(3, 1).tag(1));
    // Row 1: a tall cell (2 rows) then two small ones; row 2 gets the rest.
    ui.add(root, Node::vbox().fixed(50.0, 70.0).span(1, 2).tag(2));
    ui.add(root, Node::vbox().fixed(40.0, 30.0).tag(3));
    ui.add(root, Node::vbox().fixed(40.0, 30.0).tag(4));
    ui.add(root, Node::vbox().fixed(40.0, 30.0).tag(5));
    ui.layout();
    // The 200 px header is wider than 50 + 40 + 40 + 2 gaps (150): the
    // three columns share the missing 50 px, so the next cells move right.
    let wide = ui.rect_of(1).unwrap();
    assert_eq!(wide, [0.0, 0.0, 200.0, 20.0]);
    let share = 50.0 / 3.0;
    let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01;
    let at = |ui: &mut Ui, tag| {
        let r = ui.rect_of(tag).unwrap();
        [r[0], r[1]]
    };
    assert!(near(at(&mut ui, 2), [0.0, 30.0]));
    assert!(near(at(&mut ui, 3), [50.0 + share + 10.0, 30.0]));
    assert!(near(at(&mut ui, 4), [50.0 + 40.0 + 2.0 * share + 20.0, 30.0]));
    // The next one wraps under the cells to the right of the tall one.
    assert!(near(at(&mut ui, 5), [50.0 + share + 10.0, 70.0]));
}

#[test]
fn grid_place_skips_taken_cells_and_cuts_wide_spans() {
    use super::layout::{GridCell, grid_place};
    let cell = |col, row, cols, rows| GridCell { col, row, cols, rows };
    let placed = grid_place(&[[1, 2], [1, 1], [1, 1], [1, 1], [9, 0]], 2);
    assert_eq!(
        placed,
        vec![
            cell(0, 0, 1, 2),
            cell(1, 0, 1, 1),
            cell(1, 1, 1, 1),
            cell(0, 2, 1, 1),
            cell(0, 3, 2, 1),
        ]
    );
}

#[test]
fn layout_types_round_trip_through_ron() {
    let anchor = Anchor::new([0.5, 0.0], [0.5, 0.0], [0.0, 30.0]);
    let text = ron::to_string(&(Size::Fill(2.0), Dir::Grid(3), Align::Stretch, anchor, PanelKind::Dim))
        .unwrap();
    let back: (Size, Dir, Align, Anchor, PanelKind) = ron::from_str(&text).unwrap();
    assert_eq!(back, (Size::Fill(2.0), Dir::Grid(3), Align::Stretch, anchor, PanelKind::Dim));
}
