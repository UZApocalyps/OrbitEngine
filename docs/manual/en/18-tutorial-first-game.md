# Tutorial: your first visual novel

This tutorial takes you from an empty editor to a small game that you can send to a friend, in about fifteen minutes. You will create a project, a character with two expressions, a background, a few lines, a choice, a variable and a condition, then play the result and export it for Windows.

## Goal

A short story in which the player can share their lunch with a classmate or ignore her. The choice is remembered in a variable, and the ending depends on it.

## Before you start

- The editor is installed and opens (see *Getting started*).
- You have two pictures of a character (PNG with a transparent background, for example `neutral.png` and `happy.png`) and one background picture (PNG or JPEG). Any pictures will do for a first try: you can swap them later.

## Steps

### 1. Create the project

1. On the welcome page click **New project** (or press Ctrl+N).
2. In the **New project** window, type a **Name**, choose the **Parent folder** with **Browse…**, and pick the template **Blank**.
3. Click **Create**. The project opens with one scene called *Introduction* and two lines of example text.

### 2. Import your pictures

1. In the **Asset Library** panel, click **Import** and choose your three pictures. You can also drop them on the editor window.
2. Use the **Images** filter of the library to check that they are all there. The files are copied to the `assets/` folder of the project.

### 3. Create a character

1. Open the **Characters** panel and click **New**.
2. Fill in **Display name** (for example `Mia`) and pick a **Color**: it tints her name in the dialogue box.
3. Under **Expressions**, click **Add an expression**, name it `neutral`, and choose the picture with **File…**. Do it again for `happy`.

### 4. Set the stage

1. In the **Scene** panel, select the first line. Press Ctrl+I to open the quick insert menu, type `back` and choose the background step (**Background**), or use **Edit > Insert > Insert a background**.
2. In the **Inspector**, pick your **Image** and, as **Transition**, **Crossfade**.
3. Insert a **Show a character** step after it. In the Inspector choose the **Character** Mia, the **Expression** `neutral`, the **Position** **Center**.

### 5. Write the dialogue

1. Select the last line and press Ctrl+Enter to add a line after it. Type the text, then press Enter for the next line (Shift+Enter for a line break in the same line, Esc to stop writing).
2. For each line choose the speaker in the **Character** field of the row, or leave it empty for the narrator.
3. To change Mia's face in the middle of the conversation, insert another **Show a character** step with the expression `happy`. Showing a character who is already on stage only updates them.

Write four or five lines, for example: Mia sees you, says she forgot her lunch, and asks what you think she should do.

### 6. Add a choice

1. Choose **Edit > Insert > Insert a choice**. The step comes with two options, *Option A* and *Option B*.
2. Select the choice step. In the Inspector, write the text of each option: `Share your lunch` and `Say nothing`.
3. For each option click **New label** next to **Leads to**. This creates a landing spot at the end of the scene and points the option at it.
4. Select each new label in the scene and rename it in **Label name**: `share` for the first, `after` for the second.

The scene now ends with two labels. Execution goes through a label like through any step: the lines you write after `share` will continue into `after`.

### 7. Remember the choice with a variable

1. Open the **Variables** panel and click **New**. Name it `friendship`, choose the type **Whole number**, and keep **initial value** at 0. Leave the **Scope** on **Game**.
2. Select the label `share` and insert a **Set a variable** step after it. Choose the **Variable** `friendship`; in **New value** pick the operation **add** and the value 1.
3. Add a line of dialogue under it, with Mia happy: *"Thank you, that is so kind!"*

### 8. React to the choice with a condition

1. Select the label `after` and insert a **Condition** step after it.
2. In the Inspector, with the **Simple editor**, pick the variable `friendship`, the test **is greater than or equal to** and the value 1.
3. For **If true, go to**, click **New label** and name it `good_end`. Leave **Otherwise** on **Continue to the next line**.
4. Under the Condition step, write the bad ending (two or three lines), then insert an **End of story** step.
5. At the very end of the scene, after the label `good_end`, write the good ending and finish with another **End of story** step.

### 9. Play it

1. In the **Preview** panel press **Play** (F6). Click the picture to advance the story.
2. Try both options. Select a step and press **Here** (F7) to restart from the middle without replaying everything.
3. Open the **Problems** panel. A well-built story shows *No problems detected.* Click a problem to jump to the faulty step.

### 10. Export for Windows

1. Save with Ctrl+S, then choose **File > Import, export and translations > Export game…**.
2. In **Export profile** write the **Game name**, a short **Identifier** (lowercase letters, digits, `-` and `_`; do not change it after publishing) and the **Version**.
3. Under **Targets** tick **Windows (zip)**. A target flagged *(template not found)* has no player installed: see the *Exporting your game* chapter.
4. Click **Export**. When the log says the export is finished, click **Open folder**: the `.zip` is there. Anyone can unzip it and play, with nothing to install.

## Result

You have a complete, playable game: a scene with a background, a character with two expressions, a choice, a variable that remembers it, two endings, and a `.zip` to share.

## Going further

- Add music: insert a **Play music** step at the start (*Writing the story*).
- Use several variables and test them together in the condition of a Condition step.
- Restyle the menus and the dialogue box: tutorial *Tutorial: customize the game interface*.
- Make the game speak other languages: tutorials *Achievements, notifications and translations* and *Publish your game*.

## If something goes wrong

- **A red error in the Problems panel**: errors block the export. Click it to reach the step. Typical causes are a choice option that leads nowhere (**Leads to** empty) or a deleted label.
- **"The game ends here"**: a warning that a scene has no **End of story**, jump or return. Add one at the end of each branch.
- **The story goes from `share` straight into the bad ending**: remember that labels do not stop the story. Check the order of the steps in the Scene panel, or look at the arrows in the **Graph** panel.
- **The character is invisible**: you must have a **Show a character** step before their lines, and the expression needs a picture.
- **The preview is empty**: press Play (F6) first; if the message says no GPU could be initialized, the preview is not available on this computer, but the export still works.
