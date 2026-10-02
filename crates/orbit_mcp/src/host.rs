//! The project host: owns the [`Project`], runs tool calls, saves, logs.
//!
//! [`ProjectHost`] is the interface the MCP transport talks to. The headless
//! server uses [`HeadlessHost`] (direct execution); the live mode of M4 will
//! implement the same trait on top of the editor's command queue, so the tools
//! are written once.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use orbit_core::Story;
use orbit_plugin_api::{Plugin, PluginError, ProjectEvent, ProjectRegistry};
use orbit_project::{
    DEFAULT_STORY_FILE, MANIFEST_FILE, Project, ProjectManifest, ProjectStorage, load_manifest,
    load_story, save_manifest, save_story,
};
use serde_json::{Value as Json, json};

use crate::error::{ErrorKind, ToolError};
use crate::sandbox::{FsStorage, Sandbox};
use crate::tools::{ToolCx, ToolDef, builtin_tools, input_schema_of};
use crate::views;

/// Name of the action log, relative to the project root.
pub const LOG_FILE: &str = ".orbit/mcp.log";

/// Metadata of a tool, as advertised to clients.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub title: String,
    pub description: String,
    pub input_schema: serde_json::Map<String, Json>,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
}

/// A resource offered to clients.
#[derive(Debug, Clone)]
pub struct ResourceInfo {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub mime_type: &'static str,
}

/// What an MCP transport needs from a project.
pub trait ProjectHost {
    fn list_tools(&self) -> Vec<ToolInfo>;
    /// Runs a tool. The whole call is one undo entry and is rolled back on
    /// failure. Returns the text shown to the client.
    fn call_tool(&mut self, name: &str, args: Json) -> Result<String, ToolError>;
    fn list_resources(&self) -> Vec<ResourceInfo>;
    /// `(mime type, text)` of a resource.
    fn read_resource(&self, uri: &str) -> Result<(&'static str, String), ToolError>;
}

/// Failure to open a project folder.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("{0}")]
    Message(String),
}

fn open_err(message: impl Into<String>) -> OpenError {
    OpenError::Message(message.into())
}

/// Headless project host: a project folder, edited in memory through the
/// command API and saved atomically after every successful change.
pub struct HeadlessHost {
    sandbox: Sandbox,
    storage: FsStorage,
    manifest: ProjectManifest,
    story_file: String,
    project: Project,
    registry: ProjectRegistry,
    tools: Vec<ToolDef>,
    /// Revision of the project at the last save.
    saved_revision: u64,
}

impl HeadlessHost {
    /// Opens a project folder, or a story `.ron` file (its folder is the
    /// project). An empty folder becomes a new project.
    pub fn open(path: &Path) -> Result<Self, OpenError> {
        let (root, story_file_hint): (PathBuf, Option<String>) = if path.is_dir() {
            (path.to_owned(), None)
        } else if path.is_file() {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| open_err("invalid story file name"))?;
            (parent.to_owned(), Some(name.to_owned()))
        } else {
            return Err(open_err(format!(
                "`{}` is neither a project folder nor a story file",
                path.display()
            )));
        };
        let sandbox = Sandbox::new(&root)
            .map_err(|e| open_err(format!("cannot open {}: {e}", root.display())))?;
        let storage = FsStorage::new(sandbox.clone());

        let manifest_exists = storage.exists(MANIFEST_FILE);
        let mut manifest = if manifest_exists {
            load_manifest(&storage).map_err(|e| open_err(e.to_string()))?
        } else {
            let name = sandbox
                .root()
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Untitled")
                .to_owned();
            ProjectManifest::new(name)
        };
        if let Some(file) = story_file_hint {
            manifest.story = file;
        }
        let story_file = manifest.story.clone();

        let story = if storage.exists(&story_file) {
            load_story(&storage, &story_file).map_err(|e| open_err(e.to_string()))?
        } else if !manifest_exists && !storage.exists(DEFAULT_STORY_FILE) {
            // A brand new project.
            let story = Story::new(manifest.name.clone());
            save_manifest(&storage, &manifest).map_err(|e| open_err(e.to_string()))?;
            save_story(&storage, &story_file, &story).map_err(|e| open_err(e.to_string()))?;
            story
        } else {
            return Err(open_err(format!(
                "story file `{story_file}` not found in {}",
                sandbox.root().display()
            )));
        };

