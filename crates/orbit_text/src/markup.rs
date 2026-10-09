//! Inline tag syntax for dialogue text.
//!
//! ```text
//! Hello {color=#ff0}yellow{/color} {b}bold{/b}{w=0.5} and a pause.
//! ```
//!
//! * `{color=#rgb}`, `{color=#rrggbb}`, `{color=#rrggbbaa}` or a basic name
//!   (`red`, `green`, `blue`, `yellow`, `orange`, `cyan`, `magenta`, `white`,
//!   `black`, `gray`) ... `{/color}`
//! * `{b}...{/b}` bold, `{i}...{/i}` italic
//! * `{w=0.5}` pause of 0.5 s in the typewriter
//! * `{bubble=speech|thought|shout}` shape of the comic bubble the line is
//!   shown in (read by the UI, invisible in the text)
//! * `{{` and `}}` are literal braces
//!
//! Malformed or unknown tags are kept as literal text rather than dropped, so
//! a typo is visible on screen instead of silently eating content.

use unicode_segmentation::UnicodeSegmentation;

/// Style of one text segment.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SegmentStyle {
    /// sRGBA override, `None` = the block's default color.
    pub color: Option<[u8; 4]>,
    pub bold: bool,
    pub italic: bool,
}

/// A run of text with a uniform style.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub text: String,
    pub style: SegmentStyle,
}

/// A pause: after `grapheme` graphemes are revealed, wait `seconds`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pause {
    pub grapheme: usize,
    pub seconds: f32,
}

/// Result of [`parse_markup`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedText {
    pub segments: Vec<Segment>,
    pub pauses: Vec<Pause>,
}

impl ParsedText {
    /// Plain text (all segments concatenated, tags removed).
    pub fn plain(&self) -> String {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }

    /// Number of grapheme clusters in the plain text.
    pub fn grapheme_count(&self) -> usize {
        self.plain().graphemes(true).count()
    }
}

