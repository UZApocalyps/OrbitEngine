# Variables without code

A **variable** remembers something about the story: how much money the player has, whether they met a character, which route they chose. You never have to write code to use one.

## Declaring variables

Open the **Variables** panel and add a variable with a name, a type (number, text, yes/no) and a starting value. Choose its **scope**:

- **Game**: part of the player's saved game; starts again with a new game.
- **Persistent**: remembered across games (for example "has seen the true ending").

## Changing a variable

Add a **Set variable** step and pick an operation from the menu: set to a value, add, subtract, multiply, divide. The Inspector shows the operation in plain words, so no formula is needed.

## Reacting to a variable

- Add a **Condition** (if) step: choose the variable, the comparison (equal, greater than...) and where to go when it is true and when it is false.
- Give a **choice option** a condition so that it only appears when it applies. Make sure at least one option is always available; the Problems panel warns you otherwise.

## Showing a variable

Custom screens (see the Interface chapter) can display a variable in a text, for instance a score or a name.

Variables are checked for you: using a name that does not exist, or mixing text and numbers, appears in the Problems panel.
