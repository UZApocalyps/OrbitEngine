# Tutorial: 3D scenes and editor plugins

Two tutorials for when you want to go further. The first puts a 3D scene behind your 2D characters, with cameras and a posed 3D character. The second shows how to extend the editor itself with a plugin of your own.

## Part 1. A 3D scene in your story (experimental)

### Goal

Build a small room in 3D with a light, two cameras and a posed character, then show it in the story with a cut from a wide shot to a close-up.

### Before you start

- A project with a scene where the 3D will be displayed.
- A 3D model, for example a character in a `.glb` file (or a `.gltf` file with its `.bin` and texture files next to it). A model with a skeleton lets you pose it. Without a model you can still follow the tutorial with the built-in shapes.
- A computer with a graphics card: the 3D editor and the preview need one.

### Steps

1. **Switch the feature on.** 3D is experimental and hidden by default. Choose **View > Options > Experimental 3D features** to tick it. The 3D steps then appear in the insertion menu.
2. **Open the 3D editor.** Choose **Tools > 3D scene editor**. Three panels open: the **3D viewport**, the **3D hierarchy** and the **3D inspector**. In the viewport, right-drag orbits, middle-drag pans and the wheel zooms.
3. **Create the scene.** Click **New 3D scene**. It starts with a sun and a camera named `main`. Click **Save the 3D scene** now and choose a name: imported models are stored in a `models` folder next to the scene file, so it must be saved first.
4. **Build the room.** In the hierarchy click **Add**, then **Shape**, and add a **Plane** for the floor, scaled up in the inspector (**Size**), and a **Cube** for a piece of furniture. A new shape stands on the ground with the scene's **Default** material; change its **Base color** in the inspector. Press W, E or R to move, rotate or scale the selected object, and G to put it down on the floor.
5. **Light it.** Add a point or spot light from **Add**, or keep the sun. Adjust its **Intensity** and **Color**. With nothing selected the inspector shows the **Environment**: background color or **Skybox**, ambient light and fog.
6. **Import your character.** Click **Import glTF** and choose the `.glb` file. The model appears in the scene. Place it with the move gizmo, or drag it from the **Models** list into the viewport.
7. **Set up the cameras.** Select the camera `main` and give it a good name in **Camera name**, for example `wide`. Click **Look through this camera** to see its shot; orbit the viewport to a nice angle, then click **Align to the view** to copy that angle to the camera. Add a second camera from **Add**, name it `closeup`, and frame the character's face the same way.
8. **Pose the character.** Select the model. If it has a skeleton, the inspector has a **Skeleton** section. Click **Pose** (or press P) to enter pose mode, click a bone, and drag its rings to turn it (W on the root moves the whole body, Delete resets the bone, Esc leaves the mode). Type a **Pose name** (for example `arms_crossed`) and click **Save the pose**; **Apply** brings a saved pose back at any time, and **Reset the pose** goes back to the rest pose. Under **Animations**, **New animation** and **Key the pose here** record a short movement.
9. **Add it to the project.** Click **Add to the project (3D scene asset)** in the viewport toolbar. The scene is saved as `assets/3d/<name>.scene3d.ron` and appears in the library.
10. **Show it in the story.** Click **Show the 3D scene in the story**: the editor inserts the **Show 3D scene** step after the selected line and plays the preview from there. Or insert the step yourself from the **3D** group of the insertion menu. In the Inspector pick the **3D scene** and the **Camera** `wide`, and a **Transition**.
11. Insert a **3D camera** step a few lines later and pick `closeup`: the picture cuts to the close-up. At the end insert a **Hide 3D scene** step to return to a normal 2D background.
12. Press **Play** (F6) in the Preview. Your 2D characters and the dialogue box are drawn on top of the 3D picture.

### Result

A story that moves from a wide shot to a close-up of a posed 3D character, in a room you built, with the 2D dialogue on top.

### Going further

- Use the 3D scene as the background of the whole chapter and keep 2D characters on top of it.
- In the **Export** panel, **3D quality (desktop)** sets the quality for Windows and Linux games; web and Android games always use the lighter web profile. The **3D quality** section reports lights, triangles and textures against the budget.
- An AI assistant (MCP) can read and edit the 3D scenes and take a picture from any camera (see *3D scenes*).

### If something goes wrong

- **The 3D steps are not in the menu**: the feature is off. Tick **Experimental 3D features** in **View > Options**.
- **"Save the 3D scene first"** when importing: save the scene so that the model has a folder to be copied to.
- **The model looks flat or dark**: add a light, or raise the ambient light in the **Environment**.
- **The Problems panel names a camera**: a step refers to a camera that the scene does not have (renamed or deleted). Pick another camera in the step.
- **The story shows a copy of an old scene**: the editor says so when the story's file differs from the one you are editing; use **Use the story's file**.

