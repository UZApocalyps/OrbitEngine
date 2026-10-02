//! Typed field widgets for inspectors (the "kit" the built-in inspector is
//! made of, available to plugins too). Each widget reads the story, shows the
//! current value and returns `Some(new_value)` **only when the user changed
//! it**: the caller turns that into a command, so a widget never mutates
//! anything itself.

use std::fmt::Debug;
use std::hash::Hash;

use egui::{Color32, ComboBox, DragValue, RichText, TextEdit, Ui};
use orbit_core::{
    AssetId, AssetKind, BinOp, CharacterId, Color, CustomData, Expr, NodeId, NodeKind, Position,
    Story, Transition, TransitionKind, UnOp, Value, ValueType,
};

/// Anything usable as an egui id salt, cheap to copy.
pub trait Salt: Hash + Debug + Copy {}

impl<T: Hash + Debug + Copy> Salt for T {}

fn weak(ui: &Ui) -> Color32 {
    ui.visuals().weak_text_color()
}

fn danger(ui: &Ui) -> Color32 {
    ui.visuals().error_fg_color
}

fn warning(ui: &Ui) -> Color32 {
    ui.visuals().warn_fg_color
}

/// Converts a story color to an egui one.
pub fn to_color32(color: Color) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}

/// Display name of a character (`?` when unknown).
pub fn character_name(story: &Story, id: CharacterId) -> String {
    story
        .character(id)
        .and_then(|c| story.text(c.name_id))
        .unwrap_or("?")
        .to_owned()
}

/// A labeled row: a small caption above the widget.
pub fn field<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(6.0);
    ui.label(RichText::new(label).small().color(weak(ui)));
    add(ui)
}

/// Plain text (single line, or several with `rows > 1`). Returns the new
/// text after every edit; the caller should merge such edits into one undo
/// entry (see `EditorContext::exec_merged`). `done` is set when the field
/// lost focus (end of the editing session).
pub fn text_field(
    ui: &mut Ui,
    salt: impl Salt,
    value: &str,
    hint: &str,
    rows: usize,
    done: &mut bool,
) -> Option<String> {
    let mut buffer = value.to_owned();
    let edit = if rows > 1 {
        TextEdit::multiline(&mut buffer).desired_rows(rows)
    } else {
        TextEdit::singleline(&mut buffer)
    };
    let response = ui.add(
        edit.id_salt(salt)
            .desired_width(f32::INFINITY)
            .hint_text(hint),
    );
    if response.lost_focus() {
        *done = true;
    }
    (buffer != value).then_some(buffer)
}

/// Character chooser. `none_label` allows "no character" (narrator).
pub fn character_picker(
    ui: &mut Ui,
    salt: impl Salt,
    story: &Story,
    current: Option<CharacterId>,
    none_label: Option<&str>,
) -> Option<Option<CharacterId>> {
    let selected = match current {
        Some(c) => character_name(story, c),
        None => none_label.unwrap_or("—").to_owned(),
    };
    let mut chosen = current;
    ComboBox::from_id_salt(salt)
        .selected_text(selected)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            if let Some(label) = none_label {
                ui.selectable_value(&mut chosen, None, label);
            }
            for c in &story.characters {
                let name = story.text(c.name_id).unwrap_or("?");
                ui.selectable_value(
                    &mut chosen,
                    Some(c.id),
                    RichText::new(format!("●  {name}")).color(to_color32(c.color)),
                );
            }
        });
    (chosen != current).then_some(chosen)
}

/// Expression (sprite) chooser for a character.
pub fn expression_picker(
    ui: &mut Ui,
    salt: impl Salt,
    story: &Story,
    character: CharacterId,
    current: &str,
) -> Option<String> {
    let names: Vec<&str> = story
        .character(character)
        .map(|c| c.expressions.iter().map(|e| e.name.as_str()).collect())
        .unwrap_or_default();
    let known = names.contains(&current);
    let text = if known {
        RichText::new(current)
    } else {
        RichText::new(format!("{current} (inconnue)")).color(danger(ui))
    };
    let mut chosen = current.to_owned();
    ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            if names.is_empty() {
                ui.label(RichText::new("Ce personnage n'a aucune expression.").color(weak(ui)));
            }
            for name in names {
                ui.selectable_value(&mut chosen, name.to_owned(), name);
            }
        });
    (chosen != current).then_some(chosen)
}

