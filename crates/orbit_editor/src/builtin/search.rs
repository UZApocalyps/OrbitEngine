//! Global search panel (Ctrl+Shift+F): dialogue lines, choices, characters,
//! variables, labels, scenes and assets, grouped by kind, keyboard
//! navigable, a click (or Enter) reveals the result.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontId, Key, Modifiers, Ui};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, Location,
    PanelDef, Plugin, PluginManifest, icons,
};

use super::ids;
use super::reveal::reveal;
use super::ui_util::{accent, accent_soft, faint, list_row, weak};
use crate::search::{Hit, HitKind, SearchHandle};

const ROW_HEIGHT: f32 = 26.0;
const HEADER_HEIGHT: f32 = 24.0;

pub struct SearchPlugin {
    handle: SearchHandle,
    state: Rc<RefCell<SearchUi>>,
}

impl SearchPlugin {
    pub fn new(handle: SearchHandle) -> Self {
        Self {
            handle,
            state: Rc::default(),
        }
    }
}

/// Least time between two index rebuilds caused by story edits alone.
const REBUILD_EVERY: Duration = Duration::from_millis(400);

#[derive(Default)]
struct SearchUi {
    query: String,
    hits: Vec<Hit>,
    /// `(query, project revision, language)` the hits are for.
    computed_for: Option<(String, u64, orbit_editor_api::Lang)>,
    computed_at: Option<Instant>,
    took: Duration,
    selected: usize,
    scroll_to_selected: bool,
}

impl Plugin for SearchPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::SEARCH, "Recherche globale", "0.1.0")
    }
}

impl EditorPlugin for SearchPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        let (handle, state) = (self.handle.clone(), self.state.clone());
        reg.panel(PanelDef::new(
            "search",
            "search.title",
            icons::MAGNIFYING_GLASS,
            DockSlot::Bottom,
            move |ui, ctx| panel(ui, ctx, &handle, &state),
        ));
        reg.action(
            ActionDef::new("open", "search.open", |c| {
                c.state_mut().focus_panel = Some(super::panels::SEARCH.to_owned());
                c.request(EditorRequest::ShowPanel(super::panels::SEARCH.to_owned()));
            })
            .icon(icons::MAGNIFYING_GLASS)
            .category("category.edit")
            .shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::F)
            .menu("edit", 300)
            .enabled(|c| c.has_project()),
        );
    }
}

/// A line of the results.
enum Row {
    Header(HitKind, usize),
    Hit(usize),
}

fn rows(hits: &[Hit]) -> Vec<Row> {
    let mut rows = Vec::with_capacity(hits.len() + 8);
    let mut group: Option<HitKind> = None;
    for (i, hit) in hits.iter().enumerate() {
        if group != Some(hit.kind) {
            group = Some(hit.kind);
            let n = hits.iter().filter(|h| h.kind == hit.kind).count();
            rows.push(Row::Header(hit.kind, n));
        }
        rows.push(Row::Hit(i));
    }
    rows
}

fn kind_icon(kind: HitKind) -> &'static str {
    match kind {
        HitKind::Line => icons::CHAT_TEXT,
        HitKind::Choice => icons::GIT_BRANCH,
        HitKind::Character => icons::USER,
        HitKind::Variable => icons::BRACKETS_CURLY,
        HitKind::Label => icons::FLAG,
        HitKind::Scene => icons::FOLDER_SIMPLE,
        HitKind::Asset => icons::IMAGE,
    }
}

