//! The asset library: every image, music and sound of the project as a grid
//! or a list, with filters, search, sort, import (button, or files dropped on
//! the window), asynchronous thumbnails, audio preview, rename / remove with
//! the usage check, and drag and drop of an asset onto an asset field.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, CornerRadius, FontId, Id, Order, Sense, Stroke, Ui, vec2};
use orbit_audio::{AudioEngine, MusicOptions, Sound};
use orbit_core::{AssetEntry, AssetId, AssetKind, Story};
use orbit_editor_api::fields::AssetDrag;
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, Level,
    Location, PanelDef, Plugin, PluginManifest, Translator, UiText, icons,
};
use orbit_project::{AssetUser, asset_users, ops};

use super::thumbs::{self, ThumbState};
use super::ui_util::{
    accent, accent_soft, danger, faint, icon_button, primary_button, strong, warning, weak,
};
use super::{ids, reveal};
use crate::assets::{self, DiskFile, ImportReport, MoveJournal};

const CELL: egui::Vec2 = egui::Vec2::new(108.0, 124.0);
const THUMB_BOX: egui::Vec2 = egui::Vec2::new(96.0, 76.0);
const LIST_ROW: f32 = 30.0;

/// The library plugin; `moves` is shared with the core (undo / redo move
/// renamed files back).
pub struct LibraryPlugin {
    state: Rc<RefCell<LibUi>>,
    moves: Rc<RefCell<MoveJournal>>,
}

impl LibraryPlugin {
    pub fn new(moves: Rc<RefCell<MoveJournal>>) -> Self {
        Self {
            state: Rc::default(),
            moves,
        }
    }
}

impl Plugin for LibraryPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::LIBRARY, "Bibliothèque d'assets", "0.1.0")
    }
}

impl EditorPlugin for LibraryPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        let (state, moves) = (self.state.clone(), self.moves.clone());
        reg.panel(PanelDef::new(
            "library",
            "library.title",
            icons::IMAGES,
            DockSlot::Bottom,
            move |ui, ctx| panel(ui, ctx, &state, &moves),
        ));
        reg.action(
            ActionDef::new("import", "library.action.import", |c| {
                import_dialog(c);
            })
            .icon(icons::UPLOAD_SIMPLE)
            .category("category.file")
            .shortcut(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::I,
            )
            .menu("menu.file", 130)
            .enabled(|c| c.has_project()),
        );
    }
}

// ------------------------------------------------------------------ import

/// Imports files (picked or dropped) into the open project and tells the
/// user what happened: one toast for the successes, one per refusal, the
/// reason in the editor's language. The first new asset is selected in the library.
pub fn import_paths(ctx: &mut EditorContext<'_>, paths: &[PathBuf]) -> ImportReport {
    let Some(root) = assets_root_of(ctx) else {
        let message = ctx.t("import.no_project");
        ctx.error(message);
        return ImportReport::default();
    };
    let registry = ctx.registry();
    let texts = ctx.translator();
    let report = match ctx.ops(|p| {
        Ok(assets::import_files(
            p,
            &root,
            &registry.project,
            paths,
            &texts,
        ))
    }) {
        Ok(r) => r,
        Err(e) => {
            ctx.error(e.to_string());
            return ImportReport::default();
        }
    };
    let added = report.added();
    if added > 0 {
        let kinds: Vec<String> = [AssetKind::Image, AssetKind::Music, AssetKind::Sound]
            .into_iter()
            .filter_map(|k| {
                let n = report
                    .imported
                    .iter()
                    .filter(|i| !i.existing && i.kind == k)
                    .count();
                (n > 0).then(|| texts.tn(kind_count_key(k), n, &[]))
            })
            .collect();
        let message = texts.tn("import.done", added, &[("kinds", &kinds.join(", "))]);
        ctx.info(message);
    }
    let existing = report.imported.len() - added;
    if existing > 0 {
        let message = texts.tn("import.already", existing, &[]);
        ctx.info(message);
    }
    for (path, error) in &report.failed {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let message = texts.tf(
            "import.refused",
            &[("name", &name), ("error", &error.message(&texts))],
        );
        ctx.error(message);
    }
    if let Some(first) = report.imported.first() {
        ctx.state_mut().selected_asset = Some(first.id);
    }
    if !report.imported.is_empty() {
        ctx.state_mut().asset_epoch += 1;
    }
    report
}

/// Folder the manifest's relative paths resolve against (the project folder,
/// or the folder above a bare story file that holds its `assets/`).
fn assets_root_of(ctx: &EditorContext<'_>) -> Option<PathBuf> {
    let root = ctx.project_info()?.root.clone();
    Some(match ctx.story() {
        Some(story) => crate::preview_host::find_assets_root(&root, story),
        None => root,
    })
}

fn import_dialog(ctx: &mut EditorContext<'_>) {
    let Some(root) = assets_root_of(ctx) else {
        return;
    };
    let extensions = ctx.registry().project.import_extensions();
    let ext_refs: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let files = rfd::FileDialog::new()
        .set_title(ctx.t("import.dialog_title"))
        .set_directory(&root)
        .add_filter(ctx.t("import.filter"), &ext_refs)
        .pick_files();
    if let Some(files) = files {
        import_paths(ctx, &files);
        ctx.request(EditorRequest::ShowPanel(super::panels::LIBRARY.to_owned()));
    }
}

