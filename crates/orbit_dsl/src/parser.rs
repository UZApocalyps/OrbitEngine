//! Text → IR, merged into an existing story.
//!
//! Parsing is done in two passes. The first one reads every line on its own
//! (a broken line yields a diagnostic and is skipped, the others are still
//! checked). The second one assigns ids, resolves names against the base
//! story and the text itself, and builds the merged story. Any error makes
//! the whole parse fail with every diagnostic found; nothing is half applied.

use std::collections::{HashMap, HashSet};
use std::fmt;

use orbit_core::{
    AnimLoop, AnimProp, AnimTarget, AssetId, CharacterId, ChoiceOption, Color, CustomData, Curve,
    Ease, Effect, Expr, Key, Node, NodeId, NodeKind, Position, Rgba, Scene, SceneId, Story,
    StoryTrack, StringId, Track, TrackData, Transition, Vec2, Angle,
};

use crate::expr::{number, parse_expr};
use crate::lexer::{Line, Tok, Token, lex_line};
use crate::printer::{KEYWORDS, Names, labels_of, scene_names, unique};

/// What a text covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Every scene of the story (as printed by [`crate::print_story`]); a
    /// scene missing from the text is deleted.
    Story,
    /// One scene (as printed by [`crate::print_scene`]); the text must hold
    /// exactly one `scene` block. Other scenes are kept, except for nodes the
    /// text moves into this scene.
    Scene(SceneId),
}

/// A syntax or resolution error, at a 1-based line and column (columns count
/// characters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for Diagnostic {}

// ------------------------------------------------------------ raw statements

#[derive(Clone, Debug)]
enum RefKind {
    Name(String),
    Id(String),
}

#[derive(Clone, Debug)]
struct Ref {
    kind: RefKind,
    col: usize,
}

#[derive(Clone, Debug)]
struct RawOption {
    text: String,
    target: Ref,
    condition: Option<Expr>,
}

#[derive(Clone, Debug)]
enum Raw {
    Label(String),
    Say {
        speaker: Option<Ref>,
        text: String,
    },
    Menu(Vec<RawOption>),
    Jump(Ref),
    JumpScene {
        scene: Ref,
        label: Option<Ref>,
    },
    SetVar {
        var: String,
        value: Expr,
    },
    If {
        condition: Expr,
        then_target: Ref,
        else_target: Option<Ref>,
    },
    Background {
        asset: Ref,
        transition: Option<Transition>,
    },
    Show {
        character: Ref,
        expression: String,
        position: Position,
        transition: Option<Transition>,
        scale: Option<f32>,
        scale_y: Option<f32>,
        flip_x: bool,
        flip_y: bool,
    },
    Hide {
        character: Ref,
        transition: Option<Transition>,
    },
    Music {
        asset: Ref,
        fade_in: f32,
        looping: bool,
    },
    StopMusic(f32),
    Sound(Ref),
    Wait(f32),
    Call(Ref),
    Return,
    End,
    Custom {
        plugin_id: String,
        kind: String,
        version: u32,
        data: CustomData,
    },
    Scene3d {
        scene: Ref,
        camera: Option<String>,
        transition: Option<Transition>,
    },
    Camera3d {
        camera: String,
        transition: Option<Transition>,
    },
    HideScene3d(Option<Transition>),
    OpenScreen {
        screen: String,
        wait: bool,
        result: Option<String>,
        params: Vec<(String, Expr)>,
    },
    CloseScreen(String),
    Effect(RawEffect),
    Timeline {
        duration: f32,
        looping: AnimLoop,
        wait: bool,
        tracks: Vec<RawTrack>,
    },
}

/// What an effect or a track animates, before character names are resolved.
#[derive(Clone, Debug)]
enum RawTarget {
    Camera,
    Screen,
    Background,
    Character(Ref),
}

#[derive(Clone, Debug)]
enum RawEffect {
    Move {
        target: RawTarget,
        dx: f32,
        dy: f32,
        duration: f32,
        ease: Option<Curve>,
        wait: bool,
    },
    Shake {
        target: RawTarget,
        amplitude: f32,
        duration: f32,
        ease: Option<Curve>,
        wait: bool,
    },
    Zoom {
        factor: f32,
        duration: f32,
        ease: Option<Curve>,
        wait: bool,
    },
    Flash {
        color: Color,
        duration: f32,
        ease: Option<Curve>,
        wait: bool,
    },
    Tint {
        target: RawTarget,
        color: Color,
        strength: f32,
        duration: f32,
        ease: Option<Curve>,
        wait: bool,
    },
}

#[derive(Clone, Debug)]
struct RawTrack {
    target: RawTarget,
    prop: AnimProp,
    data: TrackData,
}

enum Stmt {
    Scene(String),
    Node(Raw),
    Option(RawOption),
    Track(RawTrack),
}

type LineError = (usize, String);

struct LineParser<'a> {
    tokens: &'a [Token],
    at: usize,
    end_col: usize,
}

