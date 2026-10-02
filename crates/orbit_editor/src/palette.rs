//! The command palette: fuzzy search over the action registry, opened with
//! Ctrl+K or Ctrl+Shift+P, shortcuts shown next to each command.

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Color32, Context, CornerRadius, FontId, Frame, Id, Key, Margin, Modifiers,
    Sense, Stroke, TextEdit, Ui, vec2,
};
use orbit_editor_api::{EditorContext, EditorRegistry, Location, UiText, icons, shortcut_text};

use crate::fuzzy::fuzzy_match;
use crate::search::{HitKind, SearchHandle};
use crate::theme::{Theme, bold};

/// Rows shown at most.
const MAX_RESULTS: usize = 60;
const ROW_HEIGHT: f32 = 34.0;

/// One line of the results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    /// Full id of the action to run.
    pub action_id: String,
    pub label: String,
    pub category: String,
    pub icon: &'static str,
    /// `Ctrl+S`-style text.
    pub shortcut: Option<String>,
    /// Char indices of `label` that matched the query.
    pub positions: Vec<usize>,
    pub score: i32,
    /// A thing of the project to show instead of running an action
    /// (navigation: scenes, lines, characters, assets...).
    pub goto: Option<Location>,
    /// A scattered match (`bob` in "Basculer ... sombre"): shown after the
    /// project's own entries when those exist.
    pub weak: bool,
}

/// Enabled actions matching `query`, best first (registry order for ties).
pub fn search(registry: &EditorRegistry, ctx: &EditorContext<'_>, query: &str) -> Vec<PaletteItem> {
    let mut items: Vec<PaletteItem> = registry
        .actions()
        .iter()
        .filter(|a| (a.def.enabled)(ctx))
        .filter_map(|a| {
            let label = a.label(ctx);
            let (score, positions) = match fuzzy_match(query, &label) {
                Some(m) => (m.score, m.positions),
                // The category can match too ("fichier" lists the File actions).
                None => {
                    let m = fuzzy_match(query, &format!("{} {label}", a.category(ctx)))?;
                    (m.score - 20, Vec::new())
                }
            };
            Some(PaletteItem {
                action_id: a.def.id.clone(),
                category: a.category(ctx),
                icon: a.def.icon,
                shortcut: a
                    .def
                    .shortcut
                    .as_ref()
                    .map(|s| shortcut_text(&ctx.translator(), s)),
                weak: is_weak(&label, &positions),
                positions,
                label,
                score,
                goto: None,
            })
        })
        .collect();
    // Stable sort: equal scores keep the registry order.
    items.sort_by_key(|i| std::cmp::Reverse(i.score));
    items.truncate(MAX_RESULTS);
    items
}

/// Whether the matched characters are scattered: neither one contiguous run
/// nor all at the start of words (`np` for "Nouveau projet").
fn is_weak(label: &str, positions: &[usize]) -> bool {
    if positions.len() < 2 {
        return false;
    }
    let chars: Vec<char> = label.chars().collect();
    let contiguous = positions.windows(2).all(|w| w[1] == w[0] + 1);
    let word_starts = positions
        .iter()
        .all(|&p| p == 0 || chars.get(p - 1).is_some_and(|c| !c.is_alphanumeric()));
    !(contiguous || word_starts)
}

/// Orders the palette: commands that match well, then the project's own
/// things (scenes, characters, lines...), then the scattered command matches.
pub fn merge(actions: Vec<PaletteItem>, navigation: Vec<PaletteItem>) -> Vec<PaletteItem> {
    let (weak, strong): (Vec<_>, Vec<_>) = actions.into_iter().partition(|i| i.weak);
    strong.into_iter().chain(navigation).chain(weak).collect()
}

