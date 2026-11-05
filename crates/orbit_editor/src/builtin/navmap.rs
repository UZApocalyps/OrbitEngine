//! "New navigation map (house)": a wizard that writes a small point &
//! click house. The author names the rooms, picks their backgrounds and
//! ticks which rooms are linked; the wizard writes one label and one hotspot
//! map per room, with a door zone (a placeholder rectangle to move onto the
//! picture) to each linked room, a "back" zone, and an example of a door
//! locked by a key (a variable: the key is picked up in the room, the door
//! is clickable once it is taken). Everything is inserted as one undo step.
//!
//! [`generate`] is pure (tested without a window); the window only edits a
//! [`NavPlan`].

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use egui::{RichText, Ui};
use orbit_core::{
    AssetId, AssetKind, Expr, Hotspot, HotspotAction, HotspotMap, HotspotShape, Node, NodeId, NodeKind, Notify,
    OpKind, Operand, Story, StringId, UnOp, Value, VarDecl,
};
use orbit_editor_api::fields::{self, asset_picker};
use orbit_editor_api::{ActionDef, EditorContext, EditorRegistry, Selection, Translator, UiText, WindowDef, icons};
use orbit_project::Command;
use orbit_project::ops::{self, NodeDraft};

/// Most rooms of one wizard run.
pub const MAX_ROOMS: usize = 12;

/// One room of the house.
#[derive(Clone, Debug, PartialEq)]
pub struct NavRoom {
    pub name: String,
    pub background: Option<AssetId>,
}

/// What the wizard writes.
#[derive(Clone, Debug, PartialEq)]
pub struct NavPlan {
    pub rooms: Vec<NavRoom>,
    /// Linked rooms `(a, b)` with `a < b`: a door each way.
    pub doors: BTreeSet<(usize, usize)>,
    /// A door locked until the key is found (the door from `a` to `b`;
    /// the key lies in room `a`).
    pub locked: Option<(usize, usize)>,
    /// Every room but the first gets a "back" zone to the room it is
    /// reached from (its first linked room listed before it).
    pub back: bool,
}

/// The nodes to insert and the variable to declare.
#[derive(Clone, Debug)]
pub struct NavOutput {
    pub drafts: Vec<NodeDraft>,
    pub variable: Option<VarDecl>,
}

/// A label name from a room name: lowercase letters, digits and `_`.
fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.ends_with('_') && !s.is_empty() {
            s.push('_');
        }
    }
    let s = s.trim_end_matches('_').to_owned();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("room_{s}")
    } else {
        s
    }
}

/// `base`, or `base_2`, `base_3`... the first one not in `taken`
/// (identifiers: usable bare in the text form).
fn unique_ident(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_owned();
    }
    (2..).map(|k| format!("{base}_{k}")).find(|c| !taken.contains(c)).expect("a free name")
}