        let project = Project::new(story);
        let saved_revision = project.revision();
        let host = Self {
            sandbox,
            storage,
            manifest,
            story_file,
            project,
            registry: ProjectRegistry::new(),
            tools: builtin_tools(),
            saved_revision,
        };
        host.registry.dispatch(&ProjectEvent::Opened {
            title: &host.project.story().title,
        });
        Ok(host)
    }

    /// Installs a plugin (node types, validators, importers, MCP tools).
    pub fn install_plugin(&mut self, plugin: &dyn Plugin) -> Result<(), PluginError> {
        self.registry.install(plugin)
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }

    pub fn story_file(&self) -> &str {
        &self.story_file
    }

    fn persist(&mut self) -> Result<(), ToolError> {
        self.registry.dispatch(&ProjectEvent::BeforeSave);
        save_story(&self.storage, &self.story_file, self.project.story())
            .map_err(|e| ToolError::new(ErrorKind::Io, format!("cannot save the story: {e}")))?;
        self.saved_revision = self.project.revision();
        Ok(())
    }

    fn log(&self, line: &str) {
        let Ok(path) = self.sandbox.resolve(LOG_FILE) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{millis} {line}");
        }
    }

    fn run_tool(&mut self, name: &str, args: Json) -> Result<String, ToolError> {
        let (result, save_requested) = self.dispatch(name, args);
        self.finish(result, save_requested)
    }

    /// Runs the tool; returns its result and whether it asked for a save.
    fn dispatch(&mut self, name: &str, args: Json) -> (Result<String, ToolError>, bool) {
        let label = format!("MCP : {name}");
        let Self {
            sandbox,
            manifest,
            project,
            registry,
            tools,
            ..
        } = self;
        let mut save_requested = false;

        if let Some(tool) = tools.iter().find(|t| t.name == name) {
            let run = |project: &mut Project, save: &mut bool| {
                let mut cx = ToolCx {
                    project,
                    sandbox,
                    registry,
                    manifest,
                    save_requested: false,
                };
                let result = (tool.run)(&mut cx, args.clone());
                *save |= cx.save_requested;
                result
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                if tool.grouped {
                    project.group(&label, |p| run(p, &mut save_requested))
                } else {
                    run(project, &mut save_requested)
                }
            }));
            let result = outcome.unwrap_or_else(|_| {
                Err(ToolError::new(
                    ErrorKind::Internal,
                    format!("tool `{name}` crashed; the call was not applied (please report this)"),
                ))
            });
            return (result, save_requested);
        }

        // Plugin tools: keyed `plugin_id.tool`.
        if let Some((_, def)) = registry.mcp_tools().find(|(key, _)| key == name) {
            let text = args.to_string();
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                project.group(&label, |p| {
                    (def.call)(p, &text).map_err(|e| ToolError::new(ErrorKind::Plugin, e))
                })
            }));
            let result = outcome.unwrap_or_else(|_| {
                Err(ToolError::new(
                    ErrorKind::Internal,
                    format!("plugin tool `{name}` crashed; the call was not applied"),
                ))
            });
            return (result, false);
        }

        (
            Err(ToolError::not_found(format!(
                "unknown tool `{name}` (see tools/list)"
            ))),
            false,
        )
    }

    /// Saves after a successful change (or when asked to).
    fn finish(
        &mut self,
        result: Result<String, ToolError>,
        force_save: bool,
    ) -> Result<String, ToolError> {
        let text = result?;
        if self.project.revision() != self.saved_revision || force_save {
            self.persist().map_err(|e| {
                ToolError::new(
                    e.kind,
                    format!(
                        "{} (the change is applied in memory; call `save` to retry)",
                        e.message
                    ),
                )
            })?;
        }
        Ok(text)
    }
}

fn truncate(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    if flat.chars().count() <= max {
        flat
    } else {
        let cut: String = flat.chars().take(max).collect();
        format!("{cut}...")
    }
}

impl ProjectHost for HeadlessHost {
    fn list_tools(&self) -> Vec<ToolInfo> {
        let mut infos: Vec<ToolInfo> = self
            .tools
            .iter()
            .map(|t| ToolInfo {
                name: t.name.to_owned(),
                title: t.title.to_owned(),
                description: t.description.to_owned(),
                input_schema: t.input_schema.clone(),
                read_only: t.read_only && t.name != "save",
                destructive: t.destructive,
                idempotent: t.idempotent,
            })
            .collect();
        for (key, def) in self.registry.mcp_tools() {
            let schema = match serde_json::from_str::<Json>(&def.input_schema) {
                Ok(Json::Object(map)) => map,
                _ => input_schema_of::<serde_json::Map<String, Json>>(),
            };
            infos.push(ToolInfo {
                title: def.name.clone(),
                name: key,
                description: def.description.clone(),
                input_schema: schema,
                read_only: false,
                destructive: def.destructive,
                idempotent: false,
            });
        }
        infos
    }

