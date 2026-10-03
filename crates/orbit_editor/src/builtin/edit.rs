//! Editing gestures shared by the scene editor, the explorer, the inspector
//! and the palette actions: insert, delete, duplicate, copy / paste, nudge,
//! scene management. Each one is one undo entry (see `orbit_project::ops`).

use std::path::Path;

use orbit_core::{AssetId, AssetKind, NodeId, NodeKind, SceneId, Story};
use orbit_editor_api::{EditorContext, Selection, Translator};
use orbit_project::ops::{self, NodeClip};

use super::drafts::{DraftError, drafts_for};

/// The scene the scene editor shows: the scene of the selected node, the
/// selected scene, the last one shown, or the first one.
pub fn current_scene(ctx: &EditorContext<'_>) -> Option<SceneId> {
    let story = ctx.story()?;
    let from_selection = match ctx.selection() {
        Some(Selection::Node(id)) => story.locate(id).map(|(s, _)| story.scenes[s].id),
        Some(Selection::Scene(id)) => story.scene(id).map(|s| s.id),
        None => None,
    };
    from_selection
        .or_else(|| {
            ctx.state()
                .current_scene
                .filter(|id| story.scene(*id).is_some())
        })
        .or_else(|| story.scenes.first().map(|s| s.id))
}

/// Where "insert" puts a new node: after the last selected node, otherwise
/// at the end of the current scene.
pub fn insertion_point(ctx: &EditorContext<'_>) -> Option<(SceneId, usize)> {
    let story = ctx.story()?;
    let last = ctx
        .selected_nodes()
        .into_iter()
        .filter_map(|id| story.locate(id))
        .max();
    if let Some((si, ni)) = last {
        return Some((story.scenes[si].id, ni + 1));
    }
    let scene = story.scene(current_scene(ctx)?)?;
    Some((scene.id, scene.nodes.len()))
}

/// Asks for a file (system dialog) of the given kind and imports it into the
/// project (validated, copied under `assets/<type>/`, declared under a new
/// stable id). Returns its id.
pub fn import_asset_dialog(ctx: &mut EditorContext<'_>, kind: AssetKind) -> Option<AssetId> {
    let root = ctx.project_info()?.root.clone();
    let (title, extensions): (String, &[&str]) = match kind {
        AssetKind::Image => (ctx.t("edit.pick_image"), &["png", "jpg", "jpeg"]),
        AssetKind::Music => (ctx.t("edit.pick_music"), &["ogg", "wav"]),
        AssetKind::Sound => (ctx.t("edit.pick_sound"), &["ogg", "wav"]),
        AssetKind::Font => (ctx.t("edit.pick_font"), &["ttf", "otf"]),
    };
    let file = rfd::FileDialog::new()
        .set_title(title)
        .set_directory(&root)
        .add_filter(kind_name(&ctx.translator(), kind), extensions)
        .pick_file()?;
    register_asset_file(ctx, &file, kind)
}

/// Asks for an animated GIF / APNG and imports its frames as images of the
/// project. Returns their ids in order and the frame rate.
pub fn import_animation_dialog(ctx: &mut EditorContext<'_>) -> Option<(Vec<AssetId>, f32)> {
    let root = ctx.project_info()?.root.clone();
    let file = rfd::FileDialog::new()
        .set_title(ctx.t("edit.pick_animation"))
        .set_directory(&root)
        .add_filter("GIF / APNG", &["gif", "png"])
        .pick_file()?;
    register_animation_file(ctx, &file)
}

/// The non-interactive half of [`import_animation_dialog`]: splits `file`
/// into `<name>_001.png`, `<name>_002.png`... under `assets/` and returns the
/// frame ids and fps.
pub fn register_animation_file(
    ctx: &mut EditorContext<'_>,
    file: &Path,
) -> Option<(Vec<AssetId>, f32)> {
    let decoded = std::fs::read(file)
        .map_err(|e| e.to_string())
        .and_then(|bytes| super::importers::decode_animation(&bytes));
    let decoded = match decoded {
        Ok(d) => d,
        Err(e) => {
            ctx.error(e);
            return None;
        }
    };
    let stem = file
        .file_stem()
        .map_or_else(|| "animation".to_owned(), |s| s.to_string_lossy().into_owned());
    let dir = std::env::temp_dir().join(format!("orbit_anim_{}_{}", std::process::id(), stem));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        ctx.error(e.to_string());
        return None;
    }
    let mut paths = Vec::new();
    for (i, png) in decoded.frames.iter().enumerate() {
        let path = dir.join(format!("{stem}_{:03}.png", i + 1));
        if let Err(e) = std::fs::write(&path, png) {
            ctx.error(e.to_string());
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        paths.push(path);
    }
    let report = super::import_paths(ctx, &paths);
    let _ = std::fs::remove_dir_all(&dir);
    if report.imported.len() != paths.len() {
        // `import_paths` already reported each refusal.
        return None;
    }
    let ids = report.imported.iter().map(|i| i.id).collect();
    Some((ids, decoded.fps))
}

