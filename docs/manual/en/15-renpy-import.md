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
| `define e = Character("Eileen", color="#…", what_color="#…", what_prefix='"', what_suffix='"', what_italic=True, image="eileen")` | a character with its color; the text color, prefix / suffix, bold / italic are written into each of its lines (and into their translations) |
| `default` / `define` with a literal (bool, number, text, list, dict) | a variable; `persistent.x` becomes the persistent variable `persistent_x` (also when it is only assigned in a label, or only tested: then a `false` flag) |
| `$ x = …`, `$ x += …` (also `-=`, `*=`, `/=`), `$ list.append(…)`, `python:` blocks of such lines | a variable change |
| `e "…"`, `"…"`, `"Name" "…"` | dialogue lines |
| `extend "…"` | after a `{nw}` line, one line; otherwise a second line repeating the first one followed by the extension |
| `e happy "…"` | the sprite changes expression, then the line |
| `show eileen happy at left with dissolve`, `hide` | show / hide with position and transition |
| `at left` / `right` / `center` / `truecenter` / `top…` / `offscreenleft` / `offscreenright`, `at Transform(xalign=…, zoom=…)`, `at Position(xpos=…)` | a stage position (built in or free) and a scale; `xzoom -1` mirrors the sprite |
| `transform name:` that only places the sprite (`xalign`, `yalign`, `xpos`, `ypos`, `align`, `pos`) | a named stage position |
| ATL (`transform` blocks, `show x:` blocks, `at a, b`): `linear` / `ease` / `easein` / `easeout` (and the `_quad`, `_cubic`… variants), `pause`, `repeat [n]`, with `xalign`, `yalign`, `xpos`, `ypos`, `xoffset`, `yoffset`, `alpha`, `zoom`, `xzoom`, `yzoom`, `rotate` | the final staging of the `Show`, plus a timeline after it (offset, scale, opacity, rotation tracks, looping for `repeat`) that does not block the story |
| `scene bg room with fade`, `scene black` | background (the sprites leave the stage) |
| `with dissolve`, `fade`, `Dissolve(t)` (with its `time_warp`), `Fade(a, b, c, color=…)`, `vpunch` / `hpunch`, `with Pause(t)` | dissolve, fade (in its color), a camera shake, a wait |
| `pixellate`, `ImageDissolve`, `MoveTransition`, `move…`, `wipe…`, `slide…`, `push…`, `iris…`, `zoom…` | a dissolve (reported once per transition) |
| `play music … fadein 1.0`, `stop music fadeout 2.0`, `play sound`, `play voice`, `voice`, `$ renpy.music.play(…)`, `renpy.music.stop(…)`, `renpy.sound.play(…)`, `renpy.play(…)` | music, sounds, voice of the next line |
| `pause 1.0`, `$ renpy.pause(1.0)` | a wait |
| `$ renpy.jump("x")`, `renpy.call("x")`, `renpy.call_screen("x")` | a jump, a call, the screen below |
| `achievement.register("x")`, `$ achievement.grant("x")` | an achievement and its unlock node |
| text tags `{b}`, `{i}`, `{color=…}`, `{w}`, `{p}` (`{nw}` and `{fast}` are dropped) | the same markup |
| `images/eileen happy.png`, `image bg room = "room.jpg"`, `Image(…)`, `im.Scale(…)` | the expression `happy` of Eileen, a background |
| `image black = Solid("#000")` (or `"#000"`) | a generated picture of that color (with its transparency) |
| `image fire:` with only pictures, `pause` and `repeat` | a frame animation of the expression |
| `Composite(…)`, `ConditionSwitch(…)`, `layeredimage` | best effort: the first layer, the default branch, the base layer (reported) |
| a screen with an `imagemap` (`ground`, `hover`, `hotspot (x, y, w, h) action …`) or `imagebutton`s, used by `call screen` / `show screen` | a point & click node (`Hotspots`): the ground is the background, the zones are the hotspots, `hover` their hover picture; `if` around a hotspot becomes its condition, `sensitive` its enabled state |
| a screen of `textbutton`s used the same way | a choice |
| button actions `Jump("x")`, `Call("x")`, `Return()`, `SetVariable("x", v)`, `SetField(persistent, "x", v)`, `ToggleVariable("x")`, `Notify("text")`, and lists of them | the block that follows each zone / option |
| `$ renpy.notify("…")` (also inside a `python:` block) and the screen action `Notify("…")` | a Notify node (a toast: text converted like dialogue, default style) |
| `tl/french/*.rpy` (`strings` blocks and dialogue blocks) | the French translation of the project |

Positions: the engine anchors a sprite by its bottom-center point, so `xalign a` becomes `x = 0.25 + 0.5 a` (the built-in `left`, `center` and `right` of both engines then match) and `yalign b` becomes `y = 0.5 + 0.5 b`; `xpos` / `ypos` place the anchor directly (pixels are divided by the screen size read from `gui.init(…)` or `config.screen_width`). Image button zones take the size of their picture (PNG / JPEG header).

## What is not converted

- Interface screens (say, menus, save / load, preferences…), styles, screens that are not navigation, the `gui/` folder: OrbitEngine screens are made in the UI editor. They are reported once, not line by line.
- Transforms with parameters, ATL `parallel`, `choice`, `on`, `function`, `contains`, `crop` and other properties: reported with their place.
- Image button pictures are not drawn (the zone glows over the background); layers of composite and layered images are not combined.
- `init python` blocks and Python that is not a simple assignment or one of the calls above: kept as a commented script node and listed in the report, so you can rewrite them (in Rhai, see the Scripting chapter).
- Conditions the engine syntax cannot hold are converted to Rhai conditions: check them.
- `[variable]` interpolation in texts stays as written; other text tags (`{size}`, `{font}`, `{a}`...) are removed (reported once per tag, with the number of lines).
- NVL mode (lines go to the normal text box, `nvl clear` is ignored), side images, `window show / hide / auto`, `pause` without a duration (one second), sound loops and other channels than music.
- WebP / AVIF images and MP3 / Opus audio: convert them to PNG / OGG, then import them in the Library.

After an import, open the **Problems** panel: the project validates, but warnings point at places worth a look.
