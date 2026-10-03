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

## Working with it

- Edit the text freely, then press **Apply** (Ctrl+Enter). The whole change is one undo step.
- Mistakes are shown on the line where they are, and in the Problems panel. Nothing is applied while the text has errors.
- **Reload** throws away your typing and shows the current story again.
- Small discreet marks at the end of lines (`#@...`) let the editor recognize each step again; leave them in place when you move lines around.

The text is only a view: the story itself is not stored as text, so you can switch between the Text view and the other editors at any time.
