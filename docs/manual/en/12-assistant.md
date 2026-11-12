# The MCP assistant

OrbitEngine can work with an AI assistant that edits your story alongside you. It uses **MCP** (Model Context Protocol), a standard way for an AI application to use tools. The assistant reads and changes your story with the same safe operations as the editor itself.

## In the editor

The editor's MCP server and the **AI Assistant** panel come with the official **MCP server** plugin (`orbit.mcp`): the base editor contains no network server and no async runtime, which keeps it lighter. Install the plugin from **Tools > Extensions > Plugins…** (see the "Editor plugins" chapter). Without it, the robot of the status bar is not shown and `--mcp-port` says the plugin is missing; the command-line server (below) still works. The port and token already saved are kept as they are.

Open the **Assistant** panel (Tools menu, Ctrl+Shift+A). It shows whether the connection is ready and gives the address to put into your AI application. While the editor is open, the assistant sees your project live: its changes appear immediately and can be undone with Ctrl+Z like your own.

## What the assistant can do

It can list and read scenes, create and rename scenes, add and edit lines and choices, place commands (backgrounds, characters, music, jumps, conditions, variable changes), read the problems list, and save. It cannot do anything that the editor refuses: a change that would break the story is rejected with an explanation, and nothing is half applied.

## Good to know

- Each request of the assistant is one entry in your undo history, labelled with the tool name.
- If you edit the same step at the same time, the assistant is told the step changed and tries again instead of overwriting your work.
- Deleting a scene or a step is marked as destructive, so your AI application can ask you before doing it.

## Without the editor

A command-line server (`orbit mcp --project my_game`) offers the same tools when the editor is closed. The technical documentation of the tools is in `docs/mcp.md` of the OrbitEngine repository.
