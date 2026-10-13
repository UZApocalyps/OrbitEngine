//! "AI Studio" panel: the full AI game-creation wizard (`orbit_ai_studio`).
//!
//! Pitch -> bible -> script -> pictures -> implementation, each step
//! reviewable, editable and re-runnable. Network work runs on the task pool
//! (progress, cancel in the status bar); every step saves the pipeline state
//! in the project's `.orbit/ai/studio.json`, so a cancelled or interrupted
//! run resumes where it stopped. Provider settings (and keys) live in the
//! user config folder, never in the project.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use orbit_editor_sdk::egui::{self, ComboBox, DragValue, RichText, ScrollArea, TextEdit, Ui};
use orbit_ai_studio::assets::{AssetKindAi, AssetStatus};
use orbit_ai_studio::civitai::{self, DownloadOutcome, ModelInfo, ModelType};
use orbit_ai_studio::pipeline::{self, PipelineState, ScriptStatus};
use orbit_ai_studio::scan::{self, Choice};
use orbit_ai_studio::sd::{self, LoraPick, ModelLists, SdPreset};
use orbit_ai_studio::{
    ImageBackend, ImplementOptions, OpenAiChat, StudioSettings, image_gen, implement,
};
use orbit_editor_sdk::native::PluginHost;
use orbit_editor_sdk::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, PanelDef,
    Plugin, PluginManifest, ProjectRegistry, SettingsDef, TaskError, TaskHandle, icons,
};

use crate::ui_util::{card, primary_button, secondary_button, section_title, weak};

pub const PLUGIN_ID: &str = "orbit.ai_studio";
/// The editor's preview panel (full id).
const PREVIEW_PANEL: &str = "orbit.preview.preview";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Settings,
    Models,
    Pitch,
    Bible,
    Script,
    Assets,
    Implement,
}

/// Selectable models: disk and backend merged.
#[derive(Default)]
struct Choices {
    checkpoints: Vec<Choice>,
    loras: Vec<Choice>,
    vaes: Vec<Choice>,
    embeddings: Vec<Choice>,
}

/// Strings of the searchable drop-downs.
#[derive(Clone)]
struct ComboText {
    none: String,
    search: String,
    use_custom: String,
    missing: String,
    disk: String,
    backend: String,
}

impl ComboText {
    fn new(ctx: &EditorContext<'_>, none_key: &str) -> Self {
        Self {
            none: ctx.t(none_key),
            search: ctx.t("aistudio.search"),
            use_custom: ctx.t("aistudio.use_custom"),
            missing: ctx.t("aistudio.not_in_list"),
            disk: ctx.t("aistudio.src_disk"),
            backend: ctx.t("aistudio.src_backend"),
        }
    }
}

/// Searchable drop-down: filter box, an empty entry, a custom value kept
/// even when absent from `items` (flagged with a warning). Returns whether
/// `value` changed.
fn search_combo(ui: &mut Ui, id: &str, value: &mut String, items: &[Choice], t: &ComboText) -> bool {
    let mut changed = false;
    let missing = !value.is_empty() && !items.is_empty() && !items.iter().any(|c| c.name == *value);
    let shown = if value.is_empty() {
        t.none.clone()
    } else if missing {
        format!("{} {value}", icons::WARNING)
    } else {
        value.clone()
    };
    let fid = ui.make_persistent_id((id, "filter"));
    let resp = ComboBox::from_id_salt(id)
        .width(300.0)
        .selected_text(shown)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            let mut filter: String = ui.data_mut(|d| d.get_temp(fid)).unwrap_or_default();
            ui.add(TextEdit::singleline(&mut filter).hint_text(&t.search).desired_width(f32::INFINITY))
                .request_focus();
            let needle = filter.trim().to_lowercase();
            let mut pick: Option<String> = None;
            ScrollArea::vertical().max_height(300.0).id_salt((id, "list")).show(ui, |ui| {
                if needle.is_empty() && ui.selectable_label(value.is_empty(), &t.none).clicked() {
                    pick = Some(String::new());
                }
                let mut exact = false;
                for c in items.iter().filter(|c| needle.is_empty() || c.name.to_lowercase().contains(&needle)) {
                    exact |= c.name.eq_ignore_ascii_case(filter.trim());
                    let tag = match (c.on_disk, c.on_backend) {
                        (true, false) => format!("  [{}]", t.disk),
                        (false, true) => format!("  [{}]", t.backend),
                        _ => String::new(),
                    };
                    if ui.selectable_label(*value == c.name, format!("{}{tag}", c.name)).clicked() {
                        pick = Some(c.name.clone());
                    }
                }
                if !needle.is_empty()
                    && !exact
                    && ui.button(format!("{} “{}”", t.use_custom, filter.trim())).clicked()
                {
                    pick = Some(filter.trim().to_owned());
                }
            });
            if missing {
                ui.label(RichText::new(format!("{} {}", icons::WARNING, t.missing)).color(ui.visuals().warn_fg_color));
            }
            if let Some(p) = pick {
                *value = p;
                changed = true;
                filter.clear();
                ui.close();
            }
            ui.data_mut(|d| d.insert_temp(fid, filter));
        });
    if missing {
        resp.response.on_hover_text(&t.missing);
    }
    changed
}

/// What a background step gives back: the new state and how it ended.
type StepOut = (PipelineState, Result<(), String>);

/// Civitai search results and their previews (version id, size, RGBA).
type SearchOut = (Vec<ModelInfo>, Vec<(u64, [usize; 2], Vec<u8>)>);

#[derive(Default)]
struct CivitaiUi {
    query: String,
    kind: Option<ModelType>,
    results: Vec<ModelInfo>,
    /// Preview pictures, by version id: (size, RGBA).
    previews: HashMap<u64, egui::TextureHandle>,
    search: Option<TaskHandle<SearchOut>>,
    download: Option<TaskHandle<String>>,
    message: String,
}

