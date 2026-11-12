# Rust plugins

Plugins add new kinds of steps and new interface actions to a game. They are written in Rust by a programmer; you do not need them to make a game. This chapter explains how to use one, for example the dice plugin shipped with the engine (`examples/plugins/dice`). Plugins that extend the editor itself (AI Studio...) are described in *Editor plugins*.

## Adding a plugin to a project

1. Copy the plugin's folder into the `plugins` folder of your project (create it if needed): `plugins/dice/`.
2. Its steps then appear in the editor's insert menu under **Plugins** (when the editor knows the plugin), with an inspector made of the fields the plugin declares: numbers with their limits, check boxes, lists of choices, and variable pickers.
3. A plugin step stores its results in story **variables** that you declare yourself (for the dice: an integer variable that receives the total). This is what makes saves, loads and rollback work: the plugin keeps nothing of its own.

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

A plugin is a Rust crate depending on `orbit_plugin_api` (API version 1, same engine sources). It implements `Plugin` (editor side: node types and their fields) and `RuntimePlugin` (handlers), declares its ids in `Cargo.toml` under `[package.metadata.orbit] plugins = ["your.id"]`, and exports `orbit_plugins()`. A handler reads variables, draws from the game's seeded random generator and returns variable operations and events; it may hold the story for a few seconds. It must not keep game state of its own: anything that has to survive a save or a rollback goes in variables.

A handler can also show a notification (a toast with a text, and optionally an icon, a sound, a duration and a style): it is displayed once the handler succeeded, like the `notify(...)` function of scripts.
