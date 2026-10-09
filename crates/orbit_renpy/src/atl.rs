//! Ren'Py transforms (ATL and the `Transform(...)` / `Position(...)`
//! calls) to the staging and the keyframe tracks of the engine.
//!
//! Only the common subset is read: positions (`xalign`, `xpos`, `align`,
//! `pos`, `xcenter`, offsets), `alpha`, `zoom` / `xzoom` / `yzoom`,
//! `rotate`, the warpers (`linear`, `ease`, `easein`, `easeout` and their
//! `_quad`... variants), `pause`, `repeat` and a trailing `block:`. The rest
//! is listed in [`Atl::ignored`] (the caller reports it).
//!
//! Coordinates. The engine anchors a sprite by its bottom-center point in
//! normalized stage coordinates, where Ren'Py positions an anchor that
//! depends on the property. The mapping keeps the built-in positions
//! aligned: `xalign a` -> `x = 0.25 + 0.5 a` (`left` = 0.25, `center` =
//! 0.5, `right` = 0.75, the engine's own `Left` / `Center` / `Right`),
//! `yalign b` -> `y = 0.5 + 0.5 b` (`1.0`: standing on the bottom edge);
//! `xpos` / `ypos` / `xcenter` place the anchor directly (pixels are divided
//! by the screen size).

use orbit_core::{Curve, Ease};

use crate::lexer::{Line, Tok, matching, split_args, tokenize};

/// An animatable property, in engine units (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prop {
    /// Anchor x (stage units).
    X,
    /// Anchor y (stage units).
    Y,
    XOff,
    YOff,
    Alpha,
    Zoom,
    XZoom,
    YZoom,
    /// Degrees.
    Rotate,
}

/// Screen size in pixels (`gui.init`, `config.screen_width`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub w: f32,
    pub h: f32,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            w: 1920.0,
            h: 1080.0,
        }
    }
}

/// One change of a property: instant (`dur == 0`) or interpolated.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub t0: f32,
    pub dur: f32,
    pub curve: Curve,
    pub prop: Prop,
    pub value: f32,
}

/// How the transform repeats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    Once,
    Times(u32),
    Forever,
}

/// A parsed transform.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Atl {
    pub steps: Vec<Step>,
    /// Seconds of one pass.
    pub duration: f32,
    pub repeat: Repeat,
    /// What was not converted (property or statement names), in order,
    /// without duplicates.
    pub ignored: Vec<String>,
}

/// Keyframes of one property: `(time, value, ease of the segment that
/// starts at this key)`.
pub type Keys = Vec<(f32, f32, Curve)>;

impl Atl {
    /// Whether something moves over time (a timeline is needed).
    pub fn is_animated(&self) -> bool {
        self.steps.iter().any(|s| s.dur > 0.0 || s.t0 > 0.0) || self.repeat != Repeat::Once
    }

    /// Whether the transform only places the sprite (a stage position).
    pub fn is_position_only(&self) -> bool {
        !self.steps.is_empty()
            && !self.is_animated()
            && self
                .steps
                .iter()
                .all(|s| matches!(s.prop, Prop::X | Prop::Y))
    }

    fn note(&mut self, what: &str) {
        if !self.ignored.iter().any(|i| i == what) {
            self.ignored.push(what.to_owned());
        }
    }

    /// Appends `other` after this transform (`at a, b`: the later one wins
    /// the properties both set).
    pub fn then(&mut self, other: Atl) {
        let shift = self.duration;
        self.steps.extend(other.steps.into_iter().map(|mut s| {
            s.t0 += shift;
            s
        }));
        self.duration += other.duration;
        if other.repeat != Repeat::Once {
            self.repeat = other.repeat;
        }
        for i in other.ignored {
            self.note(&i);
        }
    }