struct StudioUi {
    config_dir: Option<PathBuf>,
    settings: StudioSettings,
    loaded_root: Option<PathBuf>,
    state: PipelineState,
    tab: Tab,
    step: Option<TaskHandle<StepOut>>,
    message: Option<(bool, String)>,
    bible_edit: String,
    bible_errors: Vec<String>,
    scene: Option<String>,
    scene_edit: String,
    text_models: Option<TaskHandle<Vec<String>>>,
    text_model_list: Vec<String>,
    models: Option<TaskHandle<ModelLists>>,
    /// Lists read from the backend.
    model_lists: ModelLists,
    /// Disk scan of the model folders.
    scan: Option<TaskHandle<ModelLists>>,
    disk_lists: ModelLists,
    /// A scan was done for the current folders / backend (reset to rescan).
    scanned: bool,
    /// Disk + backend, merged (rebuilt when either changes).
    choices: Choices,
    detect_msg: String,
    civitai: CivitaiUi,
    replace_scenes: Option<bool>,
}

impl StudioUi {
    fn new(config_dir: Option<PathBuf>) -> Self {
        let settings = config_dir.as_deref().map(StudioSettings::load).unwrap_or_default();
        Self {
            config_dir,
            settings,
            loaded_root: None,
            state: PipelineState::default(),
            tab: Tab::Pitch,
            step: None,
            message: None,
            bible_edit: String::new(),
            bible_errors: Vec::new(),
            scene: None,
            scene_edit: String::new(),
            text_models: None,
            text_model_list: Vec::new(),
            models: None,
            model_lists: ModelLists::default(),
            scan: None,
            disk_lists: ModelLists::default(),
            scanned: false,
            choices: Choices::default(),
            detect_msg: String::new(),
            civitai: CivitaiUi::default(),
            replace_scenes: None,
        }
    }

    fn rebuild_choices(&mut self) {
        let (d, b) = (&self.disk_lists, &self.model_lists);
        self.choices = Choices {
            checkpoints: scan::merge(&d.checkpoints, &b.checkpoints),
            loras: scan::merge(&d.loras, &b.loras),
            vaes: scan::merge(&d.vaes, &b.vaes),
            embeddings: scan::merge(&d.embeddings, &b.embeddings),
        };
    }

    /// Starts a scan of the model folders (no-op while one runs).
    fn start_scan(&mut self, ctx: &mut EditorContext<'_>) {
        if self.scan.is_some() {
            return;
        }
        self.scanned = true;
        let folders = self.settings.civitai.folders.clone();
        let backend = self.settings.image.backend;
        self.scan = Some(ctx.state().spawn_task(ctx.t("aistudio.rescan"), None, move |_| {
            Ok(scan::scan_folders(&folders, backend))
        }));
    }

    fn save_settings(&self) {
        if let Some(dir) = &self.config_dir {
            let _ = self.settings.save(dir);
        }
    }

    fn save_state(&mut self) {
        if let Some(root) = &self.loaded_root
            && let Err(e) = self.state.save(root)
        {
            self.message = Some((false, e.to_string()));
        }
    }

    fn reload(&mut self, root: &Path) {
        self.state = PipelineState::load(root);
        self.bible_edit = self.state.bible_text.clone();
        self.scene = None;
        self.scene_edit.clear();
    }
}

pub struct AiStudioPlugin {
    ui: Rc<RefCell<StudioUi>>,
}

impl AiStudioPlugin {
    pub fn new(config_dir: Option<PathBuf>) -> Self {
        Self { ui: Rc::new(RefCell::new(StudioUi::new(config_dir))) }
    }

    /// The provider settings as the plugin's settings page (plugin manager).
    pub fn register_settings(&self, host: &mut PluginHost<'_>) {
        let ui = self.ui.clone();
        host.settings_page(SettingsDef::new("aistudio.title", move |u, c| {
            let mut s = ui.borrow_mut();
            poll(&mut s, c);
            settings_tab(u, c, &mut s);
        }));
    }
}

impl Plugin for AiStudioPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(PLUGIN_ID, "AI Studio", "0.1.0")
    }

    fn register(&self, registry: &mut ProjectRegistry) {
        // The studio's MCP tools (DSL guide, bible / scene checks).
        orbit_ai_studio::mcp::AiStudioTools.register(registry);
    }
}

impl EditorPlugin for AiStudioPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        let ui = self.ui.clone();
        reg.panel(PanelDef::new("studio", "aistudio.title", icons::MAGIC_WAND, DockSlot::Hidden, move |u, c| {
            panel(u, c, &mut ui.borrow_mut());
        }));
        reg.action(
            ActionDef::new("open", "aistudio.open", |c| {
                c.request(EditorRequest::ShowPanel(format!("{PLUGIN_ID}.studio")));
            })
            .icon(icons::MAGIC_WAND)
            .category("category.tools")
            .menu("tools", 210),
        );
    }
}

// ------------------------------------------------------------------ polling

fn poll(s: &mut StudioUi, ctx: &mut EditorContext<'_>) {
    if let Some(h) = &mut s.step
        && let Some(r) = h.poll()
    {
        s.step = None;
        match r {
            Ok((state, outcome)) => {
                s.state = state;
                s.bible_edit = s.state.bible_text.clone();
                if let Some(id) = s.scene.clone() {
                    s.scene_edit = s.state.scripts.get(&id).map(|x| x.text.clone()).unwrap_or_default();
                }
                s.message = Some(match outcome {
                    Ok(()) => (true, ctx.t("aistudio.step_done")),
                    Err(e) => (false, e),
                });
            }
            Err(TaskError::Cancelled) => {
                // Checkpoints were saved: pick up what was done.
                if let Some(root) = s.loaded_root.clone() {
                    s.reload(&root);
                }
                s.message = Some((false, ctx.t("aistudio.cancelled")));
            }
            Err(e) => s.message = Some((false, format!("{e:?}"))),
        }
    }
    if let Some(h) = &mut s.text_models
        && let Some(r) = h.poll()
    {
        s.text_models = None;
        match r {
            Ok(list) => {
                s.message = Some((true, ctx.tf("aistudio.models_found", &[("n", &list.len())])));
                s.text_model_list = list;
            }
            Err(TaskError::Failed(e)) => s.message = Some((false, e)),
            Err(e) => s.message = Some((false, format!("{e:?}"))),
        }
    }
    if let Some(h) = &mut s.models
        && let Some(r) = h.poll()
    {
        s.models = None;
        match r {
            Ok(lists) => {
                s.model_lists = lists;
                s.rebuild_choices();
            }
            Err(TaskError::Failed(e)) => s.message = Some((false, e)),
            Err(e) => s.message = Some((false, format!("{e:?}"))),
        }
    }
    if let Some(h) = &mut s.scan
        && let Some(r) = h.poll()
    {
        s.scan = None;
        if let Ok(lists) = r {
            s.disk_lists = lists;
            s.rebuild_choices();
        }
    }
}