fn panel(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    handle: &SearchHandle,
    state: &Rc<RefCell<SearchUi>>,
) {
    let mut st = state.borrow_mut();
    let Some(revision) = ctx.project().map(orbit_project::Project::revision) else {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(ctx.t("search.no_project")).color(weak(ui)));
        });
        return;
    };
    // Keyboard navigation, before the text field sees the keys.
    let (down, up, enter) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
        )
    });
    if let Some(query) = ctx
        .pref(super::ids::SEARCH, "initial_query")
        .map(str::to_owned)
        .filter(|q| !q.is_empty())
    {
        st.query = query;
        ctx.set_pref(super::ids::SEARCH, "initial_query", "");
    }
    let want_focus = ctx.state().focus_panel.as_deref() == Some(super::panels::SEARCH);
    if want_focus {
        ctx.state_mut().focus_panel = None;
    }
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(icons::MAGNIFYING_GLASS)
                .size(16.0)
                .color(faint(ui)),
        );
        let hint = ctx.t("search.hint");
        let edit = ui.add(
            egui::TextEdit::singleline(&mut st.query)
                .hint_text(hint)
                .desired_width(f32::INFINITY)
                .font(FontId::proportional(14.5)),
        );
        if want_focus {
            edit.request_focus();
            // Select the previous query: typing replaces it.
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit.id) {
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(st.query.chars().count()),
                    )));
                state.store(ui.ctx(), edit.id);
            }
        }
    });

    // The results, recomputed when the query or the project changed. An
    // empty query needs no index. While the story keeps changing (typing
    // elsewhere with the panel open) the index is rebuilt at most every
    // `REBUILD_EVERY`: the previous hits stay meanwhile.
    let key = (st.query.clone(), revision, ctx.lang());
    let only_revision = st
        .computed_for
        .as_ref()
        .is_some_and(|(q, r, l)| *q == key.0 && *l == key.2 && *r != revision);
    let wait = st
        .computed_at
        .map(|t| REBUILD_EVERY.saturating_sub(t.elapsed()))
        .filter(|d| only_revision && !d.is_zero());
    if let Some(wait) = wait {
        ui.ctx().request_repaint_after(wait);
    } else if st.computed_for.as_ref() != Some(&key) {
        if !st.query.trim().is_empty()
            && let Some(story) = ctx.story()
        {
            handle.refresh(story, (ctx.state().project_generation, revision), &ctx.translator());
        }
        let started = Instant::now();
        let hits = if st.query.trim().is_empty() {
            Vec::new()
        } else {
            handle.query(&st.query)
        };
        st.took = started.elapsed();
        st.hits = hits;
        st.computed_for = Some(key);
        st.computed_at = Some(Instant::now());
        if !only_revision {
            st.selected = 0;
            st.scroll_to_selected = true;
        }
        st.selected = st.selected.min(st.hits.len().saturating_sub(1));
    }
    let n = st.hits.len();
    if n > 0 {
        if down {
            st.selected = (st.selected + 1) % n;
            st.scroll_to_selected = true;
        }
        if up {
            st.selected = (st.selected + n - 1) % n;
            st.scroll_to_selected = true;
        }
    }
    let mut chosen: Option<Location> = None;
    if enter && let Some(hit) = st.hits.get(st.selected) {
        chosen = Some(hit.location.clone());
    }

    ui.add_space(2.0);
    if st.query.trim().is_empty() {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(ctx.t("search.type_to_search")).color(weak(ui)));
            ui.label(
                egui::RichText::new(ctx.t("search.keys_hint"))
                    .small()
                    .color(faint(ui)),
            );
        });
        return;
    }
    if st.hits.is_empty() {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(ctx.t("search.no_results")).color(weak(ui)));
        });
        return;
    }
    let texts = ctx.translator();
    let list = rows(&st.hits);
    let selected = st.selected;
    let scroll_to = std::mem::take(&mut st.scroll_to_selected);
    let height = ui.available_height() - 20.0;
    let mut area = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(height.max(40.0));
    if scroll_to {
        // Rows have two heights: compute the offset of the selected one.
        let mut y = 0.0;
        for row in &list {
            match row {
                Row::Header(..) => y += HEADER_HEIGHT,
                Row::Hit(i) => {
                    if *i == selected {
                        break;
                    }
                    y += ROW_HEIGHT;
                }
            }
        }
        let current = ui
            .ctx()
            .data(|d| d.get_temp::<f32>(egui::Id::new("orbit_search_scroll")))
            .unwrap_or(0.0);
        if y < current {
            area = area.vertical_scroll_offset((y - HEADER_HEIGHT).max(0.0));
        } else if y + ROW_HEIGHT > current + height {
            area = area.vertical_scroll_offset(y + ROW_HEIGHT - height + 8.0);
        }
    }
    let mut clicked: Option<usize> = None;
    let out = area.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let hits = &st.hits;
        for row in &list {
            match row {
                Row::Header(kind, count) => {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), HEADER_HEIGHT),
                        egui::Sense::hover(),
                    );
                    ui.painter().text(
                        egui::pos2(rect.left() + 10.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        format!(
                            "{}  {}  ·  {count}",
                            kind_icon(*kind),
                            kind.title(&texts).to_uppercase()
                        ),
                        crate::theme::bold(10.5),
                        faint(ui),
                    );
                }
                Row::Hit(i) => {
                    let hit = &hits[*i];
                    if !ui.is_rect_visible(egui::Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(10.0, ROW_HEIGHT),
                    )) {
                        ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), ROW_HEIGHT),
                            egui::Sense::hover(),
                        );
                        continue;
                    }
                    let (rect, response) = list_row(ui, ROW_HEIGHT, *i == selected);
                    paint_hit(ui, rect, hit, *i == selected);
                    if response.clicked() {
                        clicked = Some(*i);
                    }
                }
            }
        }
    });
    ui.ctx().data_mut(|d| {
        d.insert_temp(egui::Id::new("orbit_search_scroll"), out.state.offset.y);
    });
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(ctx.tn(
                "search.summary",
                st.hits.len(),
                &[("ms", &format!("{:.1}", st.took.as_secs_f64() * 1000.0))],
            ))
            .small()
            .color(faint(ui)),
        );
    });
    if let Some(i) = clicked {
        st.selected = i;
        chosen = Some(st.hits[i].location.clone());
    }
    drop(st);
    if let Some(location) = chosen {
        reveal(ctx, location);
    }
}

