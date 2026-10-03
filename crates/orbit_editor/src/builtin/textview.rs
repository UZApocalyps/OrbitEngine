//! "Text view": the current scene (or the whole story) as DSL text
//! (`orbit_dsl`, ADR 0015 section 5), editable, with syntax highlighting and
//! inline diagnostics. Optional: the no-code editors stay the default.
//!
//! The text is a *view*: it is applied on an explicit action (button or
//! Ctrl+Enter), or after an idle delay when auto-apply is on, as one undoable
//! transaction. When the graph changes elsewhere the text is refreshed, unless
//! it holds unapplied edits: then a conflict notice offers to reload or to
//! apply anyway (the parser merges by node id).

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Id, Key, Modifiers, RichText, TextStyle, Ui};
use orbit_dsl::{Diagnostic, Scope};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, Level, PanelDef, Plugin, PluginManifest,
    Problem, icons,
};

use super::ids;
use super::ui_util::{danger, faint, success, warning, weak};

/// Idle time (seconds) before auto-apply fires.
const IDLE: f64 = 1.0;
const PREF_AUTO: &str = "auto_apply";

pub struct TextViewPlugin;

impl Plugin for TextViewPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::TEXTVIEW, "Vue texte", "0.1.0")
    }
}

impl EditorPlugin for TextViewPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "textview",
            "textview.title",
            icons::CODE,
            DockSlot::Hidden,
            panel,
        ));
    }
}

/// Id under which the panel keeps its [`TextViewState`] in egui memory.
pub fn state_id() -> Id {
    Id::new("orbit_textview_state")
}

/// State of the text view panel.
#[derive(Clone, Default)]
pub struct TextViewState {
    /// Show the whole story instead of the current scene.
    pub whole_story: bool,
    pub auto_apply: bool,
    /// The text being edited.
    pub text: String,
    /// The text as printed when last loaded from the graph.
    loaded: String,
    scope: Option<Scope>,
    revision: u64,
    /// The graph changed while `text` holds unapplied edits.
    pub conflict: bool,
    last_edit: f64,
    /// Findings for `checked`.
    pub diagnostics: Vec<Diagnostic>,
    checked: Option<(String, Option<Scope>)>,
    prefs_read: bool,
}

impl TextViewState {
    pub fn dirty(&self) -> bool {
        self.text != self.loaded
    }

    /// Scope to show: the whole story, the current scene, or the first one.
    fn wanted_scope(&self, ctx: &EditorContext<'_>) -> Option<Scope> {
        let story = ctx.story()?;
        if self.whole_story {
            return Some(Scope::Story);
        }
        let current = ctx
            .state()
            .current_scene
            .filter(|id| story.scene(*id).is_some())
            .or_else(|| story.scenes.first().map(|s| s.id))?;
        Some(Scope::Scene(current))
    }

    fn reload(&mut self, ctx: &EditorContext<'_>, scope: Scope) {
        let (Some(story), Some(project)) = (ctx.story(), ctx.project()) else {
            return;
        };
        self.text = match scope {
            Scope::Story => orbit_dsl::print_story(story),
            Scope::Scene(id) => orbit_dsl::print_scene(story, id).unwrap_or_default(),
        };
        self.loaded = self.text.clone();
        self.scope = Some(scope);
        self.revision = project.revision();
        self.conflict = false;
        self.checked = None;
        self.diagnostics.clear();
    }

    /// Follows the graph: reloads when the scope changed or the graph moved
    /// (unless the text has unapplied edits: then flags a conflict).
    pub fn sync(&mut self, ctx: &EditorContext<'_>) {
        let Some(scope) = self.wanted_scope(ctx) else {
            return;
        };
        let Some(project) = ctx.project() else {
            return;
        };
        if self.scope != Some(scope) {
            self.reload(ctx, scope);
        } else if project.revision() != self.revision {
            if self.dirty() {
                self.conflict = true;
            } else {
                self.reload(ctx, scope);
            }
        }
    }

    /// Re-parses the text when it changed, to refresh diagnostics.
    pub fn check(&mut self, ctx: &EditorContext<'_>) {
        let (Some(story), Some(scope)) = (ctx.story(), self.scope) else {
            return;
        };
        let key = (self.text.clone(), Some(scope));
        if self.checked.as_ref() == Some(&key) {
            return;
        }
        self.diagnostics = orbit_dsl::parse(&self.text, story, scope)
            .err()
            .unwrap_or_default();
        self.checked = Some(key);
    }