/// Returns true when a download just ended (the folders changed).
fn poll_civitai(c: &mut CivitaiUi, ui: &Ui) -> bool {
    let mut downloaded = false;
    if let Some(h) = &mut c.search
        && let Some(r) = h.poll()
    {
        c.search = None;
        match r {
            Ok((results, previews)) => {
                c.message = format!("{} results", results.len());
                c.results = results;
                c.previews.clear();
                for (id, size, rgba) in previews {
                    let img = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                    let tex = ui.ctx().load_texture(format!("civitai_{id}"), img, egui::TextureOptions::LINEAR);
                    c.previews.insert(id, tex);
                }
            }
            Err(TaskError::Failed(e)) => c.message = e,
            Err(e) => c.message = format!("{e:?}"),
        }
    }
    if let Some(h) = &mut c.download
        && let Some(r) = h.poll()
    {
        c.download = None;
        downloaded = true;
        c.message = match r {
            Ok(m) | Err(TaskError::Failed(m)) => m,
            Err(e) => format!("{e:?}"),
        };
    }
    downloaded
}

// -------------------------------------------------------------------- panel

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, s: &mut StudioUi) {
    let Some(root) = ctx.project_info().map(|p| p.root.clone()) else {
        ui.label(RichText::new(ctx.t("aistudio.no_project")).color(weak(ui)));
        return;
    };
    if s.loaded_root.as_deref() != Some(root.as_path()) {
        s.reload(&root);
        s.loaded_root = Some(root.clone());
    }
    poll(s, ctx);
    if poll_civitai(&mut s.civitai, ui) {
        s.scanned = false;
    }
    // Scan the model folders when a tab that lists them opens (or after a
    // download), once the folders are set.
    if !s.scanned && matches!(s.tab, Tab::Models | Tab::Assets) {
        let f = &s.settings.civitai.folders;
        if [&f.checkpoints, &f.loras, &f.vaes, &f.embeddings].iter().any(|p| !p.trim().is_empty()) {
            s.start_scan(ctx);
        }
    }

    ui.horizontal_wrapped(|ui| {
        let tabs = [
            (Tab::Settings, "aistudio.tab.settings"),
            (Tab::Models, "aistudio.tab.models"),
            (Tab::Pitch, "aistudio.tab.pitch"),
            (Tab::Bible, "aistudio.tab.bible"),
            (Tab::Script, "aistudio.tab.script"),
            (Tab::Assets, "aistudio.tab.assets"),
            (Tab::Implement, "aistudio.tab.implement"),
        ];
        for (tab, key) in tabs {
            ui.selectable_value(&mut s.tab, tab, ctx.t(key));
        }
    });
    if let Some(h) = &s.step {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(h.label());
            if let Some(f) = h.fraction() {
                ui.add(egui::ProgressBar::new(f).desired_width(160.0));
            }
            if ui.button(ctx.t("aistudio.cancel")).clicked() {
                h.cancel();
            }
        });
    }
    if let Some((ok, m)) = &s.message {
        let color = if *ok { ui.visuals().text_color() } else { ui.visuals().error_fg_color };
        ui.label(RichText::new(m).color(color));
    }
    ui.separator();
    let busy = s.step.is_some();
    ScrollArea::vertical().id_salt("ai_studio_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_enabled_ui(!busy || matches!(s.tab, Tab::Settings | Tab::Models), |ui| match s.tab {
            Tab::Settings => settings_tab(ui, ctx, s),
            Tab::Models => models_tab(ui, ctx, s),
            Tab::Pitch => pitch_tab(ui, ctx, s, &root),
            Tab::Bible => bible_tab(ui, ctx, s, &root),
            Tab::Script => script_tab(ui, ctx, s, &root),
            Tab::Assets => assets_tab(ui, ctx, s, &root),
            Tab::Implement => implement_tab(ui, ctx, s, &root),
        });
    });
}

fn field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(TextEdit::singleline(value).desired_width(f32::INFINITY)).changed()
    })
    .inner
}

fn secret(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(TextEdit::singleline(value).password(true).desired_width(f32::INFINITY)).changed()
    })
    .inner
}