/// Plural key of "N images" / "N music tracks" / "N sounds".
fn kind_count_key(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Image => "asset.count.image",
        AssetKind::Music => "asset.count.music",
        AssetKind::Sound => "asset.count.sound",
        AssetKind::Scene3d => "asset.count.scene3d",
    }
}

fn kind_name(texts: &Translator, kind: AssetKind) -> String {
    texts.t(match kind {
        AssetKind::Image => "asset.kind.image",
        AssetKind::Music => "asset.kind.music",
        AssetKind::Sound => "asset.kind.sound",
        AssetKind::Scene3d => "asset.kind.scene3d",
    })
}

fn kind_icon(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Image => icons::IMAGE,
        AssetKind::Music => icons::MUSIC_NOTES,
        AssetKind::Sound => icons::SPEAKER_HIGH,
        AssetKind::Scene3d => icons::CUBE,
    }
}

// ------------------------------------------------------------- audio preview

/// Plays one asset at a time through its own audio output.
#[derive(Default)]
struct AudioPreview {
    engine: Option<AudioEngine>,
    playing: Option<AssetId>,
    loading: Option<(AssetId, Receiver<Result<Sound, String>>)>,
    started: Option<Instant>,
    length: Duration,
}

impl AudioPreview {
    fn play(&mut self, id: AssetId, path: PathBuf, label: String) {
        self.stop();
        let (tx, rx) = channel();
        // Decoding a music file takes a while: not on the UI thread.
        let _ = std::thread::Builder::new()
            .name("orbit-audio-decode".to_owned())
            .spawn(move || {
                let result = std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| orbit_assets::decode_audio(&label, &bytes))
                    .and_then(|pcm| Sound::from_pcm(&pcm).map_err(|e| e.to_string()));
                let _ = tx.send(result);
            });
        self.loading = Some((id, rx));
    }

    fn stop(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.stop_music(Duration::ZERO);
        }
        self.playing = None;
        self.loading = None;
        self.started = None;
    }

    /// Advances the preview; returns an error message when decoding failed.
    fn update(&mut self, waker: &orbit_editor_api::Waker) -> Option<String> {
        let mut error = None;
        if let Some((id, rx)) = &self.loading {
            match rx.try_recv() {
                Ok(Ok(sound)) => {
                    let id = *id;
                    self.loading = None;
                    let engine = self.engine.get_or_insert_with(AudioEngine::new);
                    self.length = sound.duration();
                    engine.play_music(
                        &sound,
                        MusicOptions {
                            looping: false,
                            ..MusicOptions::default()
                        },
                    );
                    self.playing = Some(id);
                    self.started = Some(Instant::now());
                }
                Ok(Err(e)) => {
                    self.loading = None;
                    error = Some(e);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // The decoding thread does not know about frames: poll.
                    let waker = waker.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(60));
                        waker.wake();
                    });
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.loading = None,
            }
        }
        if let Some(engine) = self.engine.as_mut() {
            engine.update();
            if self.playing.is_some() && !engine.is_music_playing() {
                self.playing = None;
                self.started = None;
            }
        }
        error
    }

    fn is_busy(&self) -> bool {
        self.playing.is_some() || self.loading.is_some()
    }

    fn progress(&self) -> Option<(Duration, Duration)> {
        self.started
            .map(|t| (t.elapsed().min(self.length), self.length))
    }
}

// ---------------------------------------------------------------------- state

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Kind(AssetKind),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Name,
    Kind,
    Size,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Grid,
    List,
}

struct LibUi {
    filter: Filter,
    sort: Sort,
    view: View,
    query: String,
    /// Asset being renamed, and the new file name (without extension).
    rename: Option<(AssetId, String)>,
    /// Asset waiting for the user's confirmation to be removed.
    confirm_delete: Option<AssetId>,
    scan: Vec<DiskFile>,
    scan_key: Option<(PathBuf, u64)>,
    audio: AudioPreview,
    seen_reveal: u64,
    scroll_to: Option<usize>,
}

impl Default for LibUi {
    fn default() -> Self {
        Self {
            filter: Filter::All,
            sort: Sort::Name,
            view: View::Grid,
            query: String::new(),
            rename: None,
            confirm_delete: None,
            scan: Vec::new(),
            scan_key: None,
            audio: AudioPreview::default(),
            seen_reveal: 0,
            scroll_to: None,
        }
    }
}

/// One cell of the library.
#[derive(Clone)]
struct Item {
    entry: AssetEntry,
    /// The file is in the asset folder but not in the manifest.
    undeclared: bool,
    size: u64,
    /// A problem with the file (missing, wrong case), from the live validation.
    issue: Option<(Level, String)>,
}

impl Item {
    fn file_name(&self) -> &str {
        self.entry
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&self.entry.path)
    }
}

/// What the user asked this frame; applied once the story is no longer borrowed.
enum Action {
    Select(AssetId),
    Import,
    Declare(String),
    Play(AssetId),
    Stop,
    BeginRename(AssetId),
    CommitRename(AssetId, String),
    AskDelete(AssetId),
    Delete(AssetId),
    ShowUser(Location),
}

