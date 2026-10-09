//! The pipeline state, saved in the project's `.orbit/ai/` so a run can be
//! reviewed, edited, re-run step by step and resumed after a restart.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::assets::{self, AssetJob};
use crate::bible::{self, Bible, Pitch, StepError};
use crate::image_gen::ImageModel;
use crate::script;
use crate::settings::{ImageSettings, TextSettings};
use crate::text::TextModel;

/// State file, relative to the project root.
pub const STATE_FILE: &str = ".orbit/ai/studio.json";
/// Current format of [`PipelineState`] (bump with a migration, never drop
/// one: old projects must keep loading).
pub const STATE_FORMAT: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptStatus {
    #[default]
    Pending,
    /// Parses and validates.
    Valid,
    /// Last attempt still had errors (kept for manual editing).
    Invalid(Vec<String>),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneScript {
    pub text: String,
    pub status: ScriptStatus,
    pub repairs: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineState {
    pub format: u32,
    pub pitch: Pitch,
    pub bible: Option<Bible>,
    /// Raw text of the bible, as the user may edit it.
    pub bible_text: String,
    /// What was repaired automatically in the bible (warnings).
    pub bible_warnings: Vec<String>,
    /// By scene id.
    pub scripts: BTreeMap<String, SceneScript>,
    pub assets: Vec<AssetJob>,
    /// Per-character LoRA / seed, by character id.
    pub looks: BTreeMap<String, assets::CharacterLook>,
    /// Scene names created by the last implementation, if any.
    pub implemented: Vec<String>,
    /// What happened, newest last (shown in the wizard).
    pub log: Vec<String>,
}

impl Default for PipelineState {
    fn default() -> Self {
        Self {
            format: STATE_FORMAT,
            pitch: Pitch::default(),
            bible: None,
            bible_text: String::new(),
            bible_warnings: Vec::new(),
            scripts: BTreeMap::new(),
            assets: Vec::new(),
            looks: BTreeMap::new(),
            implemented: Vec::new(),
            log: Vec::new(),
        }
    }
}

impl PipelineState {
    pub fn path_in(root: &Path) -> PathBuf {
        root.join(STATE_FILE)
    }

    /// The saved state of a project, or a fresh one.
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(Self::path_in(root))
            .ok()
            .and_then(|t| serde_json::from_str::<PipelineState>(&t).ok())
            .map(|mut s| {
                s.format = STATE_FORMAT;
                s
            })
            .unwrap_or_default()
    }

    /// Atomic write (temp file + rename).
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let path = Self::path_in(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    pub fn note(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > 200 {
            self.log.drain(..self.log.len() - 200);
        }
    }

    /// Sets the bible from (possibly hand-edited) JSON text.
    pub fn set_bible_text(&mut self, text: &str) -> Result<(), Vec<String>> {
        let (b, warnings) = bible::parse_bible_fixed(text)?;
        self.bible_warnings = warnings;
        self.bible_text = serde_json::to_string_pretty(&b).unwrap_or_else(|_| text.to_owned());
        self.bible = Some(b);
        Ok(())
    }

    /// Scene ids whose script is not valid yet, in play order.
    pub fn scenes_to_write(&self) -> Vec<String> {
        let Some(b) = &self.bible else { return Vec::new() };
        b.scenes
            .iter()
            .filter(|s| self.scripts.get(&s.id).is_none_or(|x| x.status != ScriptStatus::Valid))
            .map(|s| s.id.clone())
            .collect()
    }

    /// Every scene has a valid script.
    pub fn script_complete(&self) -> bool {
        self.bible.is_some() && self.scenes_to_write().is_empty()
    }

    /// Re-checks a hand-edited scene script.
    pub fn set_script_text(&mut self, scene_id: &str, text: &str) {
        let Some(b) = &self.bible else { return };
        let base = script::scratch_story(b);
        let text = script::normalize_scene(text, scene_id);
        let status = match script::check_scene(&base, b, scene_id, &text) {
            Ok(_) => ScriptStatus::Valid,
            Err(e) => ScriptStatus::Invalid(e),
        };
        let entry = self.scripts.entry(scene_id.to_owned()).or_default();
        entry.text = text;
        entry.status = status;
    }

    /// All scripts, in play order.
    pub fn full_script(&self) -> String {
        let Some(b) = &self.bible else { return String::new() };
        b.scenes
            .iter()
            .filter_map(|s| self.scripts.get(&s.id).map(|x| x.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

// --------------------------------------------------------------- the steps

/// Step b: pitch → bible.
pub fn run_bible(
    state: &mut PipelineState,
    model: &dyn TextModel,
    text: &TextSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), StepError> {
    let out = bible::generate_bible(model, &state.pitch, text.max_repairs, cancelled)?;
    state.bible_text = serde_json::to_string_pretty(&out.value).unwrap_or_default();
    state.bible = Some(out.value);
    state.bible_warnings = out.warnings;
    // A new bible makes old scripts and pictures meaningless.
    state.scripts.clear();
    state.assets.clear();
    state.note(format!("bible written ({} repairs)", out.repairs));
    Ok(())
}

/// Step c: writes every scene that has no valid script yet (or only
/// `only`), one scene per request so long stories fit the context.
/// `checkpoint` runs after each scene (progress + saving).
pub fn run_script(
    state: &mut PipelineState,
    model: &dyn TextModel,
    text: &TextSettings,
    only: Option<&str>,
    cancelled: &dyn Fn() -> bool,
    checkpoint: &mut dyn FnMut(&PipelineState, usize, usize),
) -> Result<(), StepError> {
    let b = state
        .bible
        .clone()
        .ok_or_else(|| StepError::Missing("write the bible first".to_owned()))?;
    let base = script::scratch_story(&b);
    let todo: Vec<String> = match only {
        Some(id) => vec![id.to_owned()],
        None => state.scenes_to_write(),
    };
    let total = todo.len();
    let mut first_error = None;
    for (n, id) in todo.iter().enumerate() {
        let Some(scene) = b.scene(id) else { continue };
        let index = b.scenes.iter().position(|s| s.id == *id).unwrap_or(0);
        let previous = index
            .checked_sub(1)
            .and_then(|i| state.scripts.get(&b.scenes[i].id))
            .map(|s| script::tail(&s.text, 12));
        let result = script::generate_scene(
            model,
            &base,
            &b,
            scene,
            &state.pitch.language,
            state.pitch.lines_per_scene,
            previous.as_deref(),
            text.max_repairs,
            cancelled,
        );
        match result {
            Ok(out) => {
                state.scripts.insert(
                    id.clone(),
                    SceneScript { text: out.value, status: ScriptStatus::Valid, repairs: out.repairs },
                );
                state.note(format!("scene `{id}` written ({} repairs)", out.repairs));
            }
            Err(StepError::Invalid { errors, last }) => {
                state.note(format!("scene `{id}` still invalid: {}", errors.join("; ")));
                state.scripts.insert(
                    id.clone(),
                    SceneScript { text: last, status: ScriptStatus::Invalid(errors.clone()), repairs: text.max_repairs },
                );
                first_error.get_or_insert(StepError::Invalid { errors, last: String::new() });
            }
            Err(e) => return Err(e),
        }
        checkpoint(state, n + 1, total);
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Builds (or refreshes) the picture plan from the bible and the scripts.
pub fn plan_assets(state: &mut PipelineState, image: &ImageSettings) -> Result<(), StepError> {
    let b = state
        .bible
        .as_ref()
        .ok_or_else(|| StepError::Missing("write the bible first".to_owned()))?;
    let base = script::scratch_story(b);
    let used = script::used_expressions(
        &base,
        b,
        state.scripts.iter().map(|(k, v)| (k.as_str(), v.text.as_str())),
    );
    let fresh = assets::plan(b, &used, image, &state.pitch.art_style, &state.looks);
    state.assets = assets::merge_plan(&state.assets, fresh);
    Ok(())
}

/// Step d: makes every missing picture. `checkpoint(done, total, jobs)` runs
/// after each one.
pub fn run_assets(
    state: &mut PipelineState,
    root: &Path,
    model: Option<&dyn ImageModel>,
    image: &ImageSettings,
    cancelled: &(dyn Fn() -> bool + Sync),
    checkpoint: &(dyn Fn(usize, usize, &[AssetJob]) + Sync),
) -> Result<(), StepError> {
    plan_assets(state, image)?;
    let b = state.bible.clone().unwrap_or_default();
    let total = state.assets.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let ok = assets::generate_all(root, &b, &mut state.assets, model, image, cancelled, &|_, jobs| {
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        checkpoint(n, total, jobs);
    });
    let placeholders = state
        .assets
        .iter()
        .filter(|j| matches!(j.status, assets::AssetStatus::Placeholder(_)))
        .count();
    state.note(format!("{total} pictures ({placeholders} placeholders)"));
    if ok { Ok(()) } else { Err(StepError::Provider(crate::text::ProviderError::Cancelled)) }
}