/// Parses a color: `#rgb`, `#rrggbb`, `#rrggbbaa` or a basic name.
pub fn parse_color(s: &str) -> Option<[u8; 4]> {
    let named = match s.to_ascii_lowercase().as_str() {
        "red" => Some([230, 57, 70, 255]),
        "green" => Some([76, 175, 80, 255]),
        "blue" => Some([66, 133, 244, 255]),
        "yellow" => Some([255, 220, 60, 255]),
        "orange" => Some([255, 152, 0, 255]),
        "cyan" => Some([0, 188, 212, 255]),
        "magenta" => Some([224, 64, 251, 255]),
        "white" => Some([255, 255, 255, 255]),
        "black" => Some([0, 0, 0, 255]),
        "gray" | "grey" => Some([158, 158, 158, 255]),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let hex = s.strip_prefix('#')?;
    if !hex.is_ascii() {
        return None;
    }
    let nib = |c: &str| u8::from_str_radix(c, 16).ok();
    match hex.len() {
        3 => {
            let v: Option<Vec<u8>> = (0..3).map(|i| nib(&hex[i..=i]).map(|n| n * 17)).collect();
            let v = v?;
            Some([v[0], v[1], v[2], 255])
        }
        6 | 8 => {
            let mut out = [255u8; 4];
            for i in 0..hex.len() / 2 {
                out[i] = nib(&hex[2 * i..2 * i + 2])?;
            }
            Some(out)
        }
        _ => None,
    }
}

/// Converts an sRGB channel (`0..=255`) to linear `0.0..=1.0`.
pub fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

enum Tag {
    ColorOpen([u8; 4]),
    ColorClose,
    BoldOpen,
    BoldClose,
    ItalicOpen,
    ItalicClose,
    Wait(f32),
    /// A display hint for the UI (`{bubble=...}`): no text, no style.
    Hint,
}

fn parse_tag(inner: &str) -> Option<Tag> {
    let inner = inner.trim();
    if let Some(v) = inner.strip_prefix("bubble=") {
        return matches!(v.trim(), "speech" | "thought" | "shout").then_some(Tag::Hint);
    }
    if let Some(v) = inner.strip_prefix("color=") {
        return parse_color(v.trim()).map(Tag::ColorOpen);
    }
    if let Some(v) = inner.strip_prefix("w=") {
        let secs: f32 = v.trim().parse().ok()?;
        return (secs.is_finite() && secs >= 0.0).then_some(Tag::Wait(secs.min(30.0)));
    }
    match inner {
        "/color" => Some(Tag::ColorClose),
        "b" => Some(Tag::BoldOpen),
        "/b" => Some(Tag::BoldClose),
        "i" => Some(Tag::ItalicOpen),
        "/i" => Some(Tag::ItalicClose),
        _ => None,
    }
}

/// Parses `input` into styled segments and pauses.
pub fn parse_markup(input: &str) -> ParsedText {
    let mut out = ParsedText::default();
    let mut colors: Vec<[u8; 4]> = Vec::new();
    let (mut bold, mut italic) = (0u32, 0u32);
    let mut buf = String::new();
    let mut graphemes_done = 0usize;

    fn flush(
        out: &mut ParsedText,
        buf: &mut String,
        style: SegmentStyle,
        graphemes_done: &mut usize,
    ) {
        if buf.is_empty() {
            return;
        }
        *graphemes_done += buf.graphemes(true).count();
        let text = std::mem::take(buf);
        match out.segments.last_mut() {
            Some(last) if last.style == style => last.text.push_str(&text),
            _ => out.segments.push(Segment { text, style }),
        }
    }

    let style_of = |colors: &[[u8; 4]], bold: u32, italic: u32| SegmentStyle {
        color: colors.last().copied(),
        bold: bold > 0,
        italic: italic > 0,
    };

    let mut rest = input;
    while let Some(pos) = rest.find(['{', '}']) {
        buf.push_str(&rest[..pos]);
        let after = &rest[pos..];
        if after.starts_with("{{") || after.starts_with("}}") {
            buf.push_str(&after[..1]);
            rest = &after[2..];
            continue;
        }
        if after.starts_with('{')
            && let Some(end) = after.find('}')
            && let Some(tag) = parse_tag(&after[1..end])
        {
            let before = style_of(&colors, bold, italic);
            match tag {
                Tag::ColorOpen(c) => {
                    flush(&mut out, &mut buf, before, &mut graphemes_done);
                    colors.push(c);
                }
                Tag::ColorClose => {
                    flush(&mut out, &mut buf, before, &mut graphemes_done);
                    colors.pop();
                }
                Tag::BoldOpen => {
                    flush(&mut out, &mut buf, before, &mut graphemes_done);
                    bold += 1;
                }
                Tag::BoldClose => {
                    flush(&mut out, &mut buf, before, &mut graphemes_done);
                    bold = bold.saturating_sub(1);
                }
                Tag::ItalicOpen => {
                    flush(&mut out, &mut buf, before, &mut graphemes_done);
                    italic += 1;
                }
                Tag::ItalicClose => {
                    flush(&mut out, &mut buf, before, &mut graphemes_done);
                    italic = italic.saturating_sub(1);
                }
                Tag::Hint => {}
                Tag::Wait(seconds) => {
                    flush(&mut out, &mut buf, before, &mut graphemes_done);
                    out.pauses.push(Pause {
                        grapheme: graphemes_done,
                        seconds,
                    });
                }
            }
            rest = &after[end + 1..];
            continue;
        }
        // Not a valid tag: keep the brace literally.
        buf.push_str(&after[..1]);
        rest = &after[1..];
    }
    buf.push_str(rest);
    let style = style_of(&colors, bold, italic);
    flush(&mut out, &mut buf, style, &mut graphemes_done);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_one_segment() {
        let p = parse_markup("Hello world");
        assert_eq!(p.segments.len(), 1);
        assert_eq!(p.plain(), "Hello world");
        assert!(p.pauses.is_empty());
    }

    #[test]
    fn color_and_style_tags() {
        let p = parse_markup("a{color=#ff0}b{b}c{/b}{/color}d");
        assert_eq!(p.plain(), "abcd");
        assert_eq!(p.segments.len(), 4);
        assert_eq!(p.segments[1].style.color, Some([255, 255, 0, 255]));
        assert!(p.segments[2].style.bold);
        assert_eq!(p.segments[2].style.color, Some([255, 255, 0, 255]));
        assert_eq!(p.segments[3].style, SegmentStyle::default());
    }

    #[test]
    fn pause_counts_graphemes_before_it() {
        let p = parse_markup("ab{w=0.5}cd{w=1}");
        assert_eq!(p.plain(), "abcd");
        assert_eq!(
            p.pauses,
            vec![
                Pause {
                    grapheme: 2,
                    seconds: 0.5
                },
                Pause {
                    grapheme: 4,
                    seconds: 1.0
                }
            ]
        );
    }

    #[test]
    fn escaped_and_malformed_braces_stay_literal() {
        assert_eq!(parse_markup("{{x}}").plain(), "{x}");
        assert_eq!(parse_markup("a {unknown} b").plain(), "a {unknown} b");
        assert_eq!(parse_markup("{bubble=thought}Hmm...").plain(), "Hmm...");
        assert_eq!(parse_markup("{bubble=nope}x").plain(), "{bubble=nope}x");
        assert_eq!(parse_markup("open { only").plain(), "open { only");
        assert_eq!(parse_markup("{color=bad}x").plain(), "{color=bad}x");
        assert_eq!(parse_markup("{w=-1}x").plain(), "{w=-1}x");
        assert_eq!(parse_markup("{/b}x").plain(), "x");
    }

    #[test]
    fn graphemes_not_bytes_and_unicode_safe() {
        let p = parse_markup("日本{w=1}語e\u{301}");
        assert_eq!(p.pauses[0].grapheme, 2);
        assert_eq!(p.grapheme_count(), 4);
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#f00"), Some([255, 0, 0, 255]));
        assert_eq!(parse_color("#11223344"), Some([0x11, 0x22, 0x33, 0x44]));
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#éé"), None);
        assert!(parse_color("Red").is_some());
    }
}