    /// Applies the text as one undoable transaction. Returns the number of
    /// commands run; syntax errors stay in `diagnostics`.
    pub fn apply(&mut self, ctx: &mut EditorContext<'_>) -> Option<usize> {
        self.check(ctx);
        if !self.diagnostics.is_empty() {
            return None;
        }
        let scope = self.scope?;
        let text = self.text.clone();
        let mut result = None;
        let _ = ctx.ops(|project| {
            result = Some(orbit_dsl::apply(project, &text, scope));
            Ok(())
        });
        match result? {
            Ok(n) => {
                if let Some(scope) = self.wanted_scope(ctx) {
                    self.reload(ctx, scope);
                }
                Some(n)
            }
            Err(orbit_dsl::ApplyError::Syntax(d)) => {
                self.diagnostics = d;
                None
            }
            Err(e) => {
                ctx.error(e.to_string());
                None
            }
        }
    }
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    if !ctx.has_project() {
        ctx.state_mut().text_problems.clear();
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ctx.t("textview.no_project")).color(weak(ui)));
        });
        return;
    }
    let mut st: TextViewState = ui.data(|d| d.get_temp(state_id())).unwrap_or_default();
    if !st.prefs_read {
        st.prefs_read = true;
        st.auto_apply = ctx.pref(ids::TEXTVIEW, PREF_AUTO) == Some("1");
    }
    let before = (st.whole_story, st.auto_apply);
    st.sync(ctx);
    if st.scope.is_none() {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ctx.t("textview.no_scene")).color(weak(ui)));
        });
        ui.data_mut(|d| d.insert_temp(state_id(), st));
        return;
    }
    let now = ui.input(|i| i.time);
    let mut apply = false;
    let mut reload = false;

    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut st.whole_story, false, ctx.t("textview.scene"));
        ui.selectable_value(&mut st.whole_story, true, ctx.t("textview.story"));
        ui.separator();
        let can_apply = st.dirty() && st.diagnostics.is_empty();
        if ui
            .add_enabled(
                can_apply,
                egui::Button::new(format!("{}  {}", icons::CHECK, ctx.t("textview.apply"))),
            )
            .on_hover_text(ctx.t("textview.apply_hint"))
            .clicked()
        {
            apply = true;
        }
        if ui
            .add_enabled(
                st.dirty(),
                egui::Button::new(format!(
                    "{}  {}",
                    icons::ARROW_COUNTER_CLOCKWISE,
                    ctx.t("textview.revert")
                )),
            )
            .clicked()
        {
            reload = true;
        }
        ui.checkbox(&mut st.auto_apply, ctx.t("textview.auto_apply"))
            .on_hover_text(ctx.t("textview.auto_apply_hint"));
    });
    if (st.whole_story, st.auto_apply) != before {
        ctx.set_pref(
            ids::TEXTVIEW,
            PREF_AUTO,
            if st.auto_apply { "1" } else { "0" },
        );
    }

    if st.conflict {
        egui::Frame::group(ui.style())
            .stroke(egui::Stroke::new(1.0, warning(ui)))
            .show(ui, |ui| {
                ui.label(RichText::new(ctx.t("textview.conflict")).color(warning(ui)));
                ui.horizontal(|ui| {
                    if ui.button(ctx.t("textview.conflict_reload")).clicked() {
                        reload = true;
                    }
                    if ui.button(ctx.t("textview.conflict_apply")).clicked() {
                        apply = true;
                    }
                });
            });
    }

    // Scope change: the sync at the next frame reloads; avoid editing a stale
    // buffer meanwhile.
    st.check(ctx);
    let error_lines: Vec<usize> = st.diagnostics.iter().map(|d| d.line).collect();
    let height = (ui.available_height() - 70.0).max(80.0);
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| -> Arc<egui::Galley> {
        let mut job = highlight(ui, buf.as_str(), &error_lines);
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut edited = false;
    egui::ScrollArea::vertical()
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let out = egui::TextEdit::multiline(&mut st.text)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(16)
                .layouter(&mut layouter)
                .show(ui);
            edited = out.response.changed();
            if out.response.has_focus()
                && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter))
            {
                apply = true;
            }
        });
    if edited {
        st.last_edit = now;
        st.check(ctx);
    }

    // Diagnostics list.
    if st.diagnostics.is_empty() {
        let (glyph, text, color) = if st.dirty() {
            (icons::PENCIL_SIMPLE, ctx.t("textview.unapplied"), weak(ui))
        } else {
            (icons::CHECK_CIRCLE, ctx.t("textview.in_sync"), success())
        };
        ui.label(RichText::new(format!("{glyph}  {text}")).color(color));
    } else {
        ui.label(
            RichText::new(ctx.tn("textview.errors", st.diagnostics.len(), &[])).color(danger(ui)),
        );
        egui::ScrollArea::vertical()
            .id_salt("textview_diag")
            .max_height(70.0)
            .show(ui, |ui| {
                for d in &st.diagnostics {
                    ui.label(
                        RichText::new(format!("{}:{}  {}", d.line, d.column, d.message))
                            .color(faint(ui))
                            .monospace(),
                    );
                }
            });
    }

    // Auto-apply once the user stopped typing.
    if st.auto_apply && st.dirty() && !st.conflict && st.diagnostics.is_empty() && !apply {
        let wait = st.last_edit + IDLE - now;
        if wait <= 0.0 {
            apply = true;
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
    }

    if reload && let Some(scope) = st.wanted_scope(ctx) {
        st.reload(ctx, scope);
    }
    if apply {
        st.check(ctx);
        if let Some(n) = st.apply(ctx) {
            ctx.info(if n == 0 {
                ctx.t("textview.nothing_changed")
            } else {
                ctx.tn("textview.applied", n, &[])
            });
        }
    }

    ctx.state_mut().text_problems = st
        .diagnostics
        .iter()
        .map(|d| Problem {
            level: Level::Error,
            message: format!("{}  {}:{}  {}", ctx.t("textview.title"), d.line, d.column, d.message),
            location: None,
        })
        .collect();
    ui.data_mut(|d| d.insert_temp(state_id(), st));
}

