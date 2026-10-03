//! Hierarchy and inspector panels of the rig editor.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{ComboBox, DragValue, RichText, TextEdit, Ui, vec2};
use orbit_anim::rig::{Attachment, BoneId, Severity, SlotId};
use orbit_core::AssetKind;
use orbit_editor_api::{EditorContext, icons};

use super::model::{self, DEFAULT_SKIN, OpError, RigDoc};
use super::{State, set_pose_mode};
use crate::builtin::ui_util::{accent, danger, faint, icon_button, list_row, section_title, warning, weak};

// ------------------------------------------------------------------ shared

pub(super) fn empty_state(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(ctx.t("rig.no_rig")).color(weak(ui)));
        ui.add_space(8.0);
        if ui.button(format!("{}  {}", icons::FILE_PLUS, ctx.t("rig.new"))).clicked() {
            st.request_new();
        }
        if ui.button(format!("{}  {}", icons::FOLDER_OPEN, ctx.t("rig.open_file"))).clicked() {
            st.request_open(ctx);
        }
    });
}

/// Toasts the reason when an operation is refused.
fn report<T>(ctx: &mut EditorContext<'_>, r: Result<T, OpError>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            ctx.warn(ctx.t(e.key()));
            None
        }
    }
}

pub(super) fn delete_bone(ctx: &mut EditorContext<'_>, st: &mut State, id: &BoneId) {
    if let Some(doc) = st.doc.as_mut() {
        report(ctx, doc.delete_bone(id));
    }
    st.sync();
}

/// A text field that edits a copy and returns the new text when committed
/// (Enter or focus lost).
fn name_field(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, current: &str) -> Option<String> {
    let id = ui.make_persistent_id(salt);
    let mut buf: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| current.to_owned());
    let r = ui.add(TextEdit::singleline(&mut buf).desired_width(ui.available_width()));
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    } else {
        ui.data_mut(|d| d.remove_temp::<String>(id));
    }
    (r.lost_focus() && buf != current).then_some(buf)
}

fn field(ui: &mut Ui, label: &str, value: &mut f32, speed: f32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized(vec2(70.0, 18.0), egui::Label::new(RichText::new(label).color(weak(ui))));
        changed = ui.add(DragValue::new(value).speed(speed)).changed();
    });
    changed
}

fn bone_name(doc: &RigDoc, id: &BoneId) -> String {
    model::bone(doc.rig(), id).map_or_else(|| id.to_string(), |b| b.name.clone())
}

// --------------------------------------------------------------- hierarchy

enum Action {
    Select(BoneId),
    StartRename(BoneId, String),
    Delete(BoneId),
    Reparent(BoneId, Option<BoneId>),
    AddSlot(BoneId),
    SelectSlot(SlotId),
    DeleteSlot(SlotId),
    MoveSlot(SlotId, i32),
}

