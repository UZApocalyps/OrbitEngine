# Importing a Ren'Py game

OrbitEngine can turn a Ren'Py game into a new project, so that you do not start from scratch when you switch engines. The original game is never modified.

## How to import

- In the editor: **File > Import from Ren'Py…** (also in the **New project** dialog). Pick the Ren'Py game folder (the folder containing `game/`, or `game/` itself), then the folder that will contain the new project. The import runs in the background; the project opens when it is done and the import report is shown.
- From the command line: `orbit import-renpy <renpy_dir> <out_dir>`. The output folder must be empty or absent.

The report is also saved as `import-report.md` in the new project: counts, every statement that could not be converted (with its file and line), and every approximation.

## What is converted

| Ren'Py | OrbitEngine |
|---|---|
| `label start:` and the other top-level labels | one scene per label, `start` first |
| nested labels, `label .local:` | labels inside their scene |
| `jump`, `call`, `return` | jumps, calls and returns (a `return` outside a called label ends the game) |
| falling into the next label | a jump to that scene |
| `menu:` with choices and `if` conditions | a choice node; the caption becomes a line before it |
| `if` / `elif` / `else`, `while` | conditions and labels |
| `define e = Character("Eileen", color="#…")` | a character with its color |
| `default` / `define` with a literal (bool, number, text, list, dict) | a variable; `persistent.x` becomes the persistent variable `persistent_x` |
| `$ x = …`, `$ x += …` (also `-=`, `*=`, `/=`), `$ list.append(…)` | a variable change |
| `e "…"`, `"…"`, `"Name" "…"`, `extend "…"` | dialogue lines |
| `e happy "…"` | the sprite changes expression, then the line |
| `show eileen happy at left with dissolve`, `hide` | show / hide with position and transition |
| `scene bg room with fade`, `scene black` | background (the sprites leave the stage) |
| `with dissolve`, `fade`, `Dissolve(t)`, `Fade(a, b, c)`, `pixellate`, `vpunch` / `hpunch` | dissolve, fade, a camera shake |
| `play music … fadein 1.0`, `stop music fadeout 2.0`, `play sound`, `play voice`, `voice` | music, sounds, voice of the next line |
| `pause 1.0` | a wait |
| text tags `{b}`, `{i}`, `{color=…}`, `{w}`, `{p}` | the same markup |
| `images/eileen happy.png`, `image bg room = "room.jpg"` | the expression `happy` of Eileen, a background |
| `tl/french/*.rpy` (`strings` blocks and dialogue blocks) | the French translation of the project |

## What is not converted

- Screens, styles, transforms and ATL, layered images, the `gui/` folder: OrbitEngine screens are made in the UI editor.
- `init python` blocks and Python that is not a simple assignment: kept as a commented script node and listed in the report, so you can rewrite them (in Rhai, see the Scripting chapter).
- Conditions the engine syntax cannot hold are converted to Rhai conditions: check them.
- `[variable]` interpolation in texts stays as written; other text tags (`{size}`, `{font}`, `{a}`...) are removed.
- NVL mode (lines go to the normal text box), `pause` without a duration (one second), sound loops and other channels than music.
- WebP / AVIF images and MP3 / Opus audio: convert them to PNG / OGG, then import them in the Library.

After an import, open the **Problems** panel: the project validates, but warnings point at places worth a look.
