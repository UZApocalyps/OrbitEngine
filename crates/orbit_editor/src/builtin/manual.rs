//! Help > Manual: the user manual (`docs/manual/{en,fr}/*.md`, embedded in
//! the binary) rendered in a panel, in the editor's language.
//!
//! The renderer is deliberately small: it understands what the manual uses
//! (headings, paragraphs, bullet and numbered lists, quotes, code blocks,
//! bold, italic, inline code, links shown as text) and nothing else.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, RichText, Ui};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, Lang,
    PanelDef, Plugin, PluginManifest, icons,
};

use super::ids;
use super::ui_util::{faint, weak};
use crate::theme::bold;

macro_rules! chapter {
    ($lang:literal, $file:literal) => {
        (
            $file,
            include_str!(concat!(
                "../../../../docs/manual/",
                $lang,
                "/",
                $file,
                ".md"
            )),
        )
    };
}

/// Chapters of the manual in `lang`: `(file stem, markdown)`, in reading order.
pub fn chapters(lang: Lang) -> &'static [(&'static str, &'static str)] {
    match lang {
        Lang::En => EN,
        Lang::Fr => FR,
    }
}

macro_rules! all_chapters {
    ($lang:literal) => {
        &[
            chapter!($lang, "01-getting-started"),
            chapter!($lang, "02-story-graph"),
            chapter!($lang, "03-characters-animations"),
            chapter!($lang, "04-variables"),
            chapter!($lang, "05-ui-splash"),
            chapter!($lang, "06-curves"),
            chapter!($lang, "07-translations"),
            chapter!($lang, "08-text-view"),
            chapter!($lang, "09-3d-scenes"),
            chapter!($lang, "10-export"),
            chapter!($lang, "11-accessibility"),
            chapter!($lang, "12-assistant"),
            chapter!($lang, "13-plugins"),
            chapter!($lang, "14-scripting"),
            chapter!($lang, "15-renpy-import"),
            chapter!($lang, "16-ai-studio"),
            chapter!($lang, "17-editor-plugins"),
            chapter!($lang, "18-tutorial-first-game"),
            chapter!($lang, "19-tutorial-interface"),
            chapter!($lang, "20-tutorial-point-and-click"),
            chapter!($lang, "21-tutorial-characters"),
            chapter!($lang, "22-tutorial-achievements-translations"),
            chapter!($lang, "23-tutorial-publish"),
            chapter!($lang, "24-tutorial-other-starts"),
            chapter!($lang, "25-tutorial-experts"),
        ]
    };
}

const EN: &[(&str, &str)] = all_chapters!("en");
const FR: &[(&str, &str)] = all_chapters!("fr");

/// Title of a chapter: its first `# ` heading.
pub fn title(markdown: &str) -> &str {
    markdown
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or("?")
        .trim()
}

/// A block of the parsed markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(u8, String),
    Paragraph(String),
    /// `Some(n)` for a numbered item, `None` for a bullet.
    Item(Option<usize>, String),
    /// A bullet indented under another item (one level only).
    SubItem(String),
    /// A pipe table: header cells, then rows (padded to the header width).
    Table(Vec<String>, Vec<Vec<String>>),
    Quote(String),
    Code(String),
}