pub(super) fn hierarchy(ui: &mut Ui, ctx: &mut EditorContext<'_>, state: &Rc<RefCell<State>>) {
    let mut guard = state.borrow_mut();
    let st = &mut *guard;
    st.begin_frame(ui);
    if st.doc.is_none() {
        empty_state(ui, ctx, st);
        return;
    }
    let mut action: Option<Action> = None;

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let sel = st.selected.clone();
        if ui
            .add_enabled(!st.pose_mode, egui::Button::new(format!("{}  {}", icons::PLUS, ctx.t("rig.add_bone"))))
            .on_hover_text(ctx.t("rig.add_bone_tip"))
            .clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            // Without a mouse: a bone growing from the selected bone's tip
            // (or from the origin when nothing is selected).
            let world = model::world_matrices(doc.rig(), &model::rest_locals(doc.rig()));
            let (head, tip) = match sel.as_ref().and_then(|s| model::bone_index(doc.rig(), s)) {
                Some(i) => {
                    let b = &doc.rig().bones[i];
                    let h = world[i].apply([b.length, 0.0]);
                    let dir = [world[i].a, world[i].b];
                    let n = dir[0].hypot(dir[1]).max(1e-6);
                    (h, [h[0] + dir[0] / n * 50.0, h[1] + dir[1] / n * 50.0])
                }
                None => ([0.0, 0.0], [0.0, -60.0]),
            };
            if let Some(id) = report(ctx, doc.add_bone(sel.as_ref(), head, tip)) {
                st.selected = Some(id);
            }
        }
        if ui
            .add_enabled(st.selected.is_some(), egui::Button::new(icons::TRASH))
            .on_hover_text(ctx.t("rig.delete_bone"))
            .clicked()
            && let Some(id) = st.selected.clone()
        {
            action = Some(Action::Delete(id));
        }
    });
    ui.separator();

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        section_title(ui, &ctx.t("rig.bones"));
        let roots = st.doc.as_ref().map(|d| model::children(d.rig(), None)).unwrap_or_default();
        if roots.is_empty() {
            ui.label(RichText::new(ctx.t("rig.no_bones")).color(weak(ui)));
        }
        for id in roots {
            bone_row(ui, ctx, st, &id, 0, &mut action);
        }
        ui.add_space(8.0);
        section_title(ui, &ctx.t("rig.slots"));
        slots_list(ui, ctx, st, &mut action);
    });

    if let Some(a) = action {
        apply(ctx, st, a);
    }
}

fn bone_row(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State, id: &BoneId, depth: usize, action: &mut Option<Action>) {
    let Some(doc) = st.doc.as_ref() else { return };
    let Some(b) = model::bone(doc.rig(), id) else { return };
    let name = b.name.clone();
    let kids = model::children(doc.rig(), Some(id));
    let selected = st.selected.as_ref() == Some(id);
    let (rect, resp) = list_row(ui, 22.0, selected);
    let x = rect.left() + 6.0 + depth as f32 * 14.0;
    let text_pos = egui::pos2(x + 18.0, rect.center().y);
    let renaming = st.renaming.as_ref().is_some_and(|(r, _)| r == id.as_str());
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icons::BONE,
        egui::FontId::proportional(13.0),
        if selected { accent(ui) } else { weak(ui) },
    );
    if renaming {
        let edit_rect = egui::Rect::from_min_max(egui::pos2(text_pos.x, rect.top() + 1.0), egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0));
        let mut commit = None;
        if let Some((_, buf)) = st.renaming.as_mut() {
            let r = ui.put(edit_rect, TextEdit::singleline(buf));
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if r.lost_focus() {
                commit = Some(ui.input(|i| !i.key_pressed(egui::Key::Escape)));
            }
        }
        if let Some(accepted) = commit {
            let (_, buf) = st.renaming.take().unwrap_or_default();
            if accepted && let Some(doc) = st.doc.as_mut() {
                report(ctx, doc.rename_bone(id, &buf));
            }
        }
    } else {
        ui.painter().text(
            text_pos,
            egui::Align2::LEFT_CENTER,
            &name,
            egui::FontId::proportional(13.0),
            ui.visuals().text_color(),
        );
        if resp.double_clicked() {
            *action = Some(Action::StartRename(id.clone(), name.clone()));
        } else if resp.clicked() {
            *action = Some(Action::Select(id.clone()));
        }
    }
    resp.context_menu(|ui| {
        if ui.button(ctx.t("rig.rename")).clicked() {
            *action = Some(Action::StartRename(id.clone(), name.clone()));
            ui.close();
        }
        if ui.button(ctx.t("rig.add_slot")).clicked() {
            *action = Some(Action::AddSlot(id.clone()));
            ui.close();
        }
        if ui.button(ctx.t("rig.make_root")).clicked() {
            *action = Some(Action::Reparent(id.clone(), None));
            ui.close();
        }
        ui.menu_button(ctx.t("rig.reparent"), |ui| {
            let Some(doc) = st.doc.as_ref() else { return };
            let banned = model::subtree(doc.rig(), id);
            for b in doc.rig().bones.iter().filter(|b| !banned.contains(&b.id)) {
                if ui.button(&b.name).clicked() {
                    *action = Some(Action::Reparent(id.clone(), Some(b.id.clone())));
                    ui.close();
                }
            }
        });
        if ui.button(RichText::new(ctx.t("rig.delete_bone")).color(danger(ui))).clicked() {
            *action = Some(Action::Delete(id.clone()));
            ui.close();
        }
    });
    for k in kids {
        bone_row(ui, ctx, st, &k, depth + 1, action);
    }
}