impl LineParser<'_> {
    fn col(&self) -> usize {
        self.tokens.get(self.at).map_or(self.end_col, |t| t.col)
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at).map(|t| &t.tok)
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T, LineError> {
        Err((self.col(), msg.into()))
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(w)) if w == word)
    }

    fn eat_word(&mut self, word: &str) -> bool {
        let yes = self.is_word(word);
        if yes {
            self.at += 1;
        }
        yes
    }

    fn expect_word(&mut self, word: &str) -> Result<(), LineError> {
        if self.eat_word(word) {
            Ok(())
        } else {
            self.err(format!("expected `{word}`"))
        }
    }

    fn eat_sym(&mut self, sym: &str) -> bool {
        let yes = matches!(self.peek(), Some(Tok::Sym(s)) if *s == sym);
        if yes {
            self.at += 1;
        }
        yes
    }

    fn expect_sym(&mut self, sym: &str) -> Result<(), LineError> {
        if self.eat_sym(sym) {
            Ok(())
        } else {
            self.err(format!("expected `{sym}`"))
        }
    }

    fn string(&mut self, what: &str) -> Result<String, LineError> {
        match self.peek() {
            Some(Tok::Str(s)) => {
                let s = s.clone();
                self.at += 1;
                Ok(s)
            }
            _ => self.err(format!("expected {what} (a quoted string)")),
        }
    }

    fn name(&mut self, what: &str) -> Result<String, LineError> {
        match self.peek() {
            Some(Tok::Str(s) | Tok::Ident(s)) => {
                let s = s.clone();
                self.at += 1;
                Ok(s)
            }
            _ => self.err(format!("expected {what}")),
        }
    }

    fn reference(&mut self, what: &str) -> Result<Ref, LineError> {
        let col = self.col();
        let kind = match self.peek() {
            Some(Tok::Str(s) | Tok::Ident(s)) => RefKind::Name(s.clone()),
            Some(Tok::At(id)) => RefKind::Id(id.clone()),
            _ => return self.err(format!("expected {what} (a name or `@id`)")),
        };
        self.at += 1;
        Ok(Ref { kind, col })
    }

    /// A number, with an optional leading minus.
    fn number(&mut self) -> Result<(String, bool, bool, usize), LineError> {
        let negative = self.eat_sym("-");
        let col = self.col();
        match self.peek() {
            Some(Tok::Num { text, float }) => {
                let r = (text.clone(), *float, negative, col);
                self.at += 1;
                Ok(r)
            }
            _ => self.err("expected a number"),
        }
    }

    fn f32(&mut self) -> Result<f32, LineError> {
        let (text, _, negative, col) = self.number()?;
        let text = if negative { format!("-{text}") } else { text };
        text.parse::<f32>()
            .map_err(|_| (col, "invalid number".into()))
    }

    fn u32(&mut self) -> Result<u32, LineError> {
        let (text, float, negative, col) = self.number()?;
        if float || negative {
            return Err((col, "expected a non-negative integer".into()));
        }
        text.parse()
            .map_err(|_| (col, "number out of range".into()))
    }

    fn expr(&mut self) -> Result<Expr, LineError> {
        parse_expr(self.tokens, &mut self.at, self.end_col)
    }

    fn done(&self) -> Result<(), LineError> {
        if self.at < self.tokens.len() {
            self.err("unexpected text at the end of the line")
        } else {
            Ok(())
        }
    }

    fn transition(&mut self) -> Result<Option<Transition>, LineError> {
        if !self.eat_word("with") {
            return Ok(None);
        }
        if self.eat_word("dissolve") {
            self.expect_sym("(")?;
            let duration = self.f32()?;
            let ease = self.transition_ease()?;
            self.expect_sym(")")?;
            let mut t = Transition::dissolve(duration);
            t.ease = ease;
            return Ok(Some(t));
        }
        if self.eat_word("fade") {
            self.expect_sym("(")?;
            let duration = self.f32()?;
            self.expect_sym(",")?;
            let col = self.col();
            let hex = self.string("a color `\"#rrggbb\"`")?;
            let color = parse_color(&hex)
                .ok_or((col, format!("invalid color `{hex}`, expected `#rrggbb`")))?;
            let ease = self.transition_ease()?;
            self.expect_sym(")")?;
            let mut t = Transition::fade(color, duration);
            t.ease = ease;
            return Ok(Some(t));
        }
        self.err("expected `dissolve(…)` or `fade(…)`")
    }

    /// The optional `, ease <curve>` of a transition.
    fn transition_ease(&mut self) -> Result<Option<Curve>, LineError> {
        if !self.eat_sym(",") {
            return Ok(None);
        }
        self.expect_word("ease")?;
        self.curve().map(Some)
    }

    /// A curve: a preset name, `bezier(x1, y1, x2, y2)` or a quoted name.
    fn curve(&mut self) -> Result<Curve, LineError> {
        let col = self.col();
        match self.peek().cloned() {
            Some(Tok::Str(name)) => {
                self.at += 1;
                Ok(Curve::Named(name))
            }
            Some(Tok::Ident(w)) if w == "bezier" => {
                self.at += 1;
                self.expect_sym("(")?;
                let x1 = self.f32()?;
                self.expect_sym(",")?;
                let y1 = self.f32()?;
                self.expect_sym(",")?;
                let x2 = self.f32()?;
                self.expect_sym(",")?;
                let y2 = self.f32()?;
                self.expect_sym(")")?;
                Ok(Curve::bezier(x1, y1, x2, y2))
            }
            Some(Tok::Ident(w)) => {
                self.at += 1;
                Ease::ALL
                    .into_iter()
                    .find(|e| e.name() == w)
                    .map(Curve::Preset)
                    .ok_or((col, format!("unknown easing `{w}`")))
            }
            _ => self.err("expected an easing: a preset name, `bezier(…)` or a quoted name"),
        }
    }

    /// `camera`, `screen`, `background` or a character reference.
    fn anim_target(&mut self) -> Result<RawTarget, LineError> {
        if self.eat_word("camera") {
            Ok(RawTarget::Camera)
        } else if self.eat_word("screen") {
            Ok(RawTarget::Screen)
        } else if self.eat_word("background") {
            Ok(RawTarget::Background)
        } else {
            self.reference("a target (`camera`, `screen`, `background` or a character)")
                .map(RawTarget::Character)
        }
    }

    /// A `"#rrggbb"` color.
    fn hex_color(&mut self) -> Result<Color, LineError> {
        let col = self.col();
        let hex = self.string("a color `\"#rrggbb\"`")?;
        parse_color(&hex).ok_or((col, format!("invalid color `{hex}`, expected `#rrggbb`")))
    }

    /// `(x, y)`.
    fn vec2(&mut self) -> Result<Vec2, LineError> {
        self.expect_sym("(")?;
        let x = self.f32()?;
        self.expect_sym(",")?;
        let y = self.f32()?;
        self.expect_sym(")")?;
        Ok(Vec2::new(x, y))
    }

    /// The tail of an effect line: `over <s> [ease <curve>] [wait]`.
    fn effect_tail(&mut self) -> Result<(f32, Option<Curve>, bool), LineError> {
        self.expect_word("over")?;
        let duration = self.f32()?;
        let mut ease = None;
        let mut wait = false;
        loop {
            if self.eat_word("ease") {
                if ease.is_some() {
                    return self.err("`ease` given twice");
                }
                ease = Some(self.curve()?);
            } else if self.eat_word("wait") {
                wait = true;
            } else {
                return Ok((duration, ease, wait));
            }
        }
    }

    /// `<keys>` of a track line, typed by `prop`.
    fn track_data(&mut self, prop: AnimProp) -> Result<TrackData, LineError> {
        macro_rules! keys {
            ($read:expr) => {{
                let mut keys = Vec::new();
                if self.at < self.tokens.len() {
                    loop {
                        let t = self.f32()?;
                        let value = $read(self)?;
                        let ease = if self.eat_word("ease") {
                            self.curve()?
                        } else {
                            Curve::default()
                        };
                        keys.push(Key::with_ease(t, value, ease));
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                }
                Track::new(keys)
            }};
        }
        Ok(match prop {
            AnimProp::Offset | AnimProp::Scale | AnimProp::Pan => {
                TrackData::Vec2(keys!(|p: &mut Self| p.vec2()))
            }
            AnimProp::Rotation => {
                TrackData::Angle(keys!(|p: &mut Self| p.f32().map(Angle)))
            }
            AnimProp::Opacity | AnimProp::Shake | AnimProp::Zoom => {
                TrackData::Scalar(keys!(|p: &mut Self| p.f32()))
            }
            AnimProp::Tint => TrackData::Color(keys!(|p: &mut Self| -> Result<Rgba, LineError> {
                p.expect_word("rgba")?;
                p.expect_sym("(")?;
                let r = p.f32()?;
                p.expect_sym(",")?;
                let g = p.f32()?;
                p.expect_sym(",")?;
                let b = p.f32()?;
                p.expect_sym(",")?;
                let a = p.f32()?;
                p.expect_sym(")")?;
                Ok(Rgba::new(r, g, b, a))
            })),
        })
    }

    fn position(&mut self) -> Result<Position, LineError> {
        if self.eat_word("left") {
            return Ok(Position::Left);
        }
        if self.eat_word("center") {
            return Ok(Position::Center);
        }
        if self.eat_word("right") {
            return Ok(Position::Right);
        }
        if let Some(Tok::Str(s)) = self.peek() {
            let s = s.clone();
            self.at += 1;
            return Ok(Position::Custom(s));
        }
        if self.eat_sym("(") {
            let x = self.f32()?;
            self.expect_sym(",")?;
            let y = self.f32()?;
            self.expect_sym(")")?;
            return Ok(Position::Free { x, y });
        }
        self.err("expected a position: `left`, `center`, `right`, \"name\" or `(x, y)`")
    }

    fn custom_data(&mut self, depth: usize) -> Result<CustomData, LineError> {
        if depth > 64 {
            return self.err("custom data nested too deeply");
        }
        let col = self.col();
        match self.peek().cloned() {
            Some(Tok::Ident(w)) => {
                self.at += 1;
                match w.as_str() {
                    "null" => Ok(CustomData::Null),
                    "true" => Ok(CustomData::Bool(true)),
                    "false" => Ok(CustomData::Bool(false)),
                    "inf" => Ok(CustomData::Float(f64::INFINITY)),
                    "nan" => Ok(CustomData::Float(f64::NAN)),
                    _ => Err((col, format!("unexpected `{w}` in custom data"))),
                }
            }
            Some(Tok::Str(s)) => {
                self.at += 1;
                Ok(CustomData::Str(s))
            }
            Some(Tok::Sym("-")) if matches!(self.tokens.get(self.at + 1).map(|t| &t.tok), Some(Tok::Ident(w)) if w == "inf") =>
            {
                self.at += 2;
                Ok(CustomData::Float(f64::NEG_INFINITY))
            }
            Some(Tok::Sym("-") | Tok::Num { .. }) => {
                let (text, float, negative, col) = self.number()?;
                match number(&text, float, negative, col)? {
                    orbit_core::Value::Int(i) => Ok(CustomData::Int(i)),
                    orbit_core::Value::Float(f) => Ok(CustomData::Float(f)),
                    _ => unreachable!("numbers are ints or floats"),
                }
            }
            Some(Tok::Sym("[")) => {
                self.at += 1;
                let mut items = Vec::new();
                if !self.eat_sym("]") {
                    loop {
                        items.push(self.custom_data(depth + 1)?);
                        if self.eat_sym("]") {
                            break;
                        }
                        self.expect_sym(",")?;
                    }
                }
                Ok(CustomData::List(items))
            }
            Some(Tok::Sym("{")) => {
                self.at += 1;
                let mut entries = Vec::new();
                if !self.eat_sym("}") {
                    loop {
                        let key = self.string("a key")?;
                        self.expect_sym(":")?;
                        entries.push((key, self.custom_data(depth + 1)?));
                        if self.eat_sym("}") {
                            break;
                        }
                        self.expect_sym(",")?;
                    }
                }
                Ok(CustomData::Map(entries))
            }
            _ => self.err("expected custom data"),
        }
    }

    fn statement(&mut self) -> Result<Stmt, LineError> {
        let first = self.peek().cloned();
        let stmt = match first {
            Some(Tok::Str(text)) => {
                self.at += 1;
                if self.eat_sym("->") {
                    let target = self.reference("a label")?;
                    let condition = if self.eat_word("if") {
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    Stmt::Option(RawOption {
                        text,
                        target,
                        condition,
                    })
                } else {
                    Stmt::Node(Raw::Say {
                        speaker: None,
                        text,
                    })
                }
            }
            Some(Tok::Sym("$")) => {
                self.at += 1;
                let var = self.name("a variable name")?;
                let compound = [
                    ("=", None),
                    ("+=", Some(orbit_core::BinOp::Add)),
                    ("-=", Some(orbit_core::BinOp::Sub)),
                    ("*=", Some(orbit_core::BinOp::Mul)),
                    ("/=", Some(orbit_core::BinOp::Div)),
                ]
                .into_iter()
                .find(|(sym, _)| self.eat_sym(sym));
                let Some((_, op)) = compound else {
                    return self.err("expected `=`, `+=`, `-=`, `*=` or `/=`");
                };
                let rhs = self.expr()?;
                let value = match op {
                    None => rhs,
                    Some(op) => Expr::binary(op, Expr::Var(var.clone()), rhs),
                };
                Stmt::Node(Raw::SetVar { var, value })
            }
            Some(Tok::Ident(word)) => {
                self.at += 1;
                match word.as_str() {
                    "scene" => Stmt::Scene(self.name("a scene name")?),
                    "label" => Stmt::Node(Raw::Label(self.name("a label name")?)),
                    "say" => {
                        let speaker = if matches!(
                            self.tokens.get(self.at + 1).map(|t| &t.tok),
                            Some(Tok::Str(_))
                        ) {
                            Some(self.reference("a character")?)
                        } else {
                            None
                        };
                        let text = self.string("the line's text")?;
                        Stmt::Node(Raw::Say { speaker, text })
                    }
                    "menu" => Stmt::Node(Raw::Menu(Vec::new())),
                    "jump" => Stmt::Node(Raw::Jump(self.reference("a label")?)),
                    "jump_scene" => {
                        let scene = self.reference("a scene")?;
                        let label = if self.eat_word("label") {
                            Some(self.reference("a label")?)
                        } else {
                            None
                        };
                        Stmt::Node(Raw::JumpScene { scene, label })
                    }
                    "if" => {
                        let condition = self.expr()?;
                        self.expect_sym("->")?;
                        let then_target = self.reference("a label")?;
                        let else_target = if self.eat_word("else") {
                            Some(self.reference("a label")?)
                        } else {
                            None
                        };
                        Stmt::Node(Raw::If {
                            condition,
                            then_target,
                            else_target,
                        })
                    }
                    "bg" => {
                        let asset = self.reference("an image")?;
                        let transition = self.transition()?;
                        Stmt::Node(Raw::Background { asset, transition })
                    }
                    "show" => {
                        let character = self.reference("a character")?;
                        let expression = self.name("an expression name")?;
                        self.expect_word("at")?;
                        let position = self.position()?;
                        let (mut scale, mut scale_y, mut flip_x, mut flip_y, mut transition) =
                            (None, None, false, false, None);
                        let mut seen = HashSet::new();
                        while let Some(Tok::Ident(opt)) = self.peek().cloned() {
                            if !seen.insert(opt.clone()) {
                                return self.err(format!("`{opt}` given twice"));
                            }
                            match opt.as_str() {
                                "scale" => {
                                    self.at += 1;
                                    scale = Some(self.f32()?);
                                }
                                "scale_y" => {
                                    self.at += 1;
                                    scale_y = Some(self.f32()?);
                                }
                                "flip_x" => {
                                    self.at += 1;
                                    flip_x = true;
                                }
                                "flip_y" => {
                                    self.at += 1;
                                    flip_y = true;
                                }
                                "with" => transition = self.transition()?,
                                _ => return self.err(format!("unknown `show` option `{opt}`")),
                            }
                        }
                        Stmt::Node(Raw::Show {
                            character,
                            expression,
                            position,
                            transition,
                            scale,
                            scale_y,
                            flip_x,
                            flip_y,
                        })
                    }
                    "hide" => {
                        let character = self.reference("a character")?;
                        let transition = self.transition()?;
                        Stmt::Node(Raw::Hide {
                            character,
                            transition,
                        })
                    }
                    "music" => {
                        let asset = self.reference("a music asset")?;
                        let mut fade_in = 0.0;
                        let mut looping = false;
                        let mut seen = HashSet::new();
                        while let Some(Tok::Ident(opt)) = self.peek().cloned() {
                            if !seen.insert(opt.clone()) {
                                return self.err(format!("`{opt}` given twice"));
                            }
                            self.at += 1;
                            match opt.as_str() {
                                "fade" => fade_in = self.f32()?,
                                "loop" => looping = true,
                                _ => {
                                    self.at -= 1;
                                    return self.err(format!("unknown `music` option `{opt}`"));
                                }
                            }
                        }
                        Stmt::Node(Raw::Music {
                            asset,
                            fade_in,
                            looping,
                        })
                    }
                    "stop" => {
                        self.expect_word("music")?;
                        let fade = if self.eat_word("fade") {
                            self.f32()?
                        } else {
                            0.0
                        };
                        Stmt::Node(Raw::StopMusic(fade))
                    }
                    "sound" => Stmt::Node(Raw::Sound(self.reference("a sound asset")?)),
                    "wait" => Stmt::Node(Raw::Wait(self.f32()?)),
                    "move" => {
                        let target = self.anim_target()?;
                        self.expect_word("by")?;
                        let d = self.vec2()?;
                        let (duration, ease, wait) = self.effect_tail()?;
                        Stmt::Node(Raw::Effect(RawEffect::Move {
                            target,
                            dx: d.x,
                            dy: d.y,
                            duration,
                            ease,
                            wait,
                        }))
                    }
                    "shake" => {
                        let target = self.anim_target()?;
                        let amplitude = self.f32()?;
                        let (duration, ease, wait) = self.effect_tail()?;
                        Stmt::Node(Raw::Effect(RawEffect::Shake {
                            target,
                            amplitude,
                            duration,
                            ease,
                            wait,
                        }))
                    }
                    "zoom" => {
                        let factor = self.f32()?;
                        let (duration, ease, wait) = self.effect_tail()?;
                        Stmt::Node(Raw::Effect(RawEffect::Zoom {
                            factor,
                            duration,
                            ease,
                            wait,
                        }))
                    }
                    "flash" => {
                        let color = self.hex_color()?;
                        let (duration, ease, wait) = self.effect_tail()?;
                        Stmt::Node(Raw::Effect(RawEffect::Flash {
                            color,
                            duration,
                            ease,
                            wait,
                        }))
                    }
                    "tint" => {
                        let target = self.anim_target()?;
                        let color = self.hex_color()?;
                        let strength = self.f32()?;
                        let (duration, ease, wait) = self.effect_tail()?;
                        Stmt::Node(Raw::Effect(RawEffect::Tint {
                            target,
                            color,
                            strength,
                            duration,
                            ease,
                            wait,
                        }))
                    }
                    "timeline" => {
                        self.expect_word("over")?;
                        let duration = self.f32()?;
                        let mut looping = AnimLoop::Once;
                        let mut wait = false;
                        loop {
                            if self.eat_word("loop") {
                                looping = AnimLoop::Loop(self.u32()?);
                            } else if self.eat_word("forever") {
                                looping = AnimLoop::Forever;
                            } else if self.eat_word("pingpong") {
                                looping = AnimLoop::PingPong;
                            } else if self.eat_word("wait") {
                                wait = true;
                            } else {
                                break;
                            }
                        }
                        Stmt::Node(Raw::Timeline {
                            duration,
                            looping,
                            wait,
                            tracks: Vec::new(),
                        })
                    }
                    "track" => {
                        let target = self.anim_target()?;
                        let col = self.col();
                        let name = self.name("a property")?;
                        let prop = AnimProp::ALL
                            .into_iter()
                            .find(|p| p.name() == name)
                            .ok_or((col, format!("unknown track property `{name}`")))?;
                        let data = self.track_data(prop)?;
                        Stmt::Track(RawTrack { target, prop, data })
                    }
                    "call" => Stmt::Node(Raw::Call(self.reference("a label")?)),
                    "scene3d" => {
                        let scene = self.reference("a 3D scene asset")?;
                        let camera = if self.eat_word("camera") {
                            Some(self.string("a camera name")?)
                        } else {
                            None
                        };
                        let transition = self.transition()?;
                        Stmt::Node(Raw::Scene3d {
                            scene,
                            camera,
                            transition,
                        })
                    }
                    "camera3d" => {
                        let camera = self.string("a camera name")?;
                        let transition = self.transition()?;
                        Stmt::Node(Raw::Camera3d { camera, transition })
                    }
                    "hide_scene3d" => Stmt::Node(Raw::HideScene3d(self.transition()?)),
                    "open" => {
                        self.expect_word("screen")?;
                        let screen = self.string("a screen id")?;
                        let wait = self.eat_word("wait");
                        let result = if self.eat_sym("->") {
                            Some(self.name("a result variable")?)
                        } else {
                            None
                        };
                        let mut params = Vec::new();
                        if self.eat_word("with") {
                            loop {
                                let name = self.name("a parameter name")?;
                                self.expect_sym("=")?;
                                params.push((name, self.expr()?));
                                if !self.eat_sym(",") {
                                    break;
                                }
                            }
                        }
                        Stmt::Node(Raw::OpenScreen {
                            screen,
                            wait,
                            result,
                            params,
                        })
                    }
                    "close" => {
                        self.expect_word("screen")?;
                        Stmt::Node(Raw::CloseScreen(self.string("a screen id")?))
                    }
                    "return" => Stmt::Node(Raw::Return),
                    "end" => Stmt::Node(Raw::End),
                    "custom" => {
                        let plugin_id = self.string("a plugin id")?;
                        let kind = self.name("a node kind")?;
                        let version = if self.eat_word("version") {
                            self.u32()?
                        } else {
                            0
                        };
                        let data = if self.eat_word("data") {
                            self.custom_data(0)?
                        } else {
                            CustomData::Null
                        };
                        Stmt::Node(Raw::Custom {
                            plugin_id,
                            kind,
                            version,
                            data,
                        })
                    }
                    _ if KEYWORDS.contains(&word.as_str()) => {
                        self.at -= 1;
                        return self.err(format!("`{word}` cannot start a line"));
                    }
                    _ => {
                        // `<speaker> "text"`.
                        self.at -= 1;
                        let speaker = self.reference("a character")?;
                        if !matches!(self.peek(), Some(Tok::Str(_))) {
                            self.at -= 1;
                            return self.err(format!("unknown statement `{word}`"));
                        }
                        let text = self.string("the line's text")?;
                        Stmt::Node(Raw::Say {
                            speaker: Some(speaker),
                            text,
                        })
                    }
                }
            }
            Some(_) => return self.err("expected a statement"),
            None => unreachable!("blank lines are skipped"),
        };
        self.done()?;
        Ok(stmt)
    }
}

fn parse_color(hex: &str) -> Option<Color> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?))
}