fn settings_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, s: &mut StudioUi) {
    let mut changed = false;
    ui.label(RichText::new(ctx.t("aistudio.keys_note")).color(weak(ui)));
    card(ui, |ui| {
        section_title(ui, &ctx.t("aistudio.text_provider"));
        let t = &mut s.settings.text;
        changed |= field(ui, &ctx.t("aistudio.base_url"), &mut t.base_url);
        ui.horizontal(|ui| {
            ui.label(ctx.t("aistudio.model"));
            changed |= ui.add(TextEdit::singleline(&mut t.model).desired_width(220.0)).changed();
            if !s.text_model_list.is_empty() {
                ComboBox::from_id_salt("ai_text_models").selected_text("…").show_ui(ui, |ui| {
                    for m in &s.text_model_list {
                        if ui.selectable_label(t.model == *m, m).clicked() {
                            t.model = m.clone();
                            changed = true;
                        }
                    }
                });
            }
        });
        changed |= secret(ui, &ctx.t("aistudio.api_key"), &mut t.api_key);
        ui.horizontal(|ui| {
            ui.label(ctx.t("aistudio.temperature"));
            changed |= ui.add(DragValue::new(&mut t.temperature).range(0.0..=2.0).speed(0.05)).changed();
            ui.label(ctx.t("aistudio.max_tokens"));
            changed |= ui.add(DragValue::new(&mut t.max_tokens).range(256..=200_000)).changed();
            ui.label(ctx.t("aistudio.repairs"));
            changed |= ui.add(DragValue::new(&mut t.max_repairs).range(0..=10)).changed();
        });
        if secondary_button(ui, &ctx.t("aistudio.test_text")).clicked() && s.text_models.is_none() {
            let settings = t.clone();
            s.text_models = Some(ctx.state().spawn_task(ctx.t("aistudio.test_text"), None, move |_| {
                OpenAiChat::new(settings).list_models().map_err(|e| TaskError::Failed(e.to_string()))
            }));
        }
    });
    ui.add_space(8.0);
    card(ui, |ui| {
        section_title(ui, &ctx.t("aistudio.image_provider"));
        let i = &mut s.settings.image;
        ComboBox::from_label(ctx.t("aistudio.backend")).selected_text(i.backend.name()).show_ui(ui, |ui| {
            for b in ImageBackend::ALL {
                if ui.selectable_value(&mut i.backend, b, b.name()).changed() {
                    changed = true;
                    s.scanned = false;
                }
            }
        });
        if i.backend != ImageBackend::Placeholder {
            changed |= field(ui, &ctx.t("aistudio.base_url"), &mut i.base_url);
        }
        if i.backend == ImageBackend::OpenAi {
            changed |= field(ui, &ctx.t("aistudio.model"), &mut i.model);
            changed |= secret(ui, &ctx.t("aistudio.api_key"), &mut i.api_key);
        }
        changed |= field(ui, &ctx.t("aistudio.style"), &mut i.style_prefix);
        changed |= field(ui, &ctx.t("aistudio.negative"), &mut i.negative_prompt);
        ui.horizontal(|ui| {
            ui.label(ctx.t("aistudio.bg_size"));
            changed |= ui.add(DragValue::new(&mut i.background_size.width).range(64..=4096)).changed();
            changed |= ui.add(DragValue::new(&mut i.background_size.height).range(64..=4096)).changed();
            ui.label(ctx.t("aistudio.sprite_size"));
            changed |= ui.add(DragValue::new(&mut i.sprite_size.width).range(64..=4096)).changed();
            changed |= ui.add(DragValue::new(&mut i.sprite_size.height).range(64..=4096)).changed();
        });
        ui.horizontal(|ui| {
            ui.label(ctx.t("aistudio.seed"));
            changed |= ui.add(DragValue::new(&mut i.seed)).changed();
            ui.label(ctx.t("aistudio.concurrency"));
            changed |= ui.add(DragValue::new(&mut i.concurrency).range(1..=8)).changed();
        });
        changed |= ui.checkbox(&mut i.remove_sprite_background, ctx.t("aistudio.remove_bg")).changed();
        if i.backend == ImageBackend::ComfyUi {
            ui.collapsing(ctx.t("aistudio.workflow"), |ui| {
                ui.label(RichText::new(ctx.t("aistudio.workflow_hint")).color(weak(ui)));
                changed |= ui.add(TextEdit::multiline(&mut i.comfy_workflow).code_editor().desired_rows(10).desired_width(f32::INFINITY)).changed();
                if ui.button(ctx.t("aistudio.workflow_reset")).clicked() {
                    i.comfy_workflow = image_gen::DEFAULT_COMFY_WORKFLOW.to_owned();
                    changed = true;
                }
            });
        }
    });
    if changed {
        s.save_settings();
    }
}

fn pick_list(ui: &mut Ui, id: &str, label: &str, value: &mut String, list: &[Choice], t: &ComboText) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        search_combo(ui, id, value, list, t)
    })
    .inner
}

fn names_as_choices(list: &[String]) -> Vec<Choice> {
    list.iter().map(|n| Choice { name: n.clone(), on_disk: false, on_backend: false }).collect()
}

fn lora_row(ui: &mut Ui, ctx: &EditorContext<'_>, l: &mut LoraPick, list: &[Choice], t: &ComboText, id: &str) -> (bool, bool) {
    let mut changed = false;
    let mut remove = false;
    ui.horizontal(|ui| {
        changed |= search_combo(ui, id, &mut l.name, list, t);
        changed |= ui.add(DragValue::new(&mut l.weight).range(-2.0..=2.0).speed(0.05)).changed();
        let mut words = l.trigger_words.join(", ");
        if ui.add(TextEdit::singleline(&mut words).desired_width(180.0).hint_text(ctx.t("aistudio.trigger_words"))).changed() {
            l.trigger_words = words.split(',').map(|w| w.trim().to_owned()).filter(|w| !w.is_empty()).collect();
            changed = true;
        }
        remove = ui.button(icons::TRASH).clicked();
    });
    (changed, remove)
}

