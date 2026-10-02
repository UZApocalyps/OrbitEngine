//! Schema migrations.
//!
//! Migrations operate on the RON *text* of a document: they are applied one
//! version step at a time until the document reaches
//! [`CURRENT_SCHEMA_VERSION`].
//!
//! * v1 -> v2 (M1): texts move to the string table, speakers become
//!   characters, conditions and `SetVar` values become expressions.
//! * v2 -> v3 (M2): variable declarations gain a `scope` (`Game` default,
//!   `Persistent`). Existing variables become `Game` variables.
//! * v3 -> v4 (M3a): `NodeKind::Custom` (plugin nodes) is added. Purely
//!   additive: a v3 document is a valid v4 document.

use std::borrow::Cow;

use serde::Deserialize;

use crate::error::CoreError;
use crate::story::CURRENT_SCHEMA_VERSION;

/// Minimal view of a document used to read its version before full parsing.
#[derive(Deserialize)]
struct Header {
    schema_version: u32,
}

/// Reads the `schema_version` field without parsing the rest of the document.
pub fn read_version(source: &str) -> Result<u32, CoreError> {
    Ok(ron::from_str::<Header>(source)?.schema_version)
}

/// One migration step: rewrites a document from version `N` to `N + 1`.
pub type MigrationStep = fn(&str) -> Result<String, CoreError>;

/// Steps indexed by source version: `STEPS[0]` migrates v1 -> v2, etc.
const STEPS: &[MigrationStep] = &[v1_to_v2, v2_to_v3, v3_to_v4];

/// Brings `source` (declared at `from`) up to the current schema version.
pub fn migrate(from: u32, source: &str) -> Result<Cow<'_, str>, CoreError> {
    migrate_with(from, source, CURRENT_SCHEMA_VERSION, STEPS)
}

fn migrate_with<'a>(
    from: u32,
    source: &'a str,
    current: u32,
    steps: &[MigrationStep],
) -> Result<Cow<'a, str>, CoreError> {
    if from == current {
        return Ok(Cow::Borrowed(source));
    }
    if from == 0 || from > current {
        return Err(CoreError::UnsupportedVersion {
            found: from,
            current,
        });
    }
    let mut text = source.to_owned();
    for version in from..current {
        let step = steps
            .get((version - 1) as usize)
            .ok_or(CoreError::Migration {
                from: version,
                reason: "no migration registered".into(),
            })?;
        text = step(&text)?;
    }
    Ok(Cow::Owned(text))
}

/// Schema version 1 types, kept only to read old files.
mod v1 {
    use serde::Deserialize;

    use crate::ids::{NodeId, SceneId, StringId};
    use crate::value::Value;

    #[derive(Deserialize)]
    pub struct Story {
        pub title: String,
        pub scenes: Vec<Scene>,
    }

    #[derive(Deserialize)]
    pub struct Scene {
        pub id: SceneId,
        pub name: String,
        pub nodes: Vec<Node>,
    }

    #[derive(Deserialize)]
    pub struct Node {
        pub id: NodeId,
        pub kind: NodeKind,
    }

    #[derive(Deserialize)]
    pub enum NodeKind {
        Label {
            name: String,
        },
        Say {
            speaker: Option<String>,
            text_id: StringId,
            text: String,
        },
        Choice {
            options: Vec<ChoiceOption>,
        },
        Jump {
            target: NodeId,
        },
        SetVar {
            var: String,
            value: Value,
        },
        If {
            condition: Condition,
            then_target: NodeId,
            else_target: Option<NodeId>,
        },
    }

    #[derive(Deserialize)]
    pub struct ChoiceOption {
        pub text_id: StringId,
        pub text: String,
        pub target: NodeId,
    }

    #[derive(Clone, Copy, Deserialize)]
    pub enum CompareOp {
        Eq,
        Ne,
        Lt,
        Le,
        Gt,
        Ge,
    }

    #[derive(Deserialize)]
    pub struct Condition {
        pub var: String,
        pub op: CompareOp,
        pub value: Value,
    }
}