    /// The keyframes of `prop` starting from `base`, and its final value.
    /// `None` when the transform never touches it.
    pub fn keys(&self, prop: Prop, base: f32) -> Option<(Keys, f32)> {
        let mut keys: Keys = vec![(0.0, base, Curve::LINEAR)];
        let mut cur = base;
        let mut touched = false;
        for s in self.steps.iter().filter(|s| s.prop == prop) {
            touched = true;
            let last_t = keys.last().map_or(0.0, |k| k.0);
            if s.dur <= 0.0 {
                if s.t0 <= last_t {
                    if let Some(k) = keys.last_mut() {
                        k.1 = s.value;
                    }
                } else {
                    // Hold, then jump.
                    if let Some(k) = keys.last_mut() {
                        k.2 = Curve::Preset(Ease::Step);
                    }
                    keys.push((s.t0, s.value, Curve::LINEAR));
                }
            } else {
                if s.t0 > last_t {
                    keys.push((s.t0, cur, Curve::LINEAR));
                }
                if let Some(k) = keys.last_mut() {
                    k.2 = s.curve.clone();
                }
                keys.push((s.t0 + s.dur, s.value, Curve::LINEAR));
            }
            cur = s.value;
        }
        touched.then_some((keys, cur))
    }
}

/// The curve of a Ren'Py warper name (`linear`, `ease`, `easein_quad`...).
pub fn warper(name: &str) -> Option<Curve> {
    let (family, kind) = match name.split_once('_') {
        Some((f, k)) => (f, Some(k)),
        None => (name, None),
    };
    use Ease::*;
    let pick = |inn: Ease, out: Ease, both: Ease| match family {
        // Ren'Py `easein` starts fast and slows down: an "out" ease.
        "easein" => Some(out),
        "easeout" => Some(inn),
        "ease" => Some(both),
        _ => None,
    };
    let ease = match (family, kind) {
        ("linear", None) => Linear,
        (_, None) => pick(InSine, OutSine, InOutSine)?,
        (_, Some("quad")) => pick(InQuad, OutQuad, InOutQuad)?,
        (_, Some("cubic")) => pick(InCubic, OutCubic, InOutCubic)?,
        (_, Some("quart")) => pick(InQuart, OutQuart, InOutQuart)?,
        (_, Some("quint")) => pick(InQuint, OutQuint, InOutQuint)?,
        (_, Some("expo")) => pick(InExpo, OutExpo, InOutExpo)?,
        (_, Some("circ")) => pick(InCirc, OutCirc, InOutCirc)?,
        (_, Some("back")) => pick(InBack, OutBack, InOutBack)?,
        (_, Some("elastic")) => pick(InElastic, OutElastic, InOutElastic)?,
        (_, Some("bounce")) => pick(InBounce, OutBounce, InOutBounce)?,
        _ => return None,
    };
    Some(Curve::Preset(ease))
}

/// A number token, possibly negative; whether it was written as an integer
/// (pixels for positions).
fn number(toks: &[Tok]) -> Option<(f32, bool)> {
    match toks {
        [Tok::Num(n)] => Some((n.parse().ok()?, !n.contains('.'))),
        [Tok::Op(m), Tok::Num(n)] if m == "-" => Some((-n.parse::<f32>().ok()?, !n.contains('.'))),
        [Tok::Op(p), Tok::Num(n)] if p == "+" => Some((n.parse().ok()?, !n.contains('.'))),
        // absolute(100) / int(...) / float(...)
        [Tok::Ident(f), Tok::Op(o), inner @ .., Tok::Op(c)]
            if o == "(" && c == ")" && matches!(f.as_str(), "absolute" | "int" | "float") =>
        {
            number(inner).map(|(v, int)| (v, int || f == "absolute" || f == "int"))
        }
        _ => None,
    }
}

/// `(a, b)`.
fn pair(toks: &[Tok]) -> Option<((f32, bool), (f32, bool))> {
    if !toks.first()?.is_op("(") || matching(toks, 0)? != toks.len() - 1 {
        return None;
    }
    let args = split_args(&toks[1..toks.len() - 1]);
    match args.as_slice() {
        [a, b] => Some((number(a)?, number(b)?)),
        _ => None,
    }
}