fn models_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, s: &mut StudioUi) {
    let mut changed = false;
    let backend = s.settings.image.backend;
    let sd_backend = matches!(backend, ImageBackend::ComfyUi | ImageBackend::Automatic1111);
    card(ui, |ui| {
        section_title(ui, &ctx.t("aistudio.sd_models"));
        if !sd_backend {
            ui.label(RichText::new(ctx.t("aistudio.sd_only")).color(weak(ui)));
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(sd_backend && s.models.is_none(), egui::Button::new(ctx.t("aistudio.refresh_models"))).clicked() {
                let settings = s.settings.image.clone();
                s.models = Some(ctx.state().spawn_task(ctx.t("aistudio.refresh_models"), None, move |_| {
                    sd::list_models(&settings).map_err(|e| TaskError::Failed(e.to_string()))
                }));
            }
            if s.models.is_some() {
                ui.spinner();
            }
            let any_folder = {
                let f = &s.settings.civitai.folders;
                [&f.checkpoints, &f.loras, &f.vaes, &f.embeddings].iter().any(|p| !p.trim().is_empty())
            };
            if ui.add_enabled(any_folder && s.scan.is_none(), egui::Button::new(ctx.t("aistudio.rescan"))).clicked() {
                s.start_scan(ctx);
            }
            if s.scan.is_some() {
                ui.spinner();
            }
            let l = &s.choices;
            ui.label(RichText::new(format!(
                "{} checkpoints · {} LoRAs · {} VAEs · {} embeddings",
                l.checkpoints.len(), l.loras.len(), l.vaes.len(), l.embeddings.len()
            )).color(weak(ui)));
        });
        let i = &mut s.settings.image;
        let lists = &s.model_lists;
        let samplers: Vec<Choice> = names_as_choices(&scan::samplers(backend, &lists.samplers));
        let schedulers: Vec<Choice> = names_as_choices(&scan::schedulers(backend, &lists.schedulers));
        let default_t = ComboText::new(ctx, "aistudio.default_value");
        let none_t = ComboText::new(ctx, "aistudio.none_value");
        let lora_t = ComboText::new(ctx, "aistudio.add_lora");
        let emb_t = ComboText::new(ctx, "aistudio.insert_embedding");
        ComboBox::from_label(ctx.t("aistudio.preset")).selected_text(i.preset.name()).show_ui(ui, |ui| {
            for p in SdPreset::ALL {
                if ui.selectable_label(i.preset == p, p.name()).clicked() {
                    sd::apply_preset(i, p);
                    changed = true;
                }
            }
        });
        let ch = &s.choices;
        changed |= pick_list(ui, "ai_ckpt", &ctx.t("aistudio.checkpoint"), &mut i.model, &ch.checkpoints, &default_t);
        changed |= pick_list(ui, "ai_vae", "VAE", &mut i.vae, &ch.vaes, &default_t);
        changed |= pick_list(ui, "ai_sampler", &ctx.t("aistudio.sampler"), &mut i.sampler, &samplers, &default_t);
        changed |= pick_list(ui, "ai_sched", &ctx.t("aistudio.scheduler"), &mut i.scheduler, &schedulers, &none_t);
        ui.horizontal(|ui| {
            ui.label(ctx.t("aistudio.steps"));
            changed |= ui.add(DragValue::new(&mut i.steps).range(1..=150)).changed();
            ui.label("CFG");
            changed |= ui.add(DragValue::new(&mut i.cfg_scale).range(1.0..=30.0).speed(0.1)).changed();
            ui.label(ctx.t("aistudio.clip_skip"));
            changed |= ui.add(DragValue::new(&mut i.clip_skip).range(1..=12)).changed();
        });
        changed |= field(ui, &ctx.t("aistudio.quality_tags"), &mut i.quality_tags);
        if !ch.embeddings.is_empty() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(ctx.t("aistudio.embeddings")).color(weak(ui)));
                for (id, target_negative) in [("ai_emb_neg", true), ("ai_emb_q", false)] {
                    let mut picked = String::new();
                    let t = ComboText { none: format!("{} ({})", emb_t.none, ctx.t(if target_negative { "aistudio.negative" } else { "aistudio.quality_tags" })), ..emb_t.clone() };
                    if search_combo(ui, id, &mut picked, &ch.embeddings, &t) && !picked.is_empty() {
                        let tag = if backend == ImageBackend::ComfyUi { format!("embedding:{picked}") } else { picked };
                        let target = if target_negative { &mut i.negative_prompt } else { &mut i.quality_tags };
                        if !target.trim().is_empty() && !target.trim_end().ends_with(',') {
                            target.push(',');
                        }
                        if !target.is_empty() && !target.ends_with(' ') {
                            target.push(' ');
                        }
                        target.push_str(&tag);
                        changed = true;
                    }
                }
            });
        }
        section_title(ui, &ctx.t("aistudio.loras"));
        let mut remove = None;
        for (n, l) in i.loras.iter_mut().enumerate() {
            let (c, r) = lora_row(ui, ctx, l, &ch.loras, &default_t, &format!("ai_lora_{n}"));
            changed |= c;
            if r {
                remove = Some(n);
            }
        }
        if let Some(n) = remove {
            i.loras.remove(n);
            changed = true;
        }
        let mut new_lora = String::new();
        if search_combo(ui, "ai_lora_add", &mut new_lora, &ch.loras, &lora_t) && !new_lora.is_empty() {
            i.loras.push(LoraPick { name: new_lora, weight: 0.8, ..LoraPick::default() });
            changed = true;
        }
    });
    ui.add_space(8.0);
    civitai_card(ui, ctx, s, &mut changed);
    if changed {
        s.save_settings();
    }
}

