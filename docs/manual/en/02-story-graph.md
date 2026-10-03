# Writing the story

A story is made of **scenes**. A scene is a list of **steps** that the game plays one after the other: a line of dialogue, a background change, a choice, and so on.

## The Scene editor

Add steps with the buttons of the Scene editor or the Explorer. The most common ones:

- **Line** (dialogue): who speaks (a character, or nobody for narration) and the text.
- **Choice**: two or more options. Each option points to a **label**.
- **Label**: a named spot in a scene, used as the target of a choice or a jump.
- **Jump**: go to a label. **Jump to scene**: go to another scene (scenes never chain on their own, so say where the story goes next).
- **Background**, **Show** / **Hide** a character, **Music**, **Sound**, **Wait**.
- **Condition** (if): go one way or another depending on a variable.
- **Call** / **Return**: reuse a small piece of story from several places.
- **Open screen** / **Close screen**: show a screen of the project (an inventory, a HUD) from the story, optionally waiting for the player to close it. See the Interface chapter.
- **End**: the story stops.

Select a step to edit all its settings in the Inspector. Transitions (dissolve, fade) can be set on backgrounds and on showing or hiding characters.

## The Graph

The Graph shows the same steps as boxes linked by arrows. It is the best place to check that every choice leads somewhere and to spot dead ends. Drag boxes to tidy the layout; this does not change the story.

## Checking your work

The **Problems** panel warns you about, for example, a choice pointing nowhere, a scene that never ends, or a missing image. Click a problem to jump to the faulty step.

## Trying it

The **Preview** panel plays the story as a player would see it. Use **Play from here** on a step to start with the state a player would have at that point (characters on stage, variables set).
