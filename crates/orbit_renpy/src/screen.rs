//! Ren'Py screens used as navigation: `imagemap` hotspots, `imagebutton`s
//! and `textbutton`s whose actions jump, call, return or set variables.
//! They become a `Hotspots` node (zones) or a `Choice` node (text buttons)
//! where the screen is called. Everything else in a screen is decoration
//! and is left out; a screen with no such button is not converted.

use crate::atl::pairs;
use crate::lexer::{Line, Tok, matching, split_args, tokenize};

/// What a button does when clicked.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Jump(String),
    Call(String),
    Return,
    /// `SetVariable("x", value)`, `SetField(persistent, "x", value)`,
    /// `ToggleVariable("x")` (value `None`).
    Set(String, Option<Vec<Tok>>),
}

/// A clickable item.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub line: usize,
    /// Zone in pixels `(x, y, w, h)` for an imagemap hotspot.
    pub rect: Option<(f32, f32, f32, f32)>,
    /// `imagebutton`: idle and hover pictures (file or image names).
    pub idle: Option<String>,
    pub hover: Option<String>,
    /// Position properties of an image button (`xpos`, `align`...).
    pub placement: Vec<(String, Vec<Tok>)>,
    /// Text of a `textbutton`, tooltip / `alt` of the others.
    pub text: Option<String>,
    pub actions: Vec<Action>,
    /// Condition (`if` blocks around it), Python tokens.
    pub condition: Option<Vec<Tok>>,
    /// `sensitive` expression.
    pub sensitive: Option<Vec<Tok>>,
}

/// A screen, as far as navigation goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Screen {
    pub line: usize,
    /// Imagemap `ground` (or `idle`) picture.
    pub ground: Option<String>,
    /// Imagemap `hover` picture.
    pub hover: Option<String>,
    pub buttons: Vec<Button>,
    /// Buttons left out, `(line, why)`.
    pub skipped: Vec<(usize, String)>,
    /// Statements that make the screen more than navigation (`for`,
    /// `use`...), `(line, what)`.
    pub unsupported: Vec<(usize, String)>,
}

/// The standard screens of the Ren'Py template (`screens.rpy`): the
/// interface the engine provides by itself.
pub const STANDARD: [&str; 27] = [
    "say",
    "input",
    "choice",
    "quick_menu",
    "navigation",
    "main_menu",
    "game_menu",
    "about",
    "save",
    "load",
    "file_slots",
    "preferences",
    "history",
    "help",
    "keyboard_help",
    "mouse_help",
    "gamepad_help",
    "confirm",
    "skip_indicator",
    "notify",
    "nvl",
    "nvl_dialogue",
    "bubble",
    "ctc",
    "window",
    "image_button",
    "text_button",
];

/// `Jump("x")`, `[SetVariable("a", 1), Jump("b")]`... `Err(action)` for an
/// action that does not convert.
pub fn actions(toks: &[Tok]) -> Result<Vec<Action>, String> {
    if toks.first().is_some_and(|t| t.is_op("[")) {
        let close = matching(toks, 0).unwrap_or(toks.len());
        let mut out = Vec::new();
        for a in split_args(&toks[1..close.min(toks.len())]) {
            out.extend(actions(a)?);
        }
        return Ok(out);
    }
    let name = toks.first().and_then(Tok::ident).unwrap_or("");
    let args = if toks.get(1).is_some_and(|t| t.is_op("(")) {
        let close = matching(toks, 1).unwrap_or(toks.len());
        split_args(&toks[2..close.min(toks.len())])
    } else {
        Vec::new()
    };
    let text = |i: usize| match args.get(i) {
        Some([Tok::Str(s)]) => Some(s.clone()),
        _ => None,
    };
    let what = || name.to_owned();
    Ok(match name {
        "Jump" => vec![Action::Jump(text(0).ok_or_else(what)?)],
        "Call" => vec![Action::Call(text(0).ok_or_else(what)?)],
        "Return" => vec![Action::Return],
        "NullAction" => Vec::new(),
        "SetVariable" | "SetLocalVariable" => {
            vec![Action::Set(
                text(0).ok_or_else(what)?,
                Some(args.get(1).ok_or_else(what)?.to_vec()),
            )]
        }
        "SetField"
            if args.first().is_some_and(
                |a| matches!(a, [Tok::Ident(o)] if o == "persistent" || o == "store"),
            ) =>
        {
            let obj = args[0][0].ident().unwrap_or("");
            let field = text(1).ok_or_else(what)?;
            let var = if obj == "persistent" {
                format!("persistent.{field}")
            } else {
                field
            };
            vec![Action::Set(
                var,
                Some(args.get(2).ok_or_else(what)?.to_vec()),
            )]
        }
        "ToggleVariable" => vec![Action::Set(text(0).ok_or_else(what)?, None)],
        _ => return Err(what()),
    })
}

