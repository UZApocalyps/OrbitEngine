//! Editing gestures shared by the scene editor, the explorer, the inspector
//! and the palette actions: insert, delete, duplicate, copy / paste, nudge,
//! scene management. Each one is one undo entry (see `orbit_project::ops`).

use std::path::Path;

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

/// Asks for a file (system dialog) of the given kind and imports it into the
/// project (validated, copied under `assets/<type>/`, declared under a new
/// stable id). Returns its id.
pub fn import_asset_dialog(ctx: &mut EditorContext<'_>, kind: AssetKind) -> Option<AssetId> {
    let root = ctx.project_info()?.root.clone();
    let (title, extensions): (&str, &[&str]) = match kind {
        AssetKind::Image => ("Choisir une image", &["png", "jpg", "jpeg"]),
        AssetKind::Music => ("Choisir une musique", &["ogg", "wav"]),
        AssetKind::Sound => ("Choisir un son", &["ogg", "wav"]),
    };
    let file = rfd::FileDialog::new()
        .set_title(title)
        .set_directory(&root)
        .add_filter(kind.to_string(), extensions)
        .pick_file()?;
    register_asset_file(ctx, &file, kind)
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
            ctx.error(format!(
                "« {} » est de type « {} », « {kind} » attendu.",
                imported.path, imported.kind
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
                ctx.error(format!(
                    "Aucune ressource de type « {kind} » dans le projet."
                ));
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
    let name = ops::unique_name(
        "Nouvelle scène",
        story.scenes.iter().map(|s| s.name.as_str()),
    );
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
