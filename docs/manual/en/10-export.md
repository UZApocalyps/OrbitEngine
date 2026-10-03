# Exporting your game

An export produces a game that players run **without installing anything**: a `.zip` for Windows, a `.tar.gz` for Linux, or a folder to put on a website.

## From the editor

Choose **File > Export game** to open the Export panel.

1. **Export profile**: the game name, an **identifier** (the folder where players' saves are kept: lowercase letters, digits, `-` and `_`; do not change it after publishing), the version, an icon (a square PNG, 256x256 recommended), the default language and the compression. Save the profiles to keep them with the project; you can have several (for example `demo` and `release`).
2. **Targets**: Windows, Linux and Web. A target whose player is not found is flagged.
3. **Output folder**: `export/` in the project by default.
4. **Export**: a progress bar and a log show each step and any warnings (unused assets, incomplete translations). **Cancel** stops it. The story is saved first, and errors in the Problems panel block the export.
5. When it is done, open the folder, and for the web use **Test in the browser**.

## What the player gets

- **Windows**: a single `.exe` with the game inside, and a short README.
- **Linux**: a single executable; a graphics driver (Vulkan or OpenGL) is needed.
- **Web**: a folder to upload as is to any static host (itch.io, GitHub Pages, Netlify...). The game loads scene by scene so it starts fast. Fullscreen is in the preferences or on F11. The game is saved when the tab is hidden.

Saves are stored under the game identifier in the player's user data folder (the browser storage for the web version).

## Android

The game player has an Android version, with touch controls and automatic saving when the app is paused. Exporting an Android package directly from the editor is not available yet.

## Command line

The `orbit` tool does the same from a terminal: `orbit export windows --project my_game --out dist`, also `linux`, `web` and `all`.