// -------------------------------------------------------------- resolution

fn resolve_target(
    diags: &mut Diags,
    line: usize,
    target: &RawTarget,
    characters: &HashMap<String, Vec<CharacterId>>,
) -> Option<AnimTarget> {
    match target {
        RawTarget::Camera => Some(AnimTarget::Camera),
        RawTarget::Screen => Some(AnimTarget::Screen),
        RawTarget::Background => Some(AnimTarget::Background),
        RawTarget::Character(r) => {
            let id = match &r.kind {
                RefKind::Id(s) => s
                    .parse()
                    .map_err(|_| diags.push(line, r.col, format!("invalid id `{s}`")))
                    .ok(),
                RefKind::Name(name) => match unique(characters, name) {
                    Some(id) => Some(id),
                    None if characters.get(name).is_some_and(|v| v.len() > 1) => {
                        diags.push(
                            line,
                            r.col,
                            format!("ambiguous character `{name}`: refer to it by `@id`"),
                        );
                        None
                    }
                    None => {
                        diags.push(line, r.col, format!("unknown character `{name}`"));
                        None
                    }
                },
            };
            id.map(AnimTarget::Character)
        }
    }
}

struct PNode {
    id: NodeId,
    line: usize,
    raw: Raw,
}

struct PScene {
    id: SceneId,
    name: String,
    nodes: Vec<PNode>,
}