fn items(
    story: &Story,
    scan: &[DiskFile],
    extensions: &[String],
    issues: &HashMap<AssetId, (Level, String)>,
) -> Vec<Item> {
    let sizes: HashMap<&str, u64> = scan.iter().map(|f| (f.path.as_str(), f.len)).collect();
    let mut out: Vec<Item> = story
        .assets
        .assets
        .iter()
        .map(|a| Item {
            entry: a.clone(),
            undeclared: false,
            size: sizes.get(a.path.as_str()).copied().unwrap_or(0),
            issue: issues.get(&a.id).cloned(),
        })
        .collect();
    let declared: std::collections::HashSet<String> = story
        .assets
        .assets
        .iter()
        .map(|a| a.path.to_lowercase())
        .collect();
    for f in scan {
        let ext = f.path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        if declared.contains(&f.path.to_lowercase())
            || !ext.is_some_and(|e| extensions.contains(&e))
        {
            continue;
        }
        // The kind is only known after decoding: guess from the folder.
        let kind = if f.path.starts_with("assets/music") {
            AssetKind::Music
        } else if f.path.starts_with("assets/sounds")
            || f.path.ends_with(".wav")
            || f.path.ends_with(".ogg")
        {
            AssetKind::Sound
        } else {
            AssetKind::Image
        };
        out.push(Item {
            entry: AssetEntry {
                id: AssetId::from_u128(0),
                kind,
                path: f.path.clone(),
            },
            undeclared: true,
            size: f.len,
            issue: None,
        });
    }
    out
}

fn human_size(texts: &Translator, bytes: u64) -> String {
    match bytes {
        0 => String::new(),
        b if b < 1024 => texts.tf("asset.size.bytes", &[("n", &b)]),
        b if b < 1024 * 1024 => texts.tf(
            "asset.size.kb",
            &[("n", &format!("{:.0}", b as f64 / 1024.0))],
        ),
        b => texts.tf(
            "asset.size.mb",
            &[("n", &format!("{:.1}", b as f64 / (1024.0 * 1024.0)))],
        ),
    }
}

// ---------------------------------------------------------------------- panel

fn panel(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    state: &Rc<RefCell<LibUi>>,
    moves: &Rc<RefCell<MoveJournal>>,
) {
    if !ctx.has_project() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(icons::IMAGES)
                    .size(28.0)
                    .color(faint(ui)),
            );
            ui.label(egui::RichText::new(ui.t("library.no_project")).color(weak(ui)));
        });
        return;
    }
    let mut st = state.borrow_mut();
    let waker = ctx.state().waker.clone();
    if let Some(error) = st.audio.update(&waker) {
        let message = ctx.tf("library.read_error", &[("error", &error)]);
        ctx.error(message);
    }
    if st.audio.is_busy() {
        ui.ctx().request_repaint_after(Duration::from_millis(120));
    }

    let Some(root) = ctx
        .story()
        .zip(ctx.project_info().map(|i| i.root.clone()))
        .map(|(story, root)| crate::preview_host::find_assets_root(&root, story))
    else {
        return;
    };
    // The files of the asset folder, looked at again when something changed.
    let key = (root.clone(), ctx.state().asset_epoch);
    if st.scan_key.as_ref() != Some(&key) {
        st.scan = assets::scan_assets(&root);
        st.scan_key = Some(key);
    }
    let extensions = ctx.registry().project.import_extensions();
    let mut issues: HashMap<AssetId, (Level, String)> = HashMap::new();
    for p in &ctx.state().diagnostics.problems {
        if let Some(Location::Asset(id)) = p.location {
            issues
                .entry(id)
                .or_insert_with(|| (p.level, p.message.clone()));
        }
    }
    let Some(story) = ctx.story() else { return };
    let all = items(story, &st.scan, &extensions, &issues);

    // A reveal (problem, search hit, drop) selects an asset: bring it into view.
    let serial = ctx.state().reveal_serial;
    if st.seen_reveal != serial {
        st.seen_reveal = serial;
        st.scroll_to = ctx.state().selected_asset.and_then(|id| {
            visible(&st, &all)
                .iter()
                .position(|i| i.entry.id == id && !i.undeclared)
        });
    }

    let mut actions: Vec<Action> = Vec::new();
    toolbar(ui, &mut st, &all, &mut actions);
    ui.add_space(2.0);

    let selected = ctx.state().selected_asset;
    let shown = visible(&st, &all);
    let details_height =
        if selected.is_some_and(|id| all.iter().any(|i| i.entry.id == id && !i.undeclared)) {
            72.0
        } else {
            0.0
        };
    let list_height = (ui.available_height() - details_height).max(40.0);
    let waker = ctx.state().waker.clone();
    ui.allocate_ui(vec2(ui.available_width(), list_height), |ui| {
        if all.is_empty() {
            empty_state(ui, &mut actions);
        } else if shown.is_empty() {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(ui.t("library.no_match")).color(weak(ui)));
            });
        } else {
            match st.view {
                View::Grid => grid(ui, &mut st, &shown, selected, &root, &waker, &mut actions),
                View::List => list(ui, &mut st, &shown, selected, &root, &waker, &mut actions),
            }
        }
    });

    if let Some(id) = selected
        && let Some(item) = all.iter().find(|i| i.entry.id == id && !i.undeclared)
    {
        details(ui, ctx, &mut st, item, &root, &waker, &mut actions);
    }
    drop(shown);
    confirm_delete(ui, ctx, &mut st, &all, &mut actions);
    drop(st);

    apply(ctx, state, moves, &root, actions);
}

