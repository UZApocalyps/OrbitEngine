# Editor plugins

Editor plugins add panels, windows, menu entries and tools to the editor itself. They are native libraries (a `.dll` on Windows, `.so` on Linux, `.dylib` on macOS) that the editor loads when it starts. The base editor ships without any; the official distributions ship a few (see *Plugins shipped with the editor*) and you install the ones you want on top. **AI Studio** is one of them.

They are different from the *Rust plugins* of the previous chapters, which add steps to a game and are exported with it. An editor plugin never ends up in an exported game.

## Installing a plugin

1. Open **Tools > Extensions > Plugins…**.
2. Click **Install from a file…** and choose a plugin package (`.orbitplugin`) or a library (`.dll`, `.so`, `.dylib`).
3. Read the warning and confirm: a plugin is native code that runs inside the editor with the same rights as the editor (your files, the network), without any sandbox. Only install plugins from sources you trust.
4. The editor checks the file (see *Compatibility* below). If it is accepted, it is copied into the plugins folder and enabled.
5. Click **Restart now** in the banner: plugins are loaded when the editor starts, never while it runs.

Each plugin in the list shows its name, version, author, description and state:

- **Enabled — running**: loaded in this session.
- **Enabled — runs after a restart**: installed or enabled since the editor started.
- **Disabled**: installed but not loaded.
- **Incompatible**: built for another editor build; the message says what differs. Rebuild the plugin, or install the version made for this editor.
- **Error** / **Disabled after a crash**: the reason is shown; tick **Enabled** again to retry after a fix.