/// Parses the markdown subset of the manual.
pub fn parse(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph = String::new();
    let mut code: Option<String> = None;
    let flush = |blocks: &mut Vec<Block>, paragraph: &mut String| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(std::mem::take(paragraph)));
        }
    };
    let mut table: Vec<&str> = Vec::new();
    for line in markdown.lines() {
        if code.is_none() {
            if line.trim_start().starts_with('|') {
                flush(&mut blocks, &mut paragraph);
                table.push(line.trim());
                continue;
            }
            flush_table(&mut blocks, &mut table);
        }
        if let Some(buffer) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                blocks.push(Block::Code(code.take().unwrap_or_default()));
            } else {
                if !buffer.is_empty() {
                    buffer.push('\n');
                }
                buffer.push_str(line);
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            flush(&mut blocks, &mut paragraph);
            code = Some(String::new());
        } else if trimmed.is_empty() {
            flush(&mut blocks, &mut paragraph);
        } else if let Some((level, text)) = heading(trimmed) {
            flush(&mut blocks, &mut paragraph);
            blocks.push(Block::Heading(level, text.to_owned()));
        } else if let Some(text) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush(&mut blocks, &mut paragraph);
            let indent = line.len() - line.trim_start().len();
            let nested = indent >= 2 && matches!(blocks.last(), Some(Block::Item(..) | Block::SubItem(_)));
            blocks.push(if nested {
                Block::SubItem(text.to_owned())
            } else {
                Block::Item(None, text.to_owned())
            });
        } else if let Some((n, text)) = numbered(trimmed) {
            flush(&mut blocks, &mut paragraph);
            blocks.push(Block::Item(Some(n), text.to_owned()));
        } else if let Some(text) = trimmed.strip_prefix('>') {
            flush(&mut blocks, &mut paragraph);
            blocks.push(Block::Quote(text.trim().to_owned()));
        } else {
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(trimmed);
        }
    }
    flush(&mut blocks, &mut paragraph);
    flush_table(&mut blocks, &mut table);
    if let Some(rest) = code {
        blocks.push(Block::Code(rest));
    }
    blocks
}

/// Cells of a `| a | b |` row (`\|` is a literal pipe).
fn table_cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|');
    let inner = inner.strip_suffix('|').filter(|_| !inner.ends_with("\\|")).unwrap_or(inner);
    let mut cells = vec![String::new()];
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            chars.next();
            cells.last_mut().unwrap().push('|');
        } else if c == '|' {
            cells.push(String::new());
        } else {
            cells.last_mut().unwrap().push(c);
        }
    }
    cells.iter().map(|c| c.trim().to_owned()).collect()
}

fn is_separator(line: &str) -> bool {
    let cells = table_cells(line);
    !cells.is_empty()
        && cells
            .iter()
            .all(|c| !c.is_empty() && c.contains('-') && c.chars().all(|ch| matches!(ch, '-' | ':')))
}

/// Turns the buffered `|` lines into a table (header, separator, rows), or a
/// paragraph when there is no separator row.
fn flush_table(blocks: &mut Vec<Block>, lines: &mut Vec<&str>) {
    if lines.is_empty() {
        return;
    }
    if lines.len() >= 2 && is_separator(lines[1]) {
        let header = table_cells(lines[0]);
        let rows = lines[2..]
            .iter()
            .map(|l| {
                let mut r = table_cells(l);
                r.resize(header.len(), String::new());
                r
            })
            .collect();
        blocks.push(Block::Table(header, rows));
    } else {
        blocks.push(Block::Paragraph(lines.join(" ")));
    }
    lines.clear();
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=4).contains(&hashes).then_some(())?;
    let text = line[hashes..].strip_prefix(' ')?;
    Some((hashes as u8, text.trim()))
}

fn numbered(line: &str) -> Option<(usize, &str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = line[digits..].strip_prefix(". ")?;
    Some((line[..digits].parse().ok()?, rest))
}

/// A run of inline text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Plain(String),
    Bold(String),
    Italic(String),
    Code(String),
}