/// Engine values of one Ren'Py property. `Err(())`: not converted.
pub fn property(name: &str, value: &[Tok], screen: Screen) -> Result<Vec<(Prop, f32)>, ()> {
    let pos = |(v, int): (f32, bool), size: f32| if int { v / size } else { v };
    let one = || number(value).ok_or(());
    Ok(match name {
        "xalign" => vec![(Prop::X, 0.25 + 0.5 * one()?.0)],
        "yalign" => vec![(Prop::Y, 0.5 + 0.5 * one()?.0)],
        "align" => {
            let (a, b) = pair(value).ok_or(())?;
            vec![(Prop::X, 0.25 + 0.5 * a.0), (Prop::Y, 0.5 + 0.5 * b.0)]
        }
        "xpos" | "xcenter" => vec![(Prop::X, pos(one()?, screen.w))],
        "ypos" => vec![(Prop::Y, pos(one()?, screen.h))],
        "pos" => {
            let (a, b) = pair(value).ok_or(())?;
            vec![(Prop::X, pos(a, screen.w)), (Prop::Y, pos(b, screen.h))]
        }
        "xoffset" => vec![(Prop::XOff, one()?.0 / screen.w)],
        "yoffset" => vec![(Prop::YOff, one()?.0 / screen.h)],
        "offset" => {
            let (a, b) = pair(value).ok_or(())?;
            vec![(Prop::XOff, a.0 / screen.w), (Prop::YOff, b.0 / screen.h)]
        }
        "alpha" => vec![(Prop::Alpha, one()?.0.clamp(0.0, 1.0))],
        "zoom" => vec![(Prop::Zoom, one()?.0)],
        "xzoom" => vec![(Prop::XZoom, one()?.0)],
        "yzoom" => vec![(Prop::YZoom, one()?.0)],
        "rotate" => vec![(Prop::Rotate, one()?.0)],
        _ => return Err(()),
    })
}

/// Properties Ren'Py needs for its own layout that change nothing here.
const HARMLESS: [&str; 6] = [
    "subpixel",
    "transform_anchor",
    "rotate_pad",
    "nearest",
    "xanchor",
    "yanchor",
];

/// Splits `prop value prop value...` (values: a number, `-n`, a tuple, a
/// call or a name).
pub fn pairs(toks: &[Tok]) -> Vec<(String, &[Tok])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let Some(name) = toks[i].ident() else {
            i += 1;
            continue;
        };
        let start = i + 1;
        let mut end = start;
        match toks.get(start) {
            Some(Tok::Op(o)) if o == "(" || o == "[" => {
                end = matching(toks, start).map_or(toks.len(), |e| e + 1);
            }
            Some(Tok::Op(o)) if o == "-" || o == "+" => end = (start + 2).min(toks.len()),
            Some(Tok::Ident(_)) if toks.get(start + 1).is_some_and(|t| t.is_op("(")) => {
                end = matching(toks, start + 1).map_or(toks.len(), |e| e + 1);
            }
            Some(_) => end = start + 1,
            None => {}
        }
        out.push((name.to_owned(), &toks[start..end]));
        i = end.max(start);
    }
    out
}

struct Parser {
    screen: Screen,
    atl: Atl,
    t: f32,
}

impl Parser {
    fn set(&mut self, toks: &[Tok], t0: f32, dur: f32, curve: &Curve) {
        for (name, value) in pairs(toks) {
            match property(&name, value, self.screen) {
                Ok(props) => {
                    for (prop, value) in props {
                        self.atl.steps.push(Step {
                            t0,
                            dur,
                            curve: curve.clone(),
                            prop,
                            value,
                        });
                    }
                }
                Err(()) if HARMLESS.contains(&name.as_str()) => {}
                Err(()) => self.atl.note(&name),
            }
        }
    }