fn slots_list(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State, action: &mut Option<Action>) {
    let Some(doc) = st.doc.as_ref() else { return };
    let rig = doc.rig();
    if rig.slots.is_empty() {
        ui.label(RichText::new(ctx.t("rig.no_slots")).color(weak(ui)));
    }
    let count = rig.slots.len();
    // Front (last) first: the top of the list is what is drawn on top.
    for (i, s) in rig.slots.iter().enumerate().rev() {
        let selected = st.selected_slot.as_ref() == Some(&s.id);
        let (rect, resp) = list_row(ui, 22.0, selected);
        let bone = bone_name(doc, &s.bone);
        let mut label = format!("{}  {}", icons::IMAGE, s.name);
        label.push_str(&format!("   ({bone})"));
        ui.painter().text(
            egui::pos2(rect.left() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            if s.attachment.is_some() { ui.visuals().text_color() } else { faint(ui) },
        );
        if resp.clicked() {
            *action = Some(Action::SelectSlot(s.id.clone()));
        }
        resp.context_menu(|ui| {
            if ui.add_enabled(i + 1 < count, egui::Button::new(ctx.t("rig.slot_front"))).clicked() {
                *action = Some(Action::MoveSlot(s.id.clone(), 1));
                ui.close();
            }
            if ui.add_enabled(i > 0, egui::Button::new(ctx.t("rig.slot_back"))).clicked() {
                *action = Some(Action::MoveSlot(s.id.clone(), -1));
                ui.close();
            }
            if ui.button(RichText::new(ctx.t("rig.delete_slot")).color(danger(ui))).clicked() {
                *action = Some(Action::DeleteSlot(s.id.clone()));
                ui.close();
            }
        });
    }
}

fn apply(ctx: &mut EditorContext<'_>, st: &mut State, a: Action) {
    match a {
        Action::Select(id) => st.selected = Some(id),
        Action::StartRename(id, name) => {
            st.selected = Some(id.clone());
            st.renaming = Some((id.to_string(), name));
        }
        Action::Delete(id) => delete_bone(ctx, st, &id),
        Action::Reparent(id, parent) => {
            if let Some(doc) = st.doc.as_mut() {
                report(ctx, doc.reparent(&id, parent.as_ref()));
            }
        }
        Action::AddSlot(bone) => {
            if let Some(doc) = st.doc.as_mut()
                && let Some(s) = report(ctx, doc.add_slot(&bone))
            {
                st.selected_slot = Some(s);
            }
        }
        Action::SelectSlot(id) => {
            if let Some(doc) = &st.doc
                && let Some(s) = model::slot(doc.rig(), &id)
            {
                st.selected = Some(s.bone.clone());
            }
            st.selected_slot = Some(id);
        }
        Action::DeleteSlot(id) => {
            if let Some(doc) = st.doc.as_mut() {
                report(ctx, doc.delete_slot(&id));
            }
        }
        Action::MoveSlot(id, d) => {
            if let Some(doc) = st.doc.as_mut() {
                report(ctx, doc.move_slot(&id, d));
            }
        }
    }
}

// --------------------------------------------------------------- inspector

pub(super) fn inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, state: &Rc<RefCell<State>>) {
    let mut guard = state.borrow_mut();
    let st = &mut *guard;
    st.begin_frame(ui);
    if st.doc.is_none() {
        empty_state(ui, ctx, st);
        return;
    }
    st.undo_redo_keys(ui);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        bone_section(ui, ctx, st);
        slot_section(ui, ctx, st);
        skins_section(ui, ctx, st);
        ik_section(ui, ctx, st);
        diagnostics_section(ui, ctx, st);
    });
}