The **Enabled** box, **Uninstall** (which also removes the plugin's data) and installing a new version all take effect at the next start. **Settings** opens the plugin's settings page when it has one. **Open the plugins folder** shows where plugins live:

- Windows: `%APPDATA%\OrbitEngine\Editor\plugins\`
- Linux: `~/.config/orbit-engine/editor/plugins/`

One folder per plugin (`plugins/<id>/`) holds its library, its texts (`locales/`), its files (`assets/`) and its own data (`data/`, kept across updates). `plugins.ron` remembers which plugins are enabled.

## When a plugin misbehaves

- A plugin that crashes while it runs is stopped and disabled, with a notice; the editor goes on. Its panels then show the error.
- If the editor itself stops while loading a plugin, that plugin is disabled at the next start and the plugin manager says so.
- Start the editor with `--no-plugins` (safe mode) to load no plugin at all; the plugin manager still works, to disable or uninstall one.

## Compatibility: rebuild for every editor version

A plugin and the editor share Rust types directly (the egui interface, the project, the editor context). That is only safe when both were built from the same sources, by the same compiler, with the same settings. The editor therefore reads a **fingerprint** in every plugin before loading it, and refuses any difference:

- editor: the version of the editor API (`orbit_editor_api`)
- plugin ABI: the version of the plugin contract
- compiler: the exact `rustc` (version, date and commit)
- egui: the egui version
- platform: the target (`x86_64-pc-windows-msvc`...)
- build profile: `release` or `debug`
- API sources: a hash of the sources of the API crates
- type layouts: a hash of the size of the shared types
- dependency build: a hash of the identity (`TypeId`) of shared types, which depends on how cargo resolved the dependencies (`Cargo.lock`, features)

The manifest of a package can also give the editor versions it accepts (`min_editor`, `max_editor`). In practice: **a plugin must be rebuilt for each new editor version**, with the editor's toolchain. After you update the editor, a plugin that worked before is listed as **Incompatible** with the field that differs; nothing is deleted. Install the rebuilt package over it (a new version replaces the old one at the next start).

Before loading, the editor also checks that the file is a library for this system and processor, reads the fingerprint in a separate short-lived process (a broken library cannot crash the editor), checks the plugin id, and compares the library with the checksum (SHA-256) recorded at install: a library changed afterwards is refused.

## Plugins shipped with the editor

The official distributions (installers and portable archive) contain a `bundled-plugins/` folder next to the executable, with `.orbitplugin` packages: by default the Ren'Py import (`orbit.renpy_import`) and the MCP server with the Assistant panel (`orbit.mcp`). At start-up the editor installs every shipped package whose id was never installed before, exactly like an install from a file (same checks, SHA-256 recorded), then enables it: the plugin is loaded from that very first launch. The features that used to be built into the editor therefore stay available after an update, with nothing to install by hand.

- The plugin manager marks these plugins with a **Ships with the editor** badge.
- A shipped plugin you disable or uninstall **does not come back**: the editor remembers the ids already offered in `plugins.ron` (`bundled_offered` field, ignored by earlier editor versions).
- A new editor version updates a shipped plugin when the shipped package has a strictly higher version, or the same version but the installed plugin no longer matches the editor (fingerprint or version range): the compatible shipped build replaces it. A plugin you installed yourself (file or Store) is never touched; if it is disabled, it stays disabled after an update.
- Only the `bundled-plugins/` folder next to the executable is read (not the current folder, not the user's), and no check is relaxed. An invalid or incompatible shipped package is skipped with a line in the log. `--no-plugins` also turns this off.
- To prepare a distribution: `cargo build --release -p orbit_editor`, then `cargo xtask plugins --bundle` copies the packages of the chosen ids into `packaging/bundled-plugins/` (which the packager places in `bundled-plugins/`). Without a list, `--bundle` ships `orbit.renpy_import` and `orbit.mcp` (the `DEFAULT_BUNDLED` constant in `xtask/src/plugins.rs`); `--bundle orbit.mcp,orbit.ai_studio` picks others, `--bundle-dir <folder>` changes the destination.

## AI Studio

AI Studio is an official plugin built from this repository:

```
cargo build --release -p orbit_editor
cargo xtask plugins
```

The second command writes `target/plugins/orbit.ai_studio-<version>-<os>-<arch>.orbitplugin`. Install that file from the plugin manager of the editor built by the first command (same profile: `cargo xtask plugins --debug` for a debug editor). Its settings (providers, keys) stay in `ai_studio.ron` in the editor's configuration folder, as before.

## MCP server

The editor's MCP server (an AI client such as Claude Code connecting to the open project) and the **AI Assistant** panel are another official plugin, `orbit.mcp` (`plugins/orbit_mcp_plugin`), built by the same `cargo xtask plugins` command (`target/plugins/orbit.mcp-<version>-<os>-<arch>.orbitplugin`). The editor keeps the tools and runs them itself on the open project: without the plugin, plugins that call tools (AI Studio) still work; only the network transport (HTTP on 127.0.0.1, token) and the panel are in the plugin. The port and token stay in the editor's configuration (`orbit.ai`), as before.

## For programmers: writing a plugin

The template is `examples/editor_plugins/hello`: a window, a **Tools** menu action that adds a note to the project as one undoable step, a settings page and its texts in French and English.

**Cargo.toml**: a `cdylib` depending on the SDK only, at the editor's version.

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
orbit_editor_sdk = { git = "https://.../OrbitEngine", tag = "v0.4.0" }

[lints.rust]
unsafe_code = "deny"   # not "forbid": export_plugin! generates exported functions
```

Out of this repository, copy the editor's `Cargo.lock` and `rust-toolchain.toml` next to your `Cargo.toml`: the SDK depends on the editor so that cargo resolves every shared crate exactly like the editor (the *dependency build* field).

**Code**:

```rust
use orbit_editor_sdk::native::{NativePlugin, PluginHost};
use orbit_editor_sdk::{ActionDef, WindowDef, egui};

#[derive(Default)]
pub struct Hello;

impl NativePlugin for Hello {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        host.action(ActionDef::new("hi", "hello.hi", |ctx| ctx.info("hi")).menu("tools", 900));
        host.window(WindowDef::new("window", |ctx, ectx| { /* egui::Window::new(...).show(ctx, ...) */ }));
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(Hello, id = "acme.hello", name = "Hello",
    description = "...", author = "ACME");
```

What `PluginHost` offers: panels (`panel`), floating windows (`window`), actions with menu entries, shortcuts and palette entries (`action`, `menu`), inspectors (`inspector`), a settings page (`settings_page`), text tables (`texts`, plus the package's `locales/<lang>/*.ron`, loaded by the editor), the project registry for MCP tools and hooks (`project`), an existing `EditorPlugin` (`install`), the plugin's folders (`plugin_dir`, `data_dir`, `editor_config_dir`).

In the closures, the `EditorContext` reads the project and edits it like the built-in tools: `exec`, `transaction` and `group` (several operations as one undo entry), `call_tool` / `take_tool_result` (the MCP tools, one undo entry per call), `spawn_task` on `state()` for background work with progress and cancel, `t` / `tf` for texts, `info` / `warn` / `error` for notices.

**Package**: a `.orbitplugin` is a zip with `manifest.ron` (`id`, `name`, `version`, `description`, `author`, `library`, optional `min_editor` / `max_editor` / `homepage`), the library, and optional `locales/` and `assets/` folders. `cargo xtask plugins --examples` builds and packages the plugins of this repository, after checking that each one resolves the crates shared with the editor exactly like the editor.

**File dialogs**: the editor provides the native dialogs, a plugin does not ship its own dialog library. `ctx.pick_folder(title)` picks a folder, `ctx.pick_file(title, &[("Images", &["png", "jpg"])])` a file to open, `ctx.save_file(title, "name.csv", &[("CSV", &["csv"])])` a file to save (the file is not created). Each returns `Option<PathBuf>` (`None`: cancelled), starts in the open project's folder and blocks the editor while it is open, like the editor's own dialogs; the MCP server answers "busy" meanwhile.

**Rules**:

- Draw with `orbit_editor_sdk::egui`, never with an egui of your own.
- Never use an `egui::Context` from a background thread: report progress through the task's `Progress` and let the interface draw.
- Everything the plugin registers is protected: a panic disables the plugin instead of closing the editor. Node types and importers (plain functions) are not: they must not panic.
- Do not set a global allocator. The library is never unloaded while the editor runs.
