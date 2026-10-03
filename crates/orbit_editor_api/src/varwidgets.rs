//! No-code variable widgets: a typed operation ("add 5 to gold") and a
//! condition builder (rows, "all / any" groups), shared by the story
//! inspector and the UI editor.
//!
//! The *model* functions (`blank_op`, `with_var`, `with_kind`, ...) are pure
//! and unit-tested; the widgets only draw them. As everywhere in
//! [`crate::fields`], a widget returns `Some(new_value)` only when the user
//! changed something.

use egui::{ComboBox, RichText, Ui};
use orbit_core::{
    Cmp, Condition, Expr, OpKind, Operand, Story, Value, ValueType, VarOp,
};

use crate::UiText;
use crate::fields::{self, Salt, value_editor};

/// A declared variable: what the pickers offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarInfo {
    pub name: String,
    pub ty: ValueType,
}

/// The variables declared by a story, in declaration order.
pub fn story_vars(story: &Story) -> Vec<VarInfo> {
    story
        .variables
        .iter()
        .map(|v| VarInfo {
            name: v.name.clone(),
            ty: v.initial.value_type(),
        })
        .collect()
}

/// Declared type of `name`.
pub fn type_of(vars: &[VarInfo], name: &str) -> Option<ValueType> {
    vars.iter().find(|v| v.name == name).map(|v| v.ty)
}

fn compatible(a: ValueType, b: ValueType) -> bool {
    ValueType::compatible(a, b)
}

/// Whether `operand` suits an operand of type `ty`.
fn operand_fits(vars: &[VarInfo], operand: &Operand, ty: ValueType) -> bool {
    match operand {
        Operand::Lit(v) => compatible(v.value_type(), ty),
        Operand::Var(n) => type_of(vars, n).is_some_and(|t| compatible(t, ty)),
    }
}

/// Neutral operand for a type (`0`, `false`, empty text, empty collection).
pub fn blank_operand(ty: ValueType) -> Operand {
    Operand::Lit(ty.default_value())
}

/// A blank text map key.
fn blank_key() -> Operand {
    Operand::Lit(Value::Str(String::new()))
}

/// The operation `kind` on a variable of type `ty`, keeping `value` / `key`
/// when they fit and resetting them otherwise.
fn build_op(
    vars: &[VarInfo],
    var: &str,
    ty: ValueType,
    kind: OpKind,
    value: &Operand,
    key: Option<&Operand>,
) -> VarOp {
    let value = match kind.operand_type(ty) {
        Some(t) if !operand_fits(vars, value, t) => blank_operand(t),
        // An ignored operand keeps a stable neutral value.
        None => Operand::Lit(Value::Bool(true)),
        _ => value.clone(),
    };
    let mut op = VarOp::new(var, kind, value);
    if kind.uses_key() {
        op.key = Some(match key {
            Some(k) if operand_fits(vars, k, ValueType::Str) => k.clone(),
            _ => blank_key(),
        });
    }
    op
}

/// A fresh operation on the first variable ("set it to its neutral value";
/// "push" for a list, "set an entry" for a map).
pub fn blank_op(vars: &[VarInfo]) -> VarOp {
    match vars.first() {
        Some(v) => {
            let kind = OpKind::for_type(v.ty)[0];
            build_op(vars, &v.name, v.ty, kind, &blank_operand(v.ty), None)
        }
        None => VarOp::new(String::new(), OpKind::Set, Operand::Lit(Value::Int(0))),
    }
}

/// The operation moved to another variable: the operation and the value are
/// kept when they still make sense for the new type, reset otherwise.
pub fn with_var(op: &VarOp, vars: &[VarInfo], name: &str) -> VarOp {
    let Some(ty) = type_of(vars, name) else {
        return VarOp {
            var: name.to_owned(),
            ..op.clone()
        };
    };
    let kind = if op.op.supports(ty) {
        op.op
    } else {
        OpKind::for_type(ty)[0]
    };
    build_op(vars, name, ty, kind, &op.value, op.key.as_ref())
}

/// The operation with another kind (the value is reset when it does not fit
/// the new kind, e.g. a list index after "push").
pub fn with_kind(op: &VarOp, vars: &[VarInfo], kind: OpKind) -> VarOp {
    match type_of(vars, &op.var) {
        Some(ty) => build_op(vars, &op.var, ty, kind, &op.value, op.key.as_ref()),
        None => VarOp {
            op: kind,
            ..op.clone()
        },
    }
}