fn civitai_card(ui: &mut Ui, ctx: &mut EditorContext<'_>, s: &mut StudioUi, changed: &mut bool) {
    card(ui, |ui| {
        section_title(ui, "Civitai");
        let c = &mut s.settings.civitai;
        *changed |= secret(ui, &ctx.t("aistudio.civitai_token"), &mut c.api_token);
        *changed |= ui.checkbox(&mut c.show_nsfw, ctx.t("aistudio.show_nsfw")).changed();
        ui.collapsing(ctx.t("aistudio.folders"), |ui| {
            ui.label(RichText::new(ctx.t("aistudio.folders_hint")).color(weak(ui)));
            let before = c.folders.clone();
            *changed |= field(ui, "Checkpoints", &mut c.folders.checkpoints);
            *changed |= field(ui, "LoRA", &mut c.folders.loras);
            *changed |= field(ui, "Embeddings", &mut c.folders.embeddings);
            *changed |= field(ui, "VAE", &mut c.folders.vaes);
            if c.folders != before {
                s.scanned = false;
            }
            let backend = s.settings.image.backend;
            if matches!(backend, ImageBackend::ComfyUi | ImageBackend::Automatic1111) {
                let key = if backend == ImageBackend::ComfyUi { "aistudio.comfy_root" } else { "aistudio.a1111_root" };
                *changed |= field(ui, &ctx.t(key), &mut s.settings.image.comfy_root);
                ui.horizontal(|ui| {
                    let can = !s.settings.image.comfy_root.trim().is_empty();
                    if ui.add_enabled(can, egui::Button::new(ctx.t("aistudio.detect"))).clicked() {
                        match scan::detect_folders(&s.settings.image.comfy_root, backend) {
                            Some(f) => {
                                c.folders = f;
                                s.scanned = false;
                                *changed = true;
                                s.detect_msg = ctx.t("aistudio.detect_ok");
                            }
                            None => s.detect_msg = ctx.t("aistudio.detect_fail"),
                        }
                    }
                    if !s.detect_msg.is_empty() {
                        ui.label(RichText::new(&s.detect_msg).color(weak(ui)));
                    }
                });
            }
        });
        let cu = &mut s.civitai;
        ui.horizontal(|ui| {
            let enter = ui.add(TextEdit::singleline(&mut cu.query).desired_width(220.0).hint_text(ctx.t("aistudio.search"))).lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ComboBox::from_id_salt("civ_kind")
                .selected_text(cu.kind.map_or("*", ModelType::api))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut cu.kind, None, "*");
                    for k in ModelType::ALL {
                        ui.selectable_value(&mut cu.kind, Some(k), k.api());
                    }
                });
            if (ui.button(icons::MAGNIFYING_GLASS).clicked() || enter) && cu.search.is_none() {
                let (settings, query, kind) = (c.clone(), cu.query.clone(), cu.kind);
                cu.search = Some(ctx.state().spawn_task("Civitai", None, move |p| {
                    let results = civitai::search(&settings, &query, kind).map_err(|e| TaskError::Failed(e.to_string()))?;
                    let mut previews = Vec::new();
                    for m in results.iter().take(12) {
                        p.check()?;
                        if let Some(v) = m.versions.first()
                            && let Some(url) = &v.preview_url
                            && let Ok(bytes) = civitai::fetch_bytes(url)
                            && let Ok(img) = image::load_from_memory(&bytes)
                        {
                            let t = img.thumbnail(96, 96).to_rgba8();
                            previews.push((v.id, [t.width() as usize, t.height() as usize], t.into_raw()));
                        }
                    }
                    Ok((results, previews))
                }));
            }
            if cu.search.is_some() || cu.download.is_some() {
                ui.spinner();
            }
        });
        if !cu.message.is_empty() {
            ui.label(&cu.message);
        }
        let mut add_lora: Option<LoraPick> = None;
        let mut start: Option<(ModelType, civitai::ModelFile)> = None;
        for m in &cu.results {
            let Some(v) = m.versions.first() else { continue };
            ui.separator();
            ui.horizontal(|ui| {
                if let Some(tex) = cu.previews.get(&v.id) {
                    ui.image((tex.id(), tex.size_vec2()));
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(&m.name).strong());
                    ui.label(RichText::new(format!("{} · {} · {}", m.kind.api(), v.base_model, v.name)).color(weak(ui)));
                    if !v.trigger_words.is_empty() {
                        ui.label(format!("{}: {}", ctx.t("aistudio.trigger_words"), v.trigger_words.join(", ")));
                    }
                    ui.horizontal(|ui| {
                        if let Some(f) = v.main_file() {
                            let mb = f.size_kb / 1024.0;
                            if ui.add_enabled(cu.download.is_none(), egui::Button::new(format!("{} {} ({mb:.0} MB)", icons::DOWNLOAD_SIMPLE, ctx.t("aistudio.download")))).clicked() {
                                start = Some((m.kind, f.clone()));
                            }
                            if m.kind == ModelType::Lora && ui.button(ctx.t("aistudio.use_lora")).clicked() {
                                add_lora = Some(LoraPick { name: f.name.clone(), weight: 0.8, trigger_words: v.trigger_words.clone() });
                            }
                        }
                    });
                });
            });
        }
        if let Some(l) = add_lora {
            s.settings.image.loras.push(l);
            *changed = true;
        }
        if let Some((kind, file)) = start {
            let folder = PathBuf::from(kind.folder(&s.settings.civitai));
            let civ = s.settings.civitai.clone();
            let image = s.settings.image.clone();
            let title = format!("Civitai: {}", file.name);
            s.civitai.download = Some(ctx.state().spawn_task(title, None, move |p| {
                let out = civitai::download(&civ, &file, &folder, &|| p.is_cancelled(), &|d, t| {
                    let f = if t > 0 { d as f32 / t as f32 } else { 0.0 };
                    p.set(f, format!("{} / {} MB", d / 1_048_576, t / 1_048_576));
                })
                .map_err(|e| TaskError::Failed(e.to_string()))?;
                let _ = sd::refresh_models(&image);
                Ok(match out {
                    DownloadOutcome::Downloaded(p) => format!("✔ {}", p.display()),
                    DownloadOutcome::AlreadyPresent(p) => format!("= {}", p.display()),
                })
            }));
        }
    });
}

fn pitch_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, s: &mut StudioUi, root: &Path) {
    let mut changed = false;
    let p = &mut s.state.pitch;
    ui.label(ctx.t("aistudio.pitch_hint"));
    changed |= ui.add(TextEdit::multiline(&mut p.text).desired_rows(8).desired_width(f32::INFINITY)).changed();
    egui::Grid::new("ai_pitch").num_columns(2).show(ui, |ui| {
        for (key, v) in [
            ("aistudio.genre", &mut p.genre),
            ("aistudio.tone", &mut p.tone),
            ("aistudio.language", &mut p.language),
            ("aistudio.art_style", &mut p.art_style),
        ] {
            ui.label(ctx.t(key));
            changed |= ui.text_edit_singleline(v).changed();
            ui.end_row();
        }
        for (key, v, max) in [
            ("aistudio.scenes", &mut p.scenes, 60),
            ("aistudio.characters", &mut p.characters, 20),
            ("aistudio.endings", &mut p.endings, 10),
            ("aistudio.lines", &mut p.lines_per_scene, 200),
        ] {
            ui.label(ctx.t(key));
            changed |= ui.add(DragValue::new(v).range(1..=max)).changed();
            ui.end_row();
        }
    });
    if changed {
        s.save_state();
    }
    ui.add_space(8.0);
    let ready = !s.state.pitch.text.trim().is_empty();
    if ui.add_enabled(ready, egui::Button::new(ctx.t("aistudio.run_bible"))).clicked() {
        spawn_step(s, ctx, root, "aistudio.tab.bible", |state, settings, _, p| {
            let model = OpenAiChat::new(settings.text.clone());
            p.set(0.1, "LLM…");
            pipeline::run_bible(state, &model, &settings.text, &|| p.is_cancelled()).map_err(|e| e.to_string())
        });
        s.tab = Tab::Bible;
    }
}

/// Runs a step on the task pool with a copy of the state. The step saves
/// its own checkpoints; the final state is saved here too.
fn spawn_step(
    s: &mut StudioUi,
    ctx: &mut EditorContext<'_>,
    root: &Path,
    title_key: &str,
    f: impl FnOnce(&mut PipelineState, &StudioSettings, &Path, &orbit_editor_sdk::Progress) -> Result<(), String> + Send + 'static,
) {
    s.save_state();
    s.message = None;
    let mut state = s.state.clone();
    let settings = s.settings.clone();
    let root = root.to_owned();
    s.step = Some(ctx.state().spawn_task(format!("AI Studio: {}", ctx.t(title_key)), None, move |p| {
        let outcome = f(&mut state, &settings, &root, p);
        if p.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        let _ = state.save(&root);
        Ok((state, outcome))
    }));
}

