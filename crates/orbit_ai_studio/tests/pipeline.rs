//! AI studio pipeline on mock providers: no network.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;

use orbit_ai_studio::assets::{self, AssetKindAi, AssetStatus};
use orbit_ai_studio::bible::{self, BIBLE_EXAMPLE, Pitch, StepError};
use orbit_ai_studio::image_gen::{ImageModel, ImageRequest};
use orbit_ai_studio::pipeline::{self, PipelineState, ScriptStatus};
use orbit_ai_studio::script::{self, DSL_EXAMPLE};
use orbit_ai_studio::settings::{ImageSettings, Size, TextSettings};
use orbit_ai_studio::text::{Message, ProviderError, Role, TextModel};
use orbit_ai_studio::{ImplementOptions, implement};
use orbit_plugin_api::ProjectRegistry;
use orbit_project::{Project, ProjectManifest};

/// Answers from a queue, and records every conversation it was given.
struct Scripted {
    answers: Mutex<VecDeque<String>>,
    seen: Mutex<Vec<Vec<Message>>>,
}

impl Scripted {
    fn new(answers: &[&str]) -> Self {
        Self {
            answers: Mutex::new(answers.iter().map(|s| (*s).to_owned()).collect()),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl TextModel for Scripted {
    fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.seen.lock().unwrap().push(messages.to_vec());
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| ProviderError::Http("no more answers".into()))
    }
}

/// Answers by step: the bible, or the scene named in the request.
struct ByScene {
    bible: String,
    scenes: BTreeMap<String, VecDeque<String>>,
}

impl TextModel for ByScene {
    fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        let first_user = messages.iter().find(|m| m.role == Role::User).unwrap();
        if messages[0].content.contains("story bible") {
            return Ok(self.bible.clone());
        }
        let id = first_user
            .content
            .split("Write scene `")
            .nth(1)
            .and_then(|r| r.split('`').next())
            .unwrap();
        // Repairs answer the next queued text, the first answer the first.
        let tries = messages.iter().filter(|m| m.role == Role::Assistant).count();
        let q = &self.scenes[id];
        Ok(q.get(tries).or(q.back()).cloned().unwrap())
    }
}

/// Paints a plain picture on a white background (sprite-like).
struct Painter;

