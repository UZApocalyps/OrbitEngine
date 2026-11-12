# Variables without code

A **variable** remembers something about the story: how much money the player has, whether they met a character, which route they chose. You never have to write code to use one.

## Declaring variables

Open the **Variables** panel and add a variable with a name, a type (number, text, yes/no) and a starting value. Choose its **scope**:

- **Game**: part of the player's saved game; starts again with a new game.
- **Persistent**: remembered across games (for example "has seen the true ending").

## Changing a variable

Add a **Set a variable** step and pick an operation from the menu: set to a value, add, subtract, multiply, divide, toggle (yes/no). The Inspector shows the operation in plain words, so no formula is needed. **Expression mode** lets you type a formula instead (an expression that can use other variables) and **Back to visual mode** returns to the menus. A clickable zone can also change variables when clicked, with the same operations (see *Writing the story*).

## Reacting to a variable

- Add a **Condition** (if) step: choose the variable, the comparison (equal, greater than...) and where to go when it is true and when it is false.
- Give a **choice option** a condition so that it only appears when it applies. Make sure at least one option is always available; the Problems panel warns you otherwise.

## Showing a variable

Custom screens (see the Interface chapter) can display a variable in a text, for instance a score or a name.

Variables are checked for you: using a name that does not exist, or mixing text and numbers, appears in the Problems panel.

## Lists and tables

A variable can also hold a **list** (an ordered collection: the objects of an inventory) or a **table** (entries with a key: the affinity of each character). Their content is edited in the Variables panel (**+ item**, **+ entry**). Operations: add to the list, remove a value, remove at a position, empty it, set or remove the entry of a key. In a condition you can test whether it **contains** a value, **has the key**, or compare its **size**. A screen can list their content with a generated list (see the *Interface* chapter).

## Building conditions

A condition (Condition step, choice option, clickable zone, widget) is built from rows: a variable, a comparison (equals, is greater than, is true, contains...) and a value. Rows are grouped by **All of these are true** or **At least one is true**, and **Add a group** nests groups. For a condition that the rows cannot express, switch to a typed expression (`affinity > 2 && !seen`) or tick **Rhai** (see *Scripting*).
