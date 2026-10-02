//! Checking a save against the (possibly edited) story.

use orbit_core::{NodeKind, Story, VarScope};
use orbit_vm::{Pending, VmState};

use crate::data::{FORMAT_VERSION, SaveData};
use crate::hash::story_hash;

/// Whether a save can be loaded with a given story.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compat {
    /// Same structure: load as is.
    Compatible,
    /// The story changed but every node the save points to still exists, so
    /// it is accepted; the warnings tell what was adjusted.
    CompatibleWithWarnings(Vec<String>),
    /// Cannot be loaded; the reason is meant for the slot list.
    Incompatible(String),
}

impl Compat {
    /// `true` unless [`Compat::Incompatible`].
    pub fn is_loadable(&self) -> bool {
        !matches!(self, Compat::Incompatible(_))
    }
}

/// Compares a save with `story`.
///
/// * Same [`story_hash`] (and a known format): `Compatible`.
/// * Different hash: every node referenced by the VM state must still exist
///   (and a pending choice must still have the options it recorded);
///   otherwise `Incompatible`. When they do, `CompatibleWithWarnings`.
///
/// Texts and assets can change freely; the VM reconciles variables and stage
/// elements on load (see `Vm::restore`).
pub fn check_compat(story: &Story, save: &SaveData) -> Compat {
    check_meta(story, save.meta.format_version, save.meta.story_hash).unwrap_or_else(|| {
        let mut warnings =
            vec!["the story was modified since this save was made; it was adjusted".to_owned()];
        match check_state(story, &save.vm_state, &mut warnings) {
            Ok(()) => Compat::CompatibleWithWarnings(warnings),
            Err(reason) => Compat::Incompatible(reason),
        }
    })
}

/// The cheap part of the check, from metadata only: `Some(verdict)` when it is
/// decided (format problem or identical hash), `None` when the VM state must
/// be inspected.
pub(crate) fn check_meta(story: &Story, format_version: u32, hash: u64) -> Option<Compat> {
    if format_version == 0 || format_version > FORMAT_VERSION {
        return Some(Compat::Incompatible(format!(
            "unsupported save format version {format_version}"
        )));
    }
    (hash == story_hash(story)).then_some(Compat::Compatible)
}

fn check_state(story: &Story, state: &VmState, warnings: &mut Vec<String>) -> Result<(), String> {
    for id in state.referenced_nodes() {
        if story.node(id).is_none() {
            return Err(format!("node {id} no longer exists in the story"));
        }
    }
    if let Pending::Choice { node, visible } = &state.pending {
        let ok = matches!(
            story.node(*node).map(|n| &n.kind),
            Some(NodeKind::Choice { options }) if visible.iter().all(|&i| i < options.len())
        );
        if !ok {
            return Err(format!("choice {node} changed since the save was made"));
        }
    }
    let missing_vars = story
        .variables
        .iter()
        .filter(|v| v.scope == VarScope::Game && !state.vars.contains_key(&v.name))
        .count();
    if missing_vars > 0 {
        warnings.push(format!(
            "{missing_vars} new variable(s) initialized to their default"
        ));
    }
    let vanished = state
        .scene
        .characters
        .iter()
        .filter(|c| story.character(c.character).is_none())
        .count()
        + usize::from(
            state
                .scene
                .background
                .is_some_and(|a| story.assets.get(a).is_none()),
        )
        + usize::from(
            state
                .scene
                .music
                .as_ref()
                .is_some_and(|m| story.assets.get(m.asset).is_none()),
        );
    if vanished > 0 {
        warnings.push(format!(
            "{vanished} stage element(s) no longer exist and were dropped"
        ));
    }
    Ok(())
}
