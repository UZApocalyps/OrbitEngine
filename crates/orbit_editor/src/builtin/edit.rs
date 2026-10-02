//! Editing gestures shared by the scene editor, the explorer, the inspector
//! and the palette actions: insert, delete, duplicate, copy / paste, nudge,
//! scene management. Each one is one undo entry (see `orbit_project::ops`).

use std::path::{Path, PathBuf};

use orbit_core::{AssetId, AssetKind, NodeId, NodeKind, SceneId, Story};
use orbit_editor_api::{EditorContext, Selection};
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

/// Copies `source` into the project's `assets/` folder when it lies outside
/// the project, and returns the manifest path (relative, `/` separators).
pub fn project_relative_path(root: &Path, source: &Path) -> Result<String, String> {
    let relative = |p: &Path| -> Option<String> {
        let rel = p.strip_prefix(root).ok()?;
        Some(
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        )
    };
    if let Some(rel) = relative(source) {
        return Ok(rel);
    }
    let name = source
        .file_name()
        .ok_or_else(|| "Nom de fichier invalide".to_owned())?
        .to_string_lossy()
        .into_owned();
    let dir = root.join("assets");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut target: PathBuf = dir.join(&name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_owned(), format!(".{e}")),
        None => (name.clone(), String::new()),
    };
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem}-{n}{ext}"));
        n += 1;
    }
    std::fs::copy(source, &target).map_err(|e| format!("Copie impossible : {e}"))?;
    relative(&target).ok_or_else(|| "Chemin hors du projet".to_owned())
}

/// Asks for a file (system dialog), copies it into the project when needed
/// and registers it in the manifest. Returns its id. (The full importer,
/// with previews and plugin importers, is M3e.)
pub fn import_asset_dialog(ctx: &mut EditorContext<'_>, kind: AssetKind) -> Option<AssetId> {
    let root = ctx.project_info()?.root.clone();
    let (title, extensions): (&str, &[&str]) = match kind {
        AssetKind::Image => ("Choisir une image", &["png", "jpg", "jpeg", "webp"]),
        AssetKind::Music => ("Choisir une musique", &["ogg", "wav", "mp3", "flac"]),
        AssetKind::Sound => ("Choisir un son", &["ogg", "wav", "mp3", "flac"]),
    };
    let file = rfd::FileDialog::new()
        .set_title(title)
        .set_directory(&root)
        .add_filter(kind.to_string(), extensions)
        .pick_file()?;
    register_asset_file(ctx, &root, &file, kind)
}

/// The non-interactive half of [`import_asset_dialog`].
pub fn register_asset_file(
    ctx: &mut EditorContext<'_>,
    root: &Path,
    file: &Path,
    kind: AssetKind,
) -> Option<AssetId> {
    let path = match project_relative_path(root, file) {
        Ok(p) => p,
        Err(e) => {
            ctx.error(e);
            return None;
        }
    };
    ctx.ops_or_toast(|p| ops::add_asset(p, kind, &path))
}

/// Inserts a node of type `key` (see `drafts`) after the selection and
/// selects it. A `Say` starts in edit mode. Returns the new node ids.
pub fn insert_kind(ctx: &mut EditorContext<'_>, key: &str) -> Option<Vec<NodeId>> {
    let (scene, index) = insertion_point(ctx).or_else(|| {
        ctx.error("Le projet n'a aucune scène : créez-en une d'abord.");
        None
    })?;
    let mut attempt = 0;
    let drafts = loop {
        let story = ctx.story()?;
        match drafts_for(story, ctx.registry(), key, scene, index) {
            Ok(drafts) => break drafts,
            Err(DraftError::NeedAsset(kind)) if attempt == 0 => {
                attempt += 1;
                ctx.info(format!(
                    "Aucune ressource de type « {kind} » : choisissez un fichier à ajouter au projet."
                ));
                import_asset_dialog(ctx, kind)?;
            }
            Err(DraftError::NeedAsset(kind)) => {
                ctx.error(format!("Aucune ressource de type « {kind} » dans le projet."));
                return None;
            }
            Err(DraftError::Message(m)) => {
                ctx.error(m);
                return None;
            }
        }
    };
    let label = match key {
        "Say" => "Ajouter une réplique".to_owned(),
        "Choice" => "Ajouter un choix".to_owned(),
        other => {
            let name = super::drafts::all_insertables(ctx.registry())
                .into_iter()
                .find(|i| i.key == other)
                .map_or_else(|| other.to_owned(), |i| i.label);
            format!("Ajouter : {name}")
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
        ctx.info(if count == 1 {
            "1 nœud copié.".to_owned()
        } else {
            format!("{count} nœuds copiés.")
        });
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
    let name = ops::unique_name("Nouvelle scène", story.scenes.iter().map(|s| s.name.as_str()));
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
    let result = ctx.transaction("Supprimer la scène", |tx| {
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

/// Whether the clipboard holds something to paste.
pub fn can_paste(ctx: &EditorContext<'_>) -> bool {
    ctx.state().clipboard.as_ref().is_some_and(|c: &NodeClip| !c.is_empty())
}

/// Whether the scene has a `Label` kind node `id` (helper for the UI).
pub fn is_label(story: &Story, id: NodeId) -> bool {
    matches!(story.node(id).map(|n| &n.kind), Some(NodeKind::Label { .. }))
}