/// The properties of an item: those on its line and on the lines under it.
fn item_props(line: &Line, skip: usize) -> Vec<(String, Vec<Tok>)> {
    let mut toks = tokenize(line.text.trim_end_matches(':'));
    toks.drain(..skip.min(toks.len()));
    let mut out: Vec<(String, Vec<Tok>)> = pairs(&toks)
        .into_iter()
        .map(|(n, v)| (n, v.to_vec()))
        .collect();
    for c in &line.children {
        let t = tokenize(&c.text);
        out.extend(pairs(&t).into_iter().map(|(n, v)| (n, v.to_vec())));
    }
    out
}

fn string(v: &[Tok]) -> Option<String> {
    match v {
        [Tok::Str(s)] => Some(s.clone()),
        // _("...")
        [Tok::Ident(u), Tok::Op(o), Tok::Str(s), Tok::Op(c)]
            if u == "_" && o == "(" && c == ")" =>
        {
            Some(s.clone())
        }
        _ => None,
    }
}

/// `(x, y, w, h)` in pixels.
fn rect(v: &[Tok]) -> Option<(f32, f32, f32, f32)> {
    if !v.first()?.is_op("(") {
        return None;
    }
    let close = matching(v, 0)?;
    let n: Vec<f32> = split_args(&v[1..close])
        .iter()
        .map(|a| match a {
            [Tok::Num(n)] => n.parse().ok(),
            _ => None,
        })
        .collect::<Option<_>>()?;
    match n.as_slice() {
        [x, y, w, h] => Some((*x, *y, *w, *h)),
        _ => None,
    }
}

struct Ctx<'a> {
    screen: &'a mut Screen,
    /// Conditions of the enclosing `if` blocks.
    conds: Vec<Vec<Tok>>,
}

impl Ctx<'_> {
    fn condition(&self) -> Option<Vec<Tok>> {
        if self.conds.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for (i, c) in self.conds.iter().enumerate() {
            if i > 0 {
                out.push(Tok::Ident("and".into()));
            }
            out.push(Tok::Op("(".into()));
            out.extend(c.iter().cloned());
            out.push(Tok::Op(")".into()));
        }
        Some(out)
    }

    fn button(&mut self, line: &Line, kind: &str) {
        let skip = if kind == "textbutton" { 2 } else { 1 };
        let props = item_props(line, skip);
        let toks = tokenize(&line.text);
        let mut b = Button {
            line: line.line,
            rect: None,
            idle: None,
            hover: None,
            placement: Vec::new(),
            text: None,
            actions: Vec::new(),
            condition: self.condition(),
            sensitive: None,
        };
        if kind == "textbutton" {
            b.text = toks
                .get(1)
                .and_then(|t| string(std::slice::from_ref(t)))
                .or_else(|| {
                    // textbutton _("Text")
                    (toks.len() >= 5).then(|| string(&toks[1..5])).flatten()
                });
        }
        if kind == "hotspot" {
            b.rect = toks.get(1..).and_then(|rest| {
                let close = matching(rest, 0)?;
                rect(&rest[..=close])
            });
        }
        let mut has_action = false;
        for (name, value) in props {
            match name.as_str() {
                "action" | "clicked" => {
                    has_action = true;
                    match actions(&value) {
                        Ok(a) => b.actions = a,
                        Err(what) => {
                            self.screen
                                .skipped
                                .push((line.line, format!("action `{what}` has no equivalent")));
                            return;
                        }
                    }
                }
                "idle" | "ground" => b.idle = string(&value),
                "hover" => b.hover = string(&value),
                "auto" => {
                    if let Some(pattern) = string(&value) {
                        b.idle = Some(pattern.replace("%s", "idle"));
                        b.hover = Some(pattern.replace("%s", "hover"));
                    }
                }
                "alt" | "tooltip" => b.text = b.text.take().or_else(|| string(&value)),
                "sensitive" => b.sensitive = Some(value),
                "xpos" | "ypos" | "pos" | "xalign" | "yalign" | "align" | "xcenter" | "ycenter"
                | "xoffset" | "yoffset" | "offset" | "xanchor" | "yanchor" | "anchor"
                | "xysize" | "xsize" | "ysize" | "area" => b.placement.push((name, value)),
                _ => {}
            }
        }
        if !has_action || b.actions.is_empty() {
            self.screen
                .skipped
                .push((line.line, "button without a navigation action".into()));
            return;
        }
        if kind == "hotspot" && b.rect.is_none() {
            self.screen
                .skipped
                .push((line.line, "hotspot without a literal rectangle".into()));
            return;
        }
        self.screen.buttons.push(b);
    }

    fn lines(&mut self, lines: &[Line]) {
        let mut prev_conds: Vec<Vec<Tok>> = Vec::new();
        for line in lines {
            let toks = tokenize(line.text.trim_end_matches(':'));
            let first = toks.first().and_then(Tok::ident).unwrap_or("");
            match first {
                "if" | "elif" | "else" => {
                    let own = toks[1..].to_vec();
                    if first == "if" {
                        prev_conds.clear();
                    }
                    // not (a) and not (b) [and (own)]
                    let mut cond = Vec::new();
                    for p in &prev_conds {
                        if !cond.is_empty() {
                            cond.push(Tok::Ident("and".into()));
                        }
                        cond.push(Tok::Ident("not".into()));
                        cond.push(Tok::Op("(".into()));
                        cond.extend(p.iter().cloned());
                        cond.push(Tok::Op(")".into()));
                    }
                    if first != "else" {
                        if !cond.is_empty() {
                            cond.push(Tok::Ident("and".into()));
                        }
                        cond.extend(own.iter().cloned());
                        prev_conds.push(own);
                    }
                    let pushed = !cond.is_empty();
                    if pushed {
                        self.conds.push(cond);
                    }
                    self.lines(&line.children);
                    if pushed {
                        self.conds.pop();
                    }
                }
                "imagemap" => {
                    for (name, value) in item_props(
                        &Line {
                            children: Vec::new(),
                            ..line.clone()
                        },
                        1,
                    ) {
                        self.imagemap_prop(&name, &value);
                    }
                    for c in &line.children {
                        let t = tokenize(&c.text);
                        match t.first().and_then(Tok::ident) {
                            Some("hotspot") => self.button(c, "hotspot"),
                            Some("if" | "elif" | "else") => self.lines(std::slice::from_ref(c)),
                            Some("hotbar") => self
                                .screen
                                .skipped
                                .push((c.line, "imagemap `hotbar`".into())),
                            _ => {
                                for (name, value) in pairs(&t) {
                                    self.imagemap_prop(&name, value);
                                }
                            }
                        }
                    }
                }
                "imagebutton" | "textbutton" | "hotspot" => self.button(line, first),
                "for" | "use" | "transclude" | "python" | "viewport" | "vpgrid" | "input"
                | "bar" | "vbar" | "timer" | "key" => {
                    self.screen.unsupported.push((line.line, first.to_owned()))
                }
                _ => self.lines(&line.children),
            }
        }
    }

    fn imagemap_prop(&mut self, name: &str, value: &[Tok]) {
        match name {
            "ground" => self.screen.ground = string(value),
            "idle" if self.screen.ground.is_none() => self.screen.ground = string(value),
            "hover" => self.screen.hover = string(value),
            "auto" => {
                if let Some(p) = string(value) {
                    self.screen
                        .ground
                        .get_or_insert_with(|| p.replace("%s", "ground"));
                    self.screen.hover = Some(p.replace("%s", "hover"));
                }
            }
            _ => {}
        }
    }
}

