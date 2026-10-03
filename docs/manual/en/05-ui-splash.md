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