fn paint_hit(ui: &Ui, rect: egui::Rect, hit: &Hit, selected: bool) {
    let painter = ui.painter_at(rect);
    let mid = rect.center().y;
    if selected {
        painter.rect_filled(
            rect.shrink2(egui::vec2(2.0, 0.0)),
            egui::CornerRadius::same(5),
            accent_soft(ui, 55),
        );
    }
    painter.text(
        egui::pos2(rect.left() + 18.0, mid),
        Align2::CENTER_CENTER,
        kind_icon(hit.kind),
        FontId::proportional(13.0),
        if selected { accent(ui) } else { weak(ui) },
    );
    // Context at the right edge, the text (with the matches in bold) before it.
    let ctx_galley =
        painter.layout_no_wrap(hit.context.clone(), FontId::proportional(11.5), faint(ui));
    let ctx_w = ctx_galley.size().x.min(rect.width() * 0.4);
    painter.galley(
        egui::pos2(rect.right() - ctx_w - 10.0, mid - ctx_galley.size().y / 2.0),
        ctx_galley,
        Color32::WHITE,
    );
    let mut job = LayoutJob::default();
    let text_color = ui.visuals().text_color();
    let mark_color = accent(ui);
    let mut at = 0;
    let chars: Vec<char> = hit.text.chars().collect();
    let mut push = |range: std::ops::Range<usize>, marked: bool| {
        if range.is_empty() {
            return;
        }
        let s: String = chars[range].iter().collect();
        job.append(
            &s,
            0.0,
            TextFormat {
                font_id: if marked {
                    crate::theme::bold(13.0)
                } else {
                    FontId::proportional(13.0)
                },
                color: if marked { mark_color } else { text_color },
                ..TextFormat::default()
            },
        );
    };
    for (s, e) in &hit.marks {
        let (s, e) = ((*s).min(chars.len()), (*e).min(chars.len()));
        push(at..s, false);
        push(s..e, true);
        at = e.max(at);
    }
    push(at..chars.len(), false);
    job.wrap.max_rows = 1;
    job.wrap.max_width = (rect.width() - ctx_w - 60.0).max(40.0);
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    painter.galley(
        egui::pos2(rect.left() + 34.0, mid - galley.size().y / 2.0),
        galley,
        Color32::WHITE,
    );
}
