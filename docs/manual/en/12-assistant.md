# The MCP assistant

OrbitEngine can work with an AI assistant that edits your story alongside you. It uses **MCP** (Model Context Protocol), a standard way for an AI application to use tools. The assistant reads and changes your story with the same safe operations as the editor itself.

## In the editor

The editor's MCP server and the **AI Assistant** panel come with the official **MCP server** plugin (`orbit.mcp`): the base editor contains no network server and no async runtime, which keeps it lighter. The official distributions ship it with the editor and install it at the first launch; in a build you compiled yourself, install it from **Tools > Extensions > Plugins…** (see the "Editor plugins" chapter). Without it, the robot of the status bar is not shown and `--mcp-port` says the plugin is missing; the command-line server (below) still works. The port and token already saved are kept as they are.

Open the **Assistant** panel (Tools menu, Ctrl+Shift+A). It shows whether the connection is ready and gives the address to put into your AI application. While the editor is open, the assistant sees your project live: its changes appear immediately and can be undone with Ctrl+Z like your own.

## What the assistant can do

It cannot do anything that the editor refuses: a change that would break the story is rejected with an explanation, and nothing is half applied. The tools are grouped by subject:

- **Story**: list, read, create, rename and delete scenes; add and edit lines and choices; place any step with `add_command` (backgrounds, characters, music, sounds, transitions, waits, variable changes, conditions, labels, jumps, calls, **notifications**, scripts, screens...); move and delete steps; read the problems list; `validate`; `simulate` (play the story with a list of choices, without a graphics card, to check every branch); save, undo and redo.
- **Definitions**: characters and expressions, variables, assets and curves.
- **Point & click**: `add_hotspots` and `edit_hotspots` create and change clickable zones (rectangle, ellipse, polygon, picture zones), their conditions and their click actions.
- **Interface**: read and write screens, styles, the dialogue box (including the comic bubbles mode), the **notifications** settings and styles, splash screens, save slots and the music of screens.
- **Achievements**: list, create, edit and delete achievements and their icons.
- **Voice and translations**: import voice files in bulk (`import_voices`, with a dry run), and read the coverage and stale translations of each language (`translation_status`).
- **Scripts**: list, read, write and check Rhai scripts and modules, and test a script in the sandbox (see *Scripting*).
- **Rigs (2D skeleton animation)**: build and animate a rig from start to finish (create from a picture, bones, binding, slots, skins, IK, clips and keys), import PSD / Krita / OpenRaster layers, cut a picture into layers, duplicate and recenter rigs, and sample a pose to check it (see *Characters and animations*).
- **3D scenes** (experimental): list, read, create and edit scenes, and pose the skeleton of a 3D character. These tools only show when the experimental 3D switch is on.
- **Editor only** (live mode): play, step and choose in the preview, take a screenshot of it, select a step in the editor, take a picture of a 3D scene from one of its cameras, and `build_project` / `export_game` (marked destructive).
- **Plugins**: the tools declared by the game plugins and the editor plugins you installed (for example the AI Studio tools).

## Good to know

- Each request of the assistant is one entry in your undo history, labelled with the tool name.
- If you edit the same step at the same time, the assistant is told the step changed and tries again instead of overwriting your work.
- Deleting a scene or a step is marked as destructive, so your AI application can ask you before doing it.
- Some tools (rigs, 3D scenes) rewrite a file instead of going through the undo history when the file is not open in its editor; when it is open, the change goes through the editor and Ctrl+Z undoes it for rigs.

## Without the editor

A command-line server (`orbit mcp --project my_game`) offers the same tools when the editor is closed. The technical documentation of the tools is in `docs/mcp.md` of the OrbitEngine repository.
