//! [`GraphView`] on `egui-snarl`.
//!
//! Snarl owns the pan / zoom, the node dragging, the wire dragging and the
//! rectangle selection; this module draws the blocks, turns what Snarl
//! reports into [`GraphEvent`]s, and never lets Snarl change the graph by
//! itself: wires are always re-derived from the story (the `connect`,
//! `disconnect`... callbacks only emit events).

use std::collections::HashMap;

use egui::emath::TSTransform;
use egui::epaint::Shadow;
use egui::{
    Align, Color32, CornerRadius, Frame, Id, Layout, Margin, PointerButton, Pos2, Rect, RichText,
    Stroke, Ui, pos2, vec2,
};
use egui_snarl::ui::{
    AnyPins, BackgroundPattern, NodeLayout, PinInfo, PinPlacement, SelectionStyle, SnarlStyle,
    SnarlViewer, SnarlWidget,
};
use egui_snarl::{InPin, InPinId, NodeId, OutPin, OutPinId, Snarl};
use orbit_editor_api::{Level, UiText};
use orbit_project::graph::{BlockId, SlotKind};
use orbit_project::graph_layout::BLOCK_WIDTH;

use super::view::{GraphEvent, GraphView, ViewContent, ViewNode};
use crate::builtin::ui_util::{danger, faint, warning, weak};
use crate::theme::bold;

/// What to do to the transform on the next frame.
#[derive(Clone, Copy, Debug)]
enum Pending {
    Fit,
    Center(Pos2, Option<f32>),
}

pub struct SnarlView {
    snarl: Snarl<ViewNode>,
    ids: HashMap<BlockId, NodeId>,
    /// Positions as last reported / set, to notice drags.
    positions: HashMap<BlockId, (f32, f32)>,
    /// Identity of the structure shown (story revision...): while it stays
    /// the same only the flags of the nodes are refreshed.
    structure: u64,
    bounds: Rect,
    pending: Option<Pending>,
    nudge: (egui::Vec2, f32),
    screen_rect: Rect,
    visible: Option<Rect>,
    last_selection: Vec<BlockId>,
    dragging: bool,
    /// Text of the rename / option text fields of the open menus.
    edit: String,
    edit_for: Option<EditTarget>,
    id_salt: Id,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditTarget {
    Block(BlockId),
    Slot(BlockId, usize),
}

impl SnarlView {
    pub fn new(id_salt: &str) -> Self {
        Self {
            snarl: Snarl::new(),
            ids: HashMap::new(),
            positions: HashMap::new(),
            structure: u64::MAX,
            bounds: Rect::NOTHING,
            pending: Some(Pending::Fit),
            nudge: (egui::Vec2::ZERO, 1.0),
            screen_rect: Rect::ZERO,
            visible: None,
            last_selection: Vec::new(),
            dragging: false,
            edit: String::new(),
            edit_for: None,
            id_salt: Id::new(id_salt),
        }
    }

    fn style() -> SnarlStyle {
        let mut style = SnarlStyle::new();
        style.node_layout = Some(NodeLayout::sandwich());
        style.pin_placement = Some(PinPlacement::Edge);
        style.pin_size = Some(10.0);
        style.collapsible = Some(false);
        style.min_scale = Some(0.03);
        style.max_scale = Some(1.6);
        style.centering = Some(false);
        style.wire_width = Some(2.0);
        style.wire_frame_size = Some(70.0);
        style.header_drag_space = Some(vec2(0.0, 0.0));
        style.bg_pattern = Some(BackgroundPattern::grid(vec2(48.0, 48.0), 0.0));
        style.select_style = Some(SelectionStyle {
            margin: Margin::ZERO,
            rounding: CornerRadius::same(10),
            fill: Color32::TRANSPARENT,
            stroke: Stroke::NONE,
        });
        style.select_fill = Some(Color32::from_rgba_unmultiplied(100, 150, 255, 24));
        style
    }