/// A fresh comparison row on the first variable (`is true` for a boolean,
/// "contains" for a list, "has key" for a map).
pub fn blank_row(vars: &[VarInfo]) -> Condition {
    match vars.first() {
        Some(v) if v.ty == ValueType::Bool => Condition::is_true(v.name.clone()),
        Some(v) => {
            let cmp = Cmp::for_type(v.ty)[0];
            Condition::Cmp {
                var: v.name.clone(),
                cmp,
                value: blank_operand(cmp.operand_type(v.ty)),
            }
        }
        None => Condition::Cmp {
            var: String::new(),
            cmp: Cmp::Eq,
            value: Operand::Lit(Value::Int(0)),
        },
    }
}

/// A comparison row moved to another variable (comparison and value kept
/// when compatible, reset otherwise).
pub fn row_with_var(row: &Condition, vars: &[VarInfo], name: &str) -> Condition {
    let Condition::Cmp { cmp, value, .. } = row else {
        return row.clone();
    };
    let Some(ty) = type_of(vars, name) else {
        return Condition::Cmp {
            var: name.to_owned(),
            cmp: *cmp,
            value: value.clone(),
        };
    };
    if ty == ValueType::Bool {
        return Condition::is_true(name);
    }
    let cmp = if Cmp::for_type(ty).contains(cmp) {
        *cmp
    } else {
        Cmp::for_type(ty)[0]
    };
    row_with_cmp(vars, name, ty, cmp, value)
}

/// A row `var cmp value` whose value is reset when it does not fit `cmp`.
fn row_with_cmp(vars: &[VarInfo], var: &str, ty: ValueType, cmp: Cmp, value: &Operand) -> Condition {
    let want = cmp.operand_type(ty);
    Condition::Cmp {
        var: var.to_owned(),
        cmp,
        value: if operand_fits(vars, value, want) {
            value.clone()
        } else {
            blank_operand(want)
        },
    }
}

/// `true` for a boolean row of the form `var == true/false` (drawn as
/// "is true" / "is false").
fn bool_row_state(row: &Condition) -> Option<bool> {
    match row {
        Condition::Cmp {
            cmp: Cmp::Eq,
            value: Operand::Lit(Value::Bool(b)),
            ..
        } => Some(*b),
        _ => None,
    }
}

/// Label key of an operation, by variable type.
pub fn op_label_key(kind: OpKind, ty: Option<ValueType>) -> String {
    if kind == OpKind::Add && ty == Some(ValueType::Str) {
        return "varop.op.append".to_owned();
    }
    format!("varop.op.{}", kind.key())
}

fn cmp_key(cmp: Cmp) -> &'static str {
    match cmp {
        Cmp::Eq => "field.cmp.eq",
        Cmp::Ne => "field.cmp.ne",
        Cmp::Gt => "field.cmp.gt",
        Cmp::Ge => "field.cmp.ge",
        Cmp::Lt => "field.cmp.lt",
        Cmp::Le => "field.cmp.le",
        Cmp::Contains => "field.cmp.contains",
        Cmp::NotContains => "field.cmp.not_contains",
        Cmp::HasKey => "field.cmp.has_key",
        Cmp::NotHasKey => "field.cmp.not_has_key",
        Cmp::LenEq => "field.cmp.len_eq",
        Cmp::LenNe => "field.cmp.len_ne",
        Cmp::LenLt => "field.cmp.len_lt",
        Cmp::LenLe => "field.cmp.len_le",
        Cmp::LenGt => "field.cmp.len_gt",
        Cmp::LenGe => "field.cmp.len_ge",
    }
}

// ----------------------------------------------------------------- widgets

fn var_combo(ui: &mut Ui, salt: impl Salt, vars: &[VarInfo], current: &str) -> Option<String> {
    let mut chosen = current.to_owned();
    let text = if current.is_empty() {
        ui.t("varop.choose_var")
    } else {
        current.to_owned()
    };
    ComboBox::from_id_salt((salt, "var"))
        .selected_text(text)
        .show_ui(ui, |ui| {
            for v in vars {
                ui.selectable_value(&mut chosen, v.name.clone(), &v.name);
            }
        });
    (chosen != current).then_some(chosen)
}