/// v1 -> v2. Speaker names become characters (one per distinct name, with no
/// expressions and a neutral color) and every text moves to the string table.
/// Node, scene and string ids are preserved.
fn v1_to_v2(source: &str) -> Result<String, CoreError> {
    use std::collections::HashMap;

    use crate::defs::{Character, Color};
    use crate::expr::{BinOp, Expr};
    use crate::ids::CharacterId;
    use crate::story::{ChoiceOption, Node, NodeKind, Scene, Story};

    let old: v1::Story = ron::from_str(source)?;
    let mut story = Story::new(old.title);
    story.schema_version = 2;
    let mut speakers: HashMap<String, CharacterId> = HashMap::new();

    for scene in old.scenes {
        let mut nodes = Vec::with_capacity(scene.nodes.len());
        for node in scene.nodes {
            let kind = match node.kind {
                v1::NodeKind::Label { name } => NodeKind::Label { name },
                v1::NodeKind::Say {
                    speaker,
                    text_id,
                    text,
                } => {
                    story.strings.set(text_id, text);
                    let speaker = speaker.map(|name| match speakers.get(&name) {
                        Some(id) => *id,
                        None => {
                            let id = CharacterId::new();
                            let name_id = story.strings.add(name.clone());
                            story.characters.push(Character {
                                id,
                                name_id,
                                color: Color::rgb(200, 200, 200),
                                expressions: Vec::new(),
                                default_position: None,
                            });
                            speakers.insert(name, id);
                            id
                        }
                    });
                    NodeKind::Say { speaker, text_id }
                }
                v1::NodeKind::Choice { options } => NodeKind::Choice {
                    options: options
                        .into_iter()
                        .map(|o| {
                            story.strings.set(o.text_id, o.text);
                            ChoiceOption {
                                text_id: o.text_id,
                                target: o.target,
                                condition: None,
                            }
                        })
                        .collect(),
                },
                v1::NodeKind::Jump { target } => NodeKind::Jump { target },
                v1::NodeKind::SetVar { var, value } => NodeKind::SetVar {
                    var,
                    value: Expr::Lit(value),
                },
                v1::NodeKind::If {
                    condition,
                    then_target,
                    else_target,
                } => {
                    let op = match condition.op {
                        v1::CompareOp::Eq => BinOp::Eq,
                        v1::CompareOp::Ne => BinOp::Ne,
                        v1::CompareOp::Lt => BinOp::Lt,
                        v1::CompareOp::Le => BinOp::Le,
                        v1::CompareOp::Gt => BinOp::Gt,
                        v1::CompareOp::Ge => BinOp::Ge,
                    };
                    NodeKind::If {
                        condition: Expr::binary(
                            op,
                            Expr::Var(condition.var),
                            Expr::Lit(condition.value),
                        ),
                        then_target,
                        else_target,
                    }
                }
            };
            nodes.push(Node { id: node.id, kind });
        }
        story.scenes.push(Scene {
            id: scene.id,
            name: scene.name,
            nodes,
        });
    }
    story.to_ron()
}

/// v2 -> v3. `VarDecl::scope` has a serde default (`Game`), so the v2
/// document is already a valid v3 one apart from its version number.
fn v2_to_v3(source: &str) -> Result<String, CoreError> {
    use crate::story::Story;

    let mut story: Story = ron::from_str(source)?;
    story.schema_version = 3;
    story.to_ron()
}