    fn block(&mut self, lines: &[Line]) {
        for (i, line) in lines.iter().enumerate() {
            let toks = tokenize(line.text.trim_end_matches(':'));
            let first = match toks.first() {
                Some(Tok::Ident(w)) => w.clone(),
                Some(Tok::Str(_)) => {
                    self.atl.note("displayable change");
                    continue;
                }
                _ => continue,
            };
            match first.as_str() {
                "pause" => {
                    if let Some((d, _)) = toks.get(1).and_then(|t| number(std::slice::from_ref(t)))
                    {
                        self.t += d.max(0.0);
                    }
                }
                "repeat" => {
                    self.atl.repeat =
                        match toks.get(1).and_then(|t| number(std::slice::from_ref(t))) {
                            Some((n, _)) if n >= 1.0 => Repeat::Times(n as u32),
                            _ => Repeat::Forever,
                        };
                    return;
                }
                "block" => {
                    if i + 1 != lines.len() {
                        self.atl.note("block");
                    }
                    self.block(&line.children);
                }
                "parallel" | "choice" | "on" | "function" | "contains" | "time" | "event"
                | "animation" => self.atl.note(&first),
                w => match warper(w) {
                    Some(curve) => {
                        let rest = &toks[1..];
                        let Some((dur, _)) =
                            rest.first().and_then(|t| number(std::slice::from_ref(t)))
                        else {
                            self.atl.note(w);
                            continue;
                        };
                        let dur = dur.max(0.0);
                        let t0 = self.t;
                        self.set(&rest[1..], t0, dur, &curve);
                        for c in &line.children {
                            self.set(&tokenize(&c.text), t0, dur, &curve);
                        }
                        self.t += dur;
                    }
                    None => {
                        let t0 = self.t;
                        self.set(&toks, t0, 0.0, &Curve::LINEAR);
                    }
                },
            }
        }
    }
}

/// Parses an ATL block (the lines under `transform name:` or `show x:`).
pub fn parse_block(lines: &[Line], screen: Screen) -> Atl {
    let mut p = Parser {
        screen,
        atl: Atl::default(),
        t: 0.0,
    };
    p.block(lines);
    p.atl.duration = p.t;
    p.atl
}

/// The instant properties of `Transform(...)` / `Position(...)` keyword
/// arguments.
pub fn parse_kwargs(args: &[&[Tok]], screen: Screen) -> Atl {
    let mut p = Parser {
        screen,
        atl: Atl::default(),
        t: 0.0,
    };
    for arg in args {
        if let [Tok::Ident(k), Tok::Op(eq), value @ ..] = arg
            && eq == "="
        {
            match property(k, value, screen) {
                Ok(props) => p
                    .atl
                    .steps
                    .extend(props.into_iter().map(|(prop, value)| Step {
                        t0: 0.0,
                        dur: 0.0,
                        curve: Curve::LINEAR,
                        prop,
                        value,
                    })),
                Err(()) if HARMLESS.contains(&k.as_str()) => {}
                Err(()) => p.atl.note(k),
            }
        } else {
            p.atl.note("positional argument");
        }
    }
    p.atl
}

