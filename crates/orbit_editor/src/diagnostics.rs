//! Validation findings per node, in French, for inline display (scene
//! editor, inspector). Recomputed by the core when the project revision
//! changes.

use orbit_core::{
    NodeId, Story, ValidationError, ValidationWarning, validate,
    NodeKind,
};
use orbit_editor_api::{Diagnostics, Level};

fn label_of(story: &Story, id: NodeId) -> String {
    match story.node(id).map(|n| &n.kind) {
        Some(NodeKind::Label { name }) => format!("« {name} »"),
        Some(_) => "un nœud qui n'est pas une étiquette".to_owned(),
        None => "un nœud supprimé".to_owned(),
    }
}

fn character_of(story: &Story, id: orbit_core::CharacterId) -> String {
    story
        .character(id)
        .and_then(|c| story.text(c.name_id))
        .map_or_else(|| "un personnage supprimé".to_owned(), |n| format!("« {n} »"))
}

/// Node an error is about, with its French message.
pub fn describe_error(story: &Story, error: &ValidationError) -> (Option<NodeId>, String) {
    use ValidationError as E;
    match error {
        E::UnknownTarget { from, target } => (
            Some(*from),
            format!("Cible introuvable ({})", label_of(story, *target)),
        ),
        E::TargetNotLabel { from, target } => (
            Some(*from),
            format!("La cible doit être une étiquette, pas {}", label_of(story, *target)),
        ),
        E::EmptyChoice(n) => (Some(*n), "Ce choix n'a aucune option".to_owned()),
        E::UnknownString { node, .. } => (*node, "Texte introuvable dans la table des textes".to_owned()),
        E::UnknownCharacter { node, character } => (
            Some(*node),
            format!("Personnage inconnu ({})", character_of(story, *character)),
        ),
        E::UnknownExpression {
            node,
            character,
            expression,
        } => (
            Some(*node),
            format!(
                "{} n'a pas d'expression « {expression} »",
                character_of(story, *character)
            ),
        ),
        E::UnknownAsset { node, .. } => (*node, "Ressource introuvable dans le manifeste".to_owned()),
        E::AssetKindMismatch {
            node,
            expected,
            found,
            ..
        } => (
            *node,
            format!("La ressource est de type « {found} », « {expected} » attendu"),
        ),
        E::UnknownPosition { node, name } => (Some(*node), format!("Position inconnue « {name} »")),
        E::InvalidDuration { node, seconds } => (
            Some(*node),
            format!("Durée invalide ({seconds}) : un nombre positif est attendu"),
        ),
        E::ExprType { node, message } => (Some(*node), format!("Expression : {message}")),
        E::DuplicateNodeId(n) => (Some(*n), "Identifiant de nœud en double".to_owned()),
        other => (None, other.to_string()),
    }
}

fn describe_warning(warning: &ValidationWarning) -> (Option<NodeId>, String) {
    use ValidationWarning as W;
    match warning {
        W::UndefinedVariable { node, var } => (
            Some(*node),
            format!("La variable « {var} » n'est ni déclarée ni assignée"),
        ),
        W::UnreachableNode(n) => (Some(*n), "Ce nœud n'est jamais atteint".to_owned()),
        W::PossibleInfiniteLoop(n) => (
            Some(*n),
            "Boucle possible sans réplique, choix ni attente".to_owned(),
        ),
        W::AllOptionsConditional(n) => (
            Some(*n),
            "Toutes les options ont une condition : le joueur peut être bloqué".to_owned(),
        ),
    }
}

/// Runs the story validation and groups the findings by node.
pub fn compute(story: &Story, revision: u64) -> Diagnostics {
    let report = validate(story);
    let mut out = Diagnostics {
        revision,
        ..Diagnostics::default()
    };
    for e in &report.errors {
        let (node, message) = describe_error(story, e);
        match node {
            Some(n) => out.by_node.entry(n).or_default().push((Level::Error, message)),
            None => out.general.push((Level::Error, message)),
        }
    }
    for w in &report.warnings {
        let (node, message) = describe_warning(w);
        match node {
            Some(n) => out.by_node.entry(n).or_default().push((Level::Warn, message)),
            None => out.general.push((Level::Warn, message)),
        }
    }
    out
}
