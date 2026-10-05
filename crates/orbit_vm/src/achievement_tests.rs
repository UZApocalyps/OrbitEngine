//! Achievements (M7-05): the `Unlock` node, its single announcement, rollback
//! and replay, and the voice of a line (M7-06).

use std::collections::BTreeSet;

use orbit_core::{
    Achievement, AchievementId, AssetEntry, AssetId, AssetKind, Node, NodeId, NodeKind, Scene,
    SceneId, Story,
};

use crate::{PresentationEvent, StepResult, Vm};

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

struct Fixture {
    story: Story,
    scene: SceneId,
    gold: AchievementId,
    secret: AchievementId,
    voice: AssetId,
    voiced: NodeId,
}

/// `one`: "a" (voiced), unlock `gold`, "b", unlock `gold` again, "c",
/// unlock `secret`. The scene ends.
fn fixture() -> Fixture {
    let mut story = Story::new("trophies");
    let (gold, secret) = (AchievementId::new(), AchievementId::new());
    for (id, title, hidden) in [(gold, "Gold", false), (secret, "Secret", true)] {
        let title_id = story.strings.add(title);
        let description_id = story.strings.add(format!("{title} description"));
        story.achievements.push(Achievement {
            id: id,
            title_id,
            description_id,
            hidden,
            icon: None,
        });
    }
    let voice = AssetId::new();
    story.assets.assets.push(AssetEntry {
        id: voice,
        kind: AssetKind::Sound,
        path: "voice/a.ogg".into(),
    });
    let say = |story: &mut Story, text: &str, voice| {
        let text_id = story.strings.add(text);
        node(NodeKind::Say {
            speaker: None,
            text_id,
            voice,
        })
    };
    let a = say(&mut story, "a", Some(voice));
    let voiced = a.id;
    let b = say(&mut story, "b", None);
    let c = say(&mut story, "c", None);
    let scene = SceneId::new();
    story.scenes = vec![Scene {
        id: scene,
        name: "one".into(),
        nodes: vec![
            a,
            node(NodeKind::Unlock { achievement: gold }),
            b,
            node(NodeKind::Unlock { achievement: gold }),
            c,
            node(NodeKind::Unlock { achievement: secret }),
        ],
    }];
    Fixture {
        story,
        scene,
        gold,
        secret,
        voice,
        voiced,
    }
}

fn unlocks(events: Vec<PresentationEvent>) -> Vec<AchievementId> {
    events
        .into_iter()
        .filter_map(|e| match e {
            PresentationEvent::AchievementUnlocked { achievement } => Some(achievement),
            _ => None,
        })
        .collect()
}

fn advance(vm: &mut Vm) -> StepResult {
    let r = vm.step();
    assert!(!matches!(r, StepResult::Error(_)), "{r:?}");
    r
}

#[test]
fn an_unlock_is_announced_once_and_remembered() {
    let f = fixture();
    let mut vm = Vm::from_start(f.story.clone()).unwrap();
    assert!(matches!(advance(&mut vm), StepResult::Say { .. })); // a
    assert!(unlocks(vm.take_events()).is_empty());
    assert!(matches!(advance(&mut vm), StepResult::Say { .. })); // b (gold unlocked)
    assert_eq!(unlocks(vm.take_events()), vec![f.gold]);
    // The second `Unlock` of the same achievement does nothing.
    assert!(matches!(advance(&mut vm), StepResult::Say { .. })); // c
    assert!(unlocks(vm.take_events()).is_empty());
    assert_eq!(advance(&mut vm), StepResult::End);
    assert_eq!(unlocks(vm.take_events()), vec![f.secret]);
    assert_eq!(
        vm.unlocked_achievements(),
        &BTreeSet::from([f.gold, f.secret])
    );
}

#[test]
fn a_rollback_does_not_announce_the_unlock_again_nor_take_it_back() {
    let f = fixture();
    let mut vm = Vm::from_start(f.story.clone()).unwrap();
    for _ in 0..3 {
        advance(&mut vm);
    }
    assert_eq!(unlocks(vm.take_events()), vec![f.gold]);
    // Back before the unlock, then forward through it again.
    assert!(vm.rollback(2).is_some());
    assert!(vm.unlocked_achievements().contains(&f.gold), "never taken back");
    assert!(matches!(advance(&mut vm), StepResult::Say { .. }));
    assert!(matches!(advance(&mut vm), StepResult::Say { .. }));
    assert!(
        unlocks(vm.take_events()).is_empty(),
        "no second toast after a rollback"
    );
}

#[test]
fn unlocked_achievements_loaded_from_the_persistent_data_are_not_announced() {
    let f = fixture();
    let mut vm = Vm::from_start(f.story.clone()).unwrap();
    vm.set_unlocked_achievements(BTreeSet::from([f.gold]));
    for _ in 0..4 {
        advance(&mut vm);
    }
    // Gold was already unlocked: only the secret is announced.
    assert_eq!(unlocks(vm.take_events()), vec![f.secret]);
    assert_eq!(vm.unlocked_achievements().len(), 2);
}

#[test]
fn a_scene_replay_unlocks_nothing() {
    let f = fixture();
    let mut vm = Vm::replay(f.story.clone(), f.scene, None, &Default::default()).unwrap();
    for _ in 0..4 {
        advance(&mut vm);
    }
    assert!(unlocks(vm.take_events()).is_empty());
    assert!(vm.unlocked_achievements().is_empty());
    assert!(!vm.unlock_achievement(f.gold));
}

#[test]
fn an_undefined_achievement_is_ignored() {
    let f = fixture();
    let mut vm = Vm::from_start(f.story.clone()).unwrap();
    assert!(!vm.unlock_achievement(AchievementId::new()));
    assert!(vm.take_events().is_empty());
    // The API unlocks too (a plugin, a script), once.
    assert!(vm.unlock_achievement(f.secret));
    assert!(!vm.unlock_achievement(f.secret));
    assert_eq!(unlocks(vm.take_events()), vec![f.secret]);
}

#[test]
fn a_line_knows_its_voice() {
    let f = fixture();
    let vm = Vm::from_start(f.story.clone()).unwrap();
    assert_eq!(vm.say_voice(f.voiced), Some(f.voice));
    let other = f.story.scenes[0].nodes[2].id;
    assert_eq!(vm.say_voice(other), None);
    // Not a line.
    assert_eq!(vm.say_voice(f.story.scenes[0].nodes[1].id), None);
}