/// A literal or another variable (of a compatible type), for a variable of
/// type `ty`.
pub fn operand_editor(
    ui: &mut Ui,
    salt: impl Salt,
    vars: &[VarInfo],
    ty: ValueType,
    operand: &Operand,
) -> Option<Operand> {
    let mut out = None;
    let is_var = matches!(operand, Operand::Var(_));
    // Another variable is only offered when one of a compatible type exists.
    let others: Vec<&VarInfo> = vars.iter().filter(|v| compatible(v.ty, ty)).collect();
    if !others.is_empty() || is_var {
        let mut want_var = is_var;
        let (lit, var) = (ui.t("varop.value"), ui.t("varop.variable"));
        ComboBox::from_id_salt((salt, "kind"))
            .selected_text(if is_var { &var } else { &lit })
            .width(80.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut want_var, false, &lit);
                ui.selectable_value(&mut want_var, true, &var);
            });
        if want_var != is_var {
            out = Some(if want_var {
                match others.first() {
                    Some(v) => Operand::Var(v.name.clone()),
                    None => Operand::Var(String::new()),
                }
            } else {
                blank_operand(ty)
            });
        }
    }
    match operand {
        Operand::Lit(v) => {
            // Edit with the variable's own type (an int variable gets an
            // int drag, a float variable a float drag).
            let shown = match (v, ty) {
                (Value::Int(i), ValueType::Float) => Value::Float(*i as f64),
                (Value::Float(f), ValueType::Int) => Value::Int(f.round() as i64),
                _ => v.clone(),
            };
            if let Some(nv) = value_editor(ui, &shown) {
                out = Some(Operand::Lit(nv));
            }
        }
        Operand::Var(name) => {
            let mut chosen = name.clone();
            ComboBox::from_id_salt((salt, "ovar"))
                .selected_text(if name.is_empty() {
                    ui.t("varop.choose_var")
                } else {
                    name.clone()
                })
                .show_ui(ui, |ui| {
                    for v in &others {
                        ui.selectable_value(&mut chosen, v.name.clone(), &v.name);
                    }
                });
            if chosen != *name {
                out = Some(Operand::Var(chosen));
            }
        }
    }
    out
}