/// Stage position chooser: the three built-in ones and the declared ones.
pub fn position_picker(
    ui: &mut Ui,
    salt: impl Salt,
    story: &Story,
    current: &Position,
) -> Option<Position> {
    let name = |p: &Position| match p {
        Position::Left => "Gauche".to_owned(),
        Position::Center => "Centre".to_owned(),
        Position::Right => "Droite".to_owned(),
        Position::Custom(n) if story.resolve_position(p).is_some() => n.clone(),
        Position::Custom(n) => format!("{n} (inconnue)"),
    };
    let mut chosen = current.clone();
    ComboBox::from_id_salt(salt)
        .selected_text(name(current))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for p in [Position::Left, Position::Center, Position::Right] {
                let label = name(&p);
                ui.selectable_value(&mut chosen, p, label);
            }
            for def in &story.positions {
                ui.selectable_value(
                    &mut chosen,
                    Position::Custom(def.name.clone()),
                    format!("{}  ({:.2}, {:.2})", def.name, def.x, def.y),
                );
            }
        });
    (chosen != *current).then_some(chosen)
}

/// Asset chooser, filtered by kind. Shows the asset's path.
pub fn asset_picker(
    ui: &mut Ui,
    salt: impl Salt,
    story: &Story,
    kind: AssetKind,
    current: AssetId,
) -> Option<AssetId> {
    let known = story.assets.get(current);
    let text = match known {
        Some(a) => RichText::new(&a.path),
        None => RichText::new("(ressource inconnue)").color(danger(ui)),
    };
    let mut chosen = current;
    ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            let mut any = false;
            for a in story.assets.assets.iter().filter(|a| a.kind == kind) {
                any = true;
                ui.selectable_value(&mut chosen, a.id, &a.path);
            }
            if !any {
                ui.label(
                    RichText::new(format!("Aucune ressource de type « {kind} » dans le projet."))
                        .color(weak(ui)),
                );
            }
        });
    if known.is_some_and(|a| a.kind != kind) {
        ui.label(
            RichText::new(format!("Cette ressource n'est pas de type « {kind} »."))
                .small()
                .color(danger(ui)),
        );
    }
    (chosen != current).then_some(chosen)
}

/// Jump target chooser among the labels of the story.
pub fn label_picker(
    ui: &mut Ui,
    salt: impl Salt,
    story: &Story,
    current: Option<NodeId>,
    none_label: Option<&str>,
) -> Option<Option<NodeId>> {
    let describe = |id: NodeId| -> String {
        match story.locate(id) {
            Some((si, ni)) => match &story.scenes[si].nodes[ni].kind {
                NodeKind::Label { name } => format!("{name}  ·  {}", story.scenes[si].name),
                _ => "(pas une étiquette)".to_owned(),
            },
            None => "(étiquette supprimée)".to_owned(),
        }
    };
    let selected = match current {
        Some(id) => describe(id),
        None => none_label.unwrap_or("—").to_owned(),
    };
    let valid = current.is_none_or(|id| {
        matches!(
            story.node(id).map(|n| &n.kind),
            Some(NodeKind::Label { .. })
        )
    });
    let mut chosen = current;
    ComboBox::from_id_salt(salt)
        .selected_text(if valid {
            RichText::new(selected)
        } else {
            RichText::new(selected).color(danger(ui))
        })
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            if let Some(label) = none_label {
                ui.selectable_value(&mut chosen, None, label);
            }
            for scene in &story.scenes {
                for n in &scene.nodes {
                    if let NodeKind::Label { name } = &n.kind {
                        ui.selectable_value(
                            &mut chosen,
                            Some(n.id),
                            format!("{name}  ·  {}", scene.name),
                        );
                    }
                }
            }
        });
    (chosen != current).then_some(chosen)
}

/// Seconds, as a drag value.
pub fn seconds_field(ui: &mut Ui, value: f32) -> Option<f32> {
    let mut v = value;
    ui.add(
        DragValue::new(&mut v)
            .speed(0.05)
            .range(0.0..=3600.0)
            .suffix(" s")
            .max_decimals(2),
    );
    (v != value).then_some(v)
}

