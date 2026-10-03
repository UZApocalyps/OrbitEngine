# Interface editor and splash screens

The menus of your game (main menu, pause, save, load, preferences) are **screens** that you can restyle without code. They live in the `ui/` folder of your project. Without that folder, the game uses stock screens.

## The Interface editor

Open the **Interface** panel. It has three tabs:

- **Screens**: pick a screen and edit it. Add buttons, texts, images and containers, move them, and set what a button does (start the game, open another screen, quit...). A live preview of the real game interface is shown next to the editor.
- **Theme**: colors, fonts and the look shared by all screens.
- **Splashes**: the screens shown when the game starts.

The **Visual novel starter** template already contains a full set of screens to adapt. Changes are saved into the `ui/` folder; use undo and redo as usual. The Interface panel lists problems (for example a button that points to a screen that does not exist).

## Splash screens

When the game starts it first shows the OrbitEngine splash. This one is required and cannot be removed or changed. After it, you can add your own splashes: a text or an image of your project, with a duration, fade in and fade out times, a background color, and whether the player may skip it. Use **Preview splashes** to watch them.

## Screens opened by the story, HUD, generated lists and transitions

Menus are not the only screens. The story itself can open a screen of your project at any point: an inventory, a phone, a map, a score counter kept in a corner. These screens are made in the same Interface editor and support a few extra settings described here.

### The "Open screen" step

Add an **Open screen** step to a scene and pick one of the project's screens by its id. Three settings decide how it behaves:

- **Wait**: when checked, the story stops on this step until the player closes the screen. It then behaves exactly like a choice: a rollback reopens the screen, a save remembers that it is open, and skipping stops there because it is a decision of the player. When unchecked, the screen opens and the story carries on at once (the usual setting for a HUD).
- **Result**: a variable of the story that receives the value the screen closes with (see below). Leave it empty if the screen returns nothing.
- **Parameters**: a list of names, each with a literal or a variable. The screen shows them through the **Screen parameter** binding, so a single "phone" screen can show different contacts depending on where it is opened.

Opening a screen that is already open replaces it. If the id does not exist in the project, the game closes the screen automatically and the **Problems** panel reports it, so the story never stays stuck.

### The "Close screen" step

**Close screen** closes a screen opened by the story, typically a HUD that is no longer needed. Nothing happens if the screen is not open.

### Closing with a value

The **Close** action of a button has an optional **return value**: a literal, a variable, the row element, the row index, the row key (see generated lists) or a screen parameter. When the screen was opened with a **Result** variable, that value is stored into it. Closing with Escape or through the screen's cancel actions returns no value: the variable keeps its previous content. A screen opened by the story that has no cancel actions lets Escape open the pause menu instead.

### HUD screens

Check the **HUD** property of a screen to make it a heads-up display. A HUD is drawn under the textbox, never pauses the game and never takes keyboard focus. Clicks outside its buttons go through to the game. It is hidden while the interface is hidden and whenever a menu (pause, save...) is open. Several HUDs can be open at the same time.

### Generated lists

The **Repeat** widget builds a list from a list or map variable of the story, or from a screen parameter. Its children are the **template of one row**: they are copied once per element. Inside the template, use the bindings **Item** (the element itself), **Item index** (its position, from 0) and **Item key** (for maps). Actions inside a row can use the row element too, for example "use this object" as a button that closes the screen with the item, or removes it from the inventory variable.

The **empty** text is shown when the collection has no element. Choose the **direction** (vertical, horizontal or a grid with a number of columns). A list shows at most 500 rows; a map lists its entries in key order.

### Screen transitions

Each screen can have an **opening transition** and a **closing transition**: fade, slide up, down, left or right, or scale, with a duration and an easing curve (the named curves of your project are accepted). Transitions are purely visual: the screen is open or closed immediately, only its drawing is animated. Rollback, loading a save and the **reduced motion** accessibility preference cut them short.

### The demo example

The demo story (`examples/stories`) uses both kinds of screen:

- When Alice leaves the shop, an **Open screen** step with **Wait** opens the `inventory` screen with the parameter `owner` set to "Alice". The screen repeats the `inventory` list; each button closes the screen with its item, which lands in the `used_item` variable. The **Back** button and Escape close without a value. It slides up when opening and fades out when closing.
- At the start of the game, an **Open screen** step without **Wait** opens the `status` HUD: a small panel in the top-right corner showing the `credits` variable, which keeps updating as the story changes it.
