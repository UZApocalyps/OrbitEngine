//! The `Project`: story + undo/redo + events + per-node versions.

use std::collections::HashMap;

use orbit_core::{NodeId, Story};
use thiserror::Error;

use crate::command::{Command, CommandError};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectError {
    #[error(transparent)]
    Command(#[from] CommandError),
}

/// Why a change event was emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Applied,
    Undone,
    Redone,
}

/// Emitted once per executed command/transaction, undo or redo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEvent {
    pub kind: ChangeKind,
    /// Nodes created, modified or removed (deduplicated, in first-touch order).
    pub touched: Vec<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

type Listener = Box<dyn FnMut(&ChangeEvent) + Send>;

/// One undo/redo entry: commands in the order they were applied. Undoing an
/// entry applies them in reverse.
type Entry = Vec<Command>;

pub struct Project {
    story: Story,
    versions: HashMap<NodeId, u64>,
    clock: u64,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    listeners: Vec<(SubscriptionId, Listener)>,
    next_sub: u64,
}

impl Project {
    pub fn new(story: Story) -> Self {
        let mut clock = 0;
        let versions = story
            .scenes
            .iter()
            .flat_map(|s| &s.nodes)
            .map(|n| {
                clock += 1;
                (n.id, clock)
            })
            .collect();
        Self {
            story,
            versions,
            clock,
            undo: Vec::new(),
            redo: Vec::new(),
            listeners: Vec::new(),
            next_sub: 0,
        }
    }

    /// Read-only view of the story. There is intentionally no `&mut` access.
    pub fn story(&self) -> &Story {
        &self.story
    }

    /// Consumes the project and returns the story (e.g. to save it).
    pub fn into_story(self) -> Story {
        self.story
    }