/// Navigation entries for `query`: scenes, characters, variables, labels,
/// assets and dialogue lines of the open project, from the global search
/// index (names first, then texts), at most [`MAX_NAV`]. Needs two letters.
pub fn navigation(handle: &SearchHandle, ctx: &EditorContext<'_>, query: &str) -> Vec<PaletteItem> {
    if query.trim().chars().count() < 2 {
        return Vec::new();
    }
    let (Some(story), Some(project)) = (ctx.story(), ctx.project()) else {
        return Vec::new();
    };
    handle.refresh(story, (0, project.revision()), &ctx.translator());
    let mut hits = handle.query(query);
    // Names are what one jumps to; long texts come after.
    hits.sort_by_key(|h| matches!(h.kind, HitKind::Line | HitKind::Choice));
    hits.into_iter()
        .take(MAX_NAV)
        .map(|hit| {
            let icon = match hit.kind {
                HitKind::Line => icons::CHAT_TEXT,
                HitKind::Choice => icons::GIT_BRANCH,
                HitKind::Character => icons::USER,
                HitKind::Variable => icons::BRACKETS_CURLY,
                HitKind::Label => icons::FLAG,
                HitKind::Scene => icons::FOLDER_SIMPLE,
                HitKind::Asset => icons::IMAGE,
            };
            let chars: Vec<char> = hit.text.chars().collect();
            let shown: String = if chars.len() > NAV_LABEL_CHARS {
                chars[..NAV_LABEL_CHARS].iter().chain(&['…']).collect()
            } else {
                hit.text.clone()
            };
            let positions = hit
                .marks
                .iter()
                .flat_map(|(s, e)| *s..*e)
                .filter(|i| *i < NAV_LABEL_CHARS)
                .collect();
            PaletteItem {
                action_id: String::new(),
                label: shown,
                category: hit.context.clone(),
                icon,
                shortcut: None,
                positions,
                score: hit.score,
                goto: Some(hit.location),
                weak: false,
            }
        })
        .collect()
}

/// Navigation entries shown at most.
const MAX_NAV: usize = 8;
/// Characters of a line shown in a navigation entry.
const NAV_LABEL_CHARS: usize = 64;

/// What the user did in the palette this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteOutcome {
    None,
    Close,
    Run(String),
    /// Show this thing of the project.
    Goto(Location),
}

#[derive(Default)]
pub struct Palette {
    open: bool,
    pub query: String,
    selected: usize,
    scroll_to_selected: bool,
    focus_input: bool,
}

impl Palette {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.focus_input = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// Draws the palette over the editor. `items_for` is called with the
    /// current query whenever the results are needed.
    pub fn show(
        &mut self,
        ctx: &Context,
        theme: &Theme,
        items_for: impl Fn(&str) -> Vec<PaletteItem>,
    ) -> PaletteOutcome {
        if !self.open {
            return PaletteOutcome::None;
        }
        let items = items_for(&self.query);
        if self.selected >= items.len() {
            self.selected = items.len().saturating_sub(1);
        }

        // Keyboard navigation, before the text field sees the keys.
        let (down, up, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if down && !items.is_empty() {
            self.selected = (self.selected + 1) % items.len();
            self.scroll_to_selected = true;
        }
        if up && !items.is_empty() {
            self.selected = (self.selected + items.len() - 1) % items.len();
            self.scroll_to_selected = true;
        }
        let mut outcome = PaletteOutcome::None;
        if enter && let Some(item) = items.get(self.selected) {
            outcome = choose(item);
        }

        let frame = Frame::new()
            .fill(theme.panel())
            .stroke(Stroke::new(1.0, theme.border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(0))
            .shadow(ctx.global_style().visuals.popup_shadow);
        let area = egui::Modal::default_area(Id::new("orbit_palette"))
            .anchor(Align2::CENTER_TOP, vec2(0.0, 84.0));
        let response = egui::Modal::new(Id::new("orbit_palette"))
            .area(area)
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(
                if theme.mode == orbit_editor_api::ThemeMode::Dark {
                    120
                } else {
                    70
                },
            ))
            .show(ctx, |ui| {
                ui.set_width(580.0);
                self.contents(ui, theme, &items, &mut outcome);
            });
        if response.should_close() && outcome == PaletteOutcome::None {
            outcome = PaletteOutcome::Close;
        }
        match &outcome {
            PaletteOutcome::None => {}
            _ => self.close(),
        }
        outcome
    }

    fn contents(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        items: &[PaletteItem],
        outcome: &mut PaletteOutcome,
    ) {
        // Search field.
        let before = self.query.clone();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(
                egui::RichText::new(icons::MAGNIFYING_GLASS)
                    .size(16.0)
                    .color(theme.text_faint()),
            );
            let edit = TextEdit::singleline(&mut self.query)
                .hint_text(ui.t("palette.input_hint"))
                .desired_width(f32::INFINITY)
                .font(FontId::proportional(16.0))
                .frame(Frame::NONE)
                .margin(Margin::symmetric(4, 12));
            let response = ui.add(edit);
            if self.focus_input {
                response.request_focus();
                self.focus_input = false;
            }
        });
        if self.query != before {
            self.selected = 0;
            self.scroll_to_selected = true;
        }
        ui.separator();

