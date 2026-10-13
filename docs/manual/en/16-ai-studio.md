# AI Studio

The AI Studio builds a whole visual novel from a few lines of text: you describe the game, a language model of your choice writes the story, an image generator draws the backgrounds and characters, and the studio puts everything into your project. Every step can be reviewed, edited and run again.

AI Studio is an **editor plugin**: the editor ships without it. Build it with `cargo xtask plugins` and install `orbit.ai_studio-….orbitplugin` from **Tools > Extensions > Plugins…**, then restart the editor (see *Editor plugins*). Its settings page in the plugin manager holds the provider settings.

Open it from **Tools > AI Studio**. It works on the open project.

The studio is part of the editor only. Nothing of it is ever included in an exported game.

## The five steps

1. **Pitch**: your base text (premise, characters, mood...) and options: genre, tone, dialogue language, number of scenes, characters and endings, art style, lines per scene.
2. **Bible**: the model writes a structured plan (title, synopsis, characters with their personality and appearance, locations, scenes with their branches and endings, variables). The studio checks it; if something is wrong (a scene pointing to an unknown location, a missing ending...), the errors are sent back to the model, which corrects itself. You can edit the bible by hand (JSON) and apply it.
3. **Script**: each scene is written in the engine's script language (see *Text view*), one scene per request so long stories fit the model's memory. Each scene is checked by the same parser as the text view; errors with their line numbers go back to the model for repair. A scene still wrong after the allowed repairs is kept, marked with a warning, for you to fix by hand (**Check and save**). **Regenerate** rewrites one scene.
4. **Pictures**: one background per location, and one sprite per character and per expression the script actually uses (`neutral`, `happy`, `sad`, `angry`, `surprised`, `embarrassed`, `scared`, `thinking`). Each prompt can be edited; **Redo** makes one picture again. A picture that fails is replaced by a placeholder, so the game always runs. Files go to `assets/ai/backgrounds/<location>.png` and `assets/ai/characters/<character>/<expression>.png`.
5. **Implement**: creates the characters with their expressions, the backgrounds, the variables and the scenes, with the same operations as the MCP assistant, as **one** undoable step (Ctrl+Z removes all of it). The story is validated and the preview opens. Tick *Replace the scenes* to start the game with the generated story (default for an empty project); otherwise the scenes are appended.

Long steps run in the background with a progress bar; **Cancel** stops them. The state is saved after each scene and each picture in `.orbit/ai/studio.json` inside the project: running a step again resumes where it stopped, also after closing the editor.

## Text model

Any server with an OpenAI-compatible `chat/completions` API works: Ollama, LM Studio, llama.cpp server, KoboldCpp, vLLM, OpenRouter, OpenAI... You choose the model; the studio adds no filtering of its own. Set it in the **Providers** tab: URL (up to `/v1`), model, key (empty for local servers), temperature, max tokens and the number of repairs allowed.

### Example: Ollama (local)

1. Install Ollama, then pull a model: `ollama pull llama3.1` (any model you like; a 12B+ model writes noticeably better scripts).
2. URL `http://localhost:11434/v1`, model `llama3.1`, no key.
3. **Test connection / list models** lists the models the server has.

Reasoning models are fine: their `<think>` section is removed from the answers.

## Image generation

Choose the backend in **Providers**:

- **Placeholder**: no generation, colored stand-ins (replace them later in the library).
- **ComfyUI**: runs a workflow in *API format* (in ComfyUI: *Workflow > Export (API)*). The default one is a plain txt2img workflow. String values may use `{{prompt}}`, `{{negative}}`, `{{model}}`, `{{sampler}}`, `{{scheduler}}`; a value written exactly `"{{seed}}"`, `"{{width}}"`, `"{{height}}"`, `"{{steps}}"` or `"{{cfg}}"` becomes a number. LoRA loaders, the VAE and the clip skip are added to the workflow automatically after its checkpoint loader.
- **AUTOMATIC1111 / Forge**: `/sdapi/v1/txt2img` (start the web UI with `--api`).
- **OpenAI-compatible**: `images/generations`.

Common settings: style prefix (or the *art style* of the pitch), negative prompt, background and sprite sizes, base seed, pictures generated at once. Each character derives a fixed seed from the base seed, so it keeps the same look across its expressions. Sprites are asked on a plain white background, which the studio turns transparent (option).

### Example: ComfyUI (local)

1. Start ComfyUI (default address `http://127.0.0.1:8188`).
2. Backend *ComfyUI*, URL `http://127.0.0.1:8188`.
3. In **SD models**, click **Read the models of the backend** and choose a checkpoint.

## Stable Diffusion models (SD models tab)

- **Model family**: *SD 1.5*, *SDXL*, *Pony*, *Illustrious / NoobAI* set suitable sizes, quality tags (`score_9, score_8_up...` for Pony), negative prompt and clip skip. You can change each value after.
- **Checkpoint, VAE, sampler, scheduler**: type a name or pick one from the lists read from the backend (A1111 / Forge: checkpoints, LoRAs, VAEs, embeddings, samplers, schedulers; ComfyUI: the choices of its loader and sampler nodes). Sampler names in the A1111 style (`DPM++ 2M Karras`) are translated for ComfyUI.
- **LoRAs for every picture**: name, weight and trigger words. With A1111 the studio writes `<lora:name:weight>` in the prompt; with ComfyUI it adds `LoraLoader` nodes. Trigger words are added to the prompts automatically.
- **Embeddings**: write their name in the quality tags, the style prefix or the negative prompt.
- **Character consistency** (Pictures tab): per character, a fixed seed and / or a character LoRA (for instance one trained on that character).

## Civitai (optional)

In the **SD models** tab, search Civitai models by name and type (Checkpoint, LORA, TextualInversion, VAE). Results show the name, type, base model, a preview and the trigger words.

- **Download folders**: set the model folders of your backend, e.g. `ComfyUI/models/checkpoints`, `ComfyUI/models/loras`, `ComfyUI/models/embeddings`, `ComfyUI/models/vae`, or for A1111 / Forge `models/Stable-diffusion`, `models/Lora`, `embeddings`, `models/VAE`.
- **Download** saves the file there, with progress and cancel in the status bar. The file is checked against the SHA-256 published by Civitai (a corrupt download is removed) and skipped if it is already present. The backend is then asked to refresh its model list (A1111 / Forge; ComfyUI sees new files by itself).
- **Use this LoRA** adds a LoRA to the list with its trigger words.
- **API token**: some models need one (Civitai account settings). It is stored with your user settings.
- **Show NSFW**: a display filter for the results and previews, off by default.

Respect the license of each model you download.

## Privacy and files

- Provider settings, keys and the Civitai token are stored in your user configuration folder (`ai_studio.ron` next to the editor's `config.ron`), never in the project.
- The pipeline state is in the project's `.orbit/ai/` folder, which exports never include.
- Plain `http://` addresses (local servers) are reached directly. `https://` addresses (OpenRouter, OpenAI, Civitai) use the system `curl` program, present on Windows 10 and later, macOS and Linux.

## For AI assistants (MCP)

With the editor's MCP server on, an assistant also gets the studio's tools: `orbit.ai_studio.dsl_guide` (the script grammar, an example, the bible schema and the file naming), `orbit.ai_studio.check_bible` and `orbit.ai_studio.check_scene` (the same checks as the wizard). It then writes the scenes with `apply_dsl` and the other built-in tools.