fn visible(st: &LibUi, all: &[Item]) -> Vec<Item> {
    let query = crate::fuzzy::fold_str(st.query.trim());
    let mut out: Vec<Item> = all
        .iter()
        .filter(|i| match st.filter {
            Filter::All => true,
            Filter::Kind(k) => i.entry.kind == k,
        })
        .filter(|i| {
            query.is_empty()
                || query
                    .split_whitespace()
                    .all(|w| crate::fuzzy::fold_str(&i.entry.path).contains(w))
        })
        .cloned()
        .collect();
    match st.sort {
        Sort::Name => out.sort_by_key(|i| i.file_name().to_lowercase()),
        Sort::Kind => out.sort_by(|a, b| {
            (a.entry.kind as u8, a.file_name().to_lowercase())
                .cmp(&(b.entry.kind as u8, b.file_name().to_lowercase()))
        }),
        Sort::Size => out.sort_by_key(|i| std::cmp::Reverse(i.size)),
    }
    // Files not yet declared come last, whatever the sort.
    out.sort_by_key(|i| i.undeclared);
    out
}

fn toolbar(ui: &mut Ui, st: &mut LibUi, all: &[Item], actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        if primary_button(
            ui,
            &format!(
                "{}  {}",
                icons::UPLOAD_SIMPLE,
                ui.t("library.toolbar.import")
            ),
        )
        .on_hover_text(ui.t("library.toolbar.import_tip"))
        .clicked()
        {
            actions.push(Action::Import);
        }
        ui.add_space(4.0);
        let count = |f: Filter| {
            all.iter()
                .filter(|i| match f {
                    Filter::All => true,
                    Filter::Kind(k) => i.entry.kind == k,
                })
                .count()
        };
        for (filter, label) in [
            (Filter::All, ui.t("library.filter.all")),
            (
                Filter::Kind(AssetKind::Image),
                format!("{} {}", icons::IMAGE, ui.t("library.filter.images")),
            ),
            (
                Filter::Kind(AssetKind::Music),
                format!("{} {}", icons::MUSIC_NOTES, ui.t("library.filter.music")),
            ),
            (
                Filter::Kind(AssetKind::Sound),
                format!("{} {}", icons::SPEAKER_HIGH, ui.t("library.filter.sounds")),
            ),
        ] {
            ui.selectable_value(
                &mut st.filter,
                filter,
                format!("{label}  {}", count(filter)),
            );
        }
        ui.add_space(6.0);
        ui.add(
            egui::TextEdit::singleline(&mut st.query)
                .hint_text(format!(
                    "{}  {}",
                    icons::MAGNIFYING_GLASS,
                    ui.t("library.filter.hint")
                ))
                .desired_width(140.0),
        );
        egui::ComboBox::from_id_salt("library_sort")
            .selected_text(match st.sort {
                Sort::Name => ui.t("library.sort.name"),
                Sort::Kind => ui.t("library.sort.kind"),
                Sort::Size => ui.t("library.sort.size"),
            })
            .width(70.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut st.sort, Sort::Name, ui.t("library.sort.name"));
                ui.selectable_value(&mut st.sort, Sort::Kind, ui.t("library.sort.kind"));
                ui.selectable_value(&mut st.sort, Sort::Size, ui.t("library.sort.size"));
            });
        ui.selectable_value(&mut st.view, View::Grid, icons::SQUARES_FOUR)
            .on_hover_text(ui.t("library.view.grid"));
        ui.selectable_value(&mut st.view, View::List, icons::LIST)
            .on_hover_text(ui.t("library.view.list"));
    });
}

fn empty_state(ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.add_space(18.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(icons::UPLOAD_SIMPLE)
                .size(30.0)
                .color(faint(ui)),
        );
        ui.add_space(4.0);
        ui.label(strong(ui.t("library.empty.title"), 14.0));
        ui.label(egui::RichText::new(ui.t("library.empty.hint")).color(weak(ui)));
        ui.add_space(8.0);
        if primary_button(ui, &ui.t("library.empty.button")).clicked() {
            actions.push(Action::Import);
        }
    });
}

fn fit(inner: egui::Vec2, outer: egui::Rect) -> egui::Rect {
    let scale = (outer.width() / inner.x).min(outer.height() / inner.y);
    egui::Rect::from_center_size(outer.center(), inner * scale)
}