/// The house of `plan`, for `story` (names unique among its labels and
/// variables). Errors: no room, a room without any zone (not linked).
pub fn generate(story: &Story, plan: &NavPlan, t: &Translator) -> Result<NavOutput, String> {
    let n = plan.rooms.len();
    if n == 0 {
        return Err(t.t("scene.navmap.err_no_room"));
    }
    let mut taken: Vec<String> = ops::labels(story).into_iter().map(|(_, name)| name).collect();
    let labels: Vec<(NodeId, String)> = plan
        .rooms
        .iter()
        .map(|r| {
            let name = unique_ident(&slug(&r.name), &taken);
            taken.push(name.clone());
            (NodeId::new(), name)
        })
        .collect();
    let room_name = |i: usize| {
        let name = plan.rooms[i].name.trim();
        if name.is_empty() { labels[i].1.clone() } else { name.to_owned() }
    };
    let neighbors = |r: usize| -> Vec<usize> {
        plan.doors
            .iter()
            .filter_map(|&(a, b)| match (a == r, b == r) {
                (true, _) if b < n => Some(b),
                (_, true) if a < n => Some(a),
                _ => None,
            })
            .collect()
    };
    let locked = plan.locked.filter(|l| plan.doors.contains(l) && l.1 < n);
    let key_var = locked.map(|_| {
        let vars: Vec<String> = story.variables.iter().map(|v| v.name.clone()).collect();
        unique_ident(&slug(&t.t("scene.navmap.key_var")), &vars)
    });
    let mut drafts = Vec::new();
    for r in 0..n {
        let mut strings = Vec::new();
        let mut text = |s: String| {
            let id = StringId::new();
            strings.push((id, s));
            id
        };
        let doors = neighbors(r);
        let mut hotspots = Vec::new();
        // Doors: placeholder rectangles side by side, to move onto the picture.
        let count = doors.len().max(1) as f32;
        let w = (0.8 / count).min(0.2);
        for (k, &other) in doors.iter().enumerate() {
            let x = 0.1 + (0.8 / count) * k as f32 + (0.8 / count - w) * 0.5;
            let mut door = Hotspot::new(
                text(t.tf("scene.navmap.door_to", &[("room", &room_name(other))])),
                HotspotShape::Rect { x, y: 0.25, w, h: 0.5 },
                labels[other].0,
            );
            door.name = format!("door_{}", labels[other].1);
            if locked == Some((r, other))
                && let Some(var) = &key_var
            {
                door.enabled = Some(Expr::Var(var.clone()));
                door.name = format!("locked_door_{}", labels[other].1);
            }
            hotspots.push(door);
        }
        // The key of the locked door, in the room of that door.
        if let (Some((a, _)), Some(var)) = (locked, &key_var)
            && a == r
        {
            let mut key = Hotspot::new(
                text(t.t("scene.navmap.key")),
                HotspotShape::Ellipse { cx: 0.85, cy: 0.82, rx: 0.04, ry: 0.06 },
                labels[r].0,
            );
            key.name = "key".into();
            key.condition = Some(Expr::Unary(UnOp::Not, Box::new(Expr::Var(var.clone()))));
            let found = text(t.t("scene.navmap.key_found"));
            key.actions = vec![
                HotspotAction::SetVar { var: var.clone(), op: OpKind::Set, value: Operand::Lit(Value::Bool(true)) },
                HotspotAction::Notify(Notify { text_id: found, icon: None, sound: None, duration: None, style: None }),
                HotspotAction::Stay,
            ];
            hotspots.push(key);
        }
        // Back: to the first linked room listed before this one.
        if plan.back
            && r > 0
            && let Some(&parent) = doors.iter().filter(|&&o| o < r).min()
        {
            let mut back = Hotspot::new(
                text(t.t("scene.navmap.back")),
                HotspotShape::Rect { x: 0.4, y: 0.88, w: 0.2, h: 0.1 },
                labels[parent].0,
            );
            back.name = "back".into();
            hotspots.push(back);
        }
        if hotspots.is_empty() {
            return Err(t.tf("scene.navmap.err_isolated", &[("room", &room_name(r))]));
        }
        drafts.push(NodeDraft {
            node: Node { id: labels[r].0, kind: NodeKind::Label { name: labels[r].1.clone() } },
            strings: Vec::new(),
        });
        drafts.push(NodeDraft {
            node: Node {
                id: NodeId::new(),
                kind: NodeKind::Hotspots(HotspotMap {
                    background: plan.rooms[r].background,
                    hotspots,
                    ..Default::default()
                }),
            },
            strings,
        });
    }
    Ok(NavOutput { drafts, variable: key_var.map(|v| VarDecl::game(v, Value::Bool(false))) })
}

