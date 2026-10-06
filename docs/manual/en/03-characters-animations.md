# Characters and animations

## Defining a character

Open the **Characters** panel and add a character. Give it:

- a **name**, shown in the dialogue box (it can be translated like any other text),
- a **color**, used for the name,
- one or more **expressions**: a name ("happy", "angry") and the image to use. An expression can also be an animated sprite sheet or a sequence of images.

Import the images first in the **Library** (drag them into the window), then pick them in the character.

## Putting a character on stage

Add a **Show** step: choose the character, the expression and the position (left, center, right, a position you defined, or a free position anywhere on the stage). You can also set the size, flip the image horizontally or vertically, and pick a transition such as a dissolve. Showing a character who is already on stage updates their expression and position. **Hide** removes them.

The **Stage view** lets you place characters by dragging them directly on the picture.

## Animations

Characters and other stage elements can move with tweens (smooth changes of position, size or opacity) and timelines. Pace and style are controlled by **curves**: see the Curves chapter.

## Parallax backgrounds

A **Parallax background** step replaces the background with a stack of image layers, from the back to the front. Each layer has a **depth**:

- `1` moves exactly like the stage (a plain background),
- `0` is glued to the screen (a sky that never moves),
- anything in between gives the parallax: the nearer the depth is to 1, the more the layer follows the camera,
- above `1` is a foreground that moves faster than the stage.

The camera pan and zoom drive it: add a **Zoom** effect, or a **Timeline** with a *pan* track on the *camera*, and every layer moves by its own depth. A slow, subtle pan (2 to 3% of the screen over 30 seconds) already gives a lot of depth. Use pictures with transparent areas for the layers in front, otherwise they hide the ones behind. Tick **Tile horizontally** for a layer that repeats (clouds, a street) so it never shows an edge when it pans. Layers are drawn slightly larger than the screen so a gentle pan never reveals their edges. The inspector shows a small panorama with a slider to see the parallax without playing. A later **Background** step replaces the layers.

## Particles

A **Particles** step starts an emitter: rain, snow, petals, dust or sparks, or your own. Pick a **preset** (it fills every setting), then adjust: births per second, lifetime, the area where particles are born, direction and spread, speed, gravity, sway, size, length (for rain streaks), colors from birth to death, opacity, an optional picture, and whether the particles fall behind or over the characters. A live preview in the inspector shows the result as you type. Give the emitter a **name**: a **Stop particles** step with that name (or empty, for all of them) stops new particles, and those already in the air finish their fall; a new Particles step with the same name replaces the first one. Particles also stop when the story changes scene.

Particles are computed from the moment the step ran, never stored: going back in the story, loading a save or skipping always shows the same picture. The editor warns when an emitter would need more than 1500 particles at once (lower the rate or the lifetime), and the screen never draws more than 4000 in all. They are drawn on the screen, not in the camera, so the weather does not shake with the camera. With **Reduce motion** on (see Accessibility), no particles are drawn.

## Skeleton animation (rigs)

A **rig** brings a drawing to life with bones: breathing, a waving arm, a head that turns. The rig viewport shows a step bar (**1. Import image → 2. Draw bones → 3. Bind → 4. Pose / animate**) with what to do next:

1. **Import the image.** Choose **Tools > New rig from image…** (or the big button of the empty rig viewport, or drop a PNG on it). The picture is copied into the project (`assets/images/`) if it is not there yet, and becomes a rig with a `root` bone at the character's feet and a mesh traced from the picture's transparency. It is framed and shown right away. Separate pieces (an arm, the head…) are added with **Add image part…** or by dropping more PNGs on the viewport: each one becomes its own slot, lined up with the first picture when it has the same size.
2. **Draw the bones** with the bone tool (**B**): drag over the picture from one joint to the next (hips to neck, shoulder to elbow, elbow to hand). A new bone hangs from the selected one; click empty space first to start a new chain.
3. **Bind.** Click **Bind to bones**: every picture gets automatic weights (each part follows the closest bone, softened between bones). To refine, take the weight brush (**P**): the heat map shows the selected bone's influence (Ctrl-click picks a bone), paint with Add / Subtract / Smooth.
4. **Pose and animate.** After binding the viewport is in **pose mode** (**Tab**): select a bone and drag its ring to bend the picture. Pose mode never changes the rig itself; record clips in the Clips and Timeline panels.