/// The no-code operation editor: variable, operation (only the ones the
/// type offers) and value. With `show_var == false` the variable is chosen
/// elsewhere (a `SetVar` node's own picker).
pub fn varop_editor(
    ui: &mut Ui,
    salt: impl Salt,
    vars: &[VarInfo],
    op: &VarOp,
    show_var: bool,
) -> Option<VarOp> {
    let mut out: Option<VarOp> = None;
    if vars.is_empty() {
        ui.label(
            RichText::new(ui.t("varop.no_vars"))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        return None;
    }
    let ty = type_of(vars, &op.var);
    ui.horizontal_wrapped(|ui| {
        if show_var && let Some(name) = var_combo(ui, salt, vars, &op.var) {
            out = Some(with_var(op, vars, &name));
        }
        let cur = out.clone().unwrap_or_else(|| op.clone());
        let ty = type_of(vars, &cur.var).or(ty);
        if let Some(ty) = ty {
            let kinds = OpKind::for_type(ty);
            let shown = if kinds.contains(&cur.op) { cur.op } else { OpKind::Set };
            let mut chosen = shown;
            let tr = ui.texts();
            let name = |k: OpKind| tr.get(&op_label_key(k, Some(ty)));
            ComboBox::from_id_salt((salt, "op"))
                .selected_text(name(shown))
                .show_ui(ui, |ui| {
                    for &k in kinds {
                        let label = name(k);
                        ui.selectable_value(&mut chosen, k, label);
                    }
                });
            if chosen != cur.op {
                out = Some(with_kind(&cur, vars, chosen));
            }
            let cur = out.clone().unwrap_or_else(|| op.clone());
            if cur.op.uses_key() {
                ui.label(ui.t("varop.key"));
                let key = cur.key.clone().unwrap_or_else(blank_key);
                if let Some(k) = operand_editor(ui, (salt, "key"), vars, ValueType::Str, &key) {
                    out = Some(VarOp {
                        key: Some(k),
                        ..cur.clone()
                    });
                }
                if cur.op == OpKind::MapSet {
                    ui.label(ui.t("varop.to"));
                }
            }
            let cur = out.clone().unwrap_or_else(|| op.clone());
            if let Some(want) = cur.op.operand_type(ty)
                && let Some(v) = operand_editor(ui, (salt, "val"), vars, want, &cur.value)
            {
                out = Some(VarOp { value: v, ..cur });
            }
        }
    });
    let shown = out.as_ref().unwrap_or(op);
    let ty_of = |n: &str| type_of(vars, n);
    if let Some(issue) = shown.issues(&ty_of).first() {
        ui.label(
            RichText::new(ui.tf("varop.issue", &[("message", issue)]))
                .small()
                .color(ui.visuals().warn_fg_color),
        );
    }
    out
}

fn row_editor(ui: &mut Ui, salt: impl Salt, vars: &[VarInfo], row: &mut Condition) -> bool {
    let before = row.clone();
    let Condition::Cmp { var, cmp, value } = row.clone() else {
        return false;
    };
    let ty = type_of(vars, &var);
    ui.horizontal_wrapped(|ui| {
        if let Some(name) = var_combo(ui, salt, vars, &var) {
            *row = row_with_var(row, vars, &name);
            return;
        }
        let Some(ty) = ty else { return };
        if ty == ValueType::Bool
            && let Some(is_true) = bool_row_state(row)
        {
            let mut new_state = is_true;
            let (yes, no) = (ui.t("field.is_true"), ui.t("field.is_false"));
            ComboBox::from_id_salt((salt, "bool"))
                .selected_text(if is_true { &yes } else { &no })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut new_state, true, &yes);
                    ui.selectable_value(&mut new_state, false, &no);
                });
            if new_state != is_true {
                *row = Condition::cmp(var.clone(), Cmp::Eq, new_state);
            }
            return;
        }
        let mut chosen = cmp;
        ComboBox::from_id_salt((salt, "cmp"))
            .selected_text(ui.t(cmp_key(cmp)))
            .show_ui(ui, |ui| {
                for &c in Cmp::for_type(ty) {
                    let label = ui.t(cmp_key(c));
                    ui.selectable_value(&mut chosen, c, label);
                }
            });
        if chosen != cmp {
            // The operand may change type (an element, a key, a length).
            *row = row_with_cmp(vars, &var, ty, chosen, &value);
            return;
        }
        if let Some(v) = operand_editor(ui, (salt, "val"), vars, cmp.operand_type(ty), &value) {
            *row = Condition::Cmp {
                var: var.clone(),
                cmp,
                value: v,
            };
        }
    });
    *row != before
}

/// Deepest group nesting the builder offers.
const MAX_GROUP_DEPTH: usize = 2;

fn group_editor(
    ui: &mut Ui,
    salt: egui::Id,
    vars: &[VarInfo],
    cond: &mut Condition,
    depth: usize,
) -> bool {
    let mut changed = false;
    let (is_all, items) = match cond {
        Condition::All(items) => (true, items),
        Condition::Any(items) => (false, items),
        _ => return false,
    };
    let mut want_all = is_all;
    ui.horizontal(|ui| {
        let (all, any) = (ui.t("varcond.all"), ui.t("varcond.any"));
        ComboBox::from_id_salt((salt, "mode"))
            .selected_text(if is_all { &all } else { &any })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut want_all, true, &all);
                ui.selectable_value(&mut want_all, false, &any);
            });
    });
    let mut remove = None;
    let mut grow = None;
    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: 12,
            ..egui::Margin::ZERO
        })
        .show(ui, |ui| {
            if items.is_empty() {
                ui.label(
                    RichText::new(ui.t(if is_all {
                        "varcond.empty_all"
                    } else {
                        "varcond.empty_any"
                    }))
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
            }
            for (i, item) in items.iter_mut().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .small_button(egui_phosphor::regular::X)
                        .on_hover_text(ui.t("varcond.remove"))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    ui.vertical(|ui| match item {
                        Condition::Cmp { .. } => {
                            changed |= row_editor(ui, salt.with(i), vars, item);
                        }
                        Condition::All(_) | Condition::Any(_) => {
                            changed |= group_editor(ui, salt.with(i), vars, item, depth + 1);
                        }
                        Condition::Expr(e) => {
                            ui.label(RichText::new(e.to_string()).monospace());
                        }
                    });
                });
            }
            ui.horizontal(|ui| {
                if ui
                    .small_button(format!(
                        "{}  {}",
                        egui_phosphor::regular::PLUS,
                        ui.t("varcond.add")
                    ))
                    .clicked()
                {
                    grow = Some(blank_row(vars));
                }
                if depth < MAX_GROUP_DEPTH
                    && ui
                        .small_button(format!(
                            "{}  {}",
                            egui_phosphor::regular::PLUS,
                            ui.t("varcond.add_group")
                        ))
                        .clicked()
                {
                    grow = Some(Condition::Any(vec![blank_row(vars)]));
                }
            });
        });
    if let Some(i) = remove {
        items.remove(i);
        changed = true;
    }
    if let Some(g) = grow {
        items.push(g);
        changed = true;
    }
    if want_all != is_all {
        let items = std::mem::take(items);
        *cond = if want_all {
            Condition::All(items)
        } else {
            Condition::Any(items)
        };
        changed = true;
    }
    changed
}