/// Paints the preview of an asset into `rect`.
fn paint_thumb(
    ui: &Ui,
    rect: egui::Rect,
    item: &Item,
    state: &ThumbState,
    playing: bool,
    max_icon: f32,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), ui.visuals().extreme_bg_color);
    match state {
        ThumbState::Ready {
            texture: Some(t), ..
        } => {
            let dest = fit(t.size_vec2(), rect.shrink(1.0));
            painter.image(
                t.id(),
                dest,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        ThumbState::Ready { info, .. } => {
            painter.text(
                rect.center() - vec2(0.0, 6.0),
                Align2::CENTER_CENTER,
                if playing {
                    icons::STOP
                } else {
                    kind_icon(item.entry.kind)
                },
                FontId::proportional(max_icon),
                if playing { accent(ui) } else { weak(ui) },
            );
            painter.text(
                rect.center() + vec2(0.0, max_icon * 0.55),
                Align2::CENTER_CENTER,
                info,
                FontId::proportional(11.0),
                faint(ui),
            );
        }
        ThumbState::Loading => {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                kind_icon(item.entry.kind),
                FontId::proportional(max_icon * 0.8),
                faint(ui).gamma_multiply(0.6),
            );
        }
        ThumbState::Failed(_) => {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                icons::IMAGE_BROKEN,
                FontId::proportional(max_icon),
                danger(ui),
            );
        }
    }
}

/// Click, double click, drag, context menu of one item; `rect` is its area.
fn interact(
    ui: &mut Ui,
    rect: egui::Rect,
    id: Id,
    item: &Item,
    state: &ThumbState,
    actions: &mut Vec<Action>,
    audio_playing: bool,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click_and_drag());
    if !item.undeclared {
        if response.clicked() {
            actions.push(Action::Select(item.entry.id));
        }
        if response.drag_started() {
            egui::DragAndDrop::set_payload(
                ui.ctx(),
                AssetDrag {
                    id: item.entry.id,
                    kind: item.entry.kind,
                },
            );
            actions.push(Action::Select(item.entry.id));
        }
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    Order::Tooltip,
                    Id::new("orbit_asset_drag_ghost"),
                ));
                let text = format!("{}  {}", kind_icon(item.entry.kind), item.file_name());
                let galley =
                    painter.layout_no_wrap(text, FontId::proportional(13.0), Color32::WHITE);
                let box_rect = egui::Rect::from_min_size(
                    pos + vec2(12.0, 8.0),
                    galley.size() + vec2(16.0, 10.0),
                );
                painter.rect_filled(
                    box_rect,
                    CornerRadius::same(6),
                    accent(ui).gamma_multiply(0.9),
                );
                painter.galley(box_rect.min + vec2(8.0, 5.0), galley, Color32::WHITE);
            }
        }
    }
    let _ = state;
    let mut tip = item.entry.path.clone();
    if let ThumbState::Ready { info, .. } = state
        && !info.is_empty()
    {
        tip.push_str(&format!("\n{info}"));
    }
    if item.size > 0 {
        tip.push_str(&format!("  ·  {}", human_size(&ui.texts(), item.size)));
    }
    if item.undeclared {
        tip.push_str(&format!("\n{}", ui.t("library.tip.undeclared")));
    }
    if let Some((_, message)) = &item.issue {
        tip.push_str(&format!("\n{message}"));
    }
    let response = response.on_hover_text(tip);
    response.context_menu(|ui| {
        if item.undeclared {
            if ui
                .button(format!("{}  {}", icons::PLUS, ui.t("library.menu.declare")))
                .clicked()
            {
                actions.push(Action::Declare(item.entry.path.clone()));
                ui.close();
            }
            return;
        }
        if item.entry.kind != AssetKind::Image {
            let label = if audio_playing {
                format!("{}  {}", icons::STOP, ui.t("library.menu.stop"))
            } else {
                format!("{}  {}", icons::PLAY, ui.t("library.menu.play"))
            };
            if ui.button(label).clicked() {
                actions.push(if audio_playing {
                    Action::Stop
                } else {
                    Action::Play(item.entry.id)
                });
                ui.close();
            }
        }
        if ui
            .button(format!(
                "{}  {}",
                icons::PENCIL_SIMPLE,
                ui.t("common.rename")
            ))
            .clicked()
        {
            actions.push(Action::Select(item.entry.id));
            actions.push(Action::BeginRename(item.entry.id));
            ui.close();
        }
        if ui
            .button(format!("{}  {}", icons::TRASH, ui.t("library.menu.remove")))
            .clicked()
        {
            actions.push(Action::AskDelete(item.entry.id));
            ui.close();
        }
    });
    if response.double_clicked() && !item.undeclared && item.entry.kind != AssetKind::Image {
        actions.push(if audio_playing {
            Action::Stop
        } else {
            Action::Play(item.entry.id)
        });
    }
    response
}

