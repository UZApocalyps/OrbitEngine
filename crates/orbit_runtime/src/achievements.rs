//! Achievements at runtime (M7-05): the platform hook and the rows of the
//! data-driven achievements screen. No GPU, unit tested with the shell.

use std::collections::BTreeSet;
use std::sync::Arc;

use orbit_core::{AchievementId, Story};
use orbit_i18n::Localizer;
use orbit_ui::AchievementRow;

use crate::format::plain_text;

/// What a platform integration (Steam, itch.io, a console...) implements to
/// follow the achievements of the player. The engine calls it, on the thread
/// of the game loop, when an achievement is unlocked for the first time in a
/// real playthrough (never in a scene replay, never again after a rollback or
/// a reload), and once at start-up for each achievement already unlocked so
/// that a platform that lost track of one can catch up.
///
/// Hand it to the runtime with `RuntimeConfig::achievement_hook`. The default
/// is none: the achievements then only live in the persistent data of the game
/// (`persistent.ron`). Steam is on the backlog: its integration is a crate
/// implementing this trait with `SteamUserStats::set_achievement(api_name)`,
/// where `api_name` is [`AchievementEvent::id`].
pub trait AchievementHook: Send + Sync + std::fmt::Debug {
    /// An achievement was unlocked.
    fn unlocked(&self, event: &AchievementEvent);
}

/// An achievement the platform is told about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AchievementEvent {
    /// The stable id of the achievement (a ULID), the name to register on
    /// the platform.
    pub id: String,
    /// Title in the source language of the story.
    pub title: String,
    /// `true` when the player just unlocked it; `false` for the catch-up at
    /// start-up (see [`AchievementHook`]).
    pub fresh: bool,
}

/// A shared hook.
pub type SharedAchievementHook = Arc<dyn AchievementHook>;

/// The title and description of an achievement for the player: the
/// localized ones, or a placeholder while it is hidden and locked.
pub(crate) fn rows(
    story: &Story,
    loc: &Localizer,
    texts: &orbit_ui::Texts,
    unlocked: &BTreeSet<AchievementId>,
) -> Vec<AchievementRow> {
    story
        .achievements
        .iter()
        .map(|a| {
            let done = unlocked.contains(&a.id);
            let shown = done || !a.hidden;
            AchievementRow {
                id: a.id.to_string(),
                title: if shown {
                    plain_text(loc.text(a.title_id))
                } else {
                    texts.get("ui.achievements.hidden_title")
                },
                description: if shown {
                    plain_text(loc.text(a.description_id))
                } else {
                    texts.get("ui.achievements.hidden_description")
                },
                // Locked: the locked icon when there is one (even while
                // hidden: it is meant to be seen), else the icon unless hidden.
                icon: if done { a.icon } else { a.locked_icon.or(a.icon.filter(|_| shown)) }
                    .and_then(|icon| story.assets.get(icon))
                    .map(|e| e.path.clone())
                    .unwrap_or_default(),
                unlocked: done,
            }
        })
        .collect()
}

/// Manifest paths of the icons (unlocked and locked) of the achievements (the host loads them as
/// textures for the achievements screen).
pub fn achievement_icon_paths(story: &Story) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in &story.achievements {
        for entry in [a.icon, a.locked_icon]
            .into_iter()
            .flatten()
            .filter_map(|icon| story.assets.get(icon))
        {
            if !out.contains(&entry.path) {
                out.push(entry.path.clone());
            }
        }
    }
    out
}
