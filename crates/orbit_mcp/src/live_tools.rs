//! The tools that only exist in live mode (the editor is open): driving the
//! preview, looking at it, selecting a node in the editor, build / export.
//!
//! This module only describes them (names, schemas, arguments); the editor
//! runs them, because it owns the GPU, the preview and the window. Keeping the
//! definitions here keeps the schemas generated from Rust types, like every
//! other tool.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as Json;

use crate::error::ToolError;
use crate::host::{ExportRequest, ToolInfo};
use crate::tools::input_schema_of;

pub const PREVIEW_PLAY: &str = "preview_play";
pub const PREVIEW_STEP: &str = "preview_step";
pub const PREVIEW_CHOOSE: &str = "preview_choose";
pub const PREVIEW_STATE: &str = "preview_state";
pub const PREVIEW_STOP: &str = "preview_stop";
pub const PREVIEW_SCREENSHOT: &str = "preview_screenshot";
pub const CAPTURE_SCENE3D: &str = "capture_scene3d";
pub const SELECT_NODE: &str = "select_node";
pub const BUILD_PROJECT: &str = "build_project";
pub const EXPORT_GAME: &str = "export_game";

/// Largest number of VM steps one `preview_step` call may take.
pub const MAX_STEPS: usize = 500;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlayArgs {
    /// Start from this node id, with the state a player would have there
    /// (variables, characters on stage, music). Omit to start from the
    /// beginning of the story.
    from_node: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StepArgs {
    /// Number of steps (lines / commands) to advance (default 1, max 500).
    steps: Option<usize>,
    /// Advance until the next choice, the end of the story or an error
    /// (bounded by 500 steps) instead of a fixed number of steps.
    until_choice: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum OptionSel {
    /// Option index (0-based).
    Index(usize),
    /// Option text (exact, or the beginning of it).
    Text(String),
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChooseArgs {
    /// The option to pick: its index (0-based) or its text.
    option: OptionSel,
}

/// Default and bounds of a 3D capture, in pixels.
pub const CAPTURE3D_DEFAULT: [u32; 2] = [1280, 720];
pub const CAPTURE3D_MIN: u32 = 64;
pub const CAPTURE3D_MAX: u32 = 2048;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CaptureScene3dArgs {
    /// The 3D scene: asset id, manifest path, file name or name without
    /// extension (see `list_scenes3d`).
    scene: String,
    /// Camera name in the scene (default: its first camera).
    camera: Option<String>,
    /// Picture width in pixels (default 1280, 64 to 2048).
    width: Option<u32>,
    /// Picture height in pixels (default 720, 64 to 2048).
    height: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SelectArgs {
    /// Id of the node to select and reveal in the editor (the scene editor
    /// scrolls to it, the inspector shows it).
    node: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BuildArgs {
    /// Export profile name (default: the project's default profile).
    profile: Option<String>,
    /// Output folder, relative to the project folder.
    out_dir: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportArgs {
    /// Target platform: `windows`, `linux` or `web`.
    target: String,
    /// Export profile name (default: the project's default profile).
    profile: Option<String>,
    /// Output folder, relative to the project folder.
    out_dir: Option<String>,
}

/// A parsed live-only tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveCall {
    PreviewPlay { from_node: Option<String> },
    PreviewStep { steps: usize, until_choice: bool },
    PreviewChoose(ChoiceSelector),
    PreviewState,
    PreviewStop,
    PreviewScreenshot,
    /// Renders a 3D scene from one of its cameras (no preview needed).
    CaptureScene3d {
        scene: String,
        camera: Option<String>,
        width: u32,
        height: u32,
    },
    SelectNode { node: String },
    Export(ExportRequest),
}

/// Which option of a choice to pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceSelector {
    Index(usize),
    Text(String),
}

fn info<A: JsonSchema>(
    name: &str,
    title: &str,
    description: &str,
    read_only: bool,
    destructive: bool,
) -> ToolInfo {
    ToolInfo {
        name: name.to_owned(),
        title: title.to_owned(),
        description: description.to_owned(),
        input_schema: input_schema_of::<A>(),
        read_only,
        destructive,
        idempotent: read_only,
    }
}

/// The live-only tools, as advertised to clients.
pub fn live_tool_infos() -> Vec<ToolInfo> {
    vec![
        info::<PlayArgs>(
            PREVIEW_PLAY,
            "Start the preview",
            "Start (or restart) the editor's preview, from the beginning or from a node (`from_node`, with the state a player would have there). Returns the preview state. Needs a GPU: without one, this returns an `unavailable` error.",
            false,
            false,
        ),
        info::<StepArgs>(
            PREVIEW_STEP,
            "Advance the preview",
            "Advance the running preview by N steps (`steps`) or until the next choice (`until_choice`): completes the typewriter, goes to the next line, skips transitions and timed waits. Returns the preview state.",
            false,
            false,
        ),
        info::<ChooseArgs>(
            PREVIEW_CHOOSE,
            "Choose an option in the preview",
            "Pick an option (index or text) of the choice currently shown by the preview. Returns the preview state.",
            false,
            false,
        ),
        info::<NoArgs>(
            PREVIEW_STATE,
            "Read the preview state",
            "Read the preview: status, current node, the line or choice on screen, variables and the declarative scene (background, characters on stage with expression and position, music).",
            true,
            false,
        ),
        info::<NoArgs>(
            PREVIEW_STOP,
            "Stop the preview",
            "Stop the preview (blank stage, no sound).",
            false,
            false,
        ),
        info::<NoArgs>(
            PREVIEW_SCREENSHOT,
            "Screenshot of the preview",
            "Capture what the preview shows right now (16:9 PNG, returned as an image) so you can see the rendering. Start the preview first (preview_play). Needs a GPU.",
            true,
            false,
        ),
        info::<CaptureScene3dArgs>(
            CAPTURE_SCENE3D,
            "Picture of a 3D scene",
            "Render a 3D scene of the project as seen from one of its cameras (PNG returned as an image) to check what edit_scene3d did: placement, lights, materials, skybox. Reads the scene file as saved on disk, with the models and images next to it. Does not need the preview. Needs a GPU: without one this returns an `unavailable` error (the structure is still readable with read_scene3d).",
            true,
            false,
        ),
        info::<SelectArgs>(
            SELECT_NODE,
            "Select a node in the editor",
            "Select a node and reveal it in the editor (the user sees what you are talking about).",
            false,
            false,
        ),
        info::<BuildArgs>(
            BUILD_PROJECT,
            "Build the project",
            "DESTRUCTIVE (writes files, replaces a previous build): build the project with a profile: validates the story (errors stop the build), compiles it and writes the asset pak to out_dir (default `build`). Unsaved edits are saved first.",
            false,
            true,
        ),
        info::<ExportArgs>(
            EXPORT_GAME,
            "Export the game",
            "DESTRUCTIVE (writes files, replaces a previous export): export the game for a target platform (windows: zip, linux: tar.gz, web: a folder for a web server) into out_dir (default `export`). Unsaved edits are saved first. Needs the player template of the target (shipped next to the editor); the answer lists the warnings.",
            false,
            true,
        ),
    ]
}

/// Whether `name` is a live-only tool.
pub fn is_live_tool(name: &str) -> bool {
    matches!(
        name,
        PREVIEW_PLAY
            | PREVIEW_STEP
            | PREVIEW_CHOOSE
            | PREVIEW_STATE
            | PREVIEW_STOP
            | PREVIEW_SCREENSHOT
            | CAPTURE_SCENE3D
            | SELECT_NODE
            | BUILD_PROJECT
            | EXPORT_GAME
    )
}

fn args<A: for<'de> Deserialize<'de>>(value: Json) -> Result<A, ToolError> {
    serde_json::from_value(value).map_err(|e| ToolError::invalid(format!("invalid arguments: {e}")))
}

/// Parses a live-only tool call; `None` when `name` is not one.
pub fn parse_live_call(name: &str, value: Json) -> Option<Result<LiveCall, ToolError>> {
    Some(match name {
        PREVIEW_PLAY => args::<PlayArgs>(value).map(|a| LiveCall::PreviewPlay {
            from_node: a.from_node,
        }),
        PREVIEW_STEP => args::<StepArgs>(value).map(|a| LiveCall::PreviewStep {
            steps: a.steps.unwrap_or(1).clamp(1, MAX_STEPS),
            until_choice: a.until_choice.unwrap_or(false),
        }),
        PREVIEW_CHOOSE => args::<ChooseArgs>(value).map(|a| {
            LiveCall::PreviewChoose(match a.option {
                OptionSel::Index(i) => ChoiceSelector::Index(i),
                OptionSel::Text(t) => ChoiceSelector::Text(t),
            })
        }),
        PREVIEW_STATE => args::<NoArgs>(value).map(|_| LiveCall::PreviewState),
        PREVIEW_STOP => args::<NoArgs>(value).map(|_| LiveCall::PreviewStop),
        PREVIEW_SCREENSHOT => args::<NoArgs>(value).map(|_| LiveCall::PreviewScreenshot),
        CAPTURE_SCENE3D => args::<CaptureScene3dArgs>(value).map(|a| LiveCall::CaptureScene3d {
            scene: a.scene,
            camera: a.camera.filter(|c| !c.trim().is_empty()),
            width: a
                .width
                .unwrap_or(CAPTURE3D_DEFAULT[0])
                .clamp(CAPTURE3D_MIN, CAPTURE3D_MAX),
            height: a
                .height
                .unwrap_or(CAPTURE3D_DEFAULT[1])
                .clamp(CAPTURE3D_MIN, CAPTURE3D_MAX),
        }),
        SELECT_NODE => args::<SelectArgs>(value).map(|a| LiveCall::SelectNode { node: a.node }),
        BUILD_PROJECT => args::<BuildArgs>(value).map(|a| {
            LiveCall::Export(ExportRequest {
                build_only: true,
                target: None,
                profile: a.profile,
                out_dir: a.out_dir,
            })
        }),
        EXPORT_GAME => args::<ExportArgs>(value).map(|a| {
            LiveCall::Export(ExportRequest {
                build_only: false,
                target: Some(a.target),
                profile: a.profile,
                out_dir: a.out_dir,
            })
        }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn every_live_tool_parses_and_has_a_schema() {
        for info in live_tool_infos() {
            assert!(is_live_tool(&info.name), "{}", info.name);
            assert_eq!(info.input_schema.get("type"), Some(&json!("object")));
        }
        assert!(parse_live_call("add_dialogue", json!({})).is_none());
    }

    #[test]
    fn arguments_are_checked() {
        let call = parse_live_call(PREVIEW_STEP, json!({"steps": 9999})).unwrap();
        assert_eq!(
            call.unwrap(),
            LiveCall::PreviewStep {
                steps: MAX_STEPS,
                until_choice: false
            }
        );
        assert!(
            parse_live_call(PREVIEW_STEP, json!({"step": 1}))
                .unwrap()
                .is_err()
        );
        let choose = parse_live_call(PREVIEW_CHOOSE, json!({"option": "Left"})).unwrap();
        assert_eq!(
            choose.unwrap(),
            LiveCall::PreviewChoose(ChoiceSelector::Text("Left".into()))
        );
    }

    #[test]
    fn capture_scene3d_clamps_its_size() {
        let call = parse_live_call(CAPTURE_SCENE3D, json!({"scene": "room", "width": 99999, "height": 1}))
            .unwrap()
            .unwrap();
        assert_eq!(
            call,
            LiveCall::CaptureScene3d {
                scene: "room".into(),
                camera: None,
                width: CAPTURE3D_MAX,
                height: CAPTURE3D_MIN
            }
        );
        assert!(parse_live_call(CAPTURE_SCENE3D, json!({})).unwrap().is_err());
    }

    #[test]
    fn export_tools_are_flagged_destructive() {
        let infos = live_tool_infos();
        for name in [BUILD_PROJECT, EXPORT_GAME] {
            assert!(infos.iter().find(|i| i.name == name).unwrap().destructive);
        }
        assert!(
            infos
                .iter()
                .find(|i| i.name == PREVIEW_STATE)
                .unwrap()
                .read_only
        );
    }
}