fn grid(
    ui: &mut Ui,
    st: &mut LibUi,
    shown: &[Item],
    selected: Option<AssetId>,
    root: &Path,
    waker: &orbit_editor_api::Waker,
    actions: &mut Vec<Action>,
) {
    let width = ui.available_width();
    let cols = ((width / CELL.x).floor() as usize).max(1);
    let rows = shown.len().div_ceil(cols);
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(index) = st.scroll_to.take() {
        area = area.vertical_scroll_offset((index / cols) as f32 * CELL.y);
    }
    let playing = st.audio.playing;
    let progress = st.audio.progress();
    area.show_rows(ui, CELL.y, rows, |ui, range| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for r in range {
            let (row, _) = ui.allocate_exact_size(vec2(width, CELL.y), Sense::hover());
            for c in 0..cols {
                let Some(item) = shown.get(r * cols + c) else {
                    break;
                };
                let rect = egui::Rect::from_min_size(row.min + vec2(c as f32 * CELL.x, 0.0), CELL)
                    .shrink(3.0);
                let thumb = thumbs::state(ui.ctx(), waker, root, &item.entry);
                let is_playing = playing == Some(item.entry.id) && !item.undeclared;
                let id = ui.id().with(("asset_cell", r, c, &item.entry.path));
                let response = interact(ui, rect, id, item, &thumb, actions, is_playing);
                let is_selected = !item.undeclared && selected == Some(item.entry.id);
                let painter = ui.painter();
                if is_selected || response.hovered() {
                    painter.rect_filled(
                        rect,
                        CornerRadius::same(8),
                        if is_selected {
                            accent_soft(ui, 60)
                        } else {
                            ui.visuals().widgets.hovered.bg_fill
                        },
                    );
                }
                if is_selected {
                    painter.rect_stroke(
                        rect,
                        CornerRadius::same(8),
                        Stroke::new(1.5, accent(ui)),
                        egui::StrokeKind::Inside,
                    );
                }
                let thumb_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.center().x - THUMB_BOX.x / 2.0, rect.top() + 6.0),
                    THUMB_BOX,
                );
                paint_thumb(ui, thumb_rect, item, &thumb, is_playing, 30.0);
                if is_playing && let Some((at, len)) = progress {
                    let frac = if len.is_zero() {
                        0.0
                    } else {
                        at.as_secs_f32() / len.as_secs_f32()
                    };
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(thumb_rect.left(), thumb_rect.bottom() - 4.0),
                        vec2(thumb_rect.width() * frac, 3.0),
                    );
                    ui.painter()
                        .rect_filled(bar, CornerRadius::same(2), accent(ui));
                }
                badges(ui, thumb_rect, item);
                // Name and details.
                let clip = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 4.0, thumb_rect.bottom() + 4.0),
                    egui::pos2(rect.right() - 4.0, rect.bottom()),
                );
                let painter = ui.painter().with_clip_rect(clip);
                painter.text(
                    egui::pos2(rect.center().x, thumb_rect.bottom() + 14.0),
                    Align2::CENTER_CENTER,
                    item.file_name(),
                    FontId::proportional(12.0),
                    ui.visuals().text_color(),
                );
                let sub = match (&thumb, item.undeclared) {
                    (_, true) => ui.t("library.undeclared"),
                    (ThumbState::Ready { info, .. }, _) => info.clone(),
                    _ => kind_name(&ui.texts(), item.entry.kind),
                };
                painter.text(
                    egui::pos2(rect.center().x, thumb_rect.bottom() + 29.0),
                    Align2::CENTER_CENTER,
                    sub,
                    FontId::proportional(10.5),
                    if item.undeclared {
                        warning(ui)
                    } else {
                        faint(ui)
                    },
                );
            }
        }
    });
}

/// Corner badges: a problem with the file, a file not in the manifest.
fn badges(ui: &Ui, thumb: egui::Rect, item: &Item) {
    let painter = ui.painter();
    if let Some((level, _)) = &item.issue {
        let color = if *level == Level::Error {
            danger(ui)
        } else {
            warning(ui)
        };
        painter.text(
            thumb.right_top() + vec2(-8.0, 9.0),
            Align2::CENTER_CENTER,
            icons::WARNING_CIRCLE,
            FontId::proportional(15.0),
            color,
        );
    }
    if item.undeclared {
        painter.rect_stroke(
            thumb,
            CornerRadius::same(6),
            Stroke::new(1.5, warning(ui)),
            egui::StrokeKind::Inside,
        );
    }
}

fn list(
    ui: &mut Ui,
    st: &mut LibUi,
    shown: &[Item],
    selected: Option<AssetId>,
    root: &Path,
    waker: &orbit_editor_api::Waker,
    actions: &mut Vec<Action>,
) {
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(index) = st.scroll_to.take() {
        area = area.vertical_scroll_offset(index as f32 * LIST_ROW);
    }
    let playing = st.audio.playing;
    let texts = ui.texts();
    area.show_rows(ui, LIST_ROW, shown.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for index in range {
            let item = &shown[index];
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), LIST_ROW), Sense::hover());
            let thumb = thumbs::state(ui.ctx(), waker, root, &item.entry);
            let is_playing = playing == Some(item.entry.id) && !item.undeclared;
            let id = ui.id().with(("asset_row", index, &item.entry.path));
            let response = interact(ui, rect, id, item, &thumb, actions, is_playing);
            let is_selected = !item.undeclared && selected == Some(item.entry.id);
            if is_selected || response.hovered() {
                ui.painter().rect_filled(
                    rect.shrink2(vec2(2.0, 0.0)),
                    CornerRadius::same(5),
                    if is_selected {
                        accent_soft(ui, 70)
                    } else {
                        ui.visuals().widgets.hovered.bg_fill
                    },
                );
            }
            let thumb_rect = egui::Rect::from_min_size(
                rect.min + vec2(8.0, 3.0),
                vec2(LIST_ROW - 6.0, LIST_ROW - 6.0),
            );
            paint_thumb(ui, thumb_rect, item, &thumb, is_playing, 13.0);
            let mid = rect.center().y;
            let painter = ui.painter();
            painter.text(
                egui::pos2(thumb_rect.right() + 10.0, mid),
                Align2::LEFT_CENTER,
                &item.entry.path,
                FontId::proportional(13.0),
                ui.visuals().text_color(),
            );
            let info = match &thumb {
                ThumbState::Ready { info, .. } => info.clone(),
                _ => String::new(),
            };
            let right = rect.right() - 10.0;
            painter.text(
                egui::pos2(right, mid),
                Align2::RIGHT_CENTER,
                human_size(&texts, item.size),
                FontId::proportional(11.5),
                faint(ui),
            );
            painter.text(
                egui::pos2(right - 70.0, mid),
                Align2::RIGHT_CENTER,
                info,
                FontId::proportional(11.5),
                weak(ui),
            );
            painter.text(
                egui::pos2(right - 150.0, mid),
                Align2::RIGHT_CENTER,
                if item.undeclared {
                    texts.t("library.undeclared")
                } else {
                    kind_name(&texts, item.entry.kind)
                },
                FontId::proportional(11.5),
                if item.undeclared {
                    warning(ui)
                } else {
                    faint(ui)
                },
            );
            if let Some((level, _)) = &item.issue {
                painter.text(
                    egui::pos2(right - 215.0, mid),
                    Align2::RIGHT_CENTER,
                    icons::WARNING_CIRCLE,
                    FontId::proportional(14.0),
                    if *level == Level::Error {
                        danger(ui)
                    } else {
                        warning(ui)
                    },
                );
            }
        }
    });
}