fn v3_to_v4(source: &str) -> Result<String, CoreError> {
    use crate::story::Story;

    let mut story: Story = ron::from_str(source)?;
    story.schema_version = 4;
    story.to_ron()
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1_STORY: &str = r#"(
    schema_version: 1,
    title: "Old",
    scenes: [(
        id: "01JBX0S0000000000000000001",
        name: "main",
        nodes: [
            (id: "01JBX0N0000000000000000001", kind: Say(speaker: Some("Ana"), text_id: "01JBX0T0000000000000000001", text: "Hi")),
            (id: "01JBX0N0000000000000000002", kind: Say(speaker: Some("Ana"), text_id: "01JBX0T0000000000000000002", text: "Again")),
            (id: "01JBX0N0000000000000000003", kind: Say(speaker: None, text_id: "01JBX0T0000000000000000003", text: "Narration")),
            (id: "01JBX0N0000000000000000004", kind: SetVar(var: "n", value: Int(2))),
            (id: "01JBX0N0000000000000000005", kind: If(condition: (var: "n", op: Ge, value: Int(2)), then_target: "01JBX0N0000000000000000007", else_target: None)),
            (id: "01JBX0N0000000000000000006", kind: Choice(options: [(text_id: "01JBX0T0000000000000000004", text: "Go", target: "01JBX0N0000000000000000007")])),
            (id: "01JBX0N0000000000000000007", kind: Label(name: "end")),
        ],
    )],
)"#;

    #[test]
    fn current_version_is_untouched() {
        let out = migrate(CURRENT_SCHEMA_VERSION, "x").unwrap();
        assert!(matches!(out, Cow::Borrowed("x")));
    }

    #[test]
    fn future_and_zero_versions_are_rejected() {
        assert!(matches!(
            migrate(CURRENT_SCHEMA_VERSION + 1, "x"),
            Err(CoreError::UnsupportedVersion { .. })
        ));
        assert!(matches!(
            migrate(0, "x"),
            Err(CoreError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn v1_story_migrates_to_v2() {
        use crate::expr::{BinOp, Expr};
        use crate::story::{NodeKind, Story};
        use crate::value::Value;

        assert_eq!(read_version(V1_STORY).unwrap(), 1);
        let story = Story::from_ron(V1_STORY).expect("migrated story must validate");
        assert_eq!(story.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(story.title, "Old");
        // One character for the two "Ana" lines.
        assert_eq!(story.characters.len(), 1);
        let ana = &story.characters[0];
        assert_eq!(story.text(ana.name_id), Some("Ana"));
        let nodes = &story.scenes[0].nodes;
        let NodeKind::Say { speaker, text_id } = &nodes[1].kind else {
            panic!("expected Say");
        };
        assert_eq!(*speaker, Some(ana.id));
        assert_eq!(story.text(*text_id), Some("Again"));
        let NodeKind::Say { speaker, .. } = &nodes[2].kind else {
            panic!("expected Say");
        };
        assert_eq!(*speaker, None);
        assert_eq!(
            nodes[4].kind,
            NodeKind::If {
                condition: Expr::binary(BinOp::Ge, Expr::var("n"), Expr::Lit(Value::Int(2))),
                then_target: nodes[6].id,
                else_target: None,
            }
        );
        // Ids are preserved and the result round-trips at v2.
        assert_eq!(nodes[0].id.to_string(), "01JBX0N0000000000000000001");
        let again = Story::from_ron(&story.to_ron().unwrap()).unwrap();
        assert_eq!(story, again);
    }

    const V2_STORY: &str = r#"(
    schema_version: 2,
    title: "Mid",
    variables: [(name: "n", initial: Int(1))],
    scenes: [(
        id: "01JBX0S0000000000000000001",
        name: "main",
        nodes: [
            (id: "01JBX0N0000000000000000001", kind: Label(name: "a")),
            (id: "01JBX0N0000000000000000002", kind: SetVar(var: "n", value: Lit(Int(2)))),
        ],
    )],
)"#;

    #[test]
    fn v2_story_migrates_with_game_scope() {
        use crate::defs::VarScope;
        use crate::story::Story;

        assert_eq!(read_version(V2_STORY).unwrap(), 2);
        let story = Story::from_ron(V2_STORY).expect("migrated story must validate");
        assert_eq!(story.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(story.variables.len(), 1);
        assert_eq!(story.variables[0].scope, VarScope::Game);
        assert_eq!(story.scenes[0].nodes.len(), 2);
        // A v1 document goes through both steps.
        let from_v1 = Story::from_ron(V1_STORY).unwrap();
        assert_eq!(from_v1.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn v3_story_migrates_to_v4_unchanged() {
        use crate::story::Story;

        let v3 = V2_STORY.replace("schema_version: 2", "schema_version: 3");
        let story = Story::from_ron(&v3).unwrap();
        assert_eq!(story.schema_version, 4);
        assert_eq!(story.scenes[0].nodes.len(), 2);
    }

    #[test]
    fn v3_persistent_scope_roundtrips() {
        use crate::defs::{VarDecl, VarScope};
        use crate::story::Story;
        use crate::value::Value;

        let mut story = Story::new("p");
        story
            .variables
            .push(VarDecl::persistent("plays", Value::Int(0)));
        let back = Story::from_ron_unchecked(&story.to_ron().unwrap()).unwrap();
        assert_eq!(back.variables[0].scope, VarScope::Persistent);
        assert_eq!(back.persistent_variables().count(), 1);
    }

    #[test]
    fn broken_v1_document_is_a_parse_error() {
        assert!(matches!(
            v1_to_v2("(schema_version: 1, title: 3)"),
            Err(CoreError::Parse(_))
        ));
    }

    #[test]
    fn steps_are_chained() {
        let steps: &[MigrationStep] = &[|s| Ok(format!("{s}a")), |s| Ok(format!("{s}b"))];
        assert_eq!(migrate_with(1, "", 3, steps).unwrap(), "ab");
        assert_eq!(migrate_with(2, "", 3, steps).unwrap(), "b");
        assert!(matches!(
            migrate_with(1, "", 4, steps),
            Err(CoreError::Migration { .. })
        ));
    }
}