impl ImageModel for Painter {
    fn generate(&self, req: &ImageRequest, _: &dyn Fn() -> bool) -> Result<Vec<u8>, ProviderError> {
        if req.prompt.contains("FAIL") {
            return Err(ProviderError::Http("boom".into()));
        }
        let img = image::RgbaImage::from_fn(req.width, req.height, |x, y| {
            let inside = x > req.width / 4 && x < 3 * req.width / 4 && y > req.height / 4;
            if inside { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([255, 255, 255, 255]) }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        Ok(out.into_inner())
    }
}

fn tmp(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ai_studio").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn never() -> bool {
    false
}

fn small_images() -> ImageSettings {
    ImageSettings {
        background_size: Size { width: 64, height: 36 },
        sprite_size: Size { width: 32, height: 48 },
        concurrency: 3,
        ..ImageSettings::default()
    }
}

const BIBLE: &str = r##"Here is the bible:
```json
{
  "title": "Night Shift",
  "synopsis": "A nurse meets a strange patient.",
  "characters": [
    {"id": "lea", "name": "Léa", "description": "a nurse", "personality": "kind", "visual": "young woman, nurse uniform, black bob", "color": "#3a7be0"},
    {"id": "victor", "name": "Victor Hale", "description": "a patient", "personality": "cryptic", "visual": "old man, hospital gown, white beard", "color": ""}
  ],
  "locations": [
    {"id": "ward", "name": "Ward", "visual": "dim hospital ward at night"},
    {"id": "roof", "name": "Roof", "visual": "hospital rooftop at dawn"}
  ],
  "scenes": [
    {"id": "meeting", "title": "Meeting", "location": "ward", "characters": ["lea", "victor"], "summary": "They talk; choice.", "next": ["dawn", "ending_sleep"], "ending": false},
    {"id": "dawn", "title": "Dawn", "location": "roof", "characters": ["lea", "victor"], "summary": "The truth.", "next": [], "ending": true},
    {"id": "ending_sleep", "title": "Sleep", "location": "ward", "characters": ["lea"], "summary": "She sleeps.", "next": [], "ending": true}
  ],
  "variables": [{"name": "trust", "initial": 0, "description": "trust in Victor"}, {"name": "met", "initial": false, "description": ""}]
}
```"##;

const MEETING: &str = r#"scene meeting
bg "assets/ai/backgrounds/ward.png" with dissolve(1.0)
"The ward is silent."
show "Léa" neutral at left
show "Victor Hale" thinking at right with dissolve(0.5)
say "Victor Hale" "You can see them too, can't you?"
$ met = true
menu
    "Listen to him" -> listen
    "Go back to the desk" -> leave
label listen
$ trust += 1
show "Léa" surprised at left
say "Léa" "See what?"
jump_scene dawn
label leave
if trust > 0 -> listen else sleep
label sleep
jump_scene ending_sleep"#;

const DAWN: &str = r#"scene dawn
bg "assets/ai/backgrounds/roof.png"
show "Victor Hale" happy at center
say "Victor Hale" "There. The sun remembers us."
end"#;

const SLEEP: &str = r#"scene ending_sleep
bg "assets/ai/backgrounds/ward.png"
show "Léa" sad at center
say "Léa" "Just a dream."
end"#;

#[test]
fn bible_example_is_valid_and_dsl_example_parses() {
    let b = bible::parse_bible(BIBLE_EXAMPLE).expect("example bible");
    let base = script::scratch_story(&b);
    script::check_scene(&base, &b, "arrival", DSL_EXAMPLE).expect("DSL example checks");
}

#[test]
fn bible_repair_loop_feeds_errors_back() {
    let broken = r#"{"title": "X", "characters": [{"id": "Bad Id", "name": "A", "visual": "v"}],
        "locations": [{"id": "l", "visual": "v"}], "scenes": [{"id": "s", "location": "nowhere", "ending": true}]}"#;
    let model = Scripted::new(&["no json at all", broken, BIBLE]);
    let out = bible::generate_bible(&model, &Pitch::default(), 3, &never).unwrap();
    assert_eq!(out.repairs, 2);
    assert_eq!(out.value.characters[1].name, "Victor Hale");
    let seen = model.seen.lock().unwrap();
    let last = &seen[2];
    let feedback = &last.last().unwrap().content;
    assert!(feedback.contains("Bad Id"), "{feedback}");
   
    assert_eq!(last.iter().filter(|m| m.role == Role::Assistant).count(), 2);
}

#[test]
fn bible_gives_up_after_max_repairs() {
    let model = Scripted::new(&["{}", "{}"]);
    match bible::generate_bible(&model, &Pitch::default(), 1, &never) {
        Err(StepError::Invalid { errors, .. }) => assert!(errors.iter().any(|e| e.contains("title"))),
        other => panic!("{other:?}"),
    }
}

#[test]
fn script_repair_loop_fixes_unknown_names() {
    let b = bible::parse_bible(BIBLE).unwrap();
    let base = script::scratch_story(&b);
    let bad = "```\nscene meeting\nshow \"Nobody\" happy at left\nsay \"Léa\" \"Hi\"\njump_scene dawn\n```";
    let model = Scripted::new(&[bad, MEETING]);
    let scene = b.scene("meeting").unwrap();
    let out = script::generate_scene(&model, &base, &b, scene, "French", 20, None, 2, &never).unwrap();
    assert_eq!(out.repairs, 1);
    let seen = model.seen.lock().unwrap();
    let feedback = &seen[1].last().unwrap().content;
    assert!(feedback.contains("line 2"), "{feedback}");
    assert!(feedback.contains("Nobody"), "{feedback}");
}

#[test]
fn unknown_expression_is_rejected() {
    let b = bible::parse_bible(BIBLE).unwrap();
    let base = script::scratch_story(&b);
    let text = "scene dawn\nshow \"Léa\" furious at left\nend";
    let errors = script::check_scene(&base, &b, "dawn", text).unwrap_err();
    assert!(errors.iter().any(|e| e.contains("furious")), "{errors:?}");
}

#[test]
fn normalize_strips_fences_annotations_and_fixes_header() {
    let t = script::normalize_scene("Sure!\n```dsl\nscene Wrong\n\"Hi\" #@01J000\n```\n", "meeting");
    assert_eq!(t, "scene meeting\n\"Hi\"");
    assert_eq!(script::normalize_scene("\"Hi\"", "x"), "scene x\n\"Hi\"");
}

#[test]
fn asset_plan_naming_and_seeds() {
    let b = bible::parse_bible(BIBLE).unwrap();
    let base = script::scratch_story(&b);
    let used = script::used_expressions(
        &base,
        &b,
        [("meeting", MEETING), ("dawn", DAWN), ("ending_sleep", SLEEP)].into_iter(),
    );
    assert_eq!(used["lea"].iter().cloned().collect::<Vec<_>>(), ["neutral", "sad", "surprised"]);
    assert_eq!(used["victor"].iter().cloned().collect::<Vec<_>>(), ["happy", "neutral", "thinking"]);
    let s = ImageSettings::default();
    let jobs = assets::plan(&b, &used, &s, "", &BTreeMap::new());
    let paths: Vec<&str> = jobs.iter().map(|j| j.path.as_str()).collect();
    assert!(paths.contains(&"assets/ai/backgrounds/ward.png"));
    assert!(paths.contains(&"assets/ai/characters/lea/surprised.png"));
    assert_eq!(jobs.len(), 2 + 6);
    let lea: Vec<u64> = jobs.iter().filter(|j| j.owner == "lea").map(|j| j.seed).collect();
    assert!(lea.windows(2).all(|w| w[0] == w[1]), "one seed per character");
    let victor = jobs.iter().find(|j| j.owner == "victor").unwrap().seed;
    assert_ne!(lea[0], victor);
    assert!(jobs.iter().all(|j| j.prompt.starts_with(&s.style_prefix)));
    // A rebuilt plan keeps what is done.
    let mut done = jobs.clone();
    done[0].status = AssetStatus::Generated;
    let merged = assets::merge_plan(&done, assets::plan(&b, &used, &s, "", &BTreeMap::new()));
    assert_eq!(merged[0].status, AssetStatus::Generated);
    assert_eq!(merged[1].status, AssetStatus::Pending);
}

#[test]
fn background_removal_keeps_the_subject() {
    let mut img = image::RgbaImage::from_fn(20, 20, |x, y| {
        if (5..15).contains(&x) && (5..15).contains(&y) { image::Rgba([10, 10, 10, 255]) } else { image::Rgba([250, 250, 250, 255]) }
    });
    assets::remove_plain_background(&mut img, 20);
    assert_eq!(img.get_pixel(0, 0).0[3], 0);
    assert_eq!(img.get_pixel(10, 10).0[3], 255);
}

#[test]
fn end_to_end_on_a_temp_project() {
    let root = tmp("e2e");
    let model = ByScene {
        bible: BIBLE.to_owned(),
        scenes: [
            ("meeting", vec!["scene meeting\nsay \"Ghost\" \"boo\"\nend", MEETING]),
            ("dawn", vec![DAWN]),
            ("ending_sleep", vec![SLEEP]),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.into_iter().map(str::to_owned).collect()))
        .collect(),
    };
    let text = TextSettings::default();
    let mut images = small_images();
    images.style_prefix = "painted".into();