If you add bones later, the step bar asks you to bind again. Then click **Add to project**: the file is saved as `assets/rigs/<name>.rig.ron` and appears in the Library. The pictures a rig draws must be files of the project.

To use it, open the character, choose the type **Rig** for an expression, then pick the rig, the **skin** (outfit), the **clip** and what happens at the end (loop, play once, ping-pong). A small animated preview shows the result. The expression's picture stays the **poster**: it is used for thumbnails and drawn while the rig loads or if it cannot be used (the Problems panel tells you why: missing file, unknown clip or skin, missing picture). The rig is drawn to the scale of the poster, with its origin at the bottom-center of the character.

- Showing the same character with another expression **of the same rig** cross-fades the two clips over 0.2 seconds instead of cutting.
- Clips keep playing while a line is displayed or waits for a click.
- With **Reduce motion** on, the rig holds the first pose of its clip.
- Going back in the story or loading a save restarts the clip: nothing about it is saved.

An AI assistant can list and read rigs, check a pose without drawing it, generate a mesh from a picture and assign a rig to an expression (see the MCP guide).

**Building and animating a rig through MCP.** An assistant can do the whole workflow without the editor UI: `rig_create` (from a project picture, imported as an asset if needed: `root` bone at the feet, a slot and a mesh traced from the alpha; or an empty rig), `rig_add_image_part`, `rig_bone_add` / `rig_bone_update` / `rig_bone_reparent` / `rig_bone_delete` (positions relative to the parent or in rig space, or a `head` and `tip` like the click-drag of the bone tool), `rig_bind` (automatic weights of every mesh), `rig_slot_update` (picture shown, tint, draw order), `rig_skin_add` / `rig_skin_duplicate` / `rig_skin_delete`, `rig_ik_set` / `rig_ik_remove`, `rig_clip_create` / `rig_clip_update` / `rig_clip_delete`, `rig_clip_key_set` / `rig_clip_key_delete` (bone rotate, translate and scale, slot picture and tint, deform, draw order; each key has an easing: a preset, a bezier or a named curve of the story), `rig_add_to_project` and `rig_delete`. Every edit is validated (an error refuses it and nothing is written; web budget warnings come back with the answer) and atomic. When the rig is open in the rig editor, the change goes through its document: it refreshes the view and **Ctrl+Z undoes it**; otherwise the file is rewritten. `rig_sample` then shows the result, including how far the mesh moved.


## Voice acting

Each line of dialogue can have a **voice**: select the line, and in the inspector choose a sound of the project under **Voice** (or *Add file* to import a .ogg or .wav, *Remove* to go back to silence).

* The voice starts with the line and is cut when the player moves on (a click, skip, rollback, a choice). It plays on its own **Voice** volume (Preferences). A line is not voiced again after a rollback or a load: use the **Voice** button of the quick menu (shown when the line on screen has a voice) or the **Voice** button of its row in the **History** to hear it again.
* **Auto mode waits for the end of the voice**, then applies its usual delay. Skipping never starts a voice.
* A translation can have its own voices: for the selected language the translation's voice replaces the one of the line (the lines without one keep the source voice).

### Importing many voices at once

*Import voices…* (inspector of a line) takes a **folder** of audio files named after the lines: `<id>.ogg`, `.wav`, `.mp3` or `.flac`, where the id is the id of the line (the id of the dialogue node, or the id of its text; the editor and the `orbit` tools show both). Put the files of a translation in a sub-folder named after the language: `voice/fr/<id>.ogg`. *From a CSV…* does the same without renaming files: one row per voice, `line,file[,language]` (a header row is skipped), the file being a path inside the project. The whole import is one undo step; the files outside the project are copied under `assets/voice/`, the files that match no line are listed and ignored. The voices of a language are written into its translation file (the language must already exist). The MCP tool `import_voices` does the same (with `dry_run` to preview).

The validator reports a voice that is not a sound of the project, and the export refuses a voice file that is missing.
