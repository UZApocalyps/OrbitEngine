# Getting started

OrbitEngine lets you make a visual novel without writing any code. You write lines of dialogue, offer choices, show characters and backgrounds, and the editor plays the result for you as you go.

## Create your first project

1. Open the editor. The welcome page offers **New project** (Ctrl+N).
2. Type a name and pick the folder that will contain your project.
3. Choose a **template**:
   - **Blank**: one scene and two lines, to start from nothing.
   - **Visual novel starter**: two scenes, two characters, a choice, a variable, custom menu screens and a splash. You can play it right away and rewrite it.
4. Press **Create**. The project opens.

A project is a normal folder. Your story is the file `story.orbit`, your images and sounds live in `assets/`, and your custom screens in `ui/`. You can put the whole folder under version control or copy it to another computer.

## The main windows

- **Scene**: the story as a list of steps, in reading order. This is where you write most of the time.
- **Graph**: the same story drawn as boxes and arrows, handy to see where choices lead.
- **Inspector**: every setting of the selected step.
- **Characters** and **Variables**: the definitions your story uses.
- **Library**: your images and sounds. Drag files into the window to add them.
- **Preview**: plays the story with the real engine, and reloads when you edit.
- **Problems**: live checks. An error blocks the export, a warning does not. Checks run in the background about 300 ms after your last edit. Turn this off with **View > Automatic validation** (useful on very large projects): the list is then dimmed when out of date and refreshes when you press **Validate** (Ctrl+F7). Export and build always validate first.

You can open any window again from the **View** menu, and rearrange them by dragging their tabs.

## Saving and undoing

Ctrl+S saves. Ctrl+Z undoes and Ctrl+Y redoes, as far back as you need. If the editor closes unexpectedly, it offers to restore your unsaved work the next time you open the project.

## Where to go next

Each chapter of this manual covers one topic. Open **Help > Manual** at any time (or press F1). The manual follows the language of the editor.

## The menus

- **File**: New project, Open, Save, Close, **Import, export and translations** (import from Ren'Py, export the game, translation import and export), Import assets.
- **Edit**: Undo / Redo, **Insert** (every kind of step, as submenus by category: Dialogue, Scene, Effects, Screens, Sound, Logic, Navigation, 3D; plus the *New navigation map (house)* wizard), **Blocks** and **Scenes** (duplicate, move, rename...), Go to next problem.
- **View**: **Panels** (grouped as Story, Editing, Assets, Tools, Diagnostics, Other), **Layout** (saved window arrangements and tool workspaces), **Options** (Automatic validation, Experimental 3D features...) and **Language**.
- **Tools**: the Assistant (MCP), **Extensions > Plugins...** (editor plugins, see the *Editor plugins* chapter), AI Studio when its plugin is installed, rig tools such as *New rig from image...*.
- **Preview** and **Help** (the manual, F1).

A right click on a line of the Scene panel, a block of the Graph, an Explorer entry, a problem or the scene, then **Edit**, opens its settings directly in the Inspector. The **command palette** (Ctrl+K) finds any action by name.

## Keyboard shortcuts

**Project**

- Ctrl+N new project, Ctrl+O open a project, Ctrl+Shift+O open a story file, Ctrl+S save, Ctrl+W close the project, Ctrl+Q quit.
- Ctrl+Z undo, Ctrl+Y (or Ctrl+Shift+Z) redo.
- Ctrl+K command palette, Ctrl+Shift+F search the whole project, Ctrl+Shift+I import assets, Ctrl+Shift+A Assistant, F1 manual.
- Ctrl+F7 validate, F8 go to the next problem.

**Preview**

- F6 play / pause, Shift+F6 stop, Ctrl+F6 restart, F7 play from the selected step.

**Scene editor**

- Ctrl+Enter add a line, Ctrl+I insert a step (quick menu), Delete remove the selection.
- Ctrl+C, Ctrl+X, Ctrl+V copy, cut, paste lines; Ctrl+D duplicate; Ctrl+A select all; Alt+Up / Alt+Down move the selection; F2 rename the scene.
- In the **Text view**, Ctrl+Enter applies the text.

**Rig editor**

- B bone tool, M mesh tool, P weight brush, W / E / R move, rotate, scale, F frame everything, Tab rest / pose mode, K record a key in pose mode, Delete remove the selected bone, [ and ] brush size, Alt-click clone source or eyedropper.

**Stage View, clickable zones**

- Delete removes the selected zone; while drawing a polygon, Enter closes it, Backspace removes the last point, Escape cancels.

**3D viewport** (experimental)

- W / E / R move, rotate, scale, F frame the selection, Delete delete, Ctrl+D duplicate, 1 / 3 / 7 front, right, top view (with Ctrl: the opposite side), G put the selection on the ground, P pose mode of a character, Escape leave pose mode.

**In the game**

- F11 fullscreen. On a point & click map, V shows every zone; the keyboard (digits included) and the controller move between zones. The controller's *show zones* action is Select / View by default.

## Troubleshooting and FAQ

**The exported web game stays blank or stuck when I open `index.html`.** Browsers refuse WebAssembly and file loading on `file://` pages. Serve the folder over http: use **Test in the browser** in the Export panel, or `orbit export web --project my_game --out dist --serve`, or `cargo xtask serve --dir dist` (it serves on `http://127.0.0.1:8080`, `--port` changes it). To publish, upload the folder to any static host.

**There is no 3D menu, panel or manual chapter.** The 3D features are experimental and hidden by default. Turn them on with **View > Options > Experimental 3D features** (or start the editor with the environment variable `ORBIT_EXPERIMENTAL_3D=1`). The *3D scenes* chapter of this manual only appears once they are on. Projects that already use 3D keep working with the switch off.

**An editor plugin says "Incompatible" after I updated the editor.** A plugin is built for one exact editor build. Rebuild it for the new version (`cargo xtask plugins`), or install the version made for it. If a plugin stops the editor from starting properly, start it with `--no-plugins`, then disable or uninstall the plugin in **Tools > Extensions > Plugins...**.

**AI Studio cannot reach my local server.** `localhost` can resolve to IPv6 (`::1`) while the server only listens on IPv4: try `http://127.0.0.1:11434/v1` (Ollama) or `http://127.0.0.1:8188` (ComfyUI). The URL of a text model ends with `/v1`.

**Ollama answers "model not found".** The model is not downloaded yet: run `ollama pull <model>` in a terminal (for example `ollama pull llama3.1`), then use **Test connection / list models** again. The model name must match exactly.

**A 3D model is missing in the preview or in the game.** The model file must sit in the `models` folder next to the scene file. The Problems panel and the export log name the missing model. Open the scene in the 3D editor and import the model again, or copy the file there. If the story shows an older copy of a scene you are editing, use *Use the story's file* in the 3D viewport (see the *3D scenes* chapter).

**Characters appear at the wrong place on stage.** The origin of a rig is where the character's bottom-center goes: use **Recenter on origin...** in the rig editor.

**Some characters of my translation show as empty squares.** The default font only covers Latin scripts: add a font for that language in the Interface editor (see *Translations*).

**Old projects, saves and games.** Old projects and saves still load in newer versions. A game exported with a newer engine may be refused by an older player ("too recent" pack): export it again with the matching player.