    fn bounds_of(content: &ViewContent) -> Rect {
        let mut bounds = Rect::NOTHING;
        for (node, (x, y)) in &content.nodes {
            bounds = bounds.union(Rect::from_min_size(
                pos2(*x, *y),
                vec2(BLOCK_WIDTH, node.height),
            ));
        }
        bounds
    }

    fn block_of(&self, id: NodeId) -> Option<BlockId> {
        self.snarl.get_node(id).map(|n| n.id)
    }
}

impl GraphView for SnarlView {
    fn set_content(&mut self, content: ViewContent) {
        self.bounds = Self::bounds_of(&content);
        // Same structure: refresh flags, keep Snarl's own state (selection,
        // dragged positions).
        let signature = content.signature;
        if signature == self.structure && content.nodes.len() == self.ids.len() {
            for (node, _) in content.nodes {
                if let Some(id) = self.ids.get(&node.id)
                    && let Some(n) = self.snarl.get_node_mut(*id)
                {
                    n.selected = node.selected;
                    n.highlighted = node.highlighted;
                }
            }
            return;
        }
        self.structure = signature;
        self.snarl = Snarl::new();
        self.ids.clear();
        self.positions.clear();
        for (node, (x, y)) in &content.nodes {
            let id = self.snarl.insert_node(pos2(*x, *y), node.clone());
            self.ids.insert(node.id, id);
            self.positions.insert(node.id, (*x, *y));
        }
        for (node, _) in &content.nodes {
            let from = self.ids[&node.id];
            for (i, slot) in node.slots.iter().enumerate() {
                if let Some(to) = slot.target.and_then(|t| self.ids.get(&t)) {
                    self.snarl.connect(
                        OutPinId {
                            node: from,
                            output: i,
                        },
                        InPinId {
                            node: *to,
                            input: 0,
                        },
                    );
                }
            }
        }
    }

    fn show(&mut self, ui: &mut Ui) -> Vec<GraphEvent> {
        let mut events = Vec::new();
        self.screen_rect = ui.available_rect_before_wrap();
        let widget = SnarlWidget::new()
            .id_salt(self.id_salt)
            .style(Self::style());
        let mut viewer = Viewer {
            events: &mut events,
            pending: &mut self.pending,
            bounds: self.bounds,
            screen_rect: self.screen_rect,
            visible: &mut self.visible,
            edit: &mut self.edit,
            edit_for: &mut self.edit_for,
            nudge: &mut self.nudge,
        };
        widget.show(&mut self.snarl, &mut viewer, ui);

        // Selection made inside Snarl (rectangle, shift-click).
        let selected: Vec<BlockId> = widget
            .get_selected_nodes_at(ui.id(), ui.ctx())
            .into_iter()
            .filter_map(|id| self.block_of(id))
            .collect();
        if selected != self.last_selection {
            if selected.len() > 1 {
                events.push(GraphEvent::Select(selected.clone()));
            }
            self.last_selection = selected;
        }

        // Dragged blocks.
        let mut moved = false;
        for (block, id) in &self.ids {
            let Some(info) = self.snarl.get_node_info(*id) else {
                continue;
            };
            let now = (info.pos.x, info.pos.y);
            let before = self.positions.get(block).copied().unwrap_or(now);
            if (now.0 - before.0).abs() > 0.01 || (now.1 - before.1).abs() > 0.01 {
                self.positions.insert(*block, now);
                events.push(GraphEvent::Moved(*block, now));
                moved = true;
            }
        }
        if moved {
            self.dragging = true;
        } else if self.dragging && !ui.input(|i| i.pointer.any_down()) {
            self.dragging = false;
            events.push(GraphEvent::MoveEnded);
        }
        events
    }

    fn fit_all(&mut self) {
        self.pending = Some(Pending::Fit);
    }

    fn focus(&mut self, point: Pos2, min_zoom: Option<f32>) {
        self.pending = Some(Pending::Center(point, min_zoom));
    }

    fn nudge(&mut self, pan: egui::Vec2, zoom: f32) {
        self.nudge.0 += pan;
        self.nudge.1 *= zoom;
    }

