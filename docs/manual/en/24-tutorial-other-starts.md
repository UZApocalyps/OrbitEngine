# Tutorial: other ways to start a project

You do not always begin with a blank page. If you already have a Ren'Py game, the editor can turn it into a project for you. If all you have is an idea, the AI Studio can write a first version of the game from a few lines of text. Both give you a project to finish by hand.

## Part 1. Import a Ren'Py game

### Goal

Convert an existing Ren'Py game into an OrbitEngine project, read the import report, and fix what the importer could not convert.

### Before you start

- The folder of the Ren'Py game: the one that contains `game/`, or `game/` itself. The importer reads the `.rpy` source files, so the scripts must be present (a game shipped only as compiled archives cannot be imported). The original game is never modified.
- An empty folder, or a place where a new folder can be created, for the new project.
- Images as PNG or JPEG and sounds as OGG or WAV. WebP and AVIF images and MP3 or Opus audio are not imported: convert them first.

### Steps

1. Choose **File > Import, export and translations > Import from Ren'Py…**.
2. Pick the Ren'Py game folder, then the folder that will contain the new project. The task **Importing the Ren'Py game** runs in the background, with a progress bar and a **Cancel** button in the status bar.
3. When it is done, the project opens and the **Ren'Py import report** window shows a summary: what was converted, what was approximated, and every statement that could not be converted with its file and line. The same report is saved in the new project as `import-report.md`, so you can come back to it.
4. Open the **Problems** panel. The project validates, but warnings point at places worth a look: an unreachable step, a scene that ends the game, a variable used before being assigned.
5. Play the beginning with **Preview** (F6) and compare it with the original game. Look at the dialogue, characters and backgrounds first.
6. Go through the report from top to bottom and fix, in this order:
7. **Script nodes**: the Python that is not a simple assignment is kept as a commented script step. Select it in the **Scene** panel and rewrite it with steps (a **Set a variable**, a **Condition**) or in Rhai (see *Scripting (Rhai)*).
8. **Conditions converted to Rhai**: open each **Condition** step that the report names and check it behaves like the original.
9. **Interface screens**: Ren'Py's say, menu, save and preferences screens are not converted. Build yours in the **Interface** panel (see *Tutorial: customize the game interface*). A screen made of an image map or image buttons used with `call screen` has become a **Zones** step (point and click).
10. **Missing or unsupported media**: add the converted images and sounds with **Import** in the **Asset Library**, and check that the expressions and backgrounds that the Problems panel marks as missing now have their pictures.
11. Save with Ctrl+S, and export when everything plays well.

### Result

A project that opens, validates and plays, with your scenes, characters, variables, choices, transitions and translations, and a report listing everything left to do.

### Going further

- The command line does the same: `orbit import-renpy <renpy_dir> <out_dir>`; the output folder must be empty or absent.
- Ren'Py translations (`tl/<language>/`) become translations of the project (see *Translations*).
- Achievements registered with `achievement.register` become achievements with their unlock steps.

### If something goes wrong

- **The import finds nothing**: check that the folder you picked contains the `.rpy` files (the `game/` folder), not only a `.rpa` archive.
- **A lot of text tags disappeared**: tags such as `{size}`, `{font}` and `{a}` are removed; the report counts them by tag.
- **A character shows the wrong picture**: Ren'Py names its images by convention (`eileen happy.png`). Check the expression in the **Characters** panel and pick the right file with **File…**.
- **A sprite is not where it was**: positions are converted to the engine's anchor (bottom center). Place it again in the **Stage View**.

## Part 2. Generate a game with the AI Studio

### Goal

Produce a playable first draft of a visual novel from a short pitch, using a language model and an image generator that run on your own computer. Count about an hour the first time, mostly waiting.

### Before you start

- **The plugin.** The AI Studio is an editor plugin: the editor ships without it. Build it from the repository with `cargo build --release -p orbit_editor`, then `cargo xtask plugins`. This writes `orbit.ai_studio-<version>-<system>.orbitplugin` in the `plugins` folder of your cargo target directory. The plugin must be built for the same editor version.
- **A text model.** Install Ollama, then pull a model in a terminal, for example `ollama pull llama3.1`. A model of about 12 billion parameters or more writes noticeably better scripts. Any server with an OpenAI-compatible `chat/completions` API also works.
- **An image generator** (optional). Start ComfyUI (default address `http://127.0.0.1:8188`) with a checkpoint installed. Without one, the studio uses colored placeholders and the game still runs.
- A project open in the editor, empty or not.