fn bible_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, s: &mut StudioUi, root: &Path) {
    if let Some(b) = &s.state.bible {
        ui.label(RichText::new(&b.title).heading());
        ui.label(&b.synopsis);
        ui.label(RichText::new(format!(
            "{} · {} · {}",
            ctx.tf("aistudio.n_characters", &[("n", &b.characters.len())]),
            ctx.tf("aistudio.n_locations", &[("n", &b.locations.len())]),
            ctx.tf("aistudio.n_scenes", &[("n", &b.scenes.len())]),
        )).color(weak(ui)));
    } else {
        ui.label(RichText::new(ctx.t("aistudio.no_bible")).color(weak(ui)));
    }
    ui.label(RichText::new(ctx.t("aistudio.bible_edit_hint")).color(weak(ui)));
    ui.add(TextEdit::multiline(&mut s.bible_edit).code_editor().desired_rows(18).desired_width(f32::INFINITY));
    for e in &s.bible_errors {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    if !s.state.bible_warnings.is_empty() {
        egui::CollapsingHeader::new(
            RichText::new(ctx.tf("aistudio.bible_autofixed", &[("n", &s.state.bible_warnings.len())]))
                .color(ui.visuals().warn_fg_color),
        )
        .default_open(true)
        .show(ui, |ui| {
            for w in &s.state.bible_warnings {
                ui.label(RichText::new(w).color(weak(ui)));
            }
        });
    }
    ui.horizontal(|ui| {
        if secondary_button(ui, &ctx.t("aistudio.apply_edit")).clicked() {
            let text = s.bible_edit.clone();
            match s.state.set_bible_text(&text) {
                Ok(()) => {
                    s.bible_errors.clear();
                    s.bible_edit = s.state.bible_text.clone();
                    s.save_state();
                }
                Err(e) => s.bible_errors = e,
            }
        }
        if secondary_button(ui, &ctx.t("aistudio.regenerate")).clicked() {
            s.tab = Tab::Pitch;
        }
        if s.state.bible.is_some() && primary_button(ui, &ctx.t("aistudio.run_script")).clicked() {
            run_script(s, ctx, root, None);
            s.tab = Tab::Script;
        }
    });
}

fn run_script(s: &mut StudioUi, ctx: &mut EditorContext<'_>, root: &Path, only: Option<String>) {
    spawn_step(s, ctx, root, "aistudio.tab.script", move |state, settings, root, p| {
        let model = OpenAiChat::new(settings.text.clone());
        let root = root.to_owned();
        pipeline::run_script(state, &model, &settings.text, only.as_deref(), &|| p.is_cancelled(), &mut |st, n, total| {
            let _ = st.save(&root);
            p.set(n as f32 / total.max(1) as f32, format!("{n}/{total}"));
        })
        .map_err(|e| e.to_string())
    });
}

fn script_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, s: &mut StudioUi, root: &Path) {
    let Some(b) = s.state.bible.clone() else {
        ui.label(RichText::new(ctx.t("aistudio.no_bible")).color(weak(ui)));
        return;
    };
    ui.horizontal(|ui| {
        if primary_button(ui, &ctx.t("aistudio.write_missing")).clicked() {
            run_script(s, ctx, root, None);
        }
        let done = b.scenes.len() - s.state.scenes_to_write().len();
        ui.label(format!("{done}/{}", b.scenes.len()));
    });
    ui.columns(2, |cols| {
        for sc in &b.scenes {
            let status = s.state.scripts.get(&sc.id).map(|x| &x.status);
            let mark = match status {
                Some(ScriptStatus::Valid) => icons::CHECK_CIRCLE,
                Some(ScriptStatus::Invalid(_)) => icons::WARNING,
                _ => icons::CIRCLE,
            };
            if cols[0].selectable_label(s.scene.as_deref() == Some(sc.id.as_str()), format!("{mark} {} — {}", sc.id, sc.title)).clicked() {
                s.scene = Some(sc.id.clone());
                s.scene_edit = s.state.scripts.get(&sc.id).map(|x| x.text.clone()).unwrap_or_default();
            }
        }
        let ui = &mut cols[1];
        let Some(id) = s.scene.clone() else { return };
        if let Some(sc) = b.scene(&id) {
            ui.label(RichText::new(&sc.summary).color(weak(ui)));
        }
        ui.add(TextEdit::multiline(&mut s.scene_edit).code_editor().desired_rows(20).desired_width(f32::INFINITY));
        if let Some(ScriptStatus::Invalid(errors)) = s.state.scripts.get(&id).map(|x| &x.status) {
            for e in errors {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
        }
        ui.horizontal(|ui| {
            if ui.button(ctx.t("aistudio.check_save")).clicked() {
                let text = s.scene_edit.clone();
                s.state.set_script_text(&id, &text);
                s.scene_edit = s.state.scripts.get(&id).map(|x| x.text.clone()).unwrap_or_default();
                s.save_state();
            }
            if ui.button(ctx.t("aistudio.regenerate")).clicked() {
                run_script(s, ctx, root, Some(id.clone()));
            }
        });
    });
}

fn assets_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, s: &mut StudioUi, root: &Path) {
    let Some(b) = s.state.bible.clone() else {
        ui.label(RichText::new(ctx.t("aistudio.no_bible")).color(weak(ui)));
        return;
    };
    ui.label(RichText::new(ctx.tf("aistudio.backend_is", &[("name", &s.settings.image.backend.name())])).color(weak(ui)));
    let none_text = ComboText::new(ctx, "aistudio.pick_lora");
    ui.collapsing(ctx.t("aistudio.looks"), |ui| {
        ui.label(RichText::new(ctx.t("aistudio.looks_hint")).color(weak(ui)));
        let mut changed = false;
        for c in &b.characters {
            let look = s.state.looks.entry(c.id.clone()).or_default();
            ui.horizontal(|ui| {
                ui.label(RichText::new(&c.name).strong());
                let mut fixed = look.seed.is_some();
                if ui.checkbox(&mut fixed, ctx.t("aistudio.seed")).changed() {
                    look.seed = fixed.then_some(s.settings.image.seed);
                    changed = true;
                }
                if let Some(seed) = &mut look.seed {
                    changed |= ui.add(DragValue::new(seed)).changed();
                }
                let mut has = look.lora.is_some();
                if ui.checkbox(&mut has, "LoRA").changed() {
                    look.lora = has.then(|| LoraPick { weight: 0.8, ..LoraPick::default() });
                    changed = true;
                }
            });
            if let Some(l) = &mut look.lora {
                let (c2, remove) = lora_row(ui, ctx, l, &s.choices.loras, &none_text, &format!("look_{}", c.id));
                changed |= c2;
                if remove {
                    look.lora = None;
                    changed = true;
                }
            }
        }
        if changed {
            s.save_state();
        }
    });
    ui.horizontal(|ui| {
        if secondary_button(ui, &ctx.t("aistudio.plan")).clicked() {
            match pipeline::plan_assets(&mut s.state, &s.settings.image) {
                Ok(()) => s.save_state(),
                Err(e) => s.message = Some((false, e.to_string())),
            }
        }
        if primary_button(ui, &ctx.t("aistudio.generate")).clicked() {
            spawn_step(s, ctx, root, "aistudio.tab.assets", |state, settings, root, p| {
                let model = image_gen::from_settings(&settings.image);
                let save_lock = Mutex::new(());
                let snapshot = Mutex::new(state.clone());
                pipeline::run_assets(state, root, model.as_deref(), &settings.image, &|| p.is_cancelled(), &|n, total, jobs| {
                    p.set(n as f32 / total.max(1) as f32, format!("{n}/{total}"));
                    // Checkpoint: the pictures done so far survive a cancel.
                    let _g = save_lock.lock();
                    if let Ok(mut snap) = snapshot.lock() {
                        snap.assets = jobs.to_vec();
                        let _ = snap.save(root);
                    }
                })
                .map_err(|e| e.to_string())
            });
        }
        if ui.button(ctx.t("aistudio.redo_all")).clicked() {
            for j in &mut s.state.assets {
                j.status = AssetStatus::Pending;
            }
            s.save_state();
        }
    });
    for (n, j) in s.state.assets.clone().iter().enumerate() {
        ui.separator();
        ui.horizontal(|ui| {
            let (mark, tip) = match &j.status {
                AssetStatus::Pending => (icons::CIRCLE, String::new()),
                AssetStatus::Generated => (icons::CHECK_CIRCLE, String::new()),
                AssetStatus::Placeholder(e) => (icons::WARNING, e.clone()),
            };
            ui.label(mark).on_hover_text(tip);
            let kind = match j.kind {
                AssetKindAi::Background => icons::IMAGE,
                AssetKindAi::Sprite => icons::USER,
            };
            ui.label(format!("{kind} {}", j.path));
            if ui.small_button(ctx.t("aistudio.redo")).clicked() {
                s.state.assets[n].status = AssetStatus::Pending;
                s.save_state();
            }
        });
        let mut prompt = j.prompt.clone();
        if ui.add(TextEdit::multiline(&mut prompt).desired_rows(2).desired_width(f32::INFINITY)).changed() {
            s.state.assets[n].prompt = prompt;
            s.state.assets[n].status = AssetStatus::Pending;
            s.save_state();
        }
    }
}