fn bone_section(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    section_title(ui, &ctx.t("rig.bone"));
    let Some(id) = st.selected.clone() else {
        ui.label(RichText::new(ctx.t("rig.no_selection")).color(weak(ui)));
        return;
    };
    let Some(doc) = st.doc.as_ref() else { return };
    let Some(b) = model::bone(doc.rig(), &id).cloned() else { return };
    if let Some(new) = name_field(ui, ("rig_bone_name", &id), &b.name)
        && let Some(doc) = st.doc.as_mut()
    {
        report(ctx, doc.rename_bone(&id, &new));
    }
    // Parent.
    let parent_name = b.parent.as_ref().map_or_else(|| ctx.t("rig.root"), |p| bone_name(st.doc.as_ref().unwrap(), p));
    let mut new_parent: Option<Option<BoneId>> = None;
    ui.horizontal(|ui| {
        ui.add_sized(vec2(70.0, 18.0), egui::Label::new(RichText::new(ctx.t("rig.parent")).color(weak(ui))));
        ComboBox::from_id_salt("rig_parent").selected_text(parent_name).width(ui.available_width()).show_ui(ui, |ui| {
            if ui.selectable_label(b.parent.is_none(), ctx.t("rig.root")).clicked() {
                new_parent = Some(None);
            }
            if let Some(doc) = st.doc.as_ref() {
                let banned = model::subtree(doc.rig(), &id);
                for o in doc.rig().bones.iter().filter(|o| !banned.contains(&o.id)) {
                    if ui.selectable_label(b.parent.as_ref() == Some(&o.id), &o.name).clicked() {
                        new_parent = Some(Some(o.id.clone()));
                    }
                }
            }
        });
    });
    if let Some(p) = new_parent
        && let Some(doc) = st.doc.as_mut()
    {
        report(ctx, doc.reparent(&id, p.as_ref()));
    }

    // Transform: the rest pose, or the pose in pose mode.
    let mut x = st.local_of(&id).unwrap_or(b.rest);
    let before = x;
    ui.label(RichText::new(ctx.t(if st.pose_mode { "rig.pose_transform" } else { "rig.rest_transform" })).small().color(faint(ui)));
    field(ui, &ctx.t("rig.x"), &mut x.x, 0.5);
    field(ui, &ctx.t("rig.y"), &mut x.y, 0.5);
    field(ui, &ctx.t("rig.rotation"), &mut x.rotation, 0.5);
    field(ui, &ctx.t("rig.scale_x"), &mut x.sx, 0.01);
    field(ui, &ctx.t("rig.scale_y"), &mut x.sy, 0.01);
    if x != before {
        st.set_local(&id, x);
    }
    if !st.pose_mode {
        let mut len = b.length;
        if field(ui, &ctx.t("rig.length"), &mut len, 0.5)
            && let Some(doc) = st.doc.as_mut()
        {
            doc.begin_gesture();
            let _ = doc.set_bone_length(&id, len);
        }
    } else if ui.button(ctx.t("rig.reset_bone")).clicked() {
        st.pose.remove(&id);
        st.sync();
        // Force a redraw.
        set_pose_mode(st, false);
        set_pose_mode(st, true);
    }
    ui.horizontal(|ui| {
        if ui.button(format!("{}  {}", icons::IMAGE, ctx.t("rig.add_slot"))).clicked()
            && let Some(doc) = st.doc.as_mut()
            && let Some(s) = report(ctx, doc.add_slot(&id))
        {
            st.selected_slot = Some(s);
        }
        if ui.button(RichText::new(icons::TRASH).color(danger(ui))).on_hover_text(ctx.t("rig.delete_bone")).clicked() {
            delete_bone(ctx, st, &id);
        }
    });
}