/// Inserts the house at the insertion point, as one undo step; selects the
/// first room's label.
fn apply(ctx: &mut EditorContext<'_>, out: NavOutput) -> Result<(), String> {
    let (scene, index) = super::edit::insertion_point(ctx).ok_or_else(|| ctx.t("edit.no_scene"))?;
    let first = out.drafts.first().map(|d| d.node.id);
    let label = ctx.t("scene.navmap.title");
    ctx.transaction(&label, |tx| {
        if let Some(decl) = out.variable {
            tx.execute(Command::SetVariable { name: decl.name.clone(), decl: Some(decl), index: None })?;
        }
        for (i, draft) in out.drafts.into_iter().enumerate() {
            for (id, text) in draft.strings {
                tx.execute(Command::SetString { id, text: Some(text) })?;
            }
            tx.execute(Command::InsertNode { scene, index: index + i, node: draft.node })?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    if let Some(first) = first {
        ctx.select(Selection::Node(first));
    }
    Ok(())
}

/// The wizard being filled in.
struct Wizard {
    plan: NavPlan,
    error: Option<String>,
}

impl Wizard {
    fn new(t: &Translator, story: Option<&Story>) -> Self {
        let bg = story.and_then(|s| s.assets.assets.iter().find(|a| a.kind == AssetKind::Image)).map(|a| a.id);
        let rooms = ["scene.navmap.default_room_1", "scene.navmap.default_room_2", "scene.navmap.default_room_3"]
            .iter()
            .map(|k| NavRoom { name: t.t(k), background: bg })
            .collect();
        Self {
            plan: NavPlan {
                rooms,
                doors: [(0, 1), (0, 2)].into_iter().collect(),
                locked: Some((0, 2)),
                back: true,
            },
            error: None,
        }
    }
}

type Shared = Rc<RefCell<Option<Wizard>>>;

/// The menu / palette action and the wizard window.
pub(super) fn register(reg: &mut EditorRegistry) {
    let state: Shared = Rc::default();
    let open = state.clone();
    reg.action(
        ActionDef::new("new_navigation_map", "scene.navmap.action", move |ctx| {
            *open.borrow_mut() = Some(Wizard::new(&ctx.translator(), ctx.story()));
        })
        .icon(icons::HOUSE)
        .category("category.edit")
        .menu_group("menu.edit.insert")
        .menu("edit", 214)
        .enabled(|c| c.story().is_some_and(|s| !s.scenes.is_empty())),
    );
    reg.window(WindowDef::new("navmap", move |egui_ctx, ctx| window(egui_ctx, ctx, &state)));
}

fn window(egui_ctx: &egui::Context, ctx: &mut EditorContext<'_>, state: &Shared) {
    let mut guard = state.borrow_mut();
    let Some(wizard) = guard.as_mut() else { return };
    let mut open = true;
    let mut create = false;
    egui::Window::new(format!("{}  {}", icons::HOUSE, ctx.t("scene.navmap.title")))
        .id(egui::Id::new("orbit.navmap.wizard"))
        .open(&mut open)
        .default_width(520.0)
        .vscroll(true)
        .show(egui_ctx, |ui| {
            if let Some(story) = ctx.story() {
                create = wizard_ui(ui, story, wizard);
            }
        });
    if create {
        match generate_and_apply(ctx, &wizard.plan) {
            Ok(()) => {
                *guard = None;
                return;
            }
            Err(e) => wizard.error = Some(e),
        }
    }
    if !open {
        *guard = None;
    }
}

fn generate_and_apply(ctx: &mut EditorContext<'_>, plan: &NavPlan) -> Result<(), String> {
    let story = ctx.story().ok_or_else(|| ctx.t("edit.no_scene"))?;
    let out = generate(story, plan, &ctx.translator())?;
    apply(ctx, out)
}

/// The wizard's fields; `true` when "Create" is clicked.
fn wizard_ui(ui: &mut Ui, story: &Story, w: &mut Wizard) -> bool {
    ui.label(RichText::new(ui.t("scene.navmap.intro")).small());
    ui.add_space(6.0);
    ui.label(RichText::new(ui.t("scene.navmap.rooms")).strong());
    let mut remove = None;
    let n = w.plan.rooms.len();
    let first_image = story.assets.assets.iter().find(|a| a.kind == AssetKind::Image).map(|a| a.id);
    for (i, room) in w.plan.rooms.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{}.", i + 1));
            ui.add(egui::TextEdit::singleline(&mut room.name).desired_width(140.0));
            match room.background {
                Some(bg) => {
                    if let Some(a) = asset_picker(ui, ("navmap_bg", i), story, AssetKind::Image, bg) {
                        room.background = Some(a);
                    }
                }
                None => {
                    if ui.add_enabled(first_image.is_some(), egui::Button::new(ui.t("scene.navmap.pick_bg"))).clicked() {
                        room.background = first_image;
                    }
                }
            }
            if n > 1 && ui.small_button(icons::TRASH).on_hover_text(ui.t("scene.navmap.remove_room")).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        w.plan.rooms.remove(i);
        // Links are by index: shift those after the removed room.
        let shift = |v: usize| if v > i { v - 1 } else { v };
        w.plan.doors = w
            .plan
            .doors
            .iter()
            .filter(|&&(a, b)| a != i && b != i)
            .map(|&(a, b)| (shift(a), shift(b)))
            .collect();
        w.plan.locked = w.plan.locked.filter(|&(a, b)| a != i && b != i).map(|(a, b)| (shift(a), shift(b)));
    }
    if w.plan.rooms.len() < MAX_ROOMS
        && ui.button(format!("{}  {}", icons::PLUS, ui.t("scene.navmap.add_room"))).clicked()
    {
        let k = w.plan.rooms.len() + 1;
        w.plan.rooms.push(NavRoom { name: ui.tf("scene.navmap.room_n", &[("n", &k)]), background: first_image });
        // Linked to the first room by default.
        w.plan.doors.insert((0, k - 1));
    }
    ui.add_space(6.0);
    ui.label(RichText::new(ui.t("scene.navmap.links")).strong());
    ui.label(RichText::new(ui.t("scene.navmap.links_hint")).small());
    let names: Vec<String> = w.plan.rooms.iter().map(|r| r.name.clone()).collect();
    egui::Grid::new("navmap_links").striped(true).show(ui, |ui| {
        ui.label("");
        for name in names.iter().skip(1) {
            ui.label(RichText::new(name).small());
        }
        ui.end_row();
        for a in 0..names.len().saturating_sub(1) {
            ui.label(RichText::new(&names[a]).small());
            for b in 1..names.len() {
                if b <= a {
                    ui.label("");
                    continue;
                }
                if let Some(on) = fields::checkbox(ui, w.plan.doors.contains(&(a, b)), "") {
                    if on {
                        w.plan.doors.insert((a, b));
                    } else {
                        w.plan.doors.remove(&(a, b));
                        if w.plan.locked == Some((a, b)) {
                            w.plan.locked = None;
                        }
                    }
                }
            }
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    let none = ui.t("scene.navmap.no_lock");
    let door_name = |(a, b): (usize, usize)| format!("{} {} {}", names[a], icons::ARROW_RIGHT, names[b]);
    ui.horizontal_wrapped(|ui| {
        ui.label(ui.t("scene.navmap.locked"));
        egui::ComboBox::from_id_salt("navmap_locked")
            .selected_text(w.plan.locked.map_or(none.clone(), door_name))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut w.plan.locked, None, &none);
                for &d in &w.plan.doors {
                    ui.selectable_value(&mut w.plan.locked, Some(d), door_name(d));
                }
            });
    });
    let back = ui.t("scene.navmap.back_zones");
    if let Some(v) = fields::checkbox(ui, w.plan.back, &back) {
        w.plan.back = v;
    }
    if let Some(e) = &w.error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    ui.add_space(8.0);
    ui.button(format!("{}  {}", icons::CHECK, ui.t("scene.navmap.create"))).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{Scene, SceneId};

    fn story() -> Story {
        let mut s = Story::new("house");
        s.scenes.push(Scene {
            id: SceneId::new(),
            name: "start".into(),
            nodes: vec![Node { id: NodeId::new(), kind: NodeKind::Label { name: "hall".into() } }],
        });
        s
    }

    fn plan() -> NavPlan {
        NavPlan {
            rooms: ["Hall", "Kitchen", "Bed room"]
                .into_iter()
                .map(|n| NavRoom { name: n.into(), background: None })
                .collect(),
            doors: [(0, 1), (0, 2)].into_iter().collect(),
            locked: Some((0, 2)),
            back: true,
        }
    }

    #[test]
    fn a_house_is_a_label_and_a_map_per_room_linked_by_doors() {
        let base = story();
        let out = generate(&base, &plan(), &Translator::default()).unwrap();
        assert_eq!(out.drafts.len(), 6);
        // Labels are unique: "hall" is taken.
        let names: Vec<String> = out
            .drafts
            .iter()
            .filter_map(|d| match &d.node.kind {
                NodeKind::Label { name } => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["hall_2", "kitchen", "bed_room"]);
        let maps: Vec<&HotspotMap> = out
            .drafts
            .iter()
            .filter_map(|d| match &d.node.kind {
                NodeKind::Hotspots(m) => Some(m),
                _ => None,
            })
            .collect();
        // Hall: 2 doors + the key; kitchen and bedroom: a door back + "back".
        assert_eq!(maps[0].hotspots.len(), 3);
        assert_eq!(maps[1].hotspots.len(), 2);
        let locked = maps[0].hotspots.iter().find(|h| h.enabled.is_some()).expect("the locked door");
        assert_eq!(locked.target, out.drafts[4].node.id, "to the bedroom");
        let key = maps[0].hotspots.iter().find(|h| !h.actions.is_empty()).expect("the key");
        assert!(key.condition.is_some());
        assert!(matches!(key.actions.last(), Some(HotspotAction::Stay)));
        let var = out.variable.as_ref().expect("the key variable");
        assert_eq!(var.initial, Value::Bool(false));
        // Every tooltip has its string.
        for (d, m) in out.drafts.iter().skip(1).step_by(2).zip(&maps) {
            assert!(m.hotspots.iter().all(|h| d.strings.iter().any(|(id, _)| *id == h.text_id)));
        }
    }

    #[test]
    fn the_generated_house_validates_and_plays() {
        use std::sync::Arc;
        let mut s = story();
        let out = generate(&s, &plan(), &Translator::default()).unwrap();
        s.variables.push(out.variable.clone().unwrap());
        for d in out.drafts {
            for (id, text) in d.strings {
                s.strings.set(id, text);
            }
            s.scenes[0].nodes.push(d.node);
        }
        let report = orbit_core::validate(&s);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let mut vm = orbit_vm::Vm::from_start(Arc::new(s)).unwrap();
        let orbit_vm::StepResult::Choice { .. } = vm.step() else { panic!("the hall's map") };
        let view = vm.hotspot_choice().unwrap();
        assert_eq!(view.disabled.len(), 1, "the locked door is dimmed");
        // The key is the last clickable zone of the hall.
        let key = view.active.len() - 1;
        vm.choose(key).unwrap();
        let orbit_vm::StepResult::Choice { .. } = vm.step() else { panic!("back on the hall") };
        let view = vm.hotspot_choice().unwrap();
        assert!(view.disabled.is_empty(), "the door is open");
        assert_eq!(view.active.len(), 2, "the key is gone");
    }

    #[test]
    fn isolated_rooms_and_empty_plans_are_refused() {
        let base = story();
        let mut p = plan();
        p.doors.remove(&(0, 2));
        p.locked = None;
        assert!(generate(&base, &p, &Translator::default()).is_err());
        p.rooms.clear();
        assert!(generate(&base, &p, &Translator::default()).is_err());
        assert_eq!(slug("  Salle à manger! "), "salle_à_manger");
        assert_eq!(slug("2nd"), "room_2nd");
    }
}