/// The non-interactive half of [`import_asset_dialog`]: imports `file` and
/// returns its id, or reports why not. A file of another kind than wanted
/// (a sound where an image is expected) is refused.
pub fn register_asset_file(
    ctx: &mut EditorContext<'_>,
    file: &Path,
    kind: AssetKind,
) -> Option<AssetId> {
    let report = super::import_paths(ctx, &[file.to_owned()]);
    let imported = report.imported.first()?;
    if imported.kind != kind {
        // Audio is long (music) or short (sound): both fit an audio field.
        // The importer guessed from the length; the field says what it is.
        let audio = |k: AssetKind| matches!(k, AssetKind::Music | AssetKind::Sound);
        if !(audio(kind) && audio(imported.kind)) {
            let texts = ctx.translator();
            ctx.error(texts.tf(
                "edit.wrong_kind",
                &[
                    ("path", &imported.path),
                    ("found", &kind_name(&texts, imported.kind)),
                    ("expected", &kind_name(&texts, kind)),
                ],
            ));
            return None;
        }
        let command = orbit_project::Command::SetAsset {
            id: imported.id,
            entry: Some(orbit_core::AssetEntry {
                id: imported.id,
                kind,
                path: imported.path.clone(),
            }),
            index: None,
        };
        if !ctx.exec_or_toast(command) {
            return None;
        }
    }
    Some(imported.id)
}

/// Inserts a node of type `key` (see `drafts`) after the selection and
/// selects it. A `Say` starts in edit mode. Returns the new node ids.
pub fn insert_kind(ctx: &mut EditorContext<'_>, key: &str) -> Option<Vec<NodeId>> {
    let (scene, index) = insertion_point(ctx).or_else(|| {
        ctx.error(ctx.t("edit.no_scene"));
        None
    })?;
    let mut attempt = 0;
    let drafts = loop {
        let story = ctx.story()?;
        match drafts_for(story, ctx.registry(), &ctx.translator(), key, scene, index) {
            Ok(drafts) => break drafts,
            Err(DraftError::NeedAsset(kind)) if attempt == 0 => {
                attempt += 1;
                let name = kind_name(&ctx.translator(), kind);
                ctx.info(ctx.tf("edit.need_asset_pick", &[("kind", &name)]));
                import_asset_dialog(ctx, kind)?;
            }
            Err(DraftError::NeedAsset(kind)) => {
                let name = kind_name(&ctx.translator(), kind);
                ctx.error(ctx.tf("edit.need_asset", &[("kind", &name)]));
                return None;
            }
            Err(DraftError::Message(m)) => {
                ctx.error(m);
                return None;
            }
        }
    };
    let label = match key {
        "Say" => ctx.t("edit.add_say"),
        "Choice" => ctx.t("edit.add_choice"),
        other => {
            let texts = ctx.translator();
            let name = super::drafts::all_insertables(ctx.registry())
                .into_iter()
                .find(|i| i.key == other)
                .map_or_else(|| other.to_owned(), |i| i.label_text(&texts));
            ctx.tf("edit.add_kind", &[("name", &name)])
        }
    };
    let ids = ctx.ops_or_toast(|p| ops::insert_drafts(p, scene, index, drafts, &label))?;
    let first = *ids.first()?;
    ctx.select(Selection::Node(first));
    if key == "Say" {
        ctx.state_mut().inline_edit = Some(first);
    }
    Some(ids)
}

/// Inserts a new line (the writing flow: Ctrl+Enter).
pub fn add_say(ctx: &mut EditorContext<'_>) {
    insert_kind(ctx, "Say");
}

/// Deletes the selected nodes; the node after (or before) the block becomes
/// selected.
pub fn delete_selection(ctx: &mut EditorContext<'_>) {
    let ids = ctx.selected_nodes();
    if ids.is_empty() {
        return;
    }
    let Some(story) = ctx.story() else { return };
    let ordered = ops::in_story_order(story, &ids);
    let neighbor = ordered.last().and_then(|last| {
        let (si, ni) = story.locate(*last)?;
        let nodes = &story.scenes[si].nodes;
        let first = story.locate(*ordered.first()?)?.1;
        nodes
            .get(ni + 1)
            .or_else(|| first.checked_sub(1).and_then(|p| nodes.get(p)))
            .map(|n| n.id)
    });
    if ctx.ops_or_toast(|p| ops::delete_nodes(p, &ids)).is_some() {
        match neighbor {
            Some(next) => ctx.select(Selection::Node(next)),
            None => ctx.clear_selection(),
        }
    }
}

/// Duplicates the selection right after itself and selects the copies.
pub fn duplicate_selection(ctx: &mut EditorContext<'_>) {
    let ids = ctx.selected_nodes();
    if ids.is_empty() {
        return;
    }
    if let Some(new) = ctx.ops_or_toast(|p| ops::duplicate_nodes(p, &ids))
        && let Some(&primary) = new.first()
    {
        ctx.select_nodes(new, primary);
    }
}

