//! Showing a thing of the project to the user: what a click on a problem, a
//! search hit or a palette entry does.

use orbit_editor_api::{EditorContext, EditorRequest, Location};

use super::panels;

/// Selects `location` and brings the panel that shows it forward: the scene
/// editor for nodes and scenes (the explorer and the inspector follow the
/// selection), the characters panel, the variables panel or the asset
/// library for definitions.
pub fn reveal(ctx: &mut EditorContext<'_>, location: Location) {
    let panel = match &location {
        Location::Node(_) | Location::Scene(_) => panels::SCENE,
        Location::Character(_) => panels::CHARACTERS,
        Location::Variable(_) => panels::VARIABLES,
        Location::Asset(_) => panels::LIBRARY,
    };
    ctx.reveal(location);
    ctx.request(EditorRequest::ShowPanel(panel.to_owned()));
}

/// "Modifier": selects the node and brings the inspector forward (shown
/// from its home area when closed), the selection feeding its fields.
pub fn edit_node(ctx: &mut EditorContext<'_>, id: orbit_core::NodeId) {
    ctx.select(orbit_editor_api::Selection::Node(id));
    ctx.request(EditorRequest::ShowPanel(panels::INSPECTOR.to_owned()));
}

/// Where a location is, for the user: `Prologue · Réplique`, `Clara`...
pub fn describe(ctx: &EditorContext<'_>, location: &Location) -> String {
    let Some(story) = ctx.story() else {
        return String::new();
    };
    match location {
        Location::Node(id) => match story.locate(*id) {
            Some((si, ni)) => {
                let scene = &story.scenes[si];
                ctx.tf(
                    "reveal.node",
                    &[
                        ("scene", &scene.name),
                        (
                            "kind",
                            &super::kind_label(&ctx.translator(), &scene.nodes[ni].kind),
                        ),
                        ("index", &(ni + 1)),
                    ],
                )
            }
            None => ctx.t("reveal.node_deleted"),
        },
        Location::Scene(id) => story
            .scene(*id)
            .map_or_else(|| ctx.t("reveal.scene_deleted"), |s| s.name.clone()),
        Location::Character(id) => story
            .character(*id)
            .and_then(|c| story.text(c.name_id))
            .map_or_else(|| ctx.t("reveal.character_deleted"), str::to_owned),
        Location::Variable(name) => ctx.tf("reveal.variable", &[("name", name)]),
        Location::Asset(id) => story
            .assets
            .get(*id)
            .map_or_else(|| ctx.t("reveal.asset_deleted"), |a| a.path.clone()),
    }
}