        // Results.
        if items.is_empty() {
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(ui.t("palette.empty")).color(theme.text_weak()));
            });
            ui.add_space(18.0);
        } else {
            egui::ScrollArea::vertical()
                .max_height(ROW_HEIGHT * 10.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (index, item) in items.iter().enumerate() {
                        if self.row(ui, theme, item, index) {
                            *outcome = choose(item);
                        }
                    }
                });
        }

        // Footer.
        ui.separator();
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let hint = egui::RichText::new(ui.t("palette.footer"))
                .small()
                .color(theme.text_faint());
            ui.label(hint);
        });
        ui.add_space(2.0);
    }

    /// One result row; `true` when clicked.
    fn row(&mut self, ui: &mut Ui, theme: &Theme, item: &PaletteItem, index: usize) -> bool {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
        if response.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
            // Pointer movement over a row selects it, like the arrow keys.
            self.selected = index;
        }
        let selected = index == self.selected;
        if selected && self.scroll_to_selected {
            ui.scroll_to_rect(rect, Some(Align::Center));
            self.scroll_to_selected = false;
        }
        let painter = ui.painter_at(rect);
        let inner = rect.shrink2(vec2(6.0, 2.0));
        if selected || response.hovered() {
            painter.rect_filled(
                inner,
                CornerRadius::same(7),
                if selected {
                    theme.accent_soft(60)
                } else {
                    theme.surface()
                },
            );
        }
        let mut x = inner.left() + 10.0;
        let center_y = rect.center().y;
        if !item.icon.is_empty() {
            let galley = painter.layout_no_wrap(
                item.icon.to_owned(),
                FontId::proportional(16.0),
                if selected {
                    theme.accent()
                } else {
                    theme.text_weak()
                },
            );
            painter.galley(
                egui::pos2(x, center_y - galley.size().y / 2.0),
                galley,
                Color32::WHITE,
            );
        }
        x += 28.0;
        let job = highlighted(
            &item.label,
            &item.positions,
            FontId::proportional(14.0),
            theme.text(),
            theme.accent(),
        );
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let label_w = galley.size().x;
        painter.galley(
            egui::pos2(x, center_y - galley.size().y / 2.0),
            galley,
            Color32::WHITE,
        );
        x += label_w + 10.0;
        if !item.category.is_empty() {
            let g = painter.layout_no_wrap(
                item.category.clone(),
                FontId::proportional(11.5),
                theme.text_faint(),
            );
            painter.galley(
                egui::pos2(x, center_y - g.size().y / 2.0),
                g,
                Color32::WHITE,
            );
        }
        if let Some(shortcut) = &item.shortcut {
            keycaps(&painter, theme, shortcut, inner.right() - 8.0, center_y);
        }
        response.clicked()
    }
}

fn choose(item: &PaletteItem) -> PaletteOutcome {
    match &item.goto {
        Some(location) => PaletteOutcome::Goto(location.clone()),
        None => PaletteOutcome::Run(item.action_id.clone()),
    }
}

/// Text with the matched characters in the accent color.
fn highlighted(
    text: &str,
    positions: &[usize],
    font: FontId,
    color: Color32,
    highlight: Color32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (i, ch) in text.chars().enumerate() {
        let hit = positions.binary_search(&i).is_ok();
        let format = TextFormat {
            font_id: if hit { bold(font.size) } else { font.clone() },
            color: if hit { highlight } else { color },
            ..Default::default()
        };
        job.append(&ch.to_string(), 0.0, format);
    }
    job
}