struct Diags(Vec<Diagnostic>);

impl Diags {
    fn push(&mut self, line: usize, column: usize, message: impl Into<String>) {
        self.0.push(Diagnostic {
            line,
            column,
            message: message.into(),
        });
    }
}

/// Parses `text` against `base` and returns the merged story, or every
/// diagnostic found.
///
/// Ids written as `#@` annotations are kept (an id unknown to `base` is kept
/// too, so text pasted from elsewhere preserves its ids); lines without one
/// get fresh ids. The text of an existing `Say` or menu option keeps its
/// string id (the source string is updated in place, unless the string is
/// shared with something that still shows the old text). Strings only used by
/// deleted lines are dropped from the table.
pub fn parse(text: &str, base: &Story, scope: Scope) -> Result<Story, Vec<Diagnostic>> {
    let mut diags = Diags(Vec::new());
    let mut scenes: Vec<PScene> = Vec::new();
    let mut seen_nodes: HashSet<NodeId> = HashSet::new();
    let mut seen_scenes: HashSet<SceneId> = HashSet::new();
    let mut menu_open = false;

    for (index, source) in text.lines().enumerate() {
        let line_no = index + 1;
        let Line { tokens, annotation } = match lex_line(source) {
            Ok(l) => l,
            Err((col, msg)) => {
                diags.push(line_no, col, msg);
                menu_open = false;
                continue;
            }
        };
        if tokens.is_empty() {
            if let Some((_, col)) = annotation {
                diags.push(line_no, col, "an id annotation needs a statement");
            }
            continue;
        }
        let mut lp = LineParser {
            tokens: &tokens,
            at: 0,
            end_col: source.chars().count() + 1,
        };
        let stmt = match lp.statement() {
            Ok(s) => s,
            Err((col, msg)) => {
                diags.push(line_no, col, msg);
                menu_open = false;
                continue;
            }
        };
        match stmt {
            Stmt::Scene(name) => {
                menu_open = false;
                let id = match &annotation {
                    None => match scope {
                        Scope::Scene(id) if scenes.is_empty() => id,
                        _ => SceneId::new(),
                    },
                    Some((s, col)) => match s.parse::<SceneId>() {
                        Ok(id) => id,
                        Err(_) => {
                            diags.push(line_no, *col, format!("invalid id `{s}`"));
                            SceneId::new()
                        }
                    },
                };
                if !seen_scenes.insert(id) {
                    diags.push(line_no, 1, format!("scene id {id} appears twice"));
                }
                if let Scope::Scene(expected) = scope {
                    if !scenes.is_empty() {
                        diags.push(line_no, 1, "this view holds a single scene");
                    } else if id != expected {
                        diags.push(
                            line_no,
                            1,
                            format!("this view edits scene {expected}, not {id}"),
                        );
                    }
                }
                scenes.push(PScene {
                    id,
                    name,
                    nodes: Vec::new(),
                });
            }
            Stmt::Option(option) => {
                if let Some((_, col)) = annotation {
                    diags.push(line_no, col, "menu options carry no id");
                }
                match scenes.last_mut().and_then(|s| s.nodes.last_mut()) {
                    Some(PNode {
                        raw: Raw::Menu(options),
                        ..
                    }) if menu_open => options.push(option),
                    _ => diags.push(
                        line_no,
                        1,
                        "a menu option must follow `menu` or another option",
                    ),
                }
            }
            Stmt::Track(track) => {
                if let Some((_, col)) = annotation {
                    diags.push(line_no, col, "timeline tracks carry no id");
                }
                match scenes.last_mut().and_then(|s| s.nodes.last_mut()) {
                    Some(PNode {
                        raw: Raw::Timeline { tracks, .. },
                        ..
                    }) if menu_open => tracks.push(track),
                    _ => diags.push(
                        line_no,
                        1,
                        "a `track` must follow `timeline` or another track",
                    ),
                }
            }
            Stmt::Node(raw) => {
                menu_open =matches!(raw, Raw::Menu(_) | Raw::Timeline { .. });
                let id = match &annotation {
                    None => NodeId::new(),
                    Some((s, col)) => match s.parse::<NodeId>() {
                        Ok(id) => id,
                        Err(_) => {
                            diags.push(line_no, *col, format!("invalid id `{s}`"));
                            NodeId::new()
                        }
                    },
                };
                if !seen_nodes.insert(id) {
                    diags.push(
                        line_no,
                        annotation.as_ref().map_or(1, |a| a.1),
                        format!("node id {id} appears twice"),
                    );
                }
                match scenes.last_mut() {
                    Some(scene) => scene.nodes.push(PNode {
                        id,
                        line: line_no,
                        raw,
                    }),
                    None => diags.push(line_no, 1, "a statement must follow a `scene` line"),
                }
            }
        }
    }
    if let Scope::Scene(id) = scope {
        if scenes.is_empty() {
            diags.push(1, 1, "expected a `scene` line");
        }
        if base.scene(id).is_none() {
            diags.push(1, 1, format!("unknown scene {id}"));
        }
    }
    if !diags.0.is_empty() {
        return Err(diags.0);
    }
    build(base, scope, scenes)
}