fn implement_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, s: &mut StudioUi, root: &Path) {
    let Some(project) = ctx.project() else { return };
    let blank = implement::is_blank(project);
    let replace = s.replace_scenes.get_or_insert(blank);
    ui.checkbox(replace, ctx.t("aistudio.replace_scenes"));
    ui.label(RichText::new(ctx.t("aistudio.implement_hint")).color(weak(ui)));
    if !s.state.script_complete() {
        ui.colored_label(ui.visuals().warn_fg_color, ctx.t("aistudio.script_incomplete"));
    }
    if s.state.assets.is_empty() {
        ui.colored_label(ui.visuals().warn_fg_color, ctx.t("aistudio.no_assets"));
    }
    let enabled = s.state.script_complete();
    if ui.add_enabled(enabled, egui::Button::new(ctx.t("aistudio.implement"))).clicked() {
        let options = ImplementOptions { replace_scenes: s.replace_scenes.unwrap_or(blank) };
        // Pictures not planned yet still get placeholders, so every
        // expression the script shows has a sprite.
        if s.state.assets.is_empty() || s.state.assets.iter().any(|j| !root.join(&j.path).is_file()) {
            let _ = pipeline::plan_assets(&mut s.state, &s.settings.image);
            let b = s.state.bible.clone().unwrap_or_default();
            orbit_ai_studio::assets::generate_all(root, &b, &mut s.state.assets, None, &s.settings.image, &|| false, &|_, _| {});
        }
        let manifest = orbit_project::load_manifest(&orbit_project::sandbox::FsStorage::new(
            match orbit_project::sandbox::Sandbox::new(root) {
                Ok(sb) => sb,
                Err(e) => {
                    s.message = Some((false, e.to_string()));
                    return;
                }
            },
        ))
        .unwrap_or_else(|_| orbit_project::ProjectManifest::new("game"));
        let registry = &ctx.registry().project;
        let state = s.state.clone();
        let result = ctx.ops(|p| {
            Ok(implement(p, root, &manifest, registry, &state, options))
        });
        match result {
            Ok(Ok(report)) => {
                s.state.implemented = state.bible.iter().flat_map(|b| b.scenes.iter().map(|x| x.id.clone())).collect();
                s.state.note("implemented");
                s.save_state();
                let text = ctx.tf("aistudio.report", &[
                    ("scenes", &report.scenes),
                    ("characters", &report.characters),
                    ("assets", &report.assets),
                    ("errors", &report.errors.len()),
                    ("warnings", &report.warnings),
                ]);
                s.message = Some((report.errors.is_empty(), text.clone()));
                if report.errors.is_empty() {
                    ctx.info(text);
                    ctx.request(EditorRequest::ShowPanel(PREVIEW_PANEL.to_owned()));
                } else {
                    ctx.warn(format!("{text}\n{}", report.errors.join("\n")));
                }
            }
            Ok(Err(e)) => s.message = Some((false, e)),
            Err(e) => s.message = Some((false, format!("{e:?}"))),
        }
    }
    if !s.state.log.is_empty() {
        section_title(ui, &ctx.t("aistudio.log"));
        for l in s.state.log.iter().rev().take(20) {
            ui.label(RichText::new(l).color(weak(ui)));
        }
    }
}