    /// Current version of a node (`None` if it does not exist). Pass it back as
    /// `expected_version` to make an edit fail instead of overwriting a
    /// concurrent change.
    pub fn node_version(&self, id: NodeId) -> Option<u64> {
        self.versions.get(&id).copied()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Registers a change listener, called after each change is committed.
    pub fn subscribe(
        &mut self,
        listener: impl FnMut(&ChangeEvent) + Send + 'static,
    ) -> SubscriptionId {
        self.next_sub += 1;
        let id = SubscriptionId(self.next_sub);
        self.listeners.push((id, Box::new(listener)));
        id
    }

    pub fn unsubscribe(&mut self, id: SubscriptionId) -> bool {
        let before = self.listeners.len();
        self.listeners.retain(|(i, _)| *i != id);
        self.listeners.len() != before
    }

    /// Executes a single command as its own undo entry.
    pub fn execute(&mut self, command: Command) -> Result<(), ProjectError> {
        self.transaction(|tx| tx.execute(command))
    }

    /// Runs several commands as **one** undo entry. If the closure (or any
    /// command) fails, every command already applied is rolled back and the
    /// project is left unchanged (no undo entry, no event).
    pub fn transaction<R>(
        &mut self,
        f: impl FnOnce(&mut Transaction<'_>) -> Result<R, ProjectError>,
    ) -> Result<R, ProjectError> {
        let mut tx = Transaction {
            project: self,
            inverses: Vec::new(),
            touched: Vec::new(),
        };
        match f(&mut tx) {
            Ok(value) => {
                let Transaction {
                    inverses, touched, ..
                } = tx;
                if !inverses.is_empty() {
                    self.undo.push(inverses);
                    self.redo.clear();
                    self.emit(ChangeKind::Applied, touched);
                }
                Ok(value)
            }
            Err(err) => {
                tx.rollback();
                Err(err)
            }
        }
    }

    /// Undoes the last entry. Returns `Ok(false)` if there is nothing to undo.
    pub fn undo(&mut self) -> Result<bool, ProjectError> {
        let Some(entry) = self.undo.pop() else {
            return Ok(false);
        };
        match self.replay(entry) {
            Ok((reverse, touched)) => {
                self.redo.push(reverse);
                self.emit(ChangeKind::Undone, touched);
                Ok(true)
            }
            Err((entry, err)) => {
                self.undo.push(entry);
                Err(err)
            }
        }
    }

    /// Redoes the last undone entry. Returns `Ok(false)` if there is none.
    pub fn redo(&mut self) -> Result<bool, ProjectError> {
        let Some(entry) = self.redo.pop() else {
            return Ok(false);
        };
        match self.replay(entry) {
            Ok((reverse, touched)) => {
                self.undo.push(reverse);
                self.emit(ChangeKind::Redone, touched);
                Ok(true)
            }
            Err((entry, err)) => {
                self.redo.push(entry);
                Err(err)
            }
        }
    }

    /// Applies an entry in reverse order. All-or-nothing: on failure the
    /// already-applied part is rolled back and the entry returned intact.
    #[allow(clippy::type_complexity)]
    fn replay(&mut self, entry: Entry) -> Result<(Entry, Vec<NodeId>), (Entry, ProjectError)> {
        let mut reverse = Vec::with_capacity(entry.len());
        let mut touched = Vec::new();
        for cmd in entry.iter().rev() {
            match self.apply_raw(cmd, &mut touched) {
                Ok(inv) => reverse.push(inv),
                Err(err) => {
                    for inv in reverse.iter().rev() {
                        let _ = self.apply_raw(inv, &mut Vec::new());
                    }
                    return Err((entry, err.into()));
                }
            }
        }
        Ok((reverse, touched))
    }

    /// Applies one command, bumping the version of the touched node.
    /// This is the only place that mutates the story.
    fn apply_raw(
        &mut self,
        cmd: &Command,
        touched: &mut Vec<NodeId>,
    ) -> Result<Command, CommandError> {
        if let Some((node, expected)) = cmd.expected() {
            let actual = self.versions.get(&node).copied();
            match actual {
                Some(actual) if actual != expected => {
                    return Err(CommandError::VersionConflict {
                        node,
                        expected,
                        actual,
                    });
                }
                None => return Err(CommandError::UnknownNode(node)),
                _ => {}
            }
        }
        let applied = cmd.apply(&mut self.story)?;
        for id in applied.touched {
            if self.story.locate(id).is_some() {
                self.clock += 1;
                self.versions.insert(id, self.clock);
            } else {
                self.versions.remove(&id);
            }
            if !touched.contains(&id) {
                touched.push(id);
            }
        }
        Ok(applied.inverse)
    }

    fn emit(&mut self, kind: ChangeKind, touched: Vec<NodeId>) {
        let event = ChangeEvent { kind, touched };
        for (_, listener) in &mut self.listeners {
            listener(&event);
        }
    }
}

/// A group of commands that becomes a single undo entry.
pub struct Transaction<'a> {
    project: &'a mut Project,
    inverses: Vec<Command>,
    touched: Vec<NodeId>,
}

impl Transaction<'_> {
    /// Applies a command inside the transaction. Later commands see the
    /// effects of earlier ones.
    pub fn execute(&mut self, command: Command) -> Result<(), ProjectError> {
        let inverse = self.project.apply_raw(&command, &mut self.touched)?;
        self.inverses.push(inverse);
        Ok(())
    }

    /// Read-only access to the project (e.g. to look up node versions).
    pub fn project(&self) -> &Project {
        self.project
    }

    fn rollback(&mut self) {
        for inv in self.inverses.drain(..).rev() {
            let _ = self.project.apply_raw(&inv, &mut Vec::new());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use orbit_core::{CharacterId, Node, NodeKind, Scene, SceneId, StringId};

    use super::*;

    fn project() -> (Project, SceneId) {
        let mut story = Story::new("t");
        let scene = SceneId::new();
        story.scenes.push(Scene {
            id: scene,
            name: "s".into(),
            nodes: vec![],
        });
        (Project::new(story), scene)
    }

    /// A `Say` node together with the command registering its text.
    fn say(text: &str) -> (Command, Node) {
        let text_id = StringId::new();
        (
            Command::SetString {
                id: text_id,
                text: Some(text.into()),
            },
            Node {
                id: NodeId::new(),
                kind: NodeKind::Say {
                    speaker: None,
                    text_id,
                },
            },
        )
    }

    /// Inserts a new `Say` (string + node) as one undo entry.
    fn insert_say(p: &mut Project, scene: SceneId, index: usize, text: &str) -> NodeId {
        let (set, node) = say(text);
        let id = node.id;
        p.transaction(|tx| {
            tx.execute(set)?;
            tx.execute(insert(scene, index, node))
        })
        .unwrap();
        id
    }

    fn texts(p: &Project) -> Vec<String> {
        p.story().scenes[0]
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Say { text_id, .. } => p.story().text(*text_id).map(str::to_owned),
                _ => None,
            })
            .collect()
    }