    let mut state = PipelineState::default();
    state.pitch.text = "A nurse on night shift.".into();
    pipeline::run_bible(&mut state, &model, &text, &never).unwrap();
    state.save(&root).unwrap();

    let mut checkpoints = 0;
    pipeline::run_script(&mut state, &model, &text, None, &never, &mut |s, _, _| {
        checkpoints += 1;
        s.save(&root).unwrap();
    })
    .unwrap();
    assert_eq!(checkpoints, 3);
    assert!(state.script_complete());
    assert_eq!(state.scripts["meeting"].repairs, 1);

    // Resumable: a reload finds the same state.
    let reloaded = PipelineState::load(&root);
    assert_eq!(reloaded.scripts, state.scripts);
    assert_eq!(reloaded.bible, state.bible);

    // One sprite fails: it becomes a placeholder, the rest is generated.
    pipeline::plan_assets(&mut state, &images).unwrap();
    for j in &mut state.assets {
        if j.owner == "victor" && j.expression.as_deref() == Some("happy") {
            j.prompt.push_str(" FAIL");
        }
    }
    let b = state.bible.clone().unwrap();
    let ok = assets::generate_all(&root, &b, &mut state.assets, Some(&Painter), &images, &never, &|_, _| {});
    assert!(ok);
    let failed: Vec<_> = state.assets.iter().filter(|j| matches!(j.status, AssetStatus::Placeholder(_))).collect();
    assert_eq!(failed.len(), 1);
    for j in &state.assets {
        let img = image::open(root.join(&j.path)).unwrap().to_rgba8();
        if j.kind == AssetKindAi::Sprite && j.status == AssetStatus::Generated {
            assert_eq!(img.get_pixel(0, 0).0[3], 0, "sprite background removed");
        }
    }