### Steps

1. **Install the plugin.** Choose **Tools > Extensions > Plugins…**, click **Install from a file…** and pick the `.orbitplugin` file. Read the warning **Install native code?**: a plugin runs with the same rights as the editor, so install only plugins you trust. Click **Install**, then **Restart now**.
2. **Open the studio.** Choose **Tools > AI Studio (create a game with AI)…**. It works on the open project.
3. **Providers tab, text model.** Under **Text model (OpenAI-compatible)** set **URL** to `http://127.0.0.1:11434/v1` and **Model** to `llama3.1`. Leave the key empty. Click **Test connection / list models**: the studio shows how many models are available. Use `127.0.0.1` rather than `localhost`: some systems resolve `localhost` to an IPv6 address the server does not listen on.
4. **Providers tab, images.** Set **Backend** to **ComfyUI** and the URL to `http://127.0.0.1:8188`. For a first test without any image generation, choose the placeholder backend instead.
5. **SD models tab** (ComfyUI or AUTOMATIC1111 / Forge only). Click **Read the models of the backend** and pick a **Checkpoint**. Choose the **Model family** that matches it (SD 1.5, SDXL, Pony, Illustrious / NoobAI): it sets sizes, quality tags and clip skip suitably.
6. **1. Pitch tab.** Describe your game: premise, characters, mood. Set the genre, tone, dialogue language, number of scenes, characters and endings, art style and lines per scene. Start small: three scenes, two characters, two endings.
7. Click **Write the story bible**. The model writes a structured plan; the studio checks it and sends any error back to the model to correct itself.
8. **2. Bible tab.** Read the title, synopsis, characters, locations, scenes and variables. You can edit the bible by hand and click **Check and apply**. A new bible clears the scripts and pictures that follow it.
9. **3. Script tab.** Click **Write the scenes**. Each scene is written one by one in the engine's script language and checked like the Text view does. A scene that still has errors is kept with a warning: fix it by hand and click **Check and save**. **Regenerate** rewrites a single scene.
10. **4. Pictures tab.** Click **Plan the pictures**: one background per location, one sprite per character and per expression the script uses. Edit any prompt you dislike, then click **Generate missing pictures**. **Redo** makes one picture again. A failed picture is replaced by a placeholder so the game always runs.
11. **5. Implement tab.** Tick **Replace the scenes of the story (otherwise the new scenes are appended)** if you want the generated story to be the game. Click **Implement in the project**. The studio creates the characters, expressions, backgrounds, variables and scenes as one undoable step (Ctrl+Z removes all of it), validates the story and opens the Preview.
12. Play it, open the **Problems** panel, and finish the game by hand: rewrite lines, swap pictures in the library, add music.

### Result

A project with characters, backgrounds, a script with choices and endings, ready to be polished like any other project.

### Going further

- Long steps run in the background with a progress bar and a **Cancel** button. The state is saved after every scene and picture in `.orbit/ai/studio.json` in the project, so running a step again resumes where it stopped, even after closing the editor.
- Download checkpoints and LoRAs from Civitai in the **SD models** tab, and give a character a fixed seed or LoRA in the Pictures tab to keep its look.
- Connect an AI assistant (MCP) instead: it gets the studio's check tools too (see *The MCP assistant*).

### If something goes wrong

- **"Test connection" fails**: check that Ollama is running (`ollama list` in a terminal) and that the URL ends with `/v1`.
- **The model's answers are poor or cut off**: pick a larger model, raise **max tokens**, or allow more repairs in the Providers tab.
- **No picture is generated**: check that ComfyUI is running at the address you gave and that a checkpoint is selected; the placeholder pictures keep the game playable meanwhile.
- **The plugin says Incompatible**: it was built for another editor version or toolchain. Rebuild it with the editor's own toolchain (see *Editor plugins*).
- **A scene stays flagged with a warning**: open the Script tab, read the error with its line number, fix the line and click **Check and save**.
