# Rust plugins

Plugins add new kinds of steps and new interface actions to a game. They are written in Rust by a programmer; you do not need them to make a game. This chapter explains how to use one, for example the dice plugin shipped with the engine (`examples/plugins/dice`). Plugins that extend the editor itself (AI Studio...) are described in *Editor plugins*.

## Adding a plugin to a project

1. Copy the plugin's folder into the `plugins` folder of your project (create it if needed): `plugins/dice/`.
2. Its steps then appear in the editor's insert menu under **Plugins** (when the editor knows the plugin), with an inspector made of the fields the plugin declares: numbers with their limits, check boxes, lists of choices, and variable pickers.
3. A plugin step stores its results in story **variables** that you declare yourself (for the dice: an integer variable that receives the total). This is what makes saves, loads and rollback work: the plugin keeps nothing of its own.

## Installing a plugin from the Store

Game plugins published in a catalog install without copying folders:

1. Open the project, then **Tools > Extensions > Plugins…**, **Store** tab, and choose **Game plugins (project)** at the top of the tab. Each plugin has a **Game** badge, its source (Official, Trusted, Unsigned), the version offered for this engine and its state in the project.
2. Click **Install in the project**. The file is downloaded, its size, SHA-256 and signature are checked, then its sources are copied into `plugins/<name>/` of the project and the plugin is added to the required plugins of `orbit.project.ron`. Nothing is installed in the editor's plugin folder.
3. Before installing from a source that is not the official one, the editor asks first: a game plugin is Rust code compiled and run on your computer when you export (build scripts included), then in your players' game. Only install plugins you trust.

The **Installed** tab has a **Game plugins of <project>** section: the crates of `plugins/`, their source (or "added by hand"), updates (only offered by the source they were installed from), **Uninstall** and **Open the folder**. It also tells:

- whether the editor knows the plugin's node types. It only does when its editor part is loaded (an editor plugin with the same id, installed from the Store, **Editor plugins**). Otherwise its steps stay read-only, like a missing plugin, but the exported game runs them;
- whether an export can build the player: it needs the Rust toolchain and the engine sources (`ORBIT_ENGINE_SRC`). Without them the plugin still installs, but the export will fail: the editor says so. If the engine sources were not found at install time, reinstall the plugin once `ORBIT_ENGINE_SRC` is set.

**Your changes are never overwritten silently.** If you changed the files of a plugin installed by the Store, or if `plugins/` already holds a crate written by hand in the same place or providing the same id, installing, updating and removing stop and list the files involved. **Replace (keep a backup)** moves the previous folder to `.orbit/plugin-backups/` of the project. Building the plugin (`target/`, `Cargo.lock`) does not count as a change.

## Interface actions

A button of a screen can call a plugin with the **Custom** action. Its name is the plugin id followed by the action, for example `orbit.dice.reroll`, and its arguments are given as text (`total`, `6`).

## Exporting

A project with plugins is exported with a player built for it: the export panel shows a **Plugins** box with the plugin crates found and whether a Rust toolchain is installed.

- Install Rust from https://rustup.rs if the panel says it is missing.
- The first export builds the player (several minutes); the next ones are much faster.
- Only the platform the editor runs on (Windows or Linux) can be exported with plugins for now. Web and Android exports are not available yet.
- An export is refused if the story uses a plugin that is not in `plugins/`.

## Missing plugins

If a project uses a plugin that is not installed, its steps are kept untouched and shown read-only, and the Problems panel lists them. When playing, such a step is skipped with a warning.

## For programmers

A plugin is a Rust crate depending on `orbit_plugin_api` (API version 1, same engine sources). It implements `Plugin` (editor side: node types and their fields) and `RuntimePlugin` (handlers), declares its ids in `Cargo.toml` under `[package.metadata.orbit] plugins = ["your.id"]`, and exports `orbit_plugins()`. To publish it in a catalog: `cargo xtask game-plugin pack <folder> --sign <key>` makes a signed source package (`.orbitsrc`), and `cargo xtask game-plugin index --index index.json` adds it to the catalog (see `docs/plugin-store.md`). Its engine dependencies are path dependencies (`../crates/orbit_*`); the editor points them at `ORBIT_ENGINE_SRC` when it installs the plugin. A handler reads variables, draws from the game's seeded random generator and returns variable operations and events; it may hold the story for a few seconds. It must not keep game state of its own: anything that has to survive a save or a rollback goes in variables.

A handler can also show a notification (a toast with a text, and optionally an icon, a sound, a duration and a style): it is displayed once the handler succeeded, like the `notify(...)` function of scripts.