/// The no-code condition builder: one comparison row, or an "all / any"
/// group of rows and sub-groups, with add / remove buttons. A raw
/// [`Condition::Expr`] is not editable here (see [`expr_condition_editor`]).
pub fn condition_editor(
    ui: &mut Ui,
    salt: impl Salt,
    vars: &[VarInfo],
    cond: &Condition,
) -> Option<Condition> {
    if vars.is_empty() {
        ui.label(
            RichText::new(ui.t("varop.no_vars"))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        return None;
    }
    let mut work = cond.clone();
    let changed = match &mut work {
        Condition::Cmp { .. } => {
            let mut changed = row_editor(ui, salt, vars, &mut work);
            if ui
                .small_button(format!(
                    "{}  {}",
                    egui_phosphor::regular::PLUS,
                    ui.t("varcond.add")
                ))
                .clicked()
            {
                work = Condition::All(vec![work.clone(), blank_row(vars)]);
                changed = true;
            }
            changed
        }
        Condition::All(_) | Condition::Any(_) => {
            let id = ui.make_persistent_id(salt);
            group_editor(ui, id, vars, &mut work, 0)
        }
        Condition::Expr(_) => false,
    };
    // A group of one row reads better as the row itself.
    if changed
        && let Condition::All(items) | Condition::Any(items) = &work
        && let [only @ Condition::Cmp { .. }] = items.as_slice()
    {
        return Some(only.clone());
    }
    changed.then_some(work)
}

/// Visible "x" label of the expression / visual mode switch.
fn mode_button(ui: &mut Ui, key: &str) -> bool {
    ui.small_button(ui.t(key)).clicked()
}

/// Condition editor for an [`Expr`] stored in a story (an `If` node, a choice
/// option): the no-code builder when the expression is a plain combination of
/// comparisons on declared variables, the expression text otherwise or on
/// request ("Expression mode").
pub fn expr_condition_editor(
    ui: &mut Ui,
    salt: impl Salt,
    story: &Story,
    expr: &Expr,
) -> Option<Expr> {
    let vars = story_vars(story);
    let cond = Condition::from_expr(expr);
    let declared = cond.variables().iter().all(|n| type_of(&vars, n).is_some());
    let visual_possible = cond.is_structured() && declared && !vars.is_empty();
    let mode_id = ui.make_persistent_id(("varcond_mode", salt));
    let advanced = ui.data(|d| d.get_temp::<bool>(mode_id)).unwrap_or(false) || !visual_possible;
    if advanced {
        let out = fields::expr_editor(ui, salt, story, expr, true);
        if visual_possible && mode_button(ui, "varop.visual_mode") {
            ui.data_mut(|d| d.insert_temp(mode_id, false));
        }
        out
    } else {
        let out = condition_editor(ui, salt, &vars, &cond).map(|c| c.to_expr());
        if mode_button(ui, "varop.expr_mode") {
            ui.data_mut(|d| d.insert_temp(mode_id, true));
        }
        out
    }
}

/// The value part of a `SetVar` node for a declared variable: the no-code
/// operation (see [`varop_editor`], the variable is picked by the caller), or
/// the expression text in "Expression mode". `None` when the variable is not
/// declared or the expression is not a plain operation and the user did not
/// ask for the visual mode: the caller keeps its own editor.
pub fn setvar_value_editor(
    ui: &mut Ui,
    salt: impl Salt,
    story: &Story,
    var: &str,
    value: &Expr,
) -> Option<Option<Expr>> {
    let vars = story_vars(story);
    type_of(&vars, var)?;
    let op = VarOp::from_expr(var, value);
    let mode_id = ui.make_persistent_id(("setvar_op_mode", salt));
    let advanced = ui.data(|d| d.get_temp::<bool>(mode_id)).unwrap_or(false) || op.is_none();
    if advanced {
        let out = fields::expr_editor(ui, salt, story, value, false);
        if op.is_some() && mode_button(ui, "varop.visual_mode") {
            ui.data_mut(|d| d.insert_temp(mode_id, false));
        }
        if op.is_none() && mode_button(ui, "varop.reset_visual") {
            // Back to the visual mode, with a neutral operation.
            let ty = type_of(&vars, var).unwrap_or(ValueType::Int);
            let blank = build_op(&vars, var, ty, OpKind::for_type(ty)[0], &blank_operand(ty), None);
            ui.data_mut(|d| d.insert_temp(mode_id, false));
            return Some(Some(blank.to_expr()));
        }
        return Some(out);
    }
    let op = op?;
    let out = varop_editor(ui, salt, &vars, &op, false).map(|o| o.to_expr());
    if mode_button(ui, "varop.expr_mode") {
        ui.data_mut(|d| d.insert_temp(mode_id, true));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vec<VarInfo> {
        vec![
            VarInfo { name: "gold".into(), ty: ValueType::Int },
            VarInfo { name: "met".into(), ty: ValueType::Bool },
            VarInfo { name: "name".into(), ty: ValueType::Str },
            VarInfo { name: "ratio".into(), ty: ValueType::Float },
        ]
    }

    #[test]
    fn blank_values_follow_the_first_variable() {
        let v = vars();
        assert_eq!(blank_op(&v), VarOp::set("gold", 0i64));
        assert_eq!(blank_row(&v), Condition::cmp("gold", Cmp::Eq, 0i64));
        let bool_first = vec![v[1].clone()];
        assert_eq!(blank_row(&bool_first), Condition::is_true("met"));
        assert_eq!(blank_op(&[]).var, "");
    }

    #[test]
    fn changing_the_variable_keeps_what_fits_and_resets_the_rest() {
        let v = vars();
        let add = VarOp::new("gold", OpKind::Mul, Operand::Lit(Value::Int(3)));
        // Number to number: everything is kept.
        let moved = with_var(&add, &v, "ratio");
        assert_eq!((moved.op, moved.value.clone()), (OpKind::Mul, Operand::Lit(Value::Int(3))));
        // Number to bool: Multiply is not offered, the value is reset.
        let moved = with_var(&add, &v, "met");
        assert_eq!(moved, VarOp::set("met", false));
        // Number to text: reset too (Set "").
        assert_eq!(with_var(&add, &v, "name"), VarOp::set("name", ""));
        // Add is offered for text (append): kept, the number reset to text.
        let plus = VarOp::new("gold", OpKind::Add, Operand::Lit(Value::Int(1)));
        let moved = with_var(&plus, &v, "name");
        assert_eq!(moved.op, OpKind::Add);
        assert_eq!(moved.value, Operand::Lit(Value::Str(String::new())));
        // A variable operand of the wrong type is replaced.
        let by_var = VarOp::new("gold", OpKind::Add, Operand::Var("ratio".into()));
        assert_eq!(with_var(&by_var, &v, "ratio").value, Operand::Var("ratio".into()));
        assert_eq!(with_var(&by_var, &v, "met"), VarOp::set("met", false));
    }

    #[test]
    fn changing_the_kind_keeps_a_fitting_value() {
        let v = vars();
        let set = VarOp::set("gold", 5i64);
        let div = with_kind(&set, &v, OpKind::Div);
        assert_eq!((div.op, div.value), (OpKind::Div, Operand::Lit(Value::Int(5))));
        let toggle = with_kind(&VarOp::set("met", true), &v, OpKind::Toggle);
        assert_eq!(toggle.op, OpKind::Toggle);
    }

    #[test]
    fn collections_get_their_own_operations_and_rows() {
        use orbit_core::ScalarType;
        let v = vec![
            VarInfo { name: "bag".into(), ty: ValueType::List(ScalarType::Str) },
            VarInfo { name: "gold".into(), ty: ValueType::Map(ScalarType::Int) },
            VarInfo { name: "n".into(), ty: ValueType::Int },
            VarInfo { name: "item".into(), ty: ValueType::Str },
        ];
        // A list starts with "push" and an element operand.
        let push = blank_op(&v);
        assert_eq!(push, VarOp::new("bag", OpKind::Push, Operand::Lit(Value::Str(String::new()))));
        assert!(push.issues(&|n| type_of(&v, n)).is_empty());
        // "remove at" wants an index: the text operand is reset to 0.
        let at = with_kind(&push, &v, OpKind::RemoveAt);
        assert_eq!(at.value, Operand::Lit(Value::Int(0)));
        // A variable operand of the element type is kept for "remove".
        let by_var = VarOp::new("bag", OpKind::Push, Operand::Var("item".into()));
        assert_eq!(with_kind(&by_var, &v, OpKind::Remove).value, Operand::Var("item".into()));
        // Moving to the map: "push" is not offered, "set an entry" with a key.
        let moved = with_var(&push, &v, "gold");
        assert_eq!(moved.op, OpKind::MapSet);
        assert_eq!(moved.key, Some(Operand::Lit(Value::Str(String::new()))));
        assert_eq!(moved.value, Operand::Lit(Value::Int(0)));
        assert!(moved.issues(&|n| type_of(&v, n)).is_empty());
        // Back to a number: the key goes away.
        let back = with_var(&moved, &v, "n");
        assert_eq!(back, VarOp::set("n", 0i64));
        // Clear ignores its operand.
        assert_eq!(with_kind(&push, &v, OpKind::Clear).to_expr().to_string(), "clear(bag)");
        // Rows: "contains" for a list, "has key" for a map, lengths are ints.
        assert_eq!(blank_row(&v), Condition::cmp("bag", Cmp::Contains, ""));
        let row = row_with_var(&blank_row(&v), &v, "gold");
        assert_eq!(row, Condition::cmp("gold", Cmp::HasKey, ""));
        assert_eq!(
            row_with_cmp(&v, "gold", ValueType::Map(ScalarType::Int), Cmp::LenGt, &Operand::Lit("".into())),
            Condition::cmp("gold", Cmp::LenGt, 0i64)
        );
        assert_eq!(cmp_key(Cmp::NotHasKey), "field.cmp.not_has_key");
    }

    #[test]
    fn op_labels_depend_on_the_type() {
        assert_eq!(op_label_key(OpKind::Add, Some(ValueType::Str)), "varop.op.append");
        assert_eq!(op_label_key(OpKind::Add, Some(ValueType::Int)), "varop.op.add");
        assert_eq!(op_label_key(OpKind::Toggle, Some(ValueType::Bool)), "varop.op.toggle");
    }

    #[test]
    fn a_row_follows_its_variable() {
        let v = vars();
        let row = Condition::cmp("gold", Cmp::Ge, 10i64);
        // Number to number keeps the comparison and the value.
        assert_eq!(row_with_var(&row, &v, "ratio"), Condition::cmp("ratio", Cmp::Ge, 10i64));
        // To a boolean: "is true".
        assert_eq!(row_with_var(&row, &v, "met"), Condition::is_true("met"));
        // From a boolean to text: ordering is allowed on text, value reset.
        assert_eq!(
            row_with_var(&Condition::is_true("met"), &v, "name"),
            Condition::Cmp {
                var: "name".into(),
                cmp: Cmp::Eq,
                value: Operand::Lit(Value::Str(String::new()))
            }
        );
        assert_eq!(bool_row_state(&Condition::is_true("met")), Some(true));
        assert_eq!(bool_row_state(&row), None);
    }

    #[test]
    fn a_default_if_condition_is_an_empty_visual_group() {
        // `true` (the default of a new If node) opens in the builder.
        let c = Condition::from_expr(&Expr::lit(true));
        assert_eq!(c, Condition::All(Vec::new()));
        assert!(c.is_structured());
        assert_eq!(c.to_expr(), Expr::lit(true));
    }
}