/// Pixel size of a project image, cached.
fn image_size(st: &mut State, root: &std::path::Path, path: &str) -> [f32; 2] {
    if let Some(s) = st.image_sizes.get(path) {
        return *s;
    }
    let size = std::fs::read(root.join(path))
        .ok()
        .and_then(|b| orbit_assets::decode_image(&b).ok())
        .map_or([64.0, 64.0], |i| [i.width as f32, i.height as f32]);
    st.image_sizes.insert(path.to_owned(), size);
    size
}

fn slot_section(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(sid) = st.selected_slot.clone() else { return };
    let Some(doc) = st.doc.as_ref() else { return };
    let Some(slot) = model::slot(doc.rig(), &sid).cloned() else { return };
    section_title(ui, &ctx.t("rig.slot"));
    if let Some(new) = name_field(ui, ("rig_slot_name", &sid), &slot.name)
        && let Some(doc) = st.doc.as_mut()
    {
        report(ctx, doc.rename_slot(&sid, &new));
    }
    // Bone.
    let mut new_bone = None;
    ui.horizontal(|ui| {
        ui.add_sized(vec2(70.0, 18.0), egui::Label::new(RichText::new(ctx.t("rig.slot_bone")).color(weak(ui))));
        ComboBox::from_id_salt("rig_slot_bone")
            .selected_text(bone_name(st.doc.as_ref().unwrap(), &slot.bone))
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for o in &st.doc.as_ref().unwrap().rig().bones {
                    if ui.selectable_label(o.id == slot.bone, &o.name).clicked() {
                        new_bone = Some(o.id.clone());
                    }
                }
            });
    });
    if let Some(b) = new_bone
        && let Some(doc) = st.doc.as_mut()
    {
        report(ctx, doc.set_slot_bone(&sid, &b));
    }

    // Picture for the active skin.
    ui.label(RichText::new(format!("{}: {}", ctx.t("rig.picture_of_skin"), st.skin)).small().color(faint(ui)));
    let skin_name = st.skin.clone();
    let current = slot.attachment.as_ref().and_then(|a| {
        let rig = st.doc.as_ref()?.rig();
        rig.skins.iter().find(|s| s.name == skin_name).and_then(|s| s.attachments.get(a)).cloned()
    });
    let current_image = current.as_ref().map(|a| a.image().to_owned());
    let images: Vec<String> = ctx
        .story()
        .map(|s| s.assets.assets.iter().filter(|a| a.kind == AssetKind::Image).map(|a| a.path.clone()).collect())
        .unwrap_or_default();
    let mut picked: Option<String> = None;
    let mut detach = false;
    ComboBox::from_id_salt("rig_slot_image")
        .selected_text(current_image.clone().unwrap_or_else(|| ctx.t("rig.no_picture")))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            if images.is_empty() {
                ui.label(RichText::new(ctx.t("rig.no_images")).color(weak(ui)));
            }
            for p in &images {
                if ui.selectable_label(current_image.as_deref() == Some(p.as_str()), p).clicked() {
                    picked = Some(p.clone());
                }
            }
        });
    if current_image.is_some() && ui.button(ctx.t("rig.remove_picture")).clicked() {
        detach = true;
    }
    let root = st.project_root(ctx);
    if let Some(path) = picked {
        let size = image_size(st, &root, &path);
        if let Some(doc) = st.doc.as_mut() {
            report(ctx, doc.attach_image(&sid, &skin_name, &path, size));
        }
    }
    if detach && let Some(doc) = st.doc.as_mut() {
        report(ctx, doc.detach(&sid, &skin_name));
    }
    if let Some(Attachment::Region(r)) = current {
        let (mut size, mut x) = (r.size, r.xform);
        let before = (size, x);
        field(ui, &ctx.t("rig.width"), &mut size[0], 0.5);
        field(ui, &ctx.t("rig.height"), &mut size[1], 0.5);
        field(ui, &ctx.t("rig.offset_x"), &mut x.x, 0.5);
        field(ui, &ctx.t("rig.offset_y"), &mut x.y, 0.5);
        field(ui, &ctx.t("rig.rotation"), &mut x.rotation, 0.5);
        if (size, x) != before
            && let Some(doc) = st.doc.as_mut()
        {
            doc.begin_gesture();
            let _ = doc.set_region(&sid, &skin_name, size, x);
        }
    }
    ui.horizontal(|ui| {
        let n = st.doc.as_ref().map_or(0, |d| d.rig().slots.len());
        let idx = st.doc.as_ref().and_then(|d| d.rig().slots.iter().position(|s| s.id == sid)).unwrap_or(0);
        if ui.add_enabled(idx + 1 < n, egui::Button::new(icons::ARROW_UP)).on_hover_text(ctx.t("rig.slot_front")).clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            report(ctx, doc.move_slot(&sid, 1));
        }
        if ui.add_enabled(idx > 0, egui::Button::new(icons::ARROW_DOWN)).on_hover_text(ctx.t("rig.slot_back")).clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            report(ctx, doc.move_slot(&sid, -1));
        }
        if ui.button(RichText::new(icons::TRASH).color(danger(ui))).on_hover_text(ctx.t("rig.delete_slot")).clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            report(ctx, doc.delete_slot(&sid));
        }
    });
}