    fn set_say(id: NodeId, text: &str, expected_version: Option<u64>) -> Command {
        Command::SetSay {
            id,
            speaker: None,
            text: text.into(),
            expected_version,
        }
    }

    fn insert(scene: SceneId, index: usize, node: Node) -> Command {
        Command::InsertNode { scene, index, node }
    }

    #[test]
    fn apply_undo_redo() {
        let (mut p, scene) = project();
        let id = insert_say(&mut p, scene, 0, "a");
        let speaker = CharacterId::new();
        p.execute(Command::SetSay {
            id,
            speaker: Some(speaker),
            text: "b".into(),
            expected_version: None,
        })
        .unwrap();
        assert_eq!(texts(&p), ["b"]);

        assert!(p.undo().unwrap());
        assert_eq!(texts(&p), ["a"]);
        assert!(p.undo().unwrap());
        assert!(texts(&p).is_empty());
        // The string registered by the transaction is undone too.
        assert!(p.story().strings.entries.is_empty());
        assert!(!p.undo().unwrap());

        assert!(p.redo().unwrap());
        assert!(p.redo().unwrap());
        assert_eq!(texts(&p), ["b"]);
        assert!(!p.redo().unwrap());
    }

    #[test]
    fn remove_restores_position_on_undo() {
        let (mut p, scene) = project();
        insert_say(&mut p, scene, 0, "a");
        let bid = insert_say(&mut p, scene, 1, "b");
        insert_say(&mut p, scene, 2, "c");
        p.execute(Command::RemoveNode {
            id: bid,
            expected_version: None,
        })
        .unwrap();
        assert_eq!(texts(&p), ["a", "c"]);
        p.undo().unwrap();
        assert_eq!(texts(&p), ["a", "b", "c"]);
    }

    #[test]
    fn new_command_clears_redo() {
        let (mut p, scene) = project();
        insert_say(&mut p, scene, 0, "a");
        p.undo().unwrap();
        assert!(p.can_redo());
        insert_say(&mut p, scene, 0, "b");
        assert!(!p.can_redo());
    }

