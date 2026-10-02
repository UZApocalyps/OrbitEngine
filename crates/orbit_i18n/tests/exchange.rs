use orbit_core::{StringId, StringTable};
use orbit_i18n::{ExchangeFormat, Translation, source_hash, stale_entries};

fn table() -> (StringTable, [StringId; 4]) {
    let mut t = StringTable::default();
    let ids = [
        t.add("Hello"),
        t.add("Say \"hi\", then\nleave\\now"),
        t.add("Thanks"),
        t.add("Tab\there"),
    ];
    (t, ids)
}

fn french(ids: [StringId; 4], source: &StringTable) -> Translation {
    let mut fr = Translation::new("fr", "Francais");
    fr.entries.insert(ids[0], "Bonjour".into());
    fr.entries
        .insert(ids[1], "Dis \"salut\", puis\npars\\maintenant".into());
    fr.entries.insert(ids[3], "Tab\ticite, avec virgule".into());
    fr.stamp_missing(source);
    fr
}

#[test]
fn round_trip_both_formats() {
    for format in [ExchangeFormat::Po, ExchangeFormat::Csv] {
        let (src, ids) = table();
        let fr = french(ids, &src);
        let text = fr.export(&src, format);
        let back = Translation::import(&text, format, if format == ExchangeFormat::Po { "" } else { "fr" }, "Francais").unwrap();
        assert_eq!(back.lang, "fr", "{format:?}");
        assert_eq!(back.entries.get(&ids[0]).unwrap(), "Bonjour");
        assert_eq!(back.entries.get(&ids[1]), fr.entries.get(&ids[1]));
        assert_eq!(back.entries.get(&ids[3]), fr.entries.get(&ids[3]));
        // Untranslated strings come back empty: nothing is missing from coverage logic.
        assert!(back.entries.get(&ids[2]).is_none_or(|s| s.is_empty()));
        assert!(stale_entries(&src, &back).is_empty());
        assert_eq!(back.source_hashes.get(&ids[0]), Some(&source_hash("Hello")));
    }
}

#[test]
fn stale_detection_after_source_edit() {
    let (mut src, ids) = table();
    let fr = french(ids, &src);
    assert!(stale_entries(&src, &fr).is_empty());
    src.set(ids[0], "Hello there");
    let stale = stale_entries(&src, &fr);
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].id, ids[0]);
    assert_eq!(stale[0].source, "Hello there");
    assert_eq!(stale[0].translation, "Bonjour");

    // Export flags it; an import of that export stays stale, not silently fresh.
    let po = fr.export(&src, ExchangeFormat::Po);
    assert!(po.contains("#, fuzzy"));
    let csv = fr.export(&src, ExchangeFormat::Csv);
    assert!(csv.contains(",stale"));

    // Edit between export and import: the file carries the old source.
    let (src2, ids2) = table();
    let exported = french(ids2, &src2).export(&src2, ExchangeFormat::Csv);
    let mut edited = src2.clone();
    edited.set(ids2[2], "Many thanks");
    let imported = Translation::import(&exported, ExchangeFormat::Csv, "fr", "").unwrap();
    assert!(stale_entries(&edited, &imported).is_empty(), "untranslated");
    edited.set(ids2[0], "Hi");
    assert_eq!(stale_entries(&edited, &imported).len(), 1);

    // Reviewing clears the flag.
    let mut fr = fr;
    assert_eq!(fr.accept_stale(&src), 1);
    assert!(stale_entries(&src, &fr).is_empty());
}

#[test]
fn legacy_files_are_unchanged() {
    let ron = "(lang: \"fr\", name: \"F\", entries: {\"01JBX0T0000000000000000001\": \"A\"})";
    let t = Translation::from_ron(ron).unwrap();
    assert!(t.source_hashes.is_empty());
    let out = t.to_ron().unwrap();
    assert!(!out.contains("source_hashes"), "{out}");
    let (src, _) = table();
    assert!(stale_entries(&src, &t).is_empty());
}

#[test]
fn ron_keeps_hashes() {
    let (src, ids) = table();
    let fr = french(ids, &src);
    let back = Translation::from_ron(&fr.to_ron().unwrap()).unwrap();
    assert_eq!(fr, back);
}

#[test]
fn merge_keeps_existing_for_empty_rows() {
    let (src, ids) = table();
    let mut fr = french(ids, &src);
    let po = fr.export(&src, ExchangeFormat::Po);
    let imported = Translation::import(&po, ExchangeFormat::Po, "fr", "").unwrap();
    assert_eq!(fr.merge(imported), 0);
    assert_eq!(fr.entries.get(&ids[0]).unwrap(), "Bonjour");
}

#[test]
fn import_errors() {
    assert!(Translation::import("id,source\n", ExchangeFormat::Csv, "fr", "").is_err());
    assert!(Translation::import("id,translation\n\"a", ExchangeFormat::Csv, "fr", "").is_err());
    assert!(Translation::import("garbage", ExchangeFormat::Po, "fr", "").is_err());
    assert!(Translation::import("id,translation\n", ExchangeFormat::Csv, "", "").is_err());
    // CSV with BOM and a different column order, foreign ids ignored.
    let t = Translation::import(
        "\u{feff}translation,id\r\nBonjour,01JBX0T0000000000000000001\r\nx,not-an-id\r\n",
        ExchangeFormat::Csv,
        "fr",
        "",
    )
    .unwrap();
    assert_eq!(t.entries.len(), 1);
    assert!(t.source_hashes.is_empty());
}
