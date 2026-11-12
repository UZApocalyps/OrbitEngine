# Interface editor and splash screens

The menus of your game (main menu, pause, save, load, preferences) are **screens** that you can restyle without code. They live in the `ui/` folder of your project. Without that folder, the game uses stock screens.

## The Interface editor

Open the **Interface** panel. It has five tabs:

- **Screens**: pick a screen and edit it. Add buttons, texts, images and containers, move them, and set what a button does (start the game, open another screen, quit...). A live preview of the real game interface is shown next to the editor.
- **Theme**: fonts and the named **styles** shared by all screens, in a visual style editor.
- **Dialogue box**: the textbox, speaker name, choices and quick menu shown during play, and the choice between the classic box and comic bubbles.
- **Notifications**: where and how the toasts of the game appear.
- **Splashes**: the screens shown when the game starts.

The **Visual novel starter** template already contains a full set of screens to adapt. Changes are saved into the `ui/` folder; use undo and redo as usual. The Interface panel lists problems (for example a button that points to a screen that does not exist).

## Styles

A **style** is a named look (background, border, shadow, padding, font, text color, sounds) that any widget can use. Select a style in the **Theme** tab to edit it:

- The **preview** at the top shows a button, a label, a panel, a slider, a toggle and a dropdown drawn with the style. The **state tabs** (Normal, Hover, Pressed, Disabled, Focused) switch both the preview and the fields you edit: each state can have its own background and text color. A state you leave empty uses the hover look (pressed and focused) or the normal one.
- **Background**: none, a color, a vertical **gradient**, an **image** (stretched, fitted or covering) or a **9-slice** image. For a 9-slice, drag the four guides over the picture to choose the borders that keep their size (the corners stay sharp, the middle stretches). Colors have an alpha channel.
- **Box**: border color and width, drop **shadow** (color and offset), inner padding. Margins are set on each widget (Layout section).
- **Text**: font, size, bold, alignment and the color of the current state. Turn **Show the text** off for an image-only button (screen readers still read the text).
- **Sounds** played on hover and on click.
- **Extends**: a style can start from another one. Fields the style changes are highlighted; the others show the parent's value, marked *inherited*. The reset button next to a field gives it back to the parent.
- **Duplicate**, rename or delete a style from its header. **Used by** lists the widgets, styles and dialogue box parts that use it (click one to go there), and **Apply to** gives the style to the widget selected in the Screens tab.

Older projects keep their look. A style made with an earlier version shows a note offering to **convert** its background to per-state backgrounds; the conversion keeps every state identical.

### Image buttons and clickable images

