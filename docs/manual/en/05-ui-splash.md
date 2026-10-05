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

## Gallery and scene replay

The stock interface has two more screens, opened from the main menu: **Gallery** and **Replay**.

* **What unlocks them.** Every image shown as a background in a played game is unlocked for the gallery, and every scene the player enters is "seen" and can be replayed. This progress is kept across playthroughs and survives deleting the saves (it lives with the persistent variables). The editor preview never records it.
* **Gallery.** Unlocked images are shown as thumbnails, locked ones as a "Locked" card.
* **Replay.** The seen scenes are listed in story order; picking one plays that scene alone. It starts with the variables the scene had the first time the player entered it (the declared initial values when that is unknown), and nothing it does is kept: no save, quicksave or autosave, no change to persistent variables or to the unlocks. When the scene ends (or the player goes to the main menu) the game comes back to the Replay screen. A scene the story *calls* runs normally; jumping to another scene ends the replay.

### Choosing what the gallery and the replay menu offer

In the **Interface** editor, select the generated list of the `gallery` (or `replay`) screen. Its **List** can be **Gallery images** or **Replayable scenes**; tick the images or scenes to offer (none ticked = every background of the story / every scene). This choice is stored in the screen, not in the story.

To build your own screens: in a generated list, an **Image** with **Image of the row** shows the row's image; the condition **the row's element is set** tells an unlocked image from a locked one; a button with the action **Replay a scene** (ticked **Scene of the row**) replays the row's scene. "Replay a scene" can also target a fixed scene. A replay can only start from the title.


## Achievements

Define achievements in the **Achievements** panel: a **title**, a **description**, an optional **icon** (an image of the project) and a **Hidden until unlocked** flag. A hidden achievement shows a neutral placeholder (title, description and icon) in the list until it is unlocked. Titles and descriptions are ordinary texts of the story: they are translated in the **Translations** panel like any other text.

Unlock one from the story with the **Unlock achievement** step (Logic group), or with the button *Add an unlock node here* of the panel. The first time the story reaches the step in a real playthrough, the achievement is unlocked for good and a **notification** announces it ("Achievement unlocked: First words"). Afterwards the step does nothing. In particular:

* rolling back and playing the line again does **not** show the notification a second time (an unlock is not part of the saved game: it is never taken back);
* a **scene replay** never unlocks anything and never writes anything;
* the unlocked achievements are kept with the persistent variables (`persistent.ron`): they survive deleting the saves and starting a new game. The editor preview never records them.

The stock interface gains an **Achievements** screen, opened from the main menu (the button only appears when the story defines achievements): a progress line ("3 / 10") and one card per achievement, with its icon, title, description and a "Locked" mention until unlocked.

To build your own screen, use in the **Interface** editor a generated list whose **List** is **Achievements**. Inside a row, bind a text to **Achievement of the row** (title, description) or an **Image** to its **icon**; the condition **the row's achievement is unlocked** tells the unlocked ones apart; **progress (3 / 10)** works anywhere, and the condition **the story has achievements** hides a button when there are none.

Platforms (Steam, itch.io, consoles) are reached through the `AchievementHook` trait of `orbit_runtime` (`RuntimeConfig::achievement_hook`): it is called with the stable id of the achievement when it is unlocked, and once at start-up for the ones the player already has. No platform is wired yet (Steam is on the backlog); without a hook only the persistent data is written.
