//! Error messages of the project layer, written in the editor language.
//!
//! `orbit_project` reports refusals with stable codes (`Rejection::code`)
//! and technical errors as `CommandError` variants; the editor shows them in
//! the user's language from the `reject.*` and `error.*` text keys, and falls
//! back to the project's own message when a key is missing.

use orbit_project::{CommandError, ProjectError, Rejection};

use crate::context::EditorError;
use crate::i18n::Translator;

/// A message for a project error, in the language of `texts`.
pub fn describe_project_error(texts: &Translator, error: &ProjectError) -> String {
    match error {
        ProjectError::Rejected(rejection) => describe_rejection(texts, rejection),
        ProjectError::Command(command) => describe_command_error(texts, command),
    }
}

/// A message for an editor error.
pub fn describe_editor_error(texts: &Translator, error: &EditorError) -> String {
    match error {
        EditorError::NoProject => texts.get("error.no_project"),
        EditorError::Project(e) => describe_project_error(texts, e),
    }
}

/// The message of a refusal: its `reject.<code>` text with the arguments
/// filled in, or the project's own wording.
pub fn describe_rejection(texts: &Translator, rejection: &Rejection) -> String {
    let key = format!("reject.{}", rejection.code);
    if rejection.code.is_empty() || !texts.has(&key) {
        return rejection.message.clone();
    }
    if rejection.code == "asset_in_use" {
        return describe_asset_in_use(texts, rejection);
    }
    let mut args: Vec<(&str, String)> = rejection
        .args
        .iter()
        .map(|(name, value)| (*name, value.clone()))
        .collect();
    // The terminator of a block is a code too ("end", "jump"...).
    if let Some((_, value)) = args.iter_mut().find(|(name, _)| *name == "terminator") {
        *value = texts.get(&format!("reject.terminator.{value}"));
    }
    let refs: Vec<(&str, &dyn std::fmt::Display)> = args
        .iter()
        .map(|(name, value)| (*name, value as &dyn std::fmt::Display))
        .collect();
    texts.format(&key, &refs)
}

/// "This asset is still used by 2 nodes and an expression of “Clara”".
fn describe_asset_in_use(texts: &Translator, rejection: &Rejection) -> String {
    let nodes: usize = rejection
        .arg("nodes")
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let characters = rejection.arg("characters").unwrap_or_default();
    let mut parts = Vec::new();
    if nodes > 0 {
        parts.push(texts.plural("reject.asset_in_use.nodes", nodes, &[]));
    }
    if !characters.is_empty() {
        parts.push(texts.format("reject.asset_in_use.expression", &[("names", &characters)]));
    }
    texts.format(
        "reject.asset_in_use",
        &[("users", &parts.join(&texts.get("reject.asset_in_use.and")))],
    )
}

/// The message of a technical command error (a node that vanished, an
/// optimistic version check...).
pub fn describe_command_error(texts: &Translator, error: &CommandError) -> String {
    use CommandError as E;
    match error {
        E::UnknownScene(id) => texts.format("error.unknown_scene", &[("id", id)]),
        E::UnknownNode(id) => texts.format("error.unknown_node", &[("id", id)]),
        E::DuplicateNode(id) => texts.format("error.duplicate_node", &[("id", id)]),
        E::IndexOutOfBounds { index, len } => texts.format(
            "error.index_out_of_bounds",
            &[("index", index), ("len", len)],
        ),
        E::NotASay(id) => texts.format("error.not_a_say", &[("id", id)]),
        E::UnknownString(id) => texts.format("error.unknown_string", &[("id", id)]),
        E::StringInUse(id, node) => {
            texts.format("error.string_in_use", &[("id", id), ("node", node)])
        }
        E::StillReferenced(id, by) => {
            texts.format("error.still_referenced", &[("id", id), ("by", by)])
        }
        E::SceneStillTargeted(id, node) => {
            texts.format("error.scene_still_targeted", &[("id", id), ("node", node)])
        }
        E::DuplicateScene(id) => texts.format("error.duplicate_scene", &[("id", id)]),
        E::UnknownCharacter(id) => texts.format("error.unknown_character", &[("id", id)]),
        E::CharacterInUse(id, node) => {
            texts.format("error.character_in_use", &[("id", id), ("node", node)])
        }
        E::UnknownVariable(name) => texts.format("error.unknown_variable", &[("name", name)]),
        E::UnknownAsset(id) => texts.format("error.unknown_asset", &[("id", id)]),
        E::AssetInUse(id, node) => {
            texts.format("error.asset_in_use", &[("id", id), ("node", node)])
        }
        E::AssetInUseByCharacter(id, character) => texts.format(
            "error.asset_in_use_by_character",
            &[("id", id), ("character", character)],
        ),
        E::EmptyCurveName => texts.get("error.empty_curve_name"),
        E::CurveChain(name) => texts.format("error.curve_chain", &[("name", name)]),
        E::VersionConflict {
            node,
            expected,
            actual,
        } => texts.format(
            "error.version_conflict",
            &[("node", node), ("expected", expected), ("actual", actual)],
        ),
    }
}

/// Translates the label of an undo entry. Labels written by `orbit_project`
/// are French sentences used as keys (`undo.<sentence>`); a label that has no
/// entry (written by the editor in the language of the moment) is shown as is.
pub fn describe_undo_label(texts: &Translator, label: &str) -> String {
    let key = format!("undo.{label}");
    if texts.has(&key) {
        texts.get(&key)
    } else {
        label.to_owned()
    }
}