/// Pass 2: resolution and merge.
fn build(base: &Story, scope: Scope, parsed: Vec<PScene>) -> Result<Story, Vec<Diagnostic>> {
    let mut diags = Diags(Vec::new());
    let names = Names::of(base);

    // Scene names and labels of the *target* story.
    let mut scene_list: Vec<(String, SceneId)> = Vec::new();
    let mut labels: HashMap<SceneId, HashMap<String, Vec<NodeId>>> = HashMap::new();
    let text_label_map = |s: &PScene| {
        let mut map: HashMap<String, Vec<NodeId>> = HashMap::new();
        for n in &s.nodes {
            if let Raw::Label(name) = &n.raw {
                map.entry(name.clone()).or_default().push(n.id);
            }
        }
        map
    };
    match scope {
        Scope::Story => {
            for s in &parsed {
                scene_list.push((s.name.clone(), s.id));
                labels.insert(s.id, text_label_map(s));
            }
        }
        Scope::Scene(edited) => {
            let moved: HashSet<NodeId> = parsed
                .iter()
                .flat_map(|s| s.nodes.iter().map(|n| n.id))
                .collect();
            for s in &base.scenes {
                if s.id == edited {
                    let p = &parsed[0];
                    scene_list.push((p.name.clone(), p.id));
                    labels.insert(p.id, text_label_map(p));
                } else {
                    scene_list.push((s.name.clone(), s.id));
                    let kept: Vec<Node> = s
                        .nodes
                        .iter()
                        .filter(|n| !moved.contains(&n.id))
                        .cloned()
                        .collect();
                    labels.insert(s.id, labels_of(&kept));
                }
            }
        }
    }
    let scene_names = scene_names(scene_list.iter().map(|(n, id)| (n.as_str(), *id)));

    // String bookkeeping: texts that must not change under their id.
    let mut strings = base.strings.clone();
    let mut assigned: HashMap<StringId, String> = HashMap::new();
    for c in &base.characters {
        if let Some(t) = base.text(c.name_id) {
            assigned.insert(c.name_id, t.to_owned());
        }
    }
    let replaced: HashSet<NodeId> = parsed
        .iter()
        .flat_map(|s| s.nodes.iter().map(|n| n.id))
        .collect();
    if let Scope::Scene(edited) = scope {
        for s in base.scenes.iter().filter(|s| s.id != edited) {
            for n in s.nodes.iter().filter(|n| !replaced.contains(&n.id)) {
                for sid in n.kind.strings() {
                    if let Some(t) = base.text(sid) {
                        assigned.insert(sid, t.to_owned());
                    }
                }
            }
        }
    }
    let mut string_for = |reuse: Option<StringId>, text: &str| -> StringId {
        if let Some(id) = reuse {
            match assigned.get(&id) {
                Some(t) if t == text => return id,
                Some(_) => {}
                None => {
                    assigned.insert(id, text.to_owned());
                    let dangling = strings.get(id).is_none() && text.is_empty();
                    if !dangling && strings.get(id) != Some(text) {
                        strings.set(id, text);
                    }
                    return id;
                }
            }
        }
        let id = strings.add(text);
        assigned.insert(id, text.to_owned());
        id
    };

    let label_ref = |diags: &mut Diags, line: usize, scene: SceneId, r: &Ref| -> Option<NodeId> {
        match &r.kind {
            RefKind::Id(s) => s
                .parse()
                .map_err(|_| diags.push(line, r.col, format!("invalid id `{s}`")))
                .ok(),
            RefKind::Name(name) => match labels
                .get(&scene)
                .and_then(|m| m.get(name))
                .map(Vec::as_slice)
            {
                Some([one]) => Some(*one),
                Some([]) | None => {
                    diags.push(line, r.col, format!("unknown label `{name}`"));
                    None
                }
                Some(_) => {
                    diags.push(
                        line,
                        r.col,
                        format!("ambiguous label `{name}`: refer to it by `@id`"),
                    );
                    None
                }
            },
        }
    };
    fn by_name<T: Copy + std::str::FromStr>(
        diags: &mut Diags,
        line: usize,
        r: &Ref,
        map: &HashMap<String, Vec<T>>,
        what: &str,
    ) -> Option<T> {
        match &r.kind {
            RefKind::Id(s) => s
                .parse()
                .map_err(|_| diags.push(line, r.col, format!("invalid id `{s}`")))
                .ok(),
            RefKind::Name(name) => match unique(map, name) {
                Some(id) => Some(id),
                None if map.get(name).is_some_and(|v| v.len() > 1) => {
                    diags.push(
                        line,
                        r.col,
                        format!("ambiguous {what} `{name}`: refer to it by `@id`"),
                    );
                    None
                }
                None => {
                    diags.push(line, r.col, format!("unknown {what} `{name}`"));
                    None
                }
            },
        }
    }

    let mut built: Vec<Scene> = Vec::new();
    for ps in &parsed {
        let mut nodes = Vec::new();
        for pn in &ps.nodes {
            let line = pn.line;
            let old = base.node(pn.id).map(|n| &n.kind);
            let d = &mut diags;
            let kind = match &pn.raw {
                Raw::Label(name) => Some(NodeKind::Label { name: name.clone() }),
                Raw::Say { speaker, text } => {
                    let speaker = match speaker {
                        None => Some(None),
                        Some(r) => {
                            by_name::<CharacterId>(d, line, r, &names.characters, "character")
                                .map(Some)
                        }
                    };
                    let reuse = match old {
                        Some(NodeKind::Say { text_id, .. }) => Some(*text_id),
                        _ => None,
                    };
                    speaker.map(|speaker| NodeKind::Say {
                        speaker,
                        text_id: string_for(reuse, text),
                    })
                }
                Raw::Menu(options) => {
                    let old_options = match old {
                        Some(NodeKind::Choice { options }) => options.as_slice(),
                        _ => &[],
                    };
                    let mut out = Vec::new();
                    let mut ok = true;
                    for (i, o) in options.iter().enumerate() {
                        match label_ref(d, line, ps.id, &o.target) {
                            Some(target) => out.push(ChoiceOption {
                                text_id: string_for(old_options.get(i).map(|o| o.text_id), &o.text),
                                target,
                                condition: o.condition.clone(),
                            }),
                            None => ok = false,
                        }
                    }
                    ok.then_some(NodeKind::Choice { options: out })
                }
                Raw::Jump(r) => {
                    label_ref(d, line, ps.id, r).map(|target| NodeKind::Jump { target })
                }
                Raw::JumpScene { scene, label } => {
                    by_name::<SceneId>(d, line, scene, &scene_names, "scene").and_then(|scene| {
                        let label = match label {
                            None => Some(None),
                            Some(r) => label_ref(d, line, scene, r).map(Some),
                        };
                        label.map(|label| NodeKind::JumpScene { scene, label })
                    })
                }
                Raw::SetVar { var, value } => Some(NodeKind::SetVar {
                    var: var.clone(),
                    value: value.clone(),
                }),
                Raw::If {
                    condition,
                    then_target,
                    else_target,
                } => {
                    let then_target = label_ref(d, line, ps.id, then_target);
                    let else_target = match else_target {
                        None => Some(None),
                        Some(r) => label_ref(d, line, ps.id, r).map(Some),
                    };
                    then_target
                        .zip(else_target)
                        .map(|(then_target, else_target)| NodeKind::If {
                            condition: condition.clone(),
                            then_target,
                            else_target,
                        })
                }
                Raw::Background { asset, transition } => {
                    by_name::<AssetId>(d, line, asset, &names.assets, "asset").map(|asset| {
                        NodeKind::Background {
                            asset,
                            transition: transition.clone(),
                        }
                    })
                }
                Raw::Show {
                    character,
                    expression,
                    position,
                    transition,
                    scale,
                    scale_y,
                    flip_x,
                    flip_y,
                } => by_name::<CharacterId>(d, line, character, &names.characters, "character")
                    .map(|character| NodeKind::Show {
                        character,
                        expression: expression.clone(),
                        position: position.clone(),
                        transition: transition.clone(),
                        scale: *scale,
                        scale_y: *scale_y,
                        flip_x: *flip_x,
                        flip_y: *flip_y,
                    }),
                Raw::Hide {
                    character,
                    transition,
                } => by_name::<CharacterId>(d, line, character, &names.characters, "character")
                    .map(|character| NodeKind::Hide {
                        character,
                        transition: transition.clone(),
                    }),
                Raw::Music {
                    asset,
                    fade_in,
                    looping,
                } => by_name::<AssetId>(d, line, asset, &names.assets, "asset").map(|asset| {
                    NodeKind::PlayMusic {
                        asset,
                        fade_in: *fade_in,
                        looping: *looping,
                    }
                }),
                Raw::StopMusic(fade_out) => Some(NodeKind::StopMusic {
                    fade_out: *fade_out,
                }),
                Raw::Sound(asset) => by_name::<AssetId>(d, line, asset, &names.assets, "asset")
                    .map(|asset| NodeKind::PlaySound { asset }),
                Raw::Wait(seconds) => Some(NodeKind::Wait { seconds: *seconds }),
                Raw::Call(r) => label_ref(d, line, ps.id, r).map(|label| NodeKind::Call { label }),
                Raw::OpenScreen {
                    screen,
                    wait,
                    result,
                    params,
                } => {
                    let mut out = Vec::new();
                    let mut ok = true;
                    for (name, e) in params {
                        match e {
                            Expr::Lit(v) => out.push(orbit_core::ScreenParam {
                                name: name.clone(),
                                value: orbit_core::Operand::Lit(v.clone()),
                            }),
                            Expr::Var(v) if !v.starts_with('@') => {
                                out.push(orbit_core::ScreenParam {
                                    name: name.clone(),
                                    value: orbit_core::Operand::Var(v.clone()),
                                })
                            }
                            _ => {
                                ok = false;
                                d.push(
                                    line,
                                    1,
                                    format!("parameter `{name}` must be a literal or a variable"),
                                );
                            }
                        }
                    }
                    ok.then(|| NodeKind::OpenScreen {
                        screen: screen.clone(),
                        wait: *wait,
                        result: result.clone(),
                        params: out,
                    })
                }
                Raw::CloseScreen(screen) => Some(NodeKind::CloseScreen {
                    screen: screen.clone(),
                }),
                Raw::Return => Some(NodeKind::Return),
                Raw::End => Some(NodeKind::End),
                Raw::Custom {
                    plugin_id,
                    kind,
                    version,
                    data,
                } => Some(NodeKind::Custom {
                    plugin_id: plugin_id.clone(),
                    kind: kind.clone(),
                    version: *version,
                    data: data.clone(),
                }),
                Raw::Scene3d {
                    scene,
                    camera,
                    transition,
                } => by_name::<AssetId>(d, line, scene, &names.assets, "asset").map(|scene| {
                    NodeKind::ShowScene3d {
                        scene,
                        camera: camera.clone(),
                        transition: transition.clone(),
                    }
                }),
                Raw::Camera3d { camera, transition } => Some(NodeKind::Camera3d {
                    camera: camera.clone(),
                    transition: transition.clone(),
                }),
                Raw::HideScene3d(transition) => Some(NodeKind::HideScene3d {
                    transition: transition.clone(),
                }),
                Raw::Effect(effect) => {
                    let mut target = |t: &RawTarget| {
                        resolve_target(d, line, t, &names.characters)
                    };
                    match effect {
                        RawEffect::Move { target: t, dx, dy, duration, ease, wait } => {
                            target(t).map(|target| Effect::Move {
                                target,
                                dx: *dx,
                                dy: *dy,
                                duration: *duration,
                                ease: ease.clone(),
                                wait: *wait,
                            })
                        }
                        RawEffect::Shake { target: t, amplitude, duration, ease, wait } => {
                            target(t).map(|target| Effect::Shake {
                                target,
                                amplitude: *amplitude,
                                duration: *duration,
                                ease: ease.clone(),
                                wait: *wait,
                            })
                        }
                        RawEffect::Zoom { factor, duration, ease, wait } => Some(Effect::Zoom {
                            factor: *factor,
                            duration: *duration,
                            ease: ease.clone(),
                            wait: *wait,
                        }),
                        RawEffect::Flash { color, duration, ease, wait } => Some(Effect::Flash {
                            color: *color,
                            duration: *duration,
                            ease: ease.clone(),
                            wait: *wait,
                        }),
                        RawEffect::Tint { target: t, color, strength, duration, ease, wait } => {
                            target(t).map(|target| Effect::Tint {
                                target,
                                color: *color,
                                strength: *strength,
                                duration: *duration,
                                ease: ease.clone(),
                                wait: *wait,
                            })
                        }
                    }
                    .map(NodeKind::Effect)
                }
                Raw::Timeline { duration, looping, wait, tracks } => {
                    let mut out = Vec::new();
                    let mut ok = true;
                    for t in tracks {
                        match resolve_target(d, line, &t.target, &names.characters) {
                            Some(target) => out.push(StoryTrack {
                                target,
                                prop: t.prop,
                                data: t.data.clone(),
                            }),
                            None => ok = false,
                        }
                    }
                    ok.then_some(NodeKind::Timeline {
                        duration: *duration,
                        looping: *looping,
                        wait: *wait,
                        tracks: out,
                    })
                }
            };
            if let Some(kind) = kind {
                nodes.push(Node { id: pn.id, kind });
            }
        }
        built.push(Scene {
            id: ps.id,
            name: ps.name.clone(),
            nodes,
        });
    }
    if !diags.0.is_empty() {
        return Err(diags.0);
    }

    let mut story = base.clone();
    match scope {
        Scope::Story => story.scenes = built,
        Scope::Scene(edited) => {
            let scene = built.pop().expect("one scene");
            for s in &mut story.scenes {
                if s.id == edited {
                    *s = scene.clone();
                } else {
                    s.nodes.retain(|n| !replaced.contains(&n.id));
                }
            }
        }
    }

    // Drop the strings only deleted lines used.
    let used: HashSet<StringId> = story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .flat_map(|n| n.kind.strings())
        .chain(story.characters.iter().map(|c| c.name_id))
        .collect();
    for sid in base
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .flat_map(|n| n.kind.strings())
    {
        if !used.contains(&sid) {
            strings.entries.remove(&sid);
        }
    }
    story.strings = strings;
    Ok(story)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_keep_going_after_a_bad_line() {
        let base = Story::new("t");
        let text = "scene main\n    jump\n    wait x\n    \"ok\"\n    jump nowhere\n";
        let errs = parse(text, &base, Scope::Story).unwrap_err();
        // Syntax errors are all reported (lines 2 and 3); resolution waits
        // for a syntactically valid text.
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert_eq!((errs[0].line, errs[0].column), (2, 9));
        assert_eq!((errs[1].line, errs[1].column), (3, 10));
    }

    #[test]
    fn resolution_errors_have_positions() {
        let base = Story::new("t");
        let errs = parse(
            "scene main\n  jump nowhere\n  bob \"hi\"\n",
            &base,
            Scope::Story,
        )
        .unwrap_err();
        assert_eq!(errs.len(), 2);
        assert_eq!((errs[0].line, errs[0].column), (2, 8));
        assert!(errs[0].message.contains("unknown label"));
        assert_eq!((errs[1].line, errs[1].column), (3, 3));
    }
}