/// Optional transition: none, fade to a color, or dissolve, with a duration.
pub fn transition_editor(
    ui: &mut Ui,
    salt: impl Salt,
    current: Option<Transition>,
) -> Option<Option<Transition>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Cut,
        Dissolve,
        Fade,
    }
    let mode = match current {
        None => Mode::Cut,
        Some(Transition {
            kind: TransitionKind::Dissolve,
            ..
        }) => Mode::Dissolve,
        Some(_) => Mode::Fade,
    };
    let mut new_mode = mode;
    ComboBox::from_id_salt(salt)
        .selected_text(match mode {
            Mode::Cut => "Aucune (coupe)",
            Mode::Dissolve => "Fondu enchaîné",
            Mode::Fade => "Fondu vers une couleur",
        })
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut new_mode, Mode::Cut, "Aucune (coupe)");
            ui.selectable_value(&mut new_mode, Mode::Dissolve, "Fondu enchaîné");
            ui.selectable_value(&mut new_mode, Mode::Fade, "Fondu vers une couleur");
        });
    let duration = current.map_or(0.5, |t| t.duration);
    if new_mode != mode {
        return Some(match new_mode {
            Mode::Cut => None,
            Mode::Dissolve => Some(Transition::dissolve(duration)),
            Mode::Fade => Some(Transition::fade(Color::BLACK, duration)),
        });
    }
    let t = current?;
    let mut out = t;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Durée").small().color(weak(ui)));
        if let Some(d) = seconds_field(ui, t.duration) {
            out.duration = d;
        }
        if let TransitionKind::Fade(c) = t.kind {
            ui.label(RichText::new("Couleur").small().color(weak(ui)));
            let mut rgb = [c.r, c.g, c.b];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                out.kind = TransitionKind::Fade(Color::rgb(rgb[0], rgb[1], rgb[2]));
            }
        }
    });
    (out != t).then_some(Some(out))
}

/// Color button.
pub fn color_field(ui: &mut Ui, color: Color) -> Option<Color> {
    let mut rgb = [color.r, color.g, color.b];
    ui.color_edit_button_srgb(&mut rgb)
        .changed()
        .then(|| Color::rgb(rgb[0], rgb[1], rgb[2]))
}

// --------------------------------------------------------------- expressions

/// A variable value editor (type chosen by the value itself).
pub fn value_editor(ui: &mut Ui, value: &Value) -> Option<Value> {
    match value {
        Value::Bool(b) => {
            let mut v = *b;
            let label = if v { "vrai" } else { "faux" };
            ui.checkbox(&mut v, label);
            (v != *b).then_some(Value::Bool(v))
        }
        Value::Int(i) => {
            let mut v = *i;
            ui.add(DragValue::new(&mut v).speed(0.2));
            (v != *i).then_some(Value::Int(v))
        }
        Value::Float(f) => {
            let mut v = *f;
            ui.add(DragValue::new(&mut v).speed(0.05));
            (v != *f).then_some(Value::Float(v))
        }
        Value::Str(s) => {
            let mut v = s.clone();
            ui.add(TextEdit::singleline(&mut v).desired_width(120.0));
            (v != *s).then_some(Value::Str(v))
        }
    }
}

/// Default value of a type.
pub fn default_value(ty: ValueType) -> Value {
    match ty {
        ValueType::Bool => Value::Bool(false),
        ValueType::Int => Value::Int(0),
        ValueType::Float => Value::Float(0.0),
        ValueType::Str => Value::Str(String::new()),
    }
}

/// Human name of a value type.
pub fn type_name(ty: ValueType) -> &'static str {
    match ty {
        ValueType::Bool => "Oui / non",
        ValueType::Int => "Nombre entier",
        ValueType::Float => "Nombre décimal",
        ValueType::Str => "Texte",
    }
}

#[derive(Clone)]
struct ExprBuffer {
    text: String,
    /// The expression the buffer was last in sync with.
    source: Expr,
    error: Option<String>,
}