/// The built-in Ren'Py positions, as transforms.
pub fn builtin(name: &str) -> Option<Atl> {
    let (x, y) = match name {
        "left" => (0.25, 1.0),
        "right" => (0.75, 1.0),
        "center" | "default" => (0.5, 1.0),
        "truecenter" => (0.5, 0.75),
        "top" => (0.5, 0.5),
        "topleft" => (0.25, 0.5),
        "topright" => (0.75, 0.5),
        "offscreenleft" => (-0.2, 1.0),
        "offscreenright" => (1.2, 1.0),
        "reset" => (0.5, 1.0),
        _ => return None,
    };
    let step = |prop, value| Step {
        t0: 0.0,
        dur: 0.0,
        curve: Curve::LINEAR,
        prop,
        value,
    };
    Some(Atl {
        steps: vec![step(Prop::X, x), step(Prop::Y, y)],
        ..Atl::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::parse_tree;

    fn block(src: &str) -> Atl {
        let tree = parse_tree(src, false);
        parse_block(
            &tree[0].children,
            Screen {
                w: 1000.0,
                h: 500.0,
            },
        )
    }

    #[test]
    fn slide_and_bob() {
        let a = block("transform t:\n    xalign -0.5\n    linear 1.0 xalign 0.0\n");
        assert!(a.is_animated());
        assert_eq!(a.duration, 1.0);
        let (keys, last) = a.keys(Prop::X, 0.5).unwrap();
        assert_eq!(last, 0.25);
        assert_eq!(keys.len(), 2);
        assert_eq!((keys[0].0, keys[0].1), (0.0, 0.0));
        assert_eq!(keys[0].2, Curve::LINEAR);
        assert_eq!((keys[1].0, keys[1].1), (1.0, 0.25));

        let b = block(
            "transform b:\n    yoffset 0\n    easein .5 yoffset -20\n    easeout .5 yoffset 0\n    repeat\n",
        );
        assert_eq!(b.repeat, Repeat::Forever);
        let (keys, _) = b.keys(Prop::YOff, 0.0).unwrap();
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0].2, Curve::Preset(Ease::OutSine));
        assert_eq!(keys[1].1, -20.0 / 500.0);
        assert!(b.keys(Prop::X, 0.5).is_none());
    }

    #[test]
    fn pauses_props_and_ignored() {
        let a = block(
            "transform t:\n    xalign 0.5 yalign 1.0 crop (0, 0, 1, 1)\n    linear 0.5 xalign 1.0 alpha 0.5\n    pause 0.2\n    ease_quad 0.3:\n        zoom 1.2\n        rotate 15\n    parallel:\n        alpha 1.0\n",
        );
        assert_eq!(a.duration, 1.0);
        assert_eq!(a.ignored, ["crop", "parallel"]);
        let (zoom, last) = a.keys(Prop::Zoom, 1.0).unwrap();
        assert_eq!(last, 1.2);
        assert_eq!(zoom[0].0, 0.0);
        assert_eq!(zoom[1].0, 0.7);
        assert_eq!(zoom[1].2, Curve::Preset(Ease::InOutQuad));
        assert_eq!(zoom[2].0, 1.0);
        let (y, _) = a.keys(Prop::Y, 0.2).unwrap();
        assert_eq!(y.len(), 1);
        assert_eq!(y[0].1, 1.0);
    }

    #[test]
    fn instant_change_later_holds_then_jumps() {
        let a = block("transform t:\n    alpha 0.0\n    pause 1.0\n    alpha 1.0\n");
        let (keys, _) = a.keys(Prop::Alpha, 1.0).unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].2, Curve::Preset(Ease::Step));
        assert_eq!((keys[1].0, keys[1].1), (1.0, 1.0));
    }

    #[test]
    fn kwargs_and_pixels() {
        let toks = tokenize("zoom=0.8, xalign=0.6, xpos=500, ypos=0.9, foo=1");
        let args = split_args(&toks);
        let a = parse_kwargs(
            &args,
            Screen {
                w: 1000.0,
                h: 500.0,
            },
        );
        assert!(!a.is_animated());
        let x: Vec<f32> = a
            .steps
            .iter()
            .filter(|s| s.prop == Prop::X)
            .map(|s| s.value)
            .collect();
        assert_eq!(x, [0.55, 0.5]);
        assert_eq!(a.ignored, ["foo"]);
        assert!(builtin("left").unwrap().is_position_only());
        assert_eq!(
            warper("easeout_bounce"),
            Some(Curve::Preset(Ease::InBounce))
        );
        assert_eq!(warper("wobble"), None);
    }
}