    fn visible_rect(&self) -> Option<Rect> {
        self.visible
    }
}

struct Viewer<'a> {
    events: &'a mut Vec<GraphEvent>,
    pending: &'a mut Option<Pending>,
    bounds: Rect,
    screen_rect: Rect,
    visible: &'a mut Option<Rect>,
    edit: &'a mut String,
    edit_for: &'a mut Option<EditTarget>,
    nudge: &'a mut (egui::Vec2, f32),
}

fn kind_text(texts: &orbit_editor_api::Translator, slot: &super::view::ViewSlot) -> String {
    let mut text = slot.label.clone();
    if text.is_empty() {
        text = match slot.slot.kind {
            SlotKind::Jump => texts.t("graph.slot.jump"),
            SlotKind::Call => texts.t("graph.slot.call"),
            _ => String::new(),
        };
    }
    text
}

impl Viewer<'_> {
    /// Menu entries shared by the block menu.
    fn block_menu(&mut self, node: &ViewNode, ui: &mut Ui) {
        let id = node.id;
        if ui.button(ui.t("graph.menu.open")).clicked() {
            self.events.push(GraphEvent::Open(id));
            ui.close();
        }
        if ui.button(ui.t("graph.menu.play")).clicked() {
            self.events.push(GraphEvent::PlayFrom(id));
            ui.close();
        }
        ui.separator();
        let open = node
            .slots
            .last()
            .is_some_and(|s| s.slot.kind == SlotKind::Next);
        ui.add_enabled_ui(open, |ui| {
            if ui.button(ui.t("graph.menu.add_choice")).clicked() {
                self.events.push(GraphEvent::AddChoice(id));
                ui.close();
            }
            if ui.button(ui.t("graph.menu.add_condition")).clicked() {
                self.events.push(GraphEvent::AddCondition(id));
                ui.close();
            }
            if ui.button(ui.t("graph.menu.end_here")).clicked() {
                self.events.push(GraphEvent::AddEnd(id));
                ui.close();
            }
        })
        .response
        .on_disabled_hover_text(ui.t("graph.menu.already_ends"));
        if !open && ui.button(ui.t("graph.menu.remove_end")).clicked() {
            self.events.push(GraphEvent::RemoveEnd(id));
            ui.close();
        }
        ui.separator();
        if *self.edit_for != Some(EditTarget::Block(id)) {
            *self.edit_for = Some(EditTarget::Block(id));
            *self.edit = node.title.clone();
        }
        ui.label(RichText::new(ui.t("common.rename")).font(bold(11.0)));
        let response = ui.add(egui::TextEdit::singleline(self.edit).desired_width(160.0));
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.events
                .push(GraphEvent::Rename(id, std::mem::take(self.edit)));
            *self.edit_for = None;
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new(ui.t("graph.menu.delete_block")).color(danger(ui)))
            .clicked()
        {
            self.events.push(GraphEvent::Delete(id));
            ui.close();
        }
    }
}

