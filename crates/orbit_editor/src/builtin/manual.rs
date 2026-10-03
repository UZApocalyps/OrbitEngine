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
    for line in markdown.lines() {
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
            blocks.push(Block::Item(None, text.to_owned()));
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
    if let Some(rest) = code {
        blocks.push(Block::Code(rest));
    }
    blocks
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

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let lang = ctx.lang();
    let chapters = chapters(lang);
    let id = egui::Id::new("orbit_manual_chapter");
    let mut current: usize = ui.data(|d| d.get_temp(id)).unwrap_or(0);
    current = current.min(chapters.len().saturating_sub(1));
    let height = ui.available_height();
    let toc_width = 200.0_f32.min(ui.available_width() * 0.4);
    ui.horizontal_top(|ui| {
        ui.allocate_ui(egui::vec2(toc_width, height), |ui| {
            ui.set_min_size(egui::vec2(toc_width, height));
            egui::ScrollArea::vertical()
                .id_salt("orbit_manual_toc")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (n, (_, md)) in chapters.iter().enumerate() {
                        if ui.selectable_label(n == current, title(md)).clicked() {
                            current = n;
                        }
                    }
                });
        });
        ui.separator();
        ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
            let Some((stem, md)) = chapters.get(current) else {
                ui.label(RichText::new(ctx.t("manual.empty")).color(faint(ui)));
                return;
            };
            egui::ScrollArea::vertical()
                .id_salt(("orbit_manual_scroll", *stem, lang.code()))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(760.0_f32.min(ui.available_width()));
                    render(ui, &parse(md));
                });
        });
    });
    ui.data_mut(|d| d.insert_temp(id, current));
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
}