    fn call_tool(&mut self, name: &str, args: Json) -> Result<String, ToolError> {
        let args = if args.is_null() { json!({}) } else { args };
        let summary = truncate(&args.to_string(), 400);
        let result = self.run_tool(name, args);
        match &result {
            Ok(text) => self.log(&format!(
                "tool={name} ok args={summary} result={}",
                truncate(text, 200)
            )),
            Err(err) => self.log(&format!("tool={name} ERROR args={summary} {err}")),
        }
        result
    }

    fn list_resources(&self) -> Vec<ResourceInfo> {
        let mut out = vec![
            ResourceInfo {
                uri: "orbit://project/manifest".into(),
                name: "Project manifest".into(),
                description: "orbit.project.ron: name, story file, required plugins.".into(),
                mime_type: "application/json",
            },
            ResourceInfo {
                uri: "orbit://project/scenes".into(),
                name: "Scenes".into(),
                description: "Scenes with id, name and node count.".into(),
                mime_type: "application/json",
            },
            ResourceInfo {
                uri: "orbit://project/characters".into(),
                name: "Characters".into(),
                description: "Characters with colors and expressions.".into(),
                mime_type: "application/json",
            },
            ResourceInfo {
                uri: "orbit://project/assets".into(),
                name: "Assets".into(),
                description: "Asset manifest (id, kind, path).".into(),
                mime_type: "application/json",
            },
            ResourceInfo {
                uri: "orbit://project/variables".into(),
                name: "Variables".into(),
                description: "Declared variables with type, initial value, scope.".into(),
                mime_type: "application/json",
            },
            ResourceInfo {
                uri: "orbit://project/validation".into(),
                name: "Validation report".into(),
                description: "Errors and warnings of the story right now.".into(),
                mime_type: "application/json",
            },
            ResourceInfo {
                uri: "orbit://schema/ir".into(),
                name: "Story IR JSON schema".into(),
                description: "JSON Schema of the story intermediate representation, generated from the Rust types.".into(),
                mime_type: "application/schema+json",
            },
        ];
        for scene in &self.project.story().scenes {
            out.push(ResourceInfo {
                uri: format!("orbit://scene/{}", scene.id),
                name: format!("Scene: {}", scene.name),
                description: format!("Numbered lines of scene `{}`.", scene.name),
                mime_type: "text/plain",
            });
        }
        out
    }

    fn read_resource(&self, uri: &str) -> Result<(&'static str, String), ToolError> {
        let story = self.project.story();
        let json = |v: Json| -> Result<(&'static str, String), ToolError> {
            Ok((
                "application/json",
                serde_json::to_string_pretty(&v).unwrap_or_default(),
            ))
        };
        match uri {
            "orbit://project/manifest" => json(json!({
                "format": self.manifest.format,
                "name": self.manifest.name,
                "story": self.story_file,
                "plugins": self.manifest.plugins.iter()
                    .map(|p| json!({"id": p.id, "version": p.version})).collect::<Vec<_>>(),
                "title": story.title,
                "schema_version": story.schema_version,
                "installed_plugins": self.registry.plugins().iter()
                    .map(|p| json!({"id": p.id, "version": p.version})).collect::<Vec<_>>(),
            })),
            "orbit://project/scenes" => json(views::scenes_json(story)),
            "orbit://project/characters" => json(views::characters_json(story)),
            "orbit://project/assets" => json(views::assets_json(story, None)),
            "orbit://project/variables" => json(views::variables_json(story)),
            "orbit://project/validation" => {
                let sandbox = self.sandbox.clone();
                let exists = move |p: &str| sandbox.resolve(p).is_ok_and(|f| f.is_file());
                json(views::validate_project(story, &self.registry, Some(&exists)).to_json())
            }
            "orbit://schema/ir" => Ok((
                "application/schema+json",
                serde_json::to_string_pretty(&crate::schema::ir_schema()).unwrap_or_default(),
            )),
            other => {
                if let Some(id) = other.strip_prefix("orbit://scene/") {
                    let scene = crate::convert::find_scene(story, id)?;
                    return Ok(("text/plain", views::render_scene(&self.project, scene)));
                }
                Err(ToolError::not_found(format!("unknown resource `{other}`")))
            }
        }
    }
}