/// The strip under the list: facts about the selected asset, rename, remove,
/// listen, where it is used.
fn details(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    st: &mut LibUi,
    item: &Item,
    root: &Path,
    waker: &orbit_editor_api::Waker,
    actions: &mut Vec<Action>,
) {
    let Some(story) = ctx.story() else { return };
    let thumb = thumbs::state(ui.ctx(), waker, root, &item.entry);
    ui.separator();
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(64.0, 52.0), Sense::hover());
        paint_thumb(
            ui,
            rect,
            item,
            &thumb,
            st.audio.playing == Some(item.entry.id),
            22.0,
        );
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if let Some((id, text)) = st.rename.as_mut().filter(|(id, _)| *id == item.entry.id)
                {
                    let (dir, ext) = split_path(&item.entry.path);
                    ui.label(egui::RichText::new(format!("{dir}/")).color(faint(ui)));
                    let edit = ui.add(egui::TextEdit::singleline(text).desired_width(180.0));
                    edit.request_focus();
                    ui.label(egui::RichText::new(format!(".{ext}")).color(faint(ui)));
                    let id = *id;
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        actions.push(Action::CommitRename(id, text.clone()));
                    } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) || edit.lost_focus() {
                        st.rename = None;
                    }
                } else {
                    ui.label(strong(item.file_name(), 14.0))
                        .on_hover_text(&item.entry.path);
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  ·  {}  ·  {}  ·  {}",
                            kind_name(&ui.texts(), item.entry.kind),
                            match &thumb {
                                ThumbState::Ready { info, .. } => info.as_str(),
                                _ => "",
                            },
                            human_size(&ui.texts(), item.size),
                            item.entry.path
                        ))
                        .color(weak(ui)),
                    );
                }
            });
            ui.horizontal(|ui| {
                if item.entry.kind != AssetKind::Image {
                    let playing = st.audio.playing == Some(item.entry.id);
                    let loading = st
                        .audio
                        .loading
                        .as_ref()
                        .is_some_and(|(id, _)| *id == item.entry.id);
                    let label = if playing {
                        format!("{}  {}", icons::STOP, ui.t("library.menu.stop"))
                    } else if loading {
                        format!("{}  {}", icons::PLAY, ui.t("library.menu.loading"))
                    } else {
                        format!("{}  {}", icons::PLAY, ui.t("library.menu.play"))
                    };
                    if ui.small_button(label).clicked() {
                        actions.push(if playing {
                            Action::Stop
                        } else {
                            Action::Play(item.entry.id)
                        });
                    }
                }
                if icon_button(
                    ui,
                    icons::PENCIL_SIMPLE,
                    &ui.t("library.details.rename_tip"),
                )
                .clicked()
                {
                    actions.push(Action::BeginRename(item.entry.id));
                }
                if icon_button(ui, icons::TRASH, &ui.t("library.menu.remove")).clicked() {
                    actions.push(Action::AskDelete(item.entry.id));
                }
                let users = asset_users(story, item.entry.id);
                if users.is_empty() {
                    ui.label(
                        egui::RichText::new(ui.t("library.details.unused"))
                            .small()
                            .color(faint(ui)),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(ui.tn("library.details.used", users.len(), &[]))
                            .small()
                            .color(weak(ui)),
                    );
                    for user in users.iter().take(3) {
                        let (label, location) = match user {
                            AssetUser::Node(n) => (
                                reveal::describe(ctx, &Location::Node(*n)),
                                Location::Node(*n),
                            ),
                            AssetUser::Character(c) => (
                                reveal::describe(ctx, &Location::Character(*c)),
                                Location::Character(*c),
                            ),
                        };
                        if ui.small_button(label).clicked() {
                            actions.push(Action::ShowUser(location));
                        }
                    }
                }
                if let Some((level, message)) = &item.issue {
                    ui.label(egui::RichText::new(message).small().color(
                        if *level == Level::Error {
                            danger(ui)
                        } else {
                            warning(ui)
                        },
                    ));
                }
            });
        });
    });
}