/// Splits inline markdown (`**bold**`, `*italic*`, `` `code` ``, `[text](url)`
/// shown as its text) into spans.
pub fn inline(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let find = |from: usize, pat: &[char]| -> Option<usize> {
        (from..chars.len().saturating_sub(pat.len() - 1)).find(|&j| chars[j..j + pat.len()] == *pat)
    };
    while i < chars.len() {
        let rest_len = chars.len() - i;
        let mut push = |spans: &mut Vec<Span>, span: Span| {
            if !plain.is_empty() {
                spans.push(Span::Plain(std::mem::take(&mut plain)));
            }
            spans.push(span);
        };
        if chars[i] == '`'
            && let Some(end) = find(i + 1, &['`'])
        {
            push(&mut spans, Span::Code(chars[i + 1..end].iter().collect()));
            i = end + 1;
        } else if rest_len > 3
            && chars[i] == '*'
            && chars[i + 1] == '*'
            && let Some(end) = find(i + 2, &['*', '*'])
        {
            push(&mut spans, Span::Bold(chars[i + 2..end].iter().collect()));
            i = end + 2;
        } else if chars[i] == '*'
            && let Some(end) = find(i + 1, &['*'])
            && end > i + 1
        {
            push(&mut spans, Span::Italic(chars[i + 1..end].iter().collect()));
            i = end + 1;
        } else if chars[i] == '['
            && let Some(close) = find(i + 1, &[']', '('])
            && let Some(end) = find(close + 2, &[')'])
        {
            plain.extend(&chars[i + 1..close]);
            i = end + 1;
        } else {
            plain.push(chars[i]);
            i += 1;
        }
    }
    if !plain.is_empty() {
        spans.push(Span::Plain(plain));
    }
    spans
}

fn job(ui: &Ui, text: &str, size: f32, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    for span in inline(text) {
        let (s, format) = match span {
            Span::Plain(s) => (s, TextFormat::simple(FontId::proportional(size), color)),
            Span::Bold(s) => (s, TextFormat::simple(bold(size), color)),
            Span::Italic(s) => (
                s,
                TextFormat {
                    italics: true,
                    ..TextFormat::simple(FontId::proportional(size), color)
                },
            ),
            Span::Code(s) => (
                s,
                TextFormat {
                    background: ui.visuals().faint_bg_color,
                    ..TextFormat::simple(FontId::new(size - 1.0, FontFamily::Monospace), color)
                },
            ),
        };
        job.append(&s, 0.0, format);
    }
    job
}

fn render(ui: &mut Ui, blocks: &[Block]) {
    let text = ui.visuals().text_color();
    for block in blocks {
        match block {
            Block::Heading(level, t) => {
                let size = match level {
                    1 => 24.0,
                    2 => 18.0,
                    _ => 15.0,
                };
                ui.add_space(if *level == 1 { 2.0 } else { 12.0 });
                ui.label(RichText::new(t).font(bold(size)));
                ui.add_space(4.0);
            }
            Block::Paragraph(t) => {
                ui.label(job(ui, t, 14.0, text));
                ui.add_space(6.0);
            }
            Block::Item(n, t) => {
                let marker = n.map_or_else(|| "•".to_owned(), |n| format!("{n}."));
                ui.horizontal_top(|ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new(marker).color(weak(ui)));
                    ui.label(job(ui, t, 14.0, text));
                });
            }
            Block::SubItem(t) => {
                ui.horizontal_top(|ui| {
                    ui.add_space(30.0);
                    ui.label(RichText::new("–").color(weak(ui)));
                    ui.label(job(ui, t, 14.0, text));
                });
            }
            Block::Table(header, rows) => {
                let id = ui.next_auto_id();
                egui::Grid::new(id).striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
                    for h in header {
                        ui.label(RichText::new(h).font(bold(14.0)));
                    }
                    ui.end_row();
                    for row in rows {
                        for cell in row {
                            ui.label(job(ui, cell, 14.0, text));
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
            }
            Block::Quote(t) => {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(job(ui, t, 13.5, weak(ui)));
                });
                ui.add_space(4.0);
            }
            Block::Code(t) => {
                egui::Frame::new()
                    .fill(ui.visuals().code_bg_color)
                    .corner_radius(4)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(t).monospace().size(12.5));
                    });
                ui.add_space(6.0);
            }
        }
    }
}

pub struct ManualPlugin;

impl Plugin for ManualPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::MANUAL, "Manual", "0.1.0")
    }
}

