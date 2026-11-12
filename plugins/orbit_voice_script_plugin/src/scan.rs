//! Reads from the project folder what the voice script needs besides the
//! story: the audio files of the asset folder, and the translations. Disk
//! work only, no UI: it runs in a background task.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use orbit_editor_sdk::orbit_project::voices::VOICE_EXTENSIONS;
use orbit_i18n::Translation;

use crate::script::{VoiceDisk, norm_key};

/// What to read. Everything is owned: the job runs on another thread.
pub struct Job {
    /// The folder the manifest's paths are relative to.
    pub assets_root: PathBuf,
    /// The story file (the language files are next to it).
    pub story_file: PathBuf,
    /// Paths of the sounds the manifest declares.
    pub sounds: Vec<String>,
}

/// What the task found.
#[derive(Default)]
pub struct Gathered {
    pub disk: VoiceDisk,
    /// Language tags of the translation files, sorted.
    pub languages: Vec<String>,
    /// The translations that could be read, by tag.
    pub translations: BTreeMap<String, Translation>,
    /// Language files that could not be read: tag and reason.
    pub unreadable: Vec<(String, String)>,
}

fn is_audio(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| VOICE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// The audio files under `<assets_root>/assets` (recursively, dot files
/// skipped), `/` separators, sorted. `None` when cancelled.
pub fn walk_audio(assets_root: &Path, cancelled: &dyn Fn() -> bool) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![(assets_root.join("assets"), "assets".to_owned())];
    while let Some((dir, rel)) = stack.pop() {
        if cancelled() {
            return None;
        }
        let Ok(read) = fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let child = format!("{rel}/{name}");
            if meta.is_dir() {
                stack.push((entry.path(), child));
            } else if meta.is_file() && is_audio(&name) {
                out.push(child);
            }
        }
    }
    out.sort();
    Some(out)
}

/// Reads the audio files, checks the declared sounds and loads the
/// translations. `None` when cancelled.
pub fn gather(job: Job, cancelled: &dyn Fn() -> bool) -> Option<Gathered> {
    let files = walk_audio(&job.assets_root, cancelled)?;
    let mut present: HashSet<String> = files.iter().map(|f| norm_key(f)).collect();
    // Declared sounds elsewhere than under `assets/`.
    for path in &job.sounds {
        if !present.contains(&norm_key(path)) && job.assets_root.join(path).is_file() {
            present.insert(norm_key(path));
        }
    }
    let mut out = Gathered { disk: VoiceDisk { files, present }, ..Default::default() };
    out.languages = orbit_i18n::files::list_languages(&job.story_file);
    for lang in &out.languages {
        if cancelled() {
            return None;
        }
        let path = orbit_i18n::files::translation_path(&job.story_file, lang);
        match orbit_i18n::files::load(&path) {
            Ok(Some(t)) => {
                out.translations.insert(lang.clone(), t);
            }
            Ok(None) => {}
            Err(e) => out.unreadable.push((lang.clone(), e)),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder under the workspace's `target` (never the system
    /// temp folder), removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/plugin-test-scratch")
                .join(format!("voice_script-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn walks_audio_files_only() {
        let s = Scratch::new("walk");
        s.write("assets/voice/a.ogg", "x");
        s.write("assets/voice/fr/b.WAV", "x");
        s.write("assets/voice/readme.txt", "x");
        s.write("assets/img.png", "x");
        s.write("assets/.hidden/c.ogg", "x");
        s.write("elsewhere/d.ogg", "x");
        let files = walk_audio(&s.0, &|| false).unwrap();
        assert_eq!(files, vec!["assets/voice/a.ogg", "assets/voice/fr/b.WAV"]);
        assert!(walk_audio(&s.0, &|| true).is_none());
        let empty = Scratch::new("walk_empty");
        assert!(walk_audio(&empty.0, &|| false).unwrap().is_empty());
    }

    #[test]
    fn gathers_files_declared_sounds_and_translations() {
        let s = Scratch::new("gather");
        s.write("assets/voice/a.ogg", "x");
        s.write("sounds_elsewhere/e.ogg", "x");
        s.write("story.orbit", "");
        s.write("i18n/story.fr.ron", r#"(lang: "fr", name: "Francais", entries: {})"#);
        s.write("i18n/story.de.ron", "this is not a translation");
        let job = Job {
            assets_root: s.0.clone(),
            story_file: s.0.join("story.orbit"),
            sounds: vec!["sounds_elsewhere/e.ogg".into(), "assets/voice/gone.ogg".into()],
        };
        let g = gather(job, &|| false).unwrap();
        assert!(g.disk.exists("assets/voice/a.ogg"));
        assert!(g.disk.exists("sounds_elsewhere/e.ogg"), "declared elsewhere");
        assert!(!g.disk.exists("assets/voice/gone.ogg"));
        assert_eq!(g.disk.files, vec!["assets/voice/a.ogg"], "only the asset folder feeds the naming convention");
        assert_eq!(g.languages, vec!["de", "fr"]);
        assert_eq!(g.translations.keys().collect::<Vec<_>>(), vec!["fr"]);
        assert_eq!(g.unreadable.len(), 1);
        assert_eq!(g.unreadable[0].0, "de");
    }
}