## Part 2. Create an editor plugin

### Goal

Build, package and install the `hello` example plugin, then change it to add your own menu action. You need Rust and the OrbitEngine sources.

### Before you start

- The Rust toolchain of the repository (`rust-toolchain.toml`) and the OrbitEngine sources, with the editor built from them: a plugin only loads in an editor built from the same sources, by the same compiler and with the same profile.
- Some Rust knowledge. The *Editor plugins* chapter explains the concepts and the compatibility rules in detail.

### Steps

1. **Look at the template.** Open the folder `examples/editor_plugins/hello`: a `Cargo.toml` (a `cdylib` depending on `orbit_editor_sdk` only), a `manifest.ron`, the code in `src/lib.rs` and the texts in `locales/en` and `locales/fr`. It adds a window, two actions to the **Tools** menu and a settings page.
2. **Build the editor.** From the repository root: `cargo build --release -p orbit_editor`.
3. **Package the plugin.** Run `cargo xtask plugins --examples --only orbit_hello_plugin`. The command builds the plugin with the editor's toolchain and profile, checks that it resolves the shared crates like the editor, and writes `<id>-<version>-<system>.orbitplugin` in the `plugins` folder of your cargo target directory. Add `--debug` if your editor is a debug build.
4. **Install it.** Start the editor, then choose **Tools > Extensions > Plugins…**. Click **Install from a file…**, pick the package and read the warning **Install native code?**: a plugin has the same rights as the editor. Click **Install** and **Restart now**.
5. **Try it.** The **Tools** menu now has **Hello window** and **Add a note**. With a project open, **Add a note** appends a narrator line to the current scene as one undo step: press Ctrl+Z to see it vanish. In the plugin manager the Hello entry shows **Enabled — running** and a **Settings** button that opens its settings page (**Text of the notes:**).
6. **Make it yours.** Copy the `hello` folder next to it and rename it. Change four things: the package `name` in `Cargo.toml`; the `id` in `manifest.ron`; the `ID` constant and the `id = "…"` in the `export_plugin!` call, which must agree with the manifest (a lowercase dotted name such as `acme.greeter`); and the texts.
7. **Add an action.** In `on_load`, register another action in the same way as the existing ones:

```rust
host.action(
    ActionDef::new("greet", "greeter.greet", |ctx| ctx.info("Hello from my plugin!"))
        .category("category.tools")
        .menu("tools", 902),
);
```

8. Give it a label in `locales/en/greeter.ron` with `"greeter.greet": "Greet the world",` and the French one in `locales/fr`. The editor loads the texts of the package by itself.
9. Rebuild with the same `cargo xtask plugins` command, install the new package from the plugin manager (a new version installs over the previous one) and restart. **Greet the world** appears in the **Tools** menu.
10. To build outside the repository, put in your `Cargo.toml` `orbit_editor_sdk = { git = "…/OrbitEngine", tag = "vX.Y.Z" }` with the editor's tag, and copy the editor's `Cargo.lock` and `rust-toolchain.toml` next to it.

### Result

A plugin of your own, installed from a `.orbitplugin` package, adding an action to the editor and removable from the plugin manager with **Uninstall**.

### Going further

- The `EditorContext` of the closures edits the project like the built-in tools: `transaction` and `group` combine several operations into one undo entry, `call_tool` uses the MCP tools, and `spawn_task` runs background work with a progress bar and **Cancel**.
- Add a panel (`panel`), a floating window (`window`), an inspector (`inspector`) or a settings page (`settings_page`) from `PluginHost`.
- A plugin must be rebuilt for each new editor version. The plugin manager says **Incompatible** and lists what differs when it does not match.

### If something goes wrong

- **Incompatible**: built for another editor build. Rebuild with `cargo xtask plugins` from the same sources and profile as the editor.
- **Disabled after a crash**: a panic in your code disables the plugin instead of closing the editor. Fix it and tick **Enabled** again.
- **The editor stopped while loading the plugin**: it is disabled at the next start. You can also start the editor with `--no-plugins` (safe mode) to load nothing and use the plugin manager to uninstall.
- **"The id is already used"**: the `id` is taken by another plugin or by a built-in tool. Choose another one.
- **The action is missing from the menu**: check the restart (changes take effect only after a restart) and that its label key exists in your `locales` file.