// ----------------------------------------------------------- highlighting

/// Colored runs of one text: keywords, strings, numbers, comments; `#@id`
/// annotations dimmed; the lines in `error_lines` (1-based) get a red
/// background.
fn highlight(ui: &Ui, text: &str, error_lines: &[usize]) -> LayoutJob {
    let font: FontId = TextStyle::Monospace.resolve(ui.style());
    let normal = ui.visuals().text_color();
    let dark = ui.visuals().dark_mode;
    let keyword = if dark {
        Color32::from_rgb(197, 134, 192)
    } else {
        Color32::from_rgb(140, 40, 150)
    };
    let string = if dark {
        Color32::from_rgb(152, 195, 121)
    } else {
        Color32::from_rgb(30, 120, 50)
    };
    let number = if dark {
        Color32::from_rgb(209, 154, 102)
    } else {
        Color32::from_rgb(160, 80, 0)
    };
    let dim = faint(ui);
    let error_bg = Color32::from_rgba_unmultiplied(220, 60, 60, 50);

    let mut job = LayoutJob::default();
    for (n, line) in text.split_inclusive('\n').enumerate() {
        let bg = if error_lines.contains(&(n + 1)) {
            error_bg
        } else {
            Color32::TRANSPARENT
        };
        for (kind, run) in tokenize(line) {
            let color = match kind {
                Kind::Plain => normal,
                Kind::Keyword => keyword,
                Kind::Str => string,
                Kind::Number => number,
                Kind::Id | Kind::Comment => dim,
            };
            job.append(
                run,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color,
                    background: bg,
                    italics: kind == Kind::Comment,
                    ..TextFormat::default()
                },
            );
        }
    }
    job
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plain,
    Keyword,
    Str,
    Number,
    /// `#@id` annotation.
    Id,
    Comment,
}

/// Splits one line into classified runs that concatenate back to the line.
pub fn tokenize(line: &str) -> Vec<(Kind, &str)> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut plain_from = 0;
    macro_rules! flush {
        ($end:expr) => {
            if plain_from < $end {
                out.push((Kind::Plain, &line[plain_from..$end]));
            }
        };
    }
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            flush!(i);
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\n' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] != b'\n' {
                    i += 1;
                }
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
            }
            out.push((Kind::Str, &line[start..i]));
            plain_from = i;
        } else if b == b'#' {
            flush!(i);
            let kind = if bytes.get(i + 1) == Some(&b'@') {
                Kind::Id
            } else {
                Kind::Comment
            };
            out.push((kind, &line[i..]));
            return out;
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if orbit_dsl::printer::KEYWORDS.contains(&&line[start..i]) {
                flush!(start);
                out.push((Kind::Keyword, &line[start..i]));
                plain_from = i;
            }
        } else if b.is_ascii_digit() {
            flush!(i);
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            out.push((Kind::Number, &line[start..i]));
            plain_from = i;
        } else {
            i += 1;
        }
    }
    flush!(bytes.len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_concatenate_back_to_the_line() {
        let line = "    say \"a # b\" 12 #@01J0 \n";
        let runs = tokenize(line);
        assert_eq!(runs.iter().map(|r| r.1).collect::<String>(), line);
        assert!(runs.contains(&(Kind::Keyword, "say")));
        assert!(runs.contains(&(Kind::Str, "\"a # b\"")));
        assert!(runs.contains(&(Kind::Number, "12")));
        assert!(runs.iter().any(|r| r.0 == Kind::Id));
    }

    #[test]
    fn an_unterminated_string_does_not_panic() {
        let runs = tokenize("say \"oops\n");
        assert_eq!(runs.iter().map(|r| r.1).collect::<String>(), "say \"oops\n");
    }

    #[test]
    fn non_ascii_text_is_kept_whole() {
        let line = "héros \"é\" # ça\n";
        assert_eq!(
            tokenize(line).iter().map(|r| r.1).collect::<String>(),
            line
        );
    }
}