/// Draws `Ctrl+Shift+P` as small key caps, right-aligned at `right`.
fn keycaps(painter: &egui::Painter, theme: &Theme, shortcut: &str, right: f32, center_y: f32) {
    let mut x = right;
    for part in shortcut.split('+').rev() {
        let galley = painter.layout_no_wrap(
            part.to_owned(),
            FontId::proportional(11.5),
            theme.text_weak(),
        );
        let size = galley.size() + vec2(12.0, 6.0);
        let rect = egui::Rect::from_min_size(egui::pos2(x - size.x, center_y - size.y / 2.0), size);
        painter.rect(
            rect,
            CornerRadius::same(5),
            theme.surface(),
            Stroke::new(1.0, theme.border()),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
        x = rect.left() - 4.0;
    }
}

#[cfg(test)]
mod tests {
    use orbit_editor_api::{ActionDef, EditorPlugin, EditorState, Plugin, PluginManifest};

    use super::*;

    struct Fixture;

    impl Plugin for Fixture {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("t.fixture", "Fixture", "1")
        }
    }

    impl EditorPlugin for Fixture {
        fn register_ui(&self, reg: &mut EditorRegistry) {
            for (id, label, category) in [
                ("fermer", "Fermer le projet", "Fichier"),
                ("nouveau", "Nouveau projet", "Fichier"),
                ("ouvrir", "Ouvrir un projet", "Fichier"),
                ("console", "Afficher la console", "Affichage"),
                ("off", "Désactivée", "Divers"),
            ] {
                let mut def = ActionDef::new(id, label, |_| {}).category(category);
                if id == "ouvrir" {
                    def = def.shortcut(Modifiers::COMMAND, Key::O);
                }
                if id == "off" {
                    def = def.enabled(|_| false);
                }
                reg.action(def);
            }
        }
    }

    fn results(query: &str) -> Vec<String> {
        let mut registry = EditorRegistry::new();
        registry.install(&Fixture).unwrap();
        let mut state = EditorState::default();
        let ctx = EditorContext::new(None, &mut state, &registry);
        search(&registry, &ctx, query)
            .into_iter()
            .map(|i| i.label)
            .collect()
    }

    #[test]
    fn empty_query_lists_enabled_actions_in_registry_order() {
        assert_eq!(
            results(""),
            [
                "Fermer le projet",
                "Nouveau projet",
                "Ouvrir un projet",
                "Afficher la console"
            ]
        );
    }

    #[test]
    fn results_are_ordered_by_match_quality() {
        assert_eq!(results("ou")[0], "Ouvrir un projet");
        assert_eq!(results("np")[0], "Nouveau projet");
        assert_eq!(results("consol"), ["Afficher la console"]);
        assert!(results("zzz").is_empty());
    }

    #[test]
    fn category_words_also_match() {
        let found = results("affichage");
        assert_eq!(found, ["Afficher la console"]);
    }

    #[test]
    fn shortcuts_are_exposed_for_display() {
        let mut registry = EditorRegistry::new();
        registry.install(&Fixture).unwrap();
        let mut state = EditorState {
            i18n: crate::i18n::translator(orbit_editor_api::Lang::Fr),
            ..EditorState::default()
        };
        let ctx = EditorContext::new(None, &mut state, &registry);
        let hit = search(&registry, &ctx, "ouvrir").remove(0);
        assert_eq!(hit.shortcut.as_deref(), Some("Ctrl+O"));
        assert_eq!(hit.positions, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn scattered_matches_come_after_the_projects_own_entries() {
        let item = |label: &str, weak: bool, goto: Option<Location>| PaletteItem {
            action_id: label.to_owned(),
            label: label.to_owned(),
            category: String::new(),
            icon: "",
            shortcut: None,
            positions: Vec::new(),
            score: 0,
            goto,
            weak,
        };
        // `bob` is scattered in "Basculer le thème clair / sombre" but
        // contiguous in "Bob"; `np` hits the word starts of "Nouveau projet".
        assert!(is_weak("Basculer le thème clair / sombre", &[0, 22, 25]));
        assert!(!is_weak("Bob", &[0, 1, 2]));
        assert!(!is_weak("Nouveau projet", &[0, 8]));
        let merged = merge(
            vec![item("strong", false, None), item("scattered", true, None)],
            vec![item(
                "Bob",
                false,
                Some(Location::Scene(orbit_core::SceneId::new())),
            )],
        );
        let order: Vec<&str> = merged.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(order, ["strong", "Bob", "scattered"]);
    }
}
