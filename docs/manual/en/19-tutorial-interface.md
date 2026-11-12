# Tutorial: customize the game interface

The menus of your game, the dialogue box and the notifications are all drawn from screens and styles that you edit without code. This tutorial gives your game its own look: a theme, a restyled dialogue box, image buttons, a main menu with music, nicer save slots and your own notification style.

## Goal

Replace the stock look with a coherent one, and see every change live in the Preview panel.

## Before you start

- A project with a few images in its library: a button picture (one for normal, one for hover is ideal), a panel or frame picture, and a music file. The **Visual novel starter** template is a good base, because it already has a full set of screens.
- *Interface editor and splash screens* explains each notion in detail; this tutorial only walks through them in order.

## Steps

### 1. Open the Interface editor

1. Choose **Tools > Interface editor**. The **Interface** panel opens, with the live **Preview** next to it.
2. If the panel says **Custom interface is off**, click **Enable custom interface**. The default screens and theme are copied into the `ui/` folder of the project, ready to edit.
3. Notice the five tabs: **Screens**, **Theme**, **Dialogue box**, **Notifications** and **Splashes**. Press **Save** at the top whenever you want to write your work into `ui/`; the **Undo (interface)** and **Redo (interface)** buttons undo your edits in the editor.

### 2. Create a theme and your first style

1. Open the **Theme** tab. If you see *No theme file*, click **Create the theme**.
2. Type a name in the field **New style name**, for example `primary`, and click **Add a style**.
3. In the style editor, switch between the state tabs **Normal**, **Hover**, **Pressed**, **Disabled** and **Focused**: the preview at the top shows the same state.
4. Under the background, pick a color, a **Gradient** or an **Image**. For a frame that keeps sharp corners at any size choose the **9-slice image**, then drag the four guides on the picture.
5. In **Text**, pick the font, the size and the color of each state. Use **Fonts** > **Add a project font** first if you imported a TTF or OTF file in the library.
6. Make a second style that starts from the first: add `primary_big`, set **Extends** to `primary`, and change only the size. The fields it changes are highlighted, the others are marked *inherited*.

### 3. Image buttons

1. Create a style `button_image`. For each state tab (**Normal**, **Hover**, **Pressed**) set the background to **Image** and pick the matching picture. A state left empty uses the hover look, or the normal one.
2. If the picture already contains the label, turn **Show the text** off: the button becomes image-only, and screen readers still read the text.
3. In the **Screens** tab, select a screen, then a button in its **Widgets** tree, and click **Apply to** (the name of the widget) in the style editor to give it the style. You can also choose the style in the **Style** field of the widget, or set images on the button alone with **Local overrides**.
4. To make a decoration clickable, add an **Image** widget and give it at least one action in **Actions on click**.

### 4. The dialogue box

1. Open the **Dialogue box** tab. Choose **A line** or **Choices** in **Preview:** and type a sample speaker and text.
2. In **Textbox**, set the placement, size and padding, then give it a **box style**: use the **+** next to the style field to create one from the current colors, or pick your frame style.
3. In **Speaker name**, choose the color source and, if you want, a **name plate** with its own style.
4. In **Choices**, set the button style: its hover look marks the selected choice.
5. Check the result in the Preview panel with a real line: select a step and press **Here** (F7).

### 5. A main menu with its own music

1. In the **Screens** tab, click **New screen from...** and choose **Main menu** to start from the template (or open the existing main menu screen).
2. Select the screen. In its properties, **Replaces** tells the game which role the screen plays: set it to **main menu**.
3. Add buttons with the widget kind chooser and the add button above the **Widgets** tree. For each button, open **Actions**, click **Add an action** and choose what it does: **New game**, **Continue**, **Open screen**, **Quit**.
4. Apply your `button_image` style to the buttons.
5. With nothing selected in the tree (the screen itself), tick **Screen music**. Pick the **Audio file**, then set the **Volume**, the fades and **Loop the track**.
6. Tick **Keep playing if the next screen has the same track** so that opening the preferences does not restart the song. For the pause menu, **Over the game** offers **Replace the game music** or **Lower the game music**.

### 6. Save slots

1. Select the save screen (**Replaces**: **save**), then its **Save slots** widget. Set **Slots** to **Save into a slot**.
2. In **Look of the slots** set a **Card style**, an **Empty card style**, the **Tab style**, **Page buttons style** and **Delete button style**. Adjust **Card height**, **Column gap** and **Row gap**, and the number of columns and rows.
3. For full control click **Customize the card**. The stock card is written as widgets: move the thumbnail, add a **play time** text bound to a **Slot field**, change fonts. **Back to the stock card** removes your changes.
4. Use the conditions **the slot is empty** and **the slot has a save** to show an "Empty" text or a padlock only where it applies.

### 7. Notifications

1. Open the **Notifications** tab. Click **Show test notification** to see a toast over the preview.
2. Set the anchor, width and animation. Under the style list, type a name in **new style name** and click **Add a notification style**.
3. Edit its **Background**, **Border**, **Corner radius**, **Icon size**, **Text size** and **Text color**. Fields you leave empty use the `default` style.
4. In the story, a **Notification** step picks the style by name. Achievements use the `achievement` style and the engine's own messages use `system`.

### 8. A splash screen

1. Open the **Splashes** tab. The *OrbitEngine (required)* splash always plays first.
2. Click **Add a splash**, then **Image of the project** or **Text splash**. Set the **Duration**, the **Fade in** and **Fade out**, and decide if it **Can be skipped**.
3. Click **Preview splashes** to watch them.

## Result

The game starts with your splash, then a main menu with your buttons and your music. The dialogue box, the save slots and the toasts share one visual identity, defined by a handful of styles.

## Going further

- Add a gallery of unlocked images, a replay menu or an achievements screen (see *Interface editor and splash screens*).
- Give each language its own font in **Theme > Fonts per language** (see *Translations*).
- Build an inventory or a phone as a screen opened by the story with **Open screen**.
- Switch the dialogue to speech bubbles: tutorial *Tutorial: expressive characters*.

## If something goes wrong

- **The game still shows the stock menus**: the `ui/` folder is only used when it exists. Enable the custom interface, then press **Save**.
- **A button does nothing**: check its **Actions**. A button that points to a screen that does not exist is reported by the Interface panel (**Validation**).
- **The text of an image button is doubled**: turn **Show the text** off in the style.
- **My music does not play**: the **Audio file** field shows a warning when the file is not in the project. Import it in the library first.
- **An edit disappeared after reloading**: unsaved interface edits are dropped when you reload the `ui/` folder or switch projects. Press **Save** often.