/// Copies the selection to the internal clipboard (and deletes it for `cut`).
pub fn copy_selection(ctx: &mut EditorContext<'_>, cut: bool) {
    let ids = ctx.selected_nodes();
    let Some(story) = ctx.story() else { return };
    let clip = ops::copy_nodes(story, &ids);
    if clip.is_empty() {
        return;
    }
    let count = clip.nodes.len();
    ctx.state_mut().clipboard = Some(clip);
    if cut {
        delete_selection(ctx);
    } else {
        ctx.info(ctx.tn("edit.copied", count, &[]));
    }
}

/// Pastes the clipboard after the selection (fresh ids) and selects the result.
pub fn paste(ctx: &mut EditorContext<'_>) {
    let Some(clip) = ctx.state().clipboard.clone().filter(|c| !c.is_empty()) else {
        return;
    };
    let Some((scene, index)) = insertion_point(ctx) else {
        return;
    };
    if let Some(new) = ctx.ops_or_toast(|p| ops::paste_nodes(p, &clip, scene, index))
        && let Some(&primary) = new.first()
    {
        ctx.select_nodes(new, primary);
    }
}

/// Moves the selection one step up (`-1`) or down (`1`).
pub fn nudge(ctx: &mut EditorContext<'_>, direction: i32) {
    let ids = ctx.selected_nodes();
    if ids.is_empty() {
        return;
    }
    // Moving is a plain reorder: the selection stays as it is.
    let _ = ctx.ops_or_toast(|p| ops::nudge_nodes(p, &ids, direction));
}

/// Selects every node of the current scene.
pub fn select_all(ctx: &mut EditorContext<'_>) {
    let Some(story) = ctx.story() else { return };
    let Some(scene) = current_scene(ctx).and_then(|id| story.scene(id)) else {
        return;
    };
    let ids: Vec<NodeId> = scene.nodes.iter().map(|n| n.id).collect();
    if let Some(&first) = ids.first() {
        ctx.select_nodes(ids, first);
    }
}

/// Creates a scene after the current one's end and selects it.
pub fn new_scene(ctx: &mut EditorContext<'_>) {
    let Some(story) = ctx.story() else { return };
    let base = ctx.t("edit.new_scene");
    let name = ops::unique_name(&base, story.scenes.iter().map(|s| s.name.as_str()));
    if let Some(id) = ctx.ops_or_toast(|p| ops::add_scene(p, &name)) {
        ctx.select(Selection::Scene(id));
    }
}

/// Deletes a scene (refused while another scene jumps into it).
pub fn delete_scene(ctx: &mut EditorContext<'_>, id: SceneId) {
    let Some(story) = ctx.story() else { return };
    let Some(index) = story.scenes.iter().position(|s| s.id == id) else {
        return;
    };
    let neighbor = story
        .scenes
        .get(index + 1)
        .or_else(|| index.checked_sub(1).and_then(|p| story.scenes.get(p)))
        .map(|s| s.id);
    // The nodes go with the scene: strings first, so one undo brings all back.
    let strings: Vec<_> = story
        .scene(id)
        .map(|s| s.nodes.iter().flat_map(|n| n.kind.strings()).collect())
        .unwrap_or_default();
    let label = ctx.t("edit.delete_scene");
    let result = ctx.transaction(&label, |tx| {
        tx.execute(orbit_project::Command::RemoveScene { id })?;
        for text in strings {
            tx.execute(orbit_project::Command::SetString {
                id: text,
                text: None,
            })?;
        }
        Ok(())
    });
    match result {
        Ok(()) => match neighbor {
            Some(next) => ctx.select(Selection::Scene(next)),
            None => ctx.clear_selection(),
        },
        Err(e) => ctx.error(e.to_string()),
    }
}

/// Moves a scene one step earlier (`-1`) or later (`1`).
pub fn move_scene(ctx: &mut EditorContext<'_>, id: SceneId, direction: i32) {
    let Some(story) = ctx.story() else { return };
    let Some(index) = story.scenes.iter().position(|s| s.id == id) else {
        return;
    };
    let target = index as i64 + i64::from(direction.signum());
    if target < 0 || target >= story.scenes.len() as i64 {
        return;
    }
    ctx.exec_or_toast(orbit_project::Command::MoveScene {
        id,
        index: target as usize,
    });
}

/// The name of an asset kind, in the language of `texts`.
pub fn kind_name(texts: &Translator, kind: AssetKind) -> String {
    texts.t(match kind {
        AssetKind::Image => "edit.kind.image",
        AssetKind::Music => "edit.kind.music",
        AssetKind::Sound => "edit.kind.sound",
        AssetKind::Font => "edit.kind.font",
    })
}

/// Whether the clipboard holds something to paste.
pub fn can_paste(ctx: &EditorContext<'_>) -> bool {
    ctx.state()
        .clipboard
        .as_ref()
        .is_some_and(|c: &NodeClip| !c.is_empty())
}

/// Whether the scene has a `Label` kind node `id` (helper for the UI).
pub fn is_label(story: &Story, id: NodeId) -> bool {
    matches!(
        story.node(id).map(|n| &n.kind),
        Some(NodeKind::Label { .. })
    )
}
