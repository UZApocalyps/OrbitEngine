# Getting started

OrbitEngine lets you make a visual novel without writing any code. You write lines of dialogue, offer choices, show characters and backgrounds, and the editor plays the result for you as you go.

## Create your first project

1. Open the editor. The welcome page offers **New project** (Ctrl+N).
2. Type a name and pick the folder that will contain your project.
3. Choose a **template**:
   - **Blank**: one scene and two lines, to start from nothing.
   - **Visual novel starter**: two scenes, two characters, a choice, a variable, custom menu screens and a splash. You can play it right away and rewrite it.
4. Press **Create**. The project opens.

A project is a normal folder. Your story is the file `story.orbit`, your images and sounds live in `assets/`, and your custom screens in `ui/`. You can put the whole folder under version control or copy it to another computer.

## The main windows

- **Scene**: the story as a list of steps, in reading order. This is where you write most of the time.
- **Graph**: the same story drawn as boxes and arrows, handy to see where choices lead.
- **Inspector**: every setting of the selected step.
- **Characters** and **Variables**: the definitions your story uses.
- **Library**: your images and sounds. Drag files into the window to add them.
- **Preview**: plays the story with the real engine, and reloads when you edit.
- **Problems**: live checks. An error blocks the export, a warning does not. Checks run in the background about 300 ms after your last edit. Turn this off with **View > Automatic validation** (useful on very large projects): the list is then dimmed when out of date and refreshes when you press **Validate** (Ctrl+F7). Export and build always validate first.

You can open any window again from the **View** menu, and rearrange them by dragging their tabs.

## Saving and undoing

Ctrl+S saves. Ctrl+Z undoes and Ctrl+Y redoes, as far back as you need. If the editor closes unexpectedly, it offers to restore your unsaved work the next time you open the project.

## Where to go next

Each chapter of this manual covers one topic. Open **Help > Manual** at any time (or press F1). The manual follows the language of the editor.