/// Expression editor: a text field with a parser and an inline error. Only a
/// text that parses is returned (`Some(expr)`); a text with a syntax error
/// leaves the story unchanged and shows the problem under the field.
/// `want_bool` additionally flags a non-boolean condition.
pub fn expr_editor(
    ui: &mut Ui,
    salt: impl Salt,
    story: &Story,
    expr: &Expr,
    want_bool: bool,
) -> Option<Expr> {
    let id = ui.make_persistent_id(("orbit_expr", salt));
    let mut buf: ExprBuffer = ui
        .data(|d| d.get_temp::<ExprBuffer>(id))
        .unwrap_or_else(|| ExprBuffer {
            text: expr.to_string(),
            source: expr.clone(),
            error: None,
        });
    if buf.source != *expr {
        // Changed from elsewhere (undo, another field): follow it.
        buf = ExprBuffer {
            text: expr.to_string(),
            source: expr.clone(),
            error: None,
        };
    }
    let response = ui.add(
        TextEdit::singleline(&mut buf.text)
            .id_salt(id)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace)
            .hint_text("ex. affinité > 2 && !vu"),
    );
    let mut out = None;
    if response.changed() {
        match Expr::parse(&buf.text) {
            Ok(parsed) => {
                buf.error = None;
                if parsed != *expr {
                    buf.source = parsed.clone();
                    out = Some(parsed);
                }
            }
            Err(e) => buf.error = Some(format!("{} (position {})", e.message, e.pos)),
        }
    }
    if !response.has_focus() && buf.error.is_some() && !response.changed() {
        // Leaving the field with a broken text: show the stored expression again.
        buf.text = expr.to_string();
        buf.error = None;
    }
    if let Some(error) = &buf.error {
        ui.label(RichText::new(error).small().color(danger(ui)));
    } else {
        let shown = out.as_ref().unwrap_or(expr);
        match shown.type_of(&|name| var_type(story, name)) {
            Err(e) => {
                ui.label(RichText::new(e.0).small().color(danger(ui)));
            }
            Ok(Some(t)) if want_bool && t != ValueType::Bool => {
                ui.label(
                    RichText::new(format!("Une condition doit être vrai / faux (ici : {t})."))
                        .small()
                        .color(danger(ui)),
                );
            }
            Ok(_) => {
                for v in shown.variables() {
                    if !story.variables.iter().any(|d| d.name == v) {
                        ui.label(
                            RichText::new(format!("Variable « {v} » non déclarée."))
                                .small()
                                .color(warning(ui)),
                        );
                        break;
                    }
                }
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(id, buf));
    out
}

/// Declared type of a variable.
pub fn var_type(story: &Story, name: &str) -> Option<ValueType> {
    story
        .variables
        .iter()
        .find(|v| v.name == name)
        .map(|v| v.initial.value_type())
}

const COMPARISONS: [(BinOp, &str); 6] = [
    (BinOp::Eq, "est égal à"),
    (BinOp::Ne, "est différent de"),
    (BinOp::Gt, "est supérieur à"),
    (BinOp::Ge, "est supérieur ou égal à"),
    (BinOp::Lt, "est inférieur à"),
    (BinOp::Le, "est inférieur ou égal à"),
];

/// Whether `expr` has the shape the simple builder understands:
/// `var`, `!var` or `var <op> literal`.
pub fn is_simple_condition(expr: &Expr) -> bool {
    match expr {
        Expr::Var(_) => true,
        Expr::Unary(UnOp::Not, e) => matches!(**e, Expr::Var(_)),
        Expr::Binary(op, l, r) => {
            COMPARISONS.iter().any(|(o, _)| o == op)
                && matches!(**l, Expr::Var(_))
                && matches!(**r, Expr::Lit(_))
        }
        _ => false,
    }
}

/// Condition builder for the simple cases (a variable, a comparison, a
/// literal), without writing an expression. Returns the new expression when
/// the user changed something.
pub fn simple_condition_editor(
    ui: &mut Ui,
    salt: impl Salt,
    story: &Story,
    expr: &Expr,
) -> Option<Expr> {
    let (var, op, lit): (String, Option<BinOp>, Value) = match expr {
        Expr::Var(v) => (v.clone(), None, Value::Bool(true)),
        Expr::Unary(UnOp::Not, e) => match &**e {
            Expr::Var(v) => (v.clone(), None, Value::Bool(false)),
            _ => return None,
        },
        Expr::Binary(op, l, r) => match (&**l, &**r) {
            (Expr::Var(v), Expr::Lit(value)) => (v.clone(), Some(*op), value.clone()),
            _ => return None,
        },
        _ => return None,
    };
    let ty = var_type(story, &var);
    let mut new_var = var.clone();
    let mut new_op = op;
    let mut new_lit = lit.clone();
    let mut negated = matches!(expr, Expr::Unary(..));

    ui.horizontal_wrapped(|ui| {
        ComboBox::from_id_salt((salt, "var"))
            .selected_text(if var.is_empty() { "variable…" } else { &var })
            .show_ui(ui, |ui| {
                for v in &story.variables {
                    ui.selectable_value(&mut new_var, v.name.clone(), &v.name);
                }
            });
        if ty == Some(ValueType::Bool) {
            let mut is_true = !negated;
            ComboBox::from_id_salt((salt, "bool"))
                .selected_text(if is_true { "est vrai" } else { "est faux" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut is_true, true, "est vrai");
                    ui.selectable_value(&mut is_true, false, "est faux");
                });
            negated = !is_true;
            new_op = None;
        } else {
            let current = new_op.unwrap_or(BinOp::Eq);
            let mut chosen = current;
            ComboBox::from_id_salt((salt, "op"))
                .selected_text(
                    COMPARISONS
                        .iter()
                        .find(|(o, _)| *o == current)
                        .map_or("?", |(_, n)| n),
                )
                .show_ui(ui, |ui| {
                    for (o, name) in COMPARISONS {
                        ui.selectable_value(&mut chosen, o, name);
                    }
                });
            new_op = Some(chosen);
            if let Some(v) = value_editor(ui, &new_lit) {
                new_lit = v;
            }
        }
    });
    if new_var != var && let Some(t) = var_type(story, &new_var) {
        // A new variable of another type: reset the operator and the value.
        new_lit = default_value(t);
        if t == ValueType::Bool {
            new_op = None;
            negated = false;
        } else if new_op.is_none() {
            new_op = Some(BinOp::Eq);
        }
    }
    let rebuilt = match new_op {
        Some(op) => Expr::binary(op, Expr::var(new_var), Expr::Lit(new_lit)),
        None if negated => Expr::negate(Expr::var(new_var)),
        None => Expr::var(new_var),
    };
    (rebuilt != *expr).then_some(rebuilt)
}