To make a button out of pictures, give its style (or the button's own **local changes**, at the bottom of its properties) an image background per state: one picture for normal, one for hover, one for pressed... Keep the text on top, or turn **Show the text** off for an image-only button. An **Image** widget becomes clickable as soon as you give it **actions on click**; its style can then give it hover and pressed backgrounds too.

## The dialogue box

The **Dialogue box** tab styles what the player sees during the story, with a live preview drawn by the game itself (choose **A line** or **Choices**, and type a sample speaker and line):

- **Textbox**: a fixed position or a placement anchored to the screen (bottom, top, center... plus an offset), its size, padding, text offset, text speed, and a **box style** for its background (image or 9-slice included), border, shadow, font and text color.
- **Speaker name**: size and style, in the character's color or the style's color, optionally on a **name plate** with its own style, offset and size (a width of 0 fits the name).
- **"Continue" indicator**: its size, color or image, and whether it blinks.
- **Choices**: button size, spacing, placement, number prefixes, the screen dimming, and a **button style** whose hover look marks the selected choice.
- **Quick menu**: placement, button style and width, spacing, or hide it.

The **+** button next to a style field creates a style from the current colors and links it; the pencil opens it in the Theme tab. A project made before these options keeps exactly the same dialogue box.

### Comic bubbles

Under **Dialogue display**, choose the **Display mode**: **Classic box** (the default) or **Comic bubbles**. In bubble mode the lines of characters appear in speech bubbles placed near the speaker's face, with a tail pointing to the head; the narrator keeps the box. The preview of the tab shows the result.

- A line is a speech bubble by default. Start it with `{bubble=thought}` for a thought cloud or `{bubble=shout}` for a shout (a jagged burst).
- **Speech bubbles** settings: fill and outline colors, outline width, corner radius, minimum and maximum width, font, whether the speaker's name is shown and its size, tail length and width, gap to the head, screen margin, the pop-in animation, and an outline in the character's color.
- **Narrator**: keep the classic box, or show a **Caption** (top left) with its own color.
- **Speaker off screen**: a bubble without a tail, or the classic box.
- **Per character**: bubble colors and the position of the head (an offset from the top of the character's picture), so the tail points at the right place.

## Music in the menus

A screen can play its own music. In the **Screens** tab, select the screen (nothing selected in its tree, or the root) and tick **Screen music** in its properties:

- **Audio file**: a music or sound file of the project. A warning appears when the file is not there. The track is packed in the exported game.
- **Volume**, **Fade in / out** (milliseconds) and **Loop the track**.
- **Keep playing if the next screen has the same track**: opening the options from the main menu with the same track does not restart it. Untick it to restart the track on that screen.
- **Over the game**: when the screen opens during play (pause menu, save, load...), choose **Replace the game music** (the menu track plays; **Put the game music back on close** restarts the story's current track when the menus close) or **Lower the game music** (the game's track keeps playing at the chosen volume, and the screen plays no track of its own).

The music of the screen set as **main menu** plays from the start screen. Starting or loading a game fades it out. When several screens are open, the topmost one that has music decides; a screen without music lets the one below keep playing. Returning to the title starts the main menu music again.

Buttons have two more actions: **Play music** (a file, looped, replacing the music; it lasts until another screen with music is shown) and **Stop music**. In the editor, the live preview plays the music of the screen shown; the preview's mute button silences it.

## Save and load slots

The **Save slots** widget shows the saves of the game as a grid of cards with category tabs, page buttons and a delete button under each card. By default it uses the stock card (thumbnail, slot name, date and last line). You can restyle everything:

- **Look of the slots** (in its properties): a **style** for the cards, for the empty cards, for the incompatible ones, for the tabs, the page buttons, the delete button, the confirmation dialog and its Yes / No buttons; the **card height** and the **gaps** between columns and rows, next to the **columns** and **rows**. Hover, pressed, focus and disabled looks come from those styles (see *Styles*).
- **Customize the card** writes the stock card as widgets: the children of the Save slots widget are the inside of one card, repeated for every slot. Edit them like any other widgets: move the thumbnail, add the play time, change fonts and colors. The look does not change until you edit something. **Back to the stock card** removes them.
- Inside the card, bind a **text** to a **slot field**: title (Slot 3), number, date and chapter, date and time, play time, chapter (scene), last line, status (Empty / Incompatible); bind an **Image** to the **thumbnail**. Use the conditions **the slot is empty**, **the slot has a save** and **the save is incompatible** to show a widget only in one state (for example an "Empty" text, or a padlock icon on incompatible saves).
- The play time and the chapter are empty for a save that does not record them.

A project made before this keeps the stock cards exactly as they were.

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

**When the rows do not fit.** Under **If the rows do not fit**, choose how a long list behaves: **Scroll** (mouse wheel, drag, keyboard and game controller, with a scrollbar), **Pages** (Previous / Next buttons and a page counter) or **Nothing (may overflow)**. Give the list a *Fill* or *Fixed* height so that it knows how much room it has. The stock Gallery, Achievements and Replay screens scroll by row, or split into pages, when their cards no longer fit on the screen. A **Scroll list** container does the same for any group of widgets.

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

Define achievements in the **Achievements** panel: a **title**, a **description**, an optional **icon** (an image of the project), an optional **Locked icon** (shown instead of the icon until the achievement is unlocked; without one, the icon is used) and a **Hidden until unlocked** flag. A hidden achievement shows a neutral placeholder (title, description and icon) in the list until it is unlocked. Titles and descriptions are ordinary texts of the story: they are translated in the **Translations** panel like any other text.

Unlock one from the story with the **Unlock achievement** step (Logic group), or with the button *Add an unlock node here* of the panel. The first time the story reaches the step in a real playthrough, the achievement is unlocked for good and a **notification** announces it ("Achievement unlocked: First words"). Afterwards the step does nothing. In particular:

* rolling back and playing the line again does **not** show the notification a second time (an unlock is not part of the saved game: it is never taken back);
* a **scene replay** never unlocks anything and never writes anything;
* the unlocked achievements are kept with the persistent variables (`persistent.ron`): they survive deleting the saves and starting a new game. The editor preview never records them.

The stock interface gains an **Achievements** screen, opened from the main menu (the button only appears when the story defines achievements): a progress line ("3 / 10") and one card per achievement, with its icon, title, description and a "Locked" mention until unlocked.

To build your own screen, use in the **Interface** editor a generated list whose **List** is **Achievements**. Inside a row, bind a text to **Achievement of the row** (title, description) or an **Image** to its **icon**; the condition **the row's achievement is unlocked** tells the unlocked ones apart; **progress (3 / 10)** works anywhere, and the condition **the story has achievements** hides a button when there are none.

Platforms (Steam, itch.io, consoles) are reached through the `AchievementHook` trait of `orbit_runtime` (`RuntimeConfig::achievement_hook`): it is called with the stable id of the achievement when it is unlocked, and once at start-up for the ones the player already has. No platform is wired yet (Steam is on the backlog); without a hook only the persistent data is written.

## Notifications

The **Notification (toast)** step (Dialogue group of the insert menu) shows a short message in a corner of the screen, a *toast*, and the story goes on at once: it never waits for the toast to disappear. Its fields:

* **Text**: markup is allowed, and `[variable]` is replaced by the value of the variable when the toast is shown. The text is translated like any other text of the story.
* **Icon** (optional): an image shown left of the text. **Sound** (optional): played with the toast. **Duration** (optional, in seconds): how long it stays. **Style** (optional): the name of a notification style of the theme; without one, the `default` style is used.

In the text format: `notify "Saved!" [icon x] [sound y] [style z] [duration n]`. In a Rhai script: `notify("text")`, `notify("text", "style")` or `notify("text", #{style: "system", icon: "bell", sound: "ping", duration: 4.0})`. A button can also do it with the action **Notify** (a text, and a style if needed).

Achievements use the `achievement` style, and the messages of the engine itself the `system` style: restyle them in the theme like any other.

Toasts are never saved. Rolling back, or loading a save, removes the toasts of the story that are on screen. In **Skip** mode a notification follows the **skip** setting below.

### The theme: `notifications`

The look is set in the **Interface** editor, **Notifications** tab:

* **Anchor** (top right by default) and **Offset** from that corner; **Width** (0 fits the text); **Max visible** (the others wait in a queue); **Gap** between two toasts and **Stacking direction** (Auto, Down or Up).
* **Duration** and **Animation**: Slide, Fade or None, with its length in seconds. When the player asks for reduced motion, Slide becomes Fade. **Default sound**, and **Click to dismiss**.
* **Skip behaviour**: Show, Brief (a shorter stay) or Suppress (nothing is shown while skipping).
* **Named styles**: each one has a background (a color, or a theme style such as a nine-slice picture), a border, a corner radius, a padding, an icon size and a default icon, the text size, color and font, and optionally its own width, duration and sound.

The preview has a **Show test notification** button to see the result without playing the story.