#[allow(refining_impl_trait)]
impl SnarlViewer<ViewNode> for Viewer<'_> {
    fn title(&mut self, node: &ViewNode) -> String {
        node.title.clone()
    }

    fn node_frame(
        &mut self,
        default: Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<ViewNode>,
    ) -> Frame {
        let n = &snarl[node];
        let mut frame = default
            .shadow(Shadow::NONE)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(0, 0));
        if n.orphan {
            frame = frame.fill(frame.fill.gamma_multiply(0.7));
        }
        let stroke = if n.selected {
            Stroke::new(2.5, Color32::from_rgb(120, 170, 255))
        } else if n.highlighted {
            Stroke::new(2.5, Color32::from_rgb(255, 214, 90))
        } else if matches!(n.problem, Some((Level::Error, _))) {
            Stroke::new(1.5, Color32::from_rgb(230, 90, 90))
        } else {
            Stroke::new(1.0, n.accent.gamma_multiply(0.6))
        };
        frame.stroke(stroke)
    }

    fn header_frame(
        &mut self,
        default: Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<ViewNode>,
    ) -> Frame {
        let n = &snarl[node];
        default
            .shadow(Shadow::NONE)
            .fill(n.accent.gamma_multiply(if n.orphan { 0.55 } else { 0.85 }))
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius {
                nw: 10,
                ne: 10,
                sw: 0,
                se: 0,
            })
            .inner_margin(Margin::symmetric(10, 6))
    }

    fn show_header(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<ViewNode>,
    ) {
        let n = &snarl[node];
        ui.set_width(BLOCK_WIDTH - 20.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(n.icon).size(15.0).color(Color32::WHITE));
            ui.add(
                egui::Label::new(
                    RichText::new(&n.title)
                        .font(bold(14.0))
                        .color(Color32::WHITE),
                )
                .truncate(),
            );
            if let Some((level, text)) = &n.problem {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (icon, color) = match level {
                        Level::Error => (egui_phosphor::regular::WARNING_OCTAGON, Color32::WHITE),
                        _ => (
                            egui_phosphor::regular::WARNING,
                            Color32::from_rgb(255, 228, 140),
                        ),
                    };
                    let badge = ui
                        .add(
                            egui::Label::new(RichText::new(icon).size(15.0).color(color))
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text(ui.tf("graph.pin.problem_tip", &[("text", text)]));
                    if badge.clicked()
                        && let Some(node) = n.problem_node
                    {
                        self.events.push(GraphEvent::Reveal(node));
                    }
                });
            }
        });
    }

    fn inputs(&mut self, _node: &ViewNode) -> usize {
        1
    }

    fn show_input(&mut self, pin: &InPin, ui: &mut Ui, snarl: &mut Snarl<ViewNode>) -> PinInfo {
        let n = &snarl[pin.id.node];
        ui.add_space(0.0);
        ui.label(
            RichText::new(ui.t("graph.pin.input"))
                .size(10.0)
                .color(faint(ui)),
        );
        PinInfo::circle()
            .with_fill(n.accent)
            .with_stroke(Stroke::new(1.5, Color32::from_rgb(24, 26, 32)))
    }

    fn outputs(&mut self, node: &ViewNode) -> usize {
        node.slots.len()
    }

    fn show_output(&mut self, pin: &OutPin, ui: &mut Ui, snarl: &mut Snarl<ViewNode>) -> PinInfo {
        let node = snarl[pin.id.node].clone();
        let slot = &node.slots[pin.id.output];
        let text = kind_text(&ui.texts(), slot);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.set_width(BLOCK_WIDTH - 24.0);
            let mut label = RichText::new(truncate(&text, 34)).size(12.0);
            label = if slot.broken {
                label.color(Color32::from_rgb(240, 110, 110))
            } else if slot.soft {
                label.color(faint(ui)).italics()
            } else {
                label.color(ui.visuals().text_color())
            };
            let response = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
            let response = if slot.broken {
                response.on_hover_text(ui.t("graph.pin.broken"))
            } else {
                response
            };
            if slot.conditional && matches!(slot.slot.kind, SlotKind::Option(_)) {
                ui.label(
                    RichText::new(egui_phosphor::regular::FUNCTION)
                        .size(11.0)
                        .color(faint(ui)),
                )
                .on_hover_text(ui.t("graph.pin.conditional"));
            }
            response.context_menu(|ui| {
                if let SlotKind::Option(i) = slot.slot.kind {
                    let target = EditTarget::Slot(node.id, i);
                    if *self.edit_for != Some(target) {
                        *self.edit_for = Some(target);
                        *self.edit = slot.label.clone();
                    }
                    ui.label(RichText::new(ui.t("graph.menu.option_text")).font(bold(11.0)));
                    let field = ui.add(egui::TextEdit::singleline(self.edit).desired_width(200.0));
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.events.push(GraphEvent::SetOptionText(
                            slot.slot,
                            std::mem::take(self.edit),
                        ));
                        *self.edit_for = None;
                        ui.close();
                    }
                    if ui.button(ui.t("graph.menu.add_option")).clicked() {
                        self.events.push(GraphEvent::AddOption(slot.slot));
                        ui.close();
                    }
                    ui.separator();
                }
                if slot.slot.kind != SlotKind::Next
                    && ui
                        .button(RichText::new(ui.t("graph.menu.remove_link")).color(danger(ui)))
                        .clicked()
                {
                    self.events.push(GraphEvent::Disconnect(slot.slot));
                    ui.close();
                }
                if slot.slot.kind == SlotKind::Next {
                    ui.label(RichText::new(ui.t("graph.menu.next_hint")).color(weak(ui)));
                }
            });
        });
        let mut info = PinInfo::circle()
            .with_fill(slot.color)
            .with_wire_color(slot.color)
            .with_stroke(Stroke::new(1.5, Color32::from_rgb(24, 26, 32)));
        if slot.soft {
            info = info.with_fill(slot.color.gamma_multiply(0.7));
        }
        info
    }

    fn has_body(&mut self, _node: &ViewNode) -> bool {
        true
    }

    fn show_body(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<ViewNode>,
    ) {
        let n = &snarl[node];
        ui.vertical(|ui| {
            ui.set_width(BLOCK_WIDTH - 24.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(
                egui::Label::new(RichText::new(&n.subtitle).size(11.0).color(weak(ui))).truncate(),
            );
            for line in &n.meta {
                ui.add(egui::Label::new(RichText::new(line).size(11.0).color(weak(ui))).truncate());
            }
            match &n.first_line {
                Some(text) => {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("« {} »", truncate(text, 60)))
                                .size(11.5)
                                .italics(),
                        )
                        .truncate(),
                    );
                }
                None => {
                    ui.label(RichText::new(" ").size(11.5));
                }
            }
            if n.orphan {
                ui.label(
                    RichText::new(ui.t("graph.orphan"))
                        .size(10.5)
                        .color(warning(ui)),
                );
            }
            if n.dead_nodes > 0 {
                ui.label(
                    RichText::new(ui.tn("graph.dead_nodes", n.dead_nodes, &[]))
                        .size(10.5)
                        .color(warning(ui)),
                );
            }
            ui.add_space(2.0);
        });
    }

    fn has_node_menu(&mut self, _node: &ViewNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<ViewNode>,
    ) {
        let n = snarl[node].clone();
        self.block_menu(&n, ui);
    }

    fn has_graph_menu(&mut self, _pos: Pos2, _snarl: &mut Snarl<ViewNode>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: Pos2, ui: &mut Ui, _snarl: &mut Snarl<ViewNode>) {
        if ui.button(ui.t("graph.menu.new_scene_here")).clicked() {
            self.events.push(GraphEvent::NewScene((pos.x, pos.y)));
            ui.close();
        }
        if ui.button(ui.t("graph.menu.new_label_here")).clicked() {
            self.events.push(GraphEvent::NewLabel((pos.x, pos.y)));
            ui.close();
        }
    }

    fn has_dropped_wire_menu(&mut self, src_pins: AnyPins, _snarl: &mut Snarl<ViewNode>) -> bool {
        matches!(src_pins, AnyPins::Out(_))
    }

    fn show_dropped_wire_menu(
        &mut self,
        pos: Pos2,
        ui: &mut Ui,
        src_pins: AnyPins,
        snarl: &mut Snarl<ViewNode>,
    ) {
        let AnyPins::Out(pins) = src_pins else { return };
        let slots: Vec<_> = pins
            .iter()
            .filter_map(|p| snarl.get_node(p.node).and_then(|n| n.slots.get(p.output)))
            .map(|s| s.slot)
            .collect();
        if ui
            .button(ui.t("graph.menu.new_linked_scene_here"))
            .clicked()
        {
            for slot in slots {
                self.events
                    .push(GraphEvent::ConnectNew(slot, (pos.x, pos.y)));
            }
            ui.close();
        }
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<ViewNode>) {
        let (Some(src), Some(dst)) = (snarl.get_node(from.id.node), snarl.get_node(to.id.node))
        else {
            return;
        };
        if let Some(slot) = src.slots.get(from.id.output) {
            self.events.push(GraphEvent::Connect(slot.slot, dst.id));
        }
    }

    fn disconnect(&mut self, from: &OutPin, _to: &InPin, snarl: &mut Snarl<ViewNode>) {
        if let Some(slot) = snarl
            .get_node(from.id.node)
            .and_then(|n| n.slots.get(from.id.output))
            && slot.slot.kind != SlotKind::Next
        {
            self.events.push(GraphEvent::Disconnect(slot.slot));
        }
    }

    fn drop_outputs(&mut self, pin: &OutPin, snarl: &mut Snarl<ViewNode>) {
        if let Some(slot) = snarl
            .get_node(pin.id.node)
            .and_then(|n| n.slots.get(pin.id.output))
            && slot.slot.kind != SlotKind::Next
        {
            self.events.push(GraphEvent::Disconnect(slot.slot));
        }
    }

    fn drop_inputs(&mut self, _pin: &InPin, _snarl: &mut Snarl<ViewNode>) {}

    fn final_node_rect(
        &mut self,
        node: NodeId,
        rect: Rect,
        ui: &mut Ui,
        snarl: &mut Snarl<ViewNode>,
    ) {
        let Some(from_global) = ui.ctx().layer_transform_from_global(ui.layer_id()) else {
            return;
        };
        let (pos, clicked, double, shift) = ui.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.button_clicked(PointerButton::Primary),
                i.pointer.button_double_clicked(PointerButton::Primary),
                i.modifiers.shift || i.modifiers.command,
            )
        });
        let Some(pos) = pos else { return };
        if !rect.contains(from_global * pos) {
            return;
        }
        let block = snarl[node].id;
        if double {
            self.events.push(GraphEvent::Open(block));
        } else if clicked && !shift {
            self.events.push(GraphEvent::Select(vec![block]));
        }
    }

    fn current_transform(&mut self, to_global: &mut TSTransform, _snarl: &mut Snarl<ViewNode>) {
        let view = self.screen_rect;
        if let Some(pending) = self.pending.take()
            && view.width() > 8.0
            && view.height() > 8.0
        {
            match pending {
                Pending::Fit if self.bounds.is_finite() => {
                    let margin = 40.0;
                    let size = self.bounds.size().max(vec2(1.0, 1.0));
                    let scale = ((view.width() - 2.0 * margin) / size.x)
                        .min((view.height() - 2.0 * margin) / size.y)
                        .clamp(0.03, 1.0);
                    *to_global = TSTransform::new(
                        view.center().to_vec2() - self.bounds.center().to_vec2() * scale,
                        scale,
                    );
                }
                Pending::Center(point, min_zoom) => {
                    let scale = min_zoom.map_or(to_global.scaling, |z| to_global.scaling.max(z));
                    *to_global =
                        TSTransform::new(view.center().to_vec2() - point.to_vec2() * scale, scale);
                }
                Pending::Fit => {}
            }
        } else if let Some(pending) = self.pending.take() {
            // The view has no size yet: try again next frame.
            *self.pending = Some(pending);
        }
        if *self.nudge != (egui::Vec2::ZERO, 1.0) {
            let (pan, zoom) = std::mem::replace(self.nudge, (egui::Vec2::ZERO, 1.0));
            let scale = (to_global.scaling * zoom).clamp(0.03, 1.6);
            let ratio = scale / to_global.scaling;
            let center = view.center().to_vec2();
            *to_global = TSTransform::new(
                center - (center - to_global.translation) * ratio + pan,
                scale,
            );
        }
        let from_global = to_global.inverse();
        *self.visible = Some(from_global * view);
    }
}

fn truncate(text: &str, max: usize) -> String {
    crate::builtin::nodeinfo::truncate(text, max)
}