impl EditorPlugin for ManualPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.menu("help", "menu.help", 50);
        reg.panel(PanelDef::new(
            "manual",
            "manual.title",
            icons::BOOK_OPEN,
            DockSlot::Hidden,
            panel,
        ));
        reg.action(
            ActionDef::new("open", "manual.open", |c| {
                c.request(EditorRequest::ShowPanel(super::panels::MANUAL.to_owned()));
            })
            .icon(icons::BOOK_OPEN)
            .category("category.help")
            .shortcut(egui::Modifiers::NONE, egui::Key::F1)
            .menu("help", 100),
        );
    }
}

/// Chapters shown in the manual: the 3D chapter only with the experimental
/// 3D switch on.
pub fn visible_chapters(lang: Lang) -> Vec<(&'static str, &'static str)> {
    let on = orbit_core::experimental::enabled_3d();
    chapters(lang)
        .iter()
        .filter(|(stem, _)| on || *stem != "09-3d-scenes")
        .copied()
        .collect()
}

/// Chapters numbered 18 and above are tutorials, the others the reference.
fn is_tutorial(stem: &str) -> bool {
    stem.split('-')
        .next()
        .and_then(|n| n.parse::<u32>().ok())
        .is_some_and(|n| n >= 18)
}

/// A chapter matches a search when one of its headings (the title included)
/// contains it, ignoring case.
pub fn chapter_matches(markdown: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || markdown
            .lines()
            .filter(|l| l.starts_with('#'))
            .any(|l| l.to_lowercase().contains(&q))
}

/// Texts of the manual view (already translated).
pub struct ManualTexts {
    pub reference: String,
    pub tutorials: String,
    pub search_hint: String,
    pub no_match: String,
    pub empty: String,
}

#[derive(Clone, Default)]
struct ManualState {
    current: usize,
    query: String,
    /// The sidebar is hidden by the user (wide panels only).
    collapsed: bool,
}

/// Below this width the chapter list becomes a drop-down.
const NARROW: f32 = 520.0;

type Chapters<'a> = &'a [(&'static str, &'static str)];

/// The manual: chapter list on the left (grouped, searchable, collapsible),
/// content on the right; a drop-down above the content when narrow.
pub fn manual_view(ui: &mut Ui, lang: Lang, chapters: Chapters<'_>, t: &ManualTexts) {
    let id = egui::Id::new("orbit_manual_state");
    let mut st: ManualState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    st.current = st.current.min(chapters.len().saturating_sub(1));
    let narrow = ui.available_width() < NARROW;
    let height = ui.available_height();
    let width = ui.available_width();
    // Explicit top-down layouts: `allocate_ui` inherits the parent layout,
    // which inside `horizontal_top` made the list a row of buttons.
    let topdown = egui::Layout::top_down(egui::Align::Min);
    if narrow {
        ui.allocate_ui_with_layout(egui::vec2(width, height), topdown, |ui| {
            let selected = chapters.get(st.current).map_or("", |(_, md)| title(md));
            egui::ComboBox::from_id_salt("orbit_manual_combo")
                .selected_text(selected)
                .width((width - 16.0).max(80.0))
                .show_ui(ui, |ui| {
                    for (n, (_, md)) in chapters.iter().enumerate() {
                        ui.selectable_value(&mut st.current, n, title(md));
                    }
                });
            ui.add_space(4.0);
            content(ui, lang, chapters, st.current, t);
        });
    } else if st.collapsed {
        ui.allocate_ui_with_layout(egui::vec2(width, height), topdown, |ui| {
            if ui.small_button(icons::LIST).on_hover_text(&t.reference).clicked() {
                st.collapsed = false;
            }
            content(ui, lang, chapters, st.current, t);
        });
    } else {
        let side = 220.0_f32.min(width * 0.35);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(side, height), topdown, |ui| {
                ui.set_min_size(egui::vec2(side, height));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut st.query)
                            .hint_text(&t.search_hint)
                            .desired_width((side - 36.0).max(40.0)),
                    );
                    if ui.small_button(icons::CARET_LEFT).clicked() {
                        st.collapsed = true;
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("orbit_manual_toc")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut any = false;
                        for (group, tutorials) in [(&t.reference, false), (&t.tutorials, true)] {
                            let items: Vec<usize> = chapters
                                .iter()
                                .enumerate()
                                .filter(|(_, (stem, md))| {
                                    is_tutorial(stem) == tutorials && chapter_matches(md, &st.query)
                                })
                                .map(|(n, _)| n)
                                .collect();
                            if items.is_empty() {
                                continue;
                            }
                            any = true;
                            ui.add_space(4.0);
                            ui.label(RichText::new(group).small().strong().color(weak(ui)));
                            for n in items {
                                let label = title(chapters[n].1);
                                if ui.selectable_label(n == st.current, label).clicked() {
                                    st.current = n;
                                }
                            }
                        }
                        if !any {
                            ui.label(RichText::new(&t.no_match).color(faint(ui)));
                        }
                    });
            });
            ui.separator();
            let rest = ui.available_width();
            ui.allocate_ui_with_layout(egui::vec2(rest, height), topdown, |ui| {
                content(ui, lang, chapters, st.current, t);
            });
        });
    }
    ui.data_mut(|d| d.insert_temp(id, st));
}