// ---------------------------------------------------------------- custom data

/// Generic editor of the open data of a plugin node, used when the plugin
/// provides no inspector of its own. Returns the new value after a change.
pub fn custom_data_editor(ui: &mut Ui, salt: egui::Id, data: &CustomData) -> Option<CustomData> {
    match data {
        CustomData::Null => {
            let mut out = None;
            ui.horizontal(|ui| {
                ui.label(RichText::new("vide").color(weak(ui)));
                ui.menu_button("Définir…", |ui| {
                    for (name, value) in [
                        ("Texte", CustomData::Str(String::new())),
                        ("Entier", CustomData::Int(0)),
                        ("Décimal", CustomData::Float(0.0)),
                        ("Oui / non", CustomData::Bool(false)),
                        ("Liste", CustomData::List(Vec::new())),
                        ("Table", CustomData::Map(Vec::new())),
                    ] {
                        if ui.button(name).clicked() {
                            out = Some(value);
                            ui.close();
                        }
                    }
                });
            });
            out
        }
        CustomData::Bool(b) => {
            let mut v = *b;
            ui.checkbox(&mut v, "");
            (v != *b).then_some(CustomData::Bool(v))
        }
        CustomData::Int(i) => {
            let mut v = *i;
            ui.add(DragValue::new(&mut v).speed(0.2));
            (v != *i).then_some(CustomData::Int(v))
        }
        CustomData::Float(f) => {
            let mut v = *f;
            ui.add(DragValue::new(&mut v).speed(0.05));
            (v != *f).then_some(CustomData::Float(v))
        }
        CustomData::Str(s) => {
            let mut v = s.clone();
            ui.add(
                TextEdit::singleline(&mut v)
                    .id_salt(salt)
                    .desired_width(f32::INFINITY),
            );
            (v != *s).then_some(CustomData::Str(v))
        }
        CustomData::List(items) => {
            let mut out: Option<CustomData> = None;
            let mut remove = None;
            for (i, item) in items.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("[{i}]")).color(weak(ui)));
                    if let Some(new) = custom_data_editor(ui, salt.with(i), item) {
                        let mut list = items.clone();
                        list[i] = new;
                        out = Some(CustomData::List(list));
                    }
                    if ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                let mut list = items.clone();
                list.remove(i);
                out = Some(CustomData::List(list));
            }
            if ui.small_button("+ élément").clicked() {
                let mut list = items.clone();
                list.push(CustomData::Null);
                out = Some(CustomData::List(list));
            }
            out
        }
        CustomData::Map(entries) => {
            let mut out: Option<CustomData> = None;
            let mut remove = None;
            for (i, (key, value)) in entries.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{key} :")).color(weak(ui)));
                    if let Some(new) = custom_data_editor(ui, salt.with(key), value) {
                        let mut map = entries.clone();
                        map[i].1 = new;
                        out = Some(CustomData::Map(map));
                    }
                    if ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                let mut map = entries.clone();
                map.remove(i);
                out = Some(CustomData::Map(map));
            }
            if ui.small_button("+ entrée").clicked() {
                let mut map = entries.clone();
                let mut n = 1;
                let key = loop {
                    let k = format!("cle{n}");
                    if map.iter().all(|(e, _)| *e != k) {
                        break k;
                    }
                    n += 1;
                };
                map.push((key, CustomData::Null));
                out = Some(CustomData::Map(map));
            }
            out
        }
    }
}
