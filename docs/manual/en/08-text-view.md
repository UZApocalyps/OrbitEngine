# Text view

The **Text view** panel shows a scene (or the whole story) as plain text that reads almost like a script. It is optional: the Scene editor and the Graph remain the main way to write. Some authors simply prefer typing.

## What it looks like

```
scene morning
label start
bg "assets/bg_dawn.png" with fade(1.0, "#000000")
"Dawn over the harbour town."
Mira "I almost did not come."
menu
    "Thank her warmly." -> thank
    "Say nothing." -> silent
$ friendship += 1
jump join
```

One step per line: a narrator line is just the text in quotes, a character line starts with the character name, `menu` introduces choices, `$` changes a variable, `jump` goes to a label, `bg`, `show`, `music` and `sound` set the stage and sounds, and `#` starts a comment.

## The other steps

The same one-line-per-step logic covers the rest of the story. A few lines, one per kind of step (voice, effects, screens, script, achievement, notification, 3D, point & click):

```
Mira "Hello." voice "assets/voice/hello.ogg"
shake screen 0.02 over 0.4
zoom 1.2 over 2.0 ease OutQuad wait
open screen "inventory" wait -> used_item
script "set_var(\"gold\", 5)" branch win -> victory on_error oops
unlock "First words"
notify "Saved!" icon "assets/ui/bell.png" style system duration 3.0
scene3d "assets/3d/room.scene3d.ron" camera "main"
hotspots bg "assets/hall.png"
    spot rect (0.1, 0.2, 0.25, 0.5) "The door" -> outside name door
        do set key = 1
        do stay
```

Zones are written in screen fractions (0 to 1): `rect (x, y, w, h)`, `ellipse (cx, cy, rx, ry)`, `poly (x, y) (x, y) ...` or `mask <picture> (x, y, w, h) [threshold f] [draw]` for a picture zone. A zone may also carry `hover`, `sound`, `style`, `enabled <condition>` and `if <condition>`, and its `do` lines are the click actions (`set`, `jump`, `scene`, `call`, `return`, `sound`, `notify`, `stay`). A Rhai condition is written `rhai "..."`. Particles, timelines and parallax backgrounds also have text forms; the Text view prints them, and the AI Studio uses the same language.

## Working with it

- Edit the text freely, then press **Apply** (Ctrl+Enter). The whole change is one undo step.
- Mistakes are shown on the line where they are, and in the Problems panel. Nothing is applied while the text has errors.
- **Reload** throws away your typing and shows the current story again.
- Small discreet marks at the end of lines (`#@...`) let the editor recognize each step again; leave them in place when you move lines around.

The text is only a view: the story itself is not stored as text, so you can switch between the Text view and the other editors at any time.