fn content(ui: &mut Ui, lang: Lang, chapters: Chapters<'_>, current: usize, t: &ManualTexts) {
    let Some((stem, md)) = chapters.get(current) else {
        ui.label(RichText::new(&t.empty).color(faint(ui)));
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt(("orbit_manual_scroll", *stem, lang.code()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_max_width(760.0_f32.min(ui.available_width()));
            render(ui, &parse(md));
        });
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let lang = ctx.lang();
    let chapters = visible_chapters(lang);
    let texts = ManualTexts {
        reference: ctx.t("manual.group.reference"),
        tutorials: ctx.t("manual.group.tutorials"),
        search_hint: ctx.t("manual.search"),
        no_match: ctx.t("manual.no_match"),
        empty: ctx.t("manual.empty"),
    };
    manual_view(ui, lang, &chapters, &texts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks() {
        let blocks = parse("# T\n\nHello\nworld\n\n- a\n1. b\n> q\n```\nx\ny\n```\n");
        assert_eq!(
            blocks,
            vec![
                Block::Heading(1, "T".into()),
                Block::Paragraph("Hello world".into()),
                Block::Item(None, "a".into()),
                Block::Item(Some(1), "b".into()),
                Block::Quote("q".into()),
                Block::Code("x\ny".into()),
            ]
        );
    }

    #[test]
    fn parses_tables_and_nested_items() {
        let blocks = parse("| A | B |\n|---|:-:|\n| 1 | x \\| y |\n| 2 |\n\ntext\n- top\n  - sub\n- next\n| not | table |\n");
        assert_eq!(
            blocks,
            vec![
                Block::Table(
                    vec!["A".into(), "B".into()],
                    vec![vec!["1".into(), "x | y".into()], vec!["2".into(), String::new()]],
                ),
                Block::Paragraph("text".into()),
                Block::Item(None, "top".into()),
                Block::SubItem("sub".into()),
                Block::Item(None, "next".into()),
                Block::Paragraph("| not | table |".into()),
            ]
        );
        // An indented bullet with nothing above it is an ordinary item.
        assert_eq!(parse("  - a"), vec![Block::Item(None, "a".into())]);
    }

    #[test]
    fn parses_inline() {
        assert_eq!(
            inline("a **b** *c* `d` [e](f) g"),
            vec![
                Span::Plain("a ".into()),
                Span::Bold("b".into()),
                Span::Plain(" ".into()),
                Span::Italic("c".into()),
                Span::Plain(" ".into()),
                Span::Code("d".into()),
                Span::Plain(" e g".into()),
            ]
        );
    }

    #[test]
    fn both_languages_have_the_same_chapters_with_titles() {
        assert_eq!(EN.len(), FR.len());
        for ((en_name, en), (fr_name, fr)) in EN.iter().zip(FR) {
            assert_eq!(en_name, fr_name);
            assert_ne!(title(en), "?", "{en_name}");
            assert_ne!(title(fr), "?", "{fr_name}");
            assert!(en.len() > 400 && fr.len() > 400, "{en_name} is too short");
        }
    }

    fn texts() -> ManualTexts {
        ManualTexts {
            reference: "Reference".into(),
            tutorials: "Tutorials".into(),
            search_hint: "Search".into(),
            no_match: "None".into(),
            empty: "Empty".into(),
        }
    }

    /// Runs the manual in a headless frame of `size`; returns the text drawn
    /// and the largest x position of any text.
    fn run(lang: Lang, current: usize, size: egui::Vec2) -> (String, f32) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let list = chapters(lang);
        ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::new("orbit_manual_state"),
                ManualState { current, query: String::new(), collapsed: false },
            )
        });
        let mut out = String::new();
        let mut max_x = 0.0_f32;
        // Two frames: the first lays out, the second is the settled one.
        for _ in 0..2 {
            out.clear();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            let full = ctx.run_ui(raw, |ui| {
                manual_view(ui, lang, list, &texts());
            });
            for clipped in &full.shapes {
                collect(&clipped.shape, &mut out, &mut max_x);
            }
            full.drop_without_applying_deltas();
        }
        (out, max_x)
    }

    fn collect(shape: &egui::epaint::Shape, out: &mut String, max_x: &mut f32) {
        match shape {
            egui::epaint::Shape::Text(t) => {
                out.push_str(t.galley.text());
                out.push('\n');
                *max_x = max_x.max(t.pos.x);
            }
            egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out, max_x)),
            _ => {}
        }
    }

    #[test]
    fn every_chapter_renders_in_both_languages() {
        for lang in [Lang::En, Lang::Fr] {
            for (n, (stem, md)) in chapters(lang).iter().enumerate() {
                let blocks = parse(md);
                assert!(blocks.len() > 3, "{stem}: parsed to nothing");
                let first = blocks
                    .iter()
                    .find_map(|b| if let Block::Paragraph(p) = b { Some(p.clone()) } else { None })
                    .unwrap_or_else(|| panic!("{stem}: no paragraph"));
                // Inline markers are stripped when drawn: compare a plain prefix.
                let plain: String = first
                    .chars()
                    .take(12)
                    .take_while(|c| c.is_alphanumeric() || *c == ' ')
                    .collect();
                for size in [egui::vec2(1000.0, 700.0), egui::vec2(400.0, 700.0)] {
                    let (text, _) = run(lang, n, size);
                    assert!(text.contains(title(md)), "{stem} {size:?}: title missing");
                    assert!(text.contains(plain.trim()), "{stem} {size:?}: content missing ({plain:?})");
                }
            }
        }
    }

    #[test]
    fn the_chapter_list_is_a_column_not_a_row() {
        // Regression: `allocate_ui` inside `horizontal_top` inherited the
        // horizontal layout, so the chapters were laid out in one row that
        // ate the width and left no room for the content.
        let (text, max_x) = run(Lang::En, 0, egui::vec2(1000.0, 700.0));
        assert!(text.contains("Reference") && text.contains("Tutorials"));
        assert!(max_x < 1000.0, "text laid out past the panel: {max_x}");
        // The content (chapter 1 text) is drawn to the right of the list.
        assert!(text.contains(title(chapters(Lang::En)[0].1)));
    }

    #[test]
    fn search_matches_titles_and_headings() {
        let md = "# Alpha chapter\n\n## Beta part\n\ntext gamma";
        assert!(chapter_matches(md, "ALPHA"));
        assert!(chapter_matches(md, "beta"));
        assert!(!chapter_matches(md, "gamma"), "body text is not searched");
        assert!(chapter_matches(md, "  "));
        assert!(is_tutorial("18-tutorial-first-game") && !is_tutorial("17-editor-plugins"));
    }
}