fn skins_section(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    section_title(ui, &ctx.t("rig.skins"));
    let names: Vec<String> = st.doc.as_ref().map(|d| d.rig().skins.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
    if names.is_empty() {
        ui.label(RichText::new(ctx.t("rig.no_skins")).color(weak(ui)));
    }
    for n in &names {
        let selected = &st.skin == n;
        let (rect, resp) = list_row(ui, 22.0, selected);
        ui.painter().text(
            egui::pos2(rect.left() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{}  {n}", icons::PERSON_SIMPLE),
            egui::FontId::proportional(13.0),
            ui.visuals().text_color(),
        );
        if resp.clicked() {
            st.skin = n.clone();
        }
    }
    ui.horizontal(|ui| {
        if ui.button(format!("{}  {}", icons::PLUS, ctx.t("rig.add_skin"))).clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            let base = if doc.rig().skins.is_empty() { DEFAULT_SKIN.to_owned() } else { ctx.t("rig.new_skin_name") };
            if let Some(n) = report(ctx, doc.add_skin(&base)) {
                st.skin = n;
            }
        }
        if icon_button(ui, icons::COPY, &ctx.t("rig.duplicate_skin")).clicked()
            && let Some(doc) = st.doc.as_mut()
            && let Some(n) = report(ctx, doc.duplicate_skin(&st.skin.clone()))
        {
            st.skin = n;
        }
        let can_delete = st.skin != DEFAULT_SKIN && names.len() > 1;
        if ui.add_enabled(can_delete, egui::Button::new(icons::TRASH)).on_hover_text(ctx.t("rig.delete_skin")).clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            report(ctx, doc.delete_skin(&st.skin.clone()));
        }
    });
    if names.contains(&st.skin) {
        let current = st.skin.clone();
        if let Some(new) = name_field(ui, ("rig_skin_name", &current), &current)
            && let Some(doc) = st.doc.as_mut()
            && report(ctx, doc.rename_skin(&current, &new)).is_some()
        {
            st.skin = new.trim().to_owned();
        }
    }
}

