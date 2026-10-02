//! Small pure helpers of the shell: dates, plain text, language names.

/// Days since 1970-01-01 to a civil (year, month, day) date (proleptic
/// Gregorian calendar; Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// `2026-10-02 18:42`: a timestamp (milliseconds since the Unix epoch) in the
/// local time zone, given as its offset from UTC in minutes.
pub fn format_timestamp(ms: u64, utc_offset_minutes: i32) -> String {
    let secs = (ms / 1000) as i64 + i64::from(utc_offset_minutes) * 60;
    let days = secs.div_euclid(86_400);
    let in_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        in_day / 3600,
        (in_day % 3600) / 60
    )
}

/// `markup` without its `{tags}`: what the history and the save previews
/// show.
pub fn plain_text(markup: &str) -> String {
    orbit_text::parse_markup(markup).plain()
}

/// The name a language is listed under: the translation's own name when it
/// has one, else a built-in name for common Latin-script languages, else its
/// code.
pub fn language_name(code: &str, name: &str) -> String {
    if !name.is_empty() && name != code {
        return name.to_owned();
    }
    let primary = code.split(['-', '_']).next().unwrap_or(code);
    match primary.to_ascii_lowercase().as_str() {
        "en" => "English",
        "fr" => "Fran\u{e7}ais",
        "de" => "Deutsch",
        "es" => "Espa\u{f1}ol",
        "it" => "Italiano",
        "pt" => "Portugu\u{ea}s",
        "nl" => "Nederlands",
        _ => return code.to_owned(),
    }
    .to_owned()
}

/// Lowercase file-system / storage friendly identifier of a game title.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "game".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_known_dates() {
        assert_eq!(format_timestamp(0, 0), "1970-01-01 00:00");
        // 2026-10-02 18:42:00 UTC
        assert_eq!(format_timestamp(1_790_966_520_000, 0), "2026-10-02 18:42");
        // Leap day.
        assert_eq!(format_timestamp(951_782_400_000, 0), "2000-02-29 00:00");
    }

    #[test]
    fn local_offset_can_change_the_day() {
        // 23:30 UTC on Dec 31st is 00:30 on Jan 1st at UTC+1.
        let ms = 1_798_759_800_000; // 2026-12-31 23:30:00 UTC
        assert_eq!(format_timestamp(ms, 0), "2026-12-31 23:30");
        assert_eq!(format_timestamp(ms, 60), "2027-01-01 00:30");
        assert_eq!(format_timestamp(ms, -300), "2026-12-31 18:30");
    }

    #[test]
    fn plain_text_drops_tags() {
        assert_eq!(plain_text("a {color=red}b{/color} c"), "a b c");
        assert_eq!(plain_text("no tags"), "no tags");
    }

    #[test]
    fn language_names() {
        assert_eq!(language_name("fr", "Fran\u{e7}ais"), "Fran\u{e7}ais");
        assert_eq!(language_name("en", "en"), "English");
        assert_eq!(language_name("pt-BR", "pt-BR"), "Portugu\u{ea}s");
        assert_eq!(language_name("xx", ""), "xx");
    }

    #[test]
    fn slugs_are_storage_friendly() {
        assert_eq!(slug("Orbit Demo!"), "orbit-demo");
        assert_eq!(slug("  "), "game");
        assert_eq!(slug("Caf\u{e9} 2"), "caf-2");
    }
}
