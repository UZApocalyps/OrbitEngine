//! Native file helpers for the `i18n/<story stem>.<lang>.ron` convention
//! (editor, CLI). The rest of the crate stays free of I/O.

use std::path::{Path, PathBuf};

use crate::Translation;

fn stem(story_path: &Path) -> String {
    story_path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// `<story dir>/i18n/<stem>.<lang>.ron`.
pub fn translation_path(story_path: &Path, lang: &str) -> PathBuf {
    story_path
        .parent()
        .unwrap_or(Path::new(""))
        .join("i18n")
        .join(format!("{}.{lang}.ron", stem(story_path)))
}

/// Language tags of the translation files next to the story, sorted.
pub fn list_languages(story_path: &Path) -> Vec<String> {
    let prefix = format!("{}.", stem(story_path));
    let dir = story_path.parent().unwrap_or(Path::new("")).join("i18n");
    let mut langs: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let mid = name.strip_prefix(&prefix)?.strip_suffix(".ron")?;
            (!mid.is_empty()).then(|| mid.to_owned())
        })
        .collect();
    langs.sort();
    langs
}

/// Reads a translation file; `Ok(None)` when it does not exist.
pub fn load(path: &Path) -> Result<Option<Translation>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Translation::from_ron(&text)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes a text file, creating its folder.
pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Writes a translation as RON.
pub fn save(path: &Path, t: &Translation) -> Result<(), String> {
    write_text(path, &t.to_ron().map_err(|e| e.to_string())?)
}
