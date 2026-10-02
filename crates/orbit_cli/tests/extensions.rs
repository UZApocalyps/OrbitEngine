//! `orbit validate` accepts `.orbit` and legacy `.ron` stories (TL-02).

use std::io::Cursor;
use std::path::{Path, PathBuf};

fn run(args: &[&str]) -> u8 {
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let mut input = Cursor::new(Vec::new());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    orbit_cli::run(&args, &mut input, &mut out, &mut err)
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("orbit_cli_ext_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn validate_accepts_orbit_and_ron_files_and_folders_of_either() {
    let dir = temp_dir("all");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stories/test_minimal.ron");
    let text = std::fs::read_to_string(fixture).unwrap();

    let file = dir.join("a.orbit");
    std::fs::write(&file, &text).unwrap();
    assert_eq!(run(&["validate", file.to_str().unwrap()]), 0);

    for (folder, name) in [("new", "story.orbit"), ("old", "story.ron")] {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(name), &text).unwrap();
        assert_eq!(run(&["validate", d.to_str().unwrap()]), 0, "{name}");
    }

    // A lone `.orbit` beside other `.ron` files (themes, translations...).
    let mixed = dir.join("mixed");
    std::fs::create_dir_all(&mixed).unwrap();
    std::fs::write(mixed.join("game.orbit"), &text).unwrap();
    std::fs::write(mixed.join("theme.ron"), "(").unwrap();
    assert_eq!(run(&["validate", mixed.to_str().unwrap()]), 0);
    let _ = std::fs::remove_dir_all(dir);
}