fn ik_section(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    section_title(ui, &ctx.t("rig.ik"));
    let Some(doc) = st.doc.as_ref() else { return };
    let list: Vec<(usize, String, f32, bool, bool)> = doc
        .rig()
        .ik
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let chain = c.bones.iter().map(|b| bone_name(doc, b)).collect::<Vec<_>>().join(" > ");
            (i, format!("{chain}  →  {}", bone_name(doc, &c.target)), c.mix, c.bend_positive, c.bones.len() == 2)
        })
        .collect();
    if list.is_empty() {
        ui.label(RichText::new(ctx.t("rig.no_ik")).color(weak(ui)));
    }
    let mut delete = None;
    for (i, label, mix, bend, two) in list {
        ui.label(label);
        let (mut m, mut bnd) = (mix, bend);
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut m, 0.0..=1.0).text(ctx.t("rig.ik_mix")));
            if two {
                ui.checkbox(&mut bnd, ctx.t("rig.ik_bend"));
            }
            if icon_button(ui, icons::TRASH, &ctx.t("rig.delete_ik")).clicked() {
                delete = Some(i);
            }
        });
        if (m, bnd) != (mix, bend)
            && let Some(doc) = st.doc.as_mut()
        {
            doc.begin_gesture();
            let _ = doc.update_ik(i, m, bnd);
        }
    }
    if let Some(i) = delete
        && let Some(doc) = st.doc.as_mut()
    {
        report(ctx, doc.delete_ik(i));
    }

    // New constraint: the selected bone is the end of the chain.
    let Some(doc) = st.doc.as_ref() else { return };
    let Some(sel) = st.selected.clone() else {
        ui.label(RichText::new(ctx.t("rig.ik_select_hint")).small().color(faint(ui)));
        return;
    };
    let parent = model::bone(doc.rig(), &sel).and_then(|b| b.parent.clone());
    let two = st.ik_two && parent.is_some();
    ui.horizontal(|ui| {
        ui.add_enabled(parent.is_some(), egui::Checkbox::new(&mut st.ik_two, ctx.t("rig.ik_two")));
    });
    let chain: Vec<BoneId> = if two { vec![parent.clone().unwrap_or_else(|| sel.clone()), sel.clone()] } else { vec![sel.clone()] };
    let target_name = st.ik_target.as_ref().map_or_else(|| ctx.t("rig.ik_target"), |t| bone_name(st.doc.as_ref().unwrap(), t));
    ui.horizontal(|ui| {
        ComboBox::from_id_salt("rig_ik_target").selected_text(target_name).show_ui(ui, |ui| {
            for o in st.doc.as_ref().unwrap().rig().bones.iter().filter(|o| !chain.contains(&o.id)) {
                if ui.selectable_label(st.ik_target.as_ref() == Some(&o.id), &o.name).clicked() {
                    st.ik_target = Some(o.id.clone());
                }
            }
        });
        let ok = st.ik_target.as_ref().is_some_and(|t| !chain.contains(t));
        if ui.add_enabled(ok, egui::Button::new(format!("{}  {}", icons::PLUS, ctx.t("rig.add_ik")))).clicked()
            && let Some(t) = st.ik_target.clone()
            && let Some(doc) = st.doc.as_mut()
        {
            report(ctx, doc.add_ik(chain.clone(), &t));
        }
    });
}

fn diagnostics_section(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    section_title(ui, &ctx.t("rig.problems"));
    let Some(doc) = st.doc.as_ref() else { return };
    let diags = model::diagnostics(doc.rig());
    let missing = st.link.borrow().missing.clone();
    if diags.is_empty() && missing.is_empty() {
        ui.label(RichText::new(ctx.t("rig.no_problems")).color(weak(ui)));
        return;
    }
    for d in &diags {
        let args: Vec<(&str, &dyn std::fmt::Display)> = d.args.iter().map(|(k, v)| (*k, v as &dyn std::fmt::Display)).collect();
        let (icon, color) = match d.severity {
            Severity::Error => (icons::X_CIRCLE, danger(ui)),
            Severity::Warning => (icons::WARNING, warning(ui)),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(icon).color(color));
            ui.label(ctx.tf(d.key, &args));
        });
    }
    for m in missing {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(icons::WARNING).color(warning(ui)));
            ui.label(ctx.tf("rig.diag.image_missing", &[("path", &m)]));
        });
    }
}
