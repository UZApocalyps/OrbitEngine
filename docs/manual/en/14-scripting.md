# Scripting (Rhai)

Scripts are optional. A whole game can be made without them. When the story graph gets too heavy for some logic (a shop, a dice game, a score with rules), you can add a little code in **Rhai**, a simple language that looks like JavaScript or Rust. Scripts run in a **sandbox**: they cannot touch files, the network or the clock, and they stop on their own if they run too long.

## The Script step

Insert a **Script** step (Logic group of the insert menu). Its inspector shows a code editor with syntax colouring. Errors appear under it and in the **Problems** panel with their line and column; click one to jump to it.

What a script can do:

- `get_var("gold")`: reads a variable of the project
- `has_var("gold")`: tells whether the variable exists
- `set_var("gold", 12)`: changes a variable (applied when the script ends)
- `rand_int(1, 6)`, `rand_float()`, `chance(0.3)`: random numbers from the game's own random generator
- `branch("win")`: chooses the output `win` (returning the text `"win"` does the same)
- `log("text")`: writes a message to the console (also `print`)
- `notify("text")`, `notify("text", "style")`, `notify("text", #{style: "system", icon: "bell", sound: "ping", duration: 4.0})`: shows a notification (toast) in the game, like the Notification step; like `set_var`, it only happens if the script ends without error

Variables are the ones you declare in the **Variables** panel; a script cannot create new ones. Their type is kept: an integer variable stays an integer.

### Outputs

A Script step has named **branches**, each going to a label, and an optional **error** label. If the script chooses a branch, the story jumps there; otherwise it goes on with the next step. If the script fails (an error, a wrong type, a limit reached) **nothing it did is applied** and the story goes to the error label, or simply to the next step, with a warning.

### Testing

The **Run in sandbox** button runs the script against the initial values of your variables and shows the branch it would take, the variables it would change (before and after), its messages and the random numbers it used. Nothing is changed in the project.

## Rhai conditions

The condition of a **Condition** step or of a choice option can be written in Rhai: tick **Rhai** next to the condition. A Rhai condition only reads variables (`get_var("gold") % 2 == 0 && get_var("round") < 3`); it cannot change them, draw random numbers or show notifications. If it fails it counts as false. In the text view it is written `rhai "…"`.

## Shared modules

Code used by several scripts goes in a **module**: a file `scripts/<name>.rhai` of the project, edited in the **Scripts** panel. A script uses it like this:

```rhai
import "dice" as dice;
dice::bet("higher")
```

The example `examples/stories/test_script.ron` (a dice duel in three rounds) and its module `examples/stories/scripts/dice.rhai` show a module, a script choosing its branch and Rhai conditions.

## Saves, rollback and determinism

A script keeps nothing of its own: everything that matters is in variables, and its random numbers come from the game's generator, which is saved. Loading a save or rolling back and playing again gives exactly the same result, on desktop and on the web.

## Limits

To protect players, a script stops with an error when it runs more than 100 000 operations, calls functions more than 32 levels deep, or builds a text larger than 64 KB or a list or map of more than 1 024 elements (nested lists and maps count together, and nest at most 64 deep). Modules import each other at most 8 deep and never in a cycle. Anonymous functions (`|x| x + 1`) cannot capture variables, and `curry` and `sleep` are not available.

## Export

The export packs the modules your scripts import. The web export uses a slightly larger player when the game has scripts (about 0.9 MB more, 230 KB once compressed); games without scripts keep the small one. Desktop and Android players always support scripts.

## For the assistant (MCP)

The tools `list_scripts`, `read_script`, `set_script`, `write_script_module`, `delete_script_module`, `check_scripts` and `test_script` read, edit, check and test scripts; `add_command` with `type: script` adds a Script step.