/// Reads a `screen name(...):` statement.
pub fn parse(line: &Line) -> Screen {
    let mut screen = Screen {
        line: line.line,
        ..Screen::default()
    };
    let mut ctx = Ctx {
        screen: &mut screen,
        conds: Vec::new(),
    };
    ctx.lines(&line.children);
    screen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::parse_tree;

    #[test]
    fn imagemap_and_buttons() {
        let src = "screen m():\n    imagemap:\n        ground \"g.png\"\n        hover \"h.png\"\n        hotspot (1, 2, 3, 4) action Jump(\"a\") alt \"A\"\n        if x > 1:\n            hotspot (5, 6, 7, 8) action [SetVariable(\"x\", 2), Return()]\n        hotspot (0, 0, 1, 1) action ShowMenu(\"save\")\n";
        let s = parse(&parse_tree(src, false)[0]);
        assert_eq!(s.ground.as_deref(), Some("g.png"));
        assert_eq!(s.buttons.len(), 2);
        assert_eq!(s.buttons[0].rect, Some((1.0, 2.0, 3.0, 4.0)));
        assert_eq!(s.buttons[0].text.as_deref(), Some("A"));
        assert!(s.buttons[1].condition.is_some());
        assert_eq!(s.buttons[1].actions.len(), 2);
        assert_eq!(s.buttons[1].actions[1], Action::Return);
        assert_eq!(s.skipped.len(), 1);

        let src = "screen b():\n    vbox:\n        textbutton \"Go\" action Jump(\"go\")\n        imagebutton:\n            auto \"d_%s.png\"\n            xpos 10\n            action Call(\"c\")\n";
        let s = parse(&parse_tree(src, false)[0]);
        assert_eq!(s.buttons.len(), 2);
        assert_eq!(s.buttons[0].text.as_deref(), Some("Go"));
        assert_eq!(s.buttons[1].idle.as_deref(), Some("d_idle.png"));
        assert_eq!(s.buttons[1].placement.len(), 1);
    }
}