    // Implement into a project: one undo entry, a story that validates.
    let original = orbit_core::Story::new("Empty");
    let mut project = Project::new(original.clone());
    let manifest = ProjectManifest::new("e2e");
    let registry = ProjectRegistry::default();
    let report = implement(&mut project, &root, &manifest, &registry, &state, ImplementOptions { replace_scenes: true }).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.characters, 2);
    assert_eq!(report.scenes, 3);
    assert_eq!(report.variables, 2);
    let story = project.story();
    assert_eq!(story.scenes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["meeting", "dawn", "ending_sleep"]);
    assert_eq!(story.characters.len(), 2);
    assert!(story.assets.assets.iter().all(|a| a.path.starts_with("assets/ai/")));
    assert!(orbit_core::validate::validate(story).is_ok());

    assert!(project.undo().unwrap());
    assert_eq!(project.story().scenes.len(), 0);
    assert!(project.story().characters.is_empty());
    assert!(!project.can_undo());

    // Appending refuses a name clash, then works into a story with content.
    let mut project = Project::new(original);
    implement(&mut project, &root, &manifest, &registry, &state, ImplementOptions { replace_scenes: false }).unwrap();
    let err = implement(&mut project, &root, &manifest, &registry, &state, ImplementOptions { replace_scenes: false }).unwrap_err();
    assert!(err.contains("meeting"), "{err}");
}

#[test]
fn script_step_keeps_invalid_scene_for_editing() {
    let b = bible::parse_bible(BIBLE).unwrap();
    let mut state = PipelineState { bible: Some(b), ..PipelineState::default() };
    let model = Scripted::new(&["scene dawn\nsay \"Nobody\" \"x\"\nend", "scene dawn\nsay \"Nobody\" \"y\"\nend"]);
    let text = TextSettings { max_repairs: 1, ..TextSettings::default() };
    let r = pipeline::run_script(&mut state, &model, &text, Some("dawn"), &never, &mut |_, _, _| {});
    assert!(matches!(r, Err(StepError::Invalid { .. })));
    assert!(matches!(state.scripts["dawn"].status, ScriptStatus::Invalid(_)));
    // The user fixes it by hand.
    state.set_script_text("dawn", DAWN);
    assert_eq!(state.scripts["dawn"].status, ScriptStatus::Valid);
}

#[test]
fn mcp_tools_check_scripts() {
    let tools = orbit_ai_studio::mcp::tools();
    let mut project = Project::new(orbit_core::Story::new("x"));
    let check = tools.iter().find(|t| t.name == "check_scene").unwrap();
    let args = serde_json::json!({"bible": bible::extract_json(BIBLE).unwrap(), "scene": "dawn", "text": DAWN});
    let out = (check.call)(&mut project, &args.to_string()).unwrap();
    assert!(out.contains("happy"), "{out}");
    let bad = serde_json::json!({"bible": bible::extract_json(BIBLE).unwrap(), "scene": "dawn", "text": "show \"X\" happy at left"});
    assert!((check.call)(&mut project, &bad.to_string()).is_err());
    let guide = tools.iter().find(|t| t.name == "dsl_guide").unwrap();
    assert!((guide.call)(&mut project, "{}").unwrap().contains("jump_scene"));
}

fn dangling_bible(next: &str) -> String {
    format!(
        r##"{{"title":"T","characters":[{{"id":"mara","name":"Mara","visual":"v"}}],
        "locations":[{{"id":"hall","visual":"v"}}],
        "scenes":[
          {{"id":"a","location":"hall","characters":["mara","ghost"],"next":[{next}]}},
          {{"id":"talk_to_sophia","location":"hall","characters":["mara"],"next":["b"]}},
          {{"id":"b","location":"hall","characters":["mara"],"ending":true}}]}}"##
    )
}

#[test]
fn dangling_next_gets_a_stub_scene() {
    let model = Scripted::new(&[&dangling_bible(r#""keeping_secret""#)]);
    let out = bible::generate_bible(&model, &Pitch::default(), 0, &never).unwrap();
    assert_eq!(out.repairs, 0);
    let b = out.value;
    let stub = b.scene("keeping_secret").expect("stub");
    assert_eq!(stub.location, "hall");
    assert_eq!(stub.next, vec!["talk_to_sophia".to_owned()]);
    assert!(stub.summary.contains("Continuation of a"));
    assert!(bible::check(&b).is_empty());
    assert!(out.warnings.iter().any(|w| w.contains("stub created")));
    assert!(out.warnings.iter().any(|w| w.contains("ghost")));
    assert_eq!(b.scene("a").unwrap().characters, vec!["mara".to_owned()]);
}

#[test]
fn near_miss_next_is_remapped() {
    let model = Scripted::new(&[&dangling_bible(r#""Talk-To-Sophia""#)]);
    let out = bible::generate_bible(&model, &Pitch::default(), 0, &never).unwrap();
    let b = out.value;
    assert_eq!(b.scenes.len(), 3, "no stub");
    assert_eq!(b.scene("a").unwrap().next, vec!["talk_to_sophia".to_owned()]);
    assert!(bible::check(&b).is_empty());
}