fn split_path(path: &str) -> (&str, &str) {
    let (dir, file) = path.rsplit_once('/').unwrap_or(("", path));
    let ext = file.rsplit_once('.').map_or("", |(_, e)| e);
    (dir, ext)
}

fn confirm_delete(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    st: &mut LibUi,
    all: &[Item],
    actions: &mut Vec<Action>,
) {
    let Some(id) = st.confirm_delete else { return };
    let Some(item) = all.iter().find(|i| i.entry.id == id) else {
        st.confirm_delete = None;
        return;
    };
    let name = item.file_name().to_owned();
    let uses = ctx.story().map_or(0, |s| asset_users(s, id).len());
    let modal = egui::Modal::new(Id::new("orbit_asset_delete")).show(ui.ctx(), |ui| {
        ui.set_width(380.0);
        ui.label(strong(ui.t("library.delete.title"), 15.0));
        ui.add_space(6.0);
        if uses > 0 {
            ui.label(
                egui::RichText::new(ui.tn("library.delete.in_use", uses, &[("name", &name)]))
                    .color(danger(ui)),
            );
        } else {
            ui.label(ui.tf("library.delete.confirm", &[("name", &name)]));
        }
        ui.add_space(10.0);
        let mut close = false;
        ui.horizontal(|ui| {
            if uses == 0 && primary_button(ui, &ui.t("common.remove")).clicked() {
                actions.push(Action::Delete(id));
                close = true;
            }
            let dismiss = ui.t(if uses == 0 {
                "common.cancel"
            } else {
                "common.close"
            });
            if ui.button(dismiss).clicked() {
                close = true;
            }
        });
        close
    });
    if modal.inner || modal.should_close() {
        st.confirm_delete = None;
    }
}

// --------------------------------------------------------------------- apply

fn apply(
    ctx: &mut EditorContext<'_>,
    state: &Rc<RefCell<LibUi>>,
    moves: &Rc<RefCell<MoveJournal>>,
    root: &Path,
    actions: Vec<Action>,
) {
    for action in actions {
        match action {
            Action::Select(id) => ctx.state_mut().selected_asset = Some(id),
            Action::Import => import_dialog(ctx),
            Action::Declare(path) => {
                import_paths(ctx, &[root.join(path)]);
            }
            Action::Play(id) => {
                let Some(entry) = ctx.story().and_then(|s| s.assets.get(id)).cloned() else {
                    continue;
                };
                state
                    .borrow_mut()
                    .audio
                    .play(id, root.join(&entry.path), entry.path.clone());
            }
            Action::Stop => state.borrow_mut().audio.stop(),
            Action::BeginRename(id) => {
                let stem = ctx
                    .story()
                    .and_then(|s| s.assets.get(id))
                    .map(|a| {
                        let file = a.path.rsplit('/').next().unwrap_or(&a.path);
                        file.rsplit_once('.').map_or(file, |(s, _)| s).to_owned()
                    })
                    .unwrap_or_default();
                state.borrow_mut().rename = Some((id, stem));
            }
            Action::CommitRename(id, stem) => {
                state.borrow_mut().rename = None;
                rename(ctx, moves, root, id, &stem);
            }
            Action::AskDelete(id) => state.borrow_mut().confirm_delete = Some(id),
            Action::Delete(id) => {
                state.borrow_mut().audio.stop();
                if ctx.ops_or_toast(|p| ops::remove_asset(p, id)).is_some() {
                    if ctx.state().selected_asset == Some(id) {
                        ctx.state_mut().selected_asset = None;
                    }
                    ctx.state_mut().asset_epoch += 1;
                    let message = ctx.t("library.removed");
                    ctx.log(Level::Info, message);
                }
            }
            Action::ShowUser(location) => reveal::reveal(ctx, location),
        }
    }
}

/// Renames the file of an asset (same folder and extension) and its manifest
/// path; the id, and so every reference, is unchanged.
fn rename(
    ctx: &mut EditorContext<'_>,
    moves: &Rc<RefCell<MoveJournal>>,
    root: &Path,
    id: AssetId,
    stem: &str,
) {
    let Some(entry) = ctx.story().and_then(|s| s.assets.get(id)).cloned() else {
        return;
    };
    let stem = stem.trim();
    if stem.is_empty() {
        let message = ctx.t("library.name_empty");
        ctx.error(message);
        return;
    }
    let (dir, ext) = split_path(&entry.path);
    let file = assets::sanitize_file_name(&format!("{stem}.{ext}"));
    let new_path = if dir.is_empty() {
        file
    } else {
        format!("{dir}/{file}")
    };
    let mut journal = moves.borrow_mut();
    let texts = ctx.translator();
    if ctx
        .ops_or_toast(|p| assets::move_asset(p, root, &mut journal, id, &new_path, &texts))
        .is_some()
    {
        ctx.state_mut().asset_epoch += 1;
        let message = ctx.tf("library.renamed", &[("path", &new_path)]);
        ctx.log(Level::Info, message);
    }
}