    #[test]
    fn transaction_is_a_single_undo_entry() {
        let (mut p, scene) = project();
        p.transaction(|tx| {
            for (i, t) in ["a", "b", "c"].into_iter().enumerate() {
                let (set, node) = say(t);
                tx.execute(set)?;
                tx.execute(insert(scene, i, node))?;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(texts(&p), ["a", "b", "c"]);
        assert!(p.undo().unwrap());
        assert!(texts(&p).is_empty());
        assert!(!p.can_undo());
        p.redo().unwrap();
        assert_eq!(texts(&p), ["a", "b", "c"]);
    }

    #[test]
    fn failed_transaction_rolls_back_everything() {
        let (mut p, scene) = project();
        let events = Arc::new(Mutex::new(0));
        let e = events.clone();
        p.subscribe(move |_| *e.lock().unwrap() += 1);

        let err = p.transaction(|tx| {
            let (set, node) = say("a");
            tx.execute(set)?;
            tx.execute(insert(scene, 0, node))?;
            let (set, node) = say("b");
            tx.execute(set)?;
            tx.execute(insert(SceneId::new(), 0, node)) // unknown scene
        });
        assert!(matches!(
            err,
            Err(ProjectError::Command(CommandError::UnknownScene(_)))
        ));
        assert!(texts(&p).is_empty());
        assert!(p.story().strings.entries.is_empty());
        assert!(!p.can_undo());
        assert_eq!(*events.lock().unwrap(), 0);
    }

    #[test]
    fn version_conflict_is_detected_without_modification() {
        let (mut p, scene) = project();
        let id = insert_say(&mut p, scene, 0, "a");
        let seen = p.node_version(id).unwrap();

        // Someone else edits the node in the meantime.
        p.execute(set_say(id, "theirs", Some(seen))).unwrap();

        let err = p.execute(set_say(id, "mine", Some(seen)));
        assert!(matches!(
            err,
            Err(ProjectError::Command(CommandError::VersionConflict { .. }))
        ));
        assert_eq!(texts(&p), ["theirs"]);

        // Retrying with the fresh version succeeds.
        let fresh = p.node_version(id).unwrap();
        assert!(fresh > seen);
        p.execute(set_say(id, "mine", Some(fresh))).unwrap();
        assert_eq!(texts(&p), ["mine"]);
    }

    #[test]
    fn events_are_emitted_per_change() {
        let (mut p, scene) = project();
        let log: Arc<Mutex<Vec<ChangeEvent>>> = Arc::default();
        let l = log.clone();
        let sub = p.subscribe(move |e| l.lock().unwrap().push(e.clone()));

        let id = insert_say(&mut p, scene, 0, "a");
        p.undo().unwrap();
        p.redo().unwrap();
        {
            let log = log.lock().unwrap();
            let kinds: Vec<_> = log.iter().map(|e| e.kind).collect();
            assert_eq!(
                kinds,
                [ChangeKind::Applied, ChangeKind::Undone, ChangeKind::Redone]
            );
            assert!(log.iter().all(|e| e.touched == [id]));
        }

        assert!(p.unsubscribe(sub));
        p.undo().unwrap();
        assert_eq!(log.lock().unwrap().len(), 3);
    }

    #[test]
    fn invalid_commands_are_rejected() {
        let (mut p, scene) = project();
        let (set, n) = say("a");
        let dup = n.clone();
        p.transaction(|tx| {
            tx.execute(set)?;
            tx.execute(insert(scene, 0, n))
        })
        .unwrap();
        assert!(p.execute(insert(scene, 0, dup)).is_err());
        let (_, other) = say("x");
        assert!(p.execute(insert(scene, 5, other)).is_err());
        assert!(
            p.execute(Command::RemoveNode {
                id: NodeId::new(),
                expected_version: None
            })
            .is_err()
        );
        assert_eq!(texts(&p), ["a"]);
    }

    #[test]
    fn referenced_label_cannot_be_removed() {
        let (mut p, scene) = project();
        let label = Node {
            id: NodeId::new(),
            kind: NodeKind::Label { name: "l".into() },
        };
        let lid = label.id;
        let call = Node {
            id: NodeId::new(),
            kind: NodeKind::Call { label: lid },
        };
        p.execute(insert(scene, 0, label)).unwrap();
        p.execute(insert(scene, 1, call)).unwrap();
        let err = p.execute(Command::RemoveNode {
            id: lid,
            expected_version: None,
        });
        assert!(matches!(
            err,
            Err(ProjectError::Command(CommandError::StillReferenced(..)))
        ));
    }

    #[test]
    fn set_say_on_wrong_kind_or_missing_string_fails() {
        let (mut p, scene) = project();
        let label = Node {
            id: NodeId::new(),
            kind: NodeKind::Label { name: "l".into() },
        };
        let id = label.id;
        p.execute(insert(scene, 0, label)).unwrap();
        assert!(matches!(
            p.execute(set_say(id, "x", None)),
            Err(ProjectError::Command(CommandError::NotASay(_)))
        ));

        // A Say whose string was never registered: the edit is refused and
        // the speaker is not modified.
        let (_, orphan) = say("lost");
        let oid = orphan.id;
        p.execute(insert(scene, 1, orphan)).unwrap();
        let before = p.story().clone();
        assert!(matches!(
            p.execute(Command::SetSay {
                id: oid,
                speaker: Some(CharacterId::new()),
                text: "x".into(),
                expected_version: None
            }),
            Err(ProjectError::Command(CommandError::UnknownString(_)))
        ));
        assert_eq!(*p.story(), before);
    }

    #[test]
    fn strings_in_use_cannot_be_removed() {
        let (mut p, scene) = project();
        let (set, node) = say("a");
        let Command::SetString { id: sid, .. } = set else {
            unreachable!()
        };
        p.transaction(|tx| {
            tx.execute(set)?;
            tx.execute(insert(scene, 0, node))
        })
        .unwrap();
        assert!(matches!(
            p.execute(Command::SetString {
                id: sid,
                text: None
            }),
            Err(ProjectError::Command(CommandError::StringInUse(..)))
        ));
        assert!(matches!(
            p.execute(Command::SetString {
                id: StringId::new(),
                text: None
            }),
            Err(ProjectError::Command(CommandError::UnknownString(_)))
        ));
    }
}
