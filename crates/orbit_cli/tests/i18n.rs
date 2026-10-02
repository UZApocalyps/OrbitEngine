use std::io::Cursor;
use std::path::{Path, PathBuf};

fn orbit(args: &[&str]) -> (u8, String, String) {
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let mut input = Cursor::new(Vec::new());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = orbit_cli::run(&args, &mut input, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orbit_cli_i18n_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("i18n")).unwrap();
    let stories = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stories");
    std::fs::copy(stories.join("test_minimal.ron"), dir.join("story.ron")).unwrap();
    std::fs::copy(
        stories.join("i18n/test_minimal.fr.ron"),
        dir.join("i18n/story.fr.ron"),
    )
    .unwrap();
    dir
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn export_import_round_trip_and_stale() {
    for ext in ["po", "csv"] {
        let dir = project(ext);
        let story = dir.join("story.ron");
        let file = dir.join(format!("out.{ext}"));
        let (code, _, e) = orbit(&["i18n", "export", s(&story), "--lang", "fr", "--out", s(&file)]);
        assert_eq!(code, 0, "{e}");

        // Legacy file: nothing is stale until stamped.
        let (code, _, _) = orbit(&["i18n", "stale", s(&story)]);
        assert_eq!(code, 0);

        // Importing the export into a fresh language file works (csv needs --lang).
        let (code, out, e) = orbit(&["i18n", "import", s(&file), s(&story), "--lang", "fr"]);
        assert_eq!(code, 0, "{e}");
        assert!(out.contains("0 stale"), "{out}");

        // Edit a source text: the imported hashes now flag it.
        let src = std::fs::read_to_string(&story).unwrap();
        let edited = src.replacen("Take the job.", "Grab the job.", 1);
        assert_ne!(edited, src);
        std::fs::write(&story, edited).unwrap();
        let (code, out, err) = orbit(&["i18n", "stale", s(&story)]);
        assert_eq!(code, 1, "{out}{err}");
        assert!(err.contains("stale: fr"));

        let (code, _, _) = orbit(&["i18n", "accept", s(&story), "--lang", "fr"]);
        assert_eq!(code, 0);
        let (code, _, _) = orbit(&["i18n", "stale", s(&story)]);
        assert_eq!(code, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn usage_errors() {
    assert_eq!(orbit(&["i18n"]).0, 2);
    assert_eq!(orbit(&["i18n", "export", "x.ron"]).0, 2);
    assert_eq!(orbit(&["i18n", "bogus"]).0, 2);
    assert!(orbit(&["help"]).1.contains("orbit i18n import"));
}
