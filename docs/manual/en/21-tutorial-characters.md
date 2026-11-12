# Tutorial: expressive characters

Two ways to make your characters feel alive. The first changes how their words are shown: comic-style speech bubbles that point at the speaker. The second changes how they move: a skeleton with bones lets a drawing breathe, blink and wave. They are independent, and work well together.

## Part 1. Comic-style dialogue with speech bubbles

### Goal

Lines spoken by characters appear in bubbles next to their heads, each character with its own colors. The narrator keeps a caption, and a line can become a thought cloud or a shout.

### Before you start

- A project with at least one character who is shown on stage (a **Show a character** step before their lines). The bubble points at the character's head.
- The custom interface must be enabled: **Tools > Interface editor**, then **Enable custom interface** if the panel says it is off (see *Tutorial: customize the game interface*).

### Steps

1. In the **Interface** panel, open the **Dialogue box** tab. Find **Dialogue display** and set **Display mode** to **Comic bubbles**. Character lines now appear in bubbles; the narrator keeps the box.
2. In the **Speech bubbles** section, adjust the general look in the preview: **Fill / outline** colors, **Outline width**, **Corner radius**, **Width min / max**, **Font**, and **Pop-in** for the entrance animation.
3. Use **Show the speaker's name** and **Name size** if you want the name written in the bubble. Tick **Outline in the character's color** to tint each bubble with its speaker's color.
4. Set the **Tail length / width** and the **Gap to the head**. Keep a **Screen margin** so that bubbles stay inside the picture.
5. Under **Narrator** choose how narration appears: **Classic box** or **Caption (top left)**, with its **Caption color**.
6. Under **Speaker off screen** choose **Bubble without tail** (the line is still in a bubble but points at nobody) or **Classic box**.
7. In **Per character**, give each character its own **Fill** and **Outline**, and a **Head offset**: how far the tail should aim from the top of the sprite. A tall sprite with a hat needs a larger offset than a small one. Adjust it with the preview until the tail touches the face.
8. In the story, put `{bubble=thought}` in a line (usually at its start) to draw a thought cloud, or `{bubble=shout}` for a jagged shout bubble. For example `{bubble=shout}Get out of here!`. The tag is not displayed.
9. Press **Preview** (F6) or select a step and press **Here** (F7) to see the real result.

### Result

Each character speaks in its own colors, bubbles point at the heads, the narrator uses a caption and the tags give you thoughts and shouts.

### Going further

- Move characters with the **Stage View** panel, or give them other positions: bubbles follow the character.
- Combine with the **Effect** step (a shake on a `{bubble=shout}` line) for comic timing.
- Switch back to **Classic box** at any time: nothing in the story changes.

### If something goes wrong

- **The tail points at the wrong place**: adjust the **Head offset** of that character.
- **No tail at all**: the character is not on stage at that moment, so the **Speaker off screen** setting applies.
- **The tag shows as text**: check the syntax: curly braces, no space, and one of the three known shapes (`{bubble=speech}`, `{bubble=thought}` or `{bubble=shout}`). Any other value is left as plain text.
- **The bubble overflows the screen**: increase the **Screen margin** or reduce the maximum of **Width min / max**.

## Part 2. Animated characters with a rig

### Goal

Turn one drawing into a character that breathes, then use it in your story like any expression. Allow about twenty minutes the first time.

### Before you start

- A PNG of your character with a transparent background, ideally cut like a paper doll: head, torso, arms as separate pictures if you want them to move independently. Or a layered file (PSD, Krita `.kra` or OpenRaster `.ora`) with one layer per part.
- A project open in the editor, with the character already created in the **Characters** panel.

### Steps

1. Choose **Tools > Rig > New rig from image…** and pick your PNG. The picture is copied into the project, and the rig editor opens: **Rig viewport**, **Rig hierarchy**, **Rig inspector**, **Rig clips** and **Rig timeline**. A step bar shows where you are: **Import image**, **Draw bones**, **Bind**, **Pose / animate**.
2. If you work from a layered file, choose **Tools > Rig > New rig from layered file…** instead. Every visible layer becomes a part at its exact position. When asked whether to create one bone per layer group, answer yes if your groups are meaningful (an arm group, a head group), no otherwise. Add other pictures with **Add image part…** or drop them on the viewport.
3. **Draw the bones.** Press **B** and drag over the picture from joint to joint: hips to neck, shoulder to elbow, elbow to hand. A new bone hangs from the selected one; click an empty spot to start a new chain.
4. Click **Bind to bones**. Each part follows its nearest bone, softened between bones. To refine the result, press **P** for the weight brush and paint the influence of the selected bone.
5. The viewport is now in pose mode (**Tab** toggles it with the rest pose). Select a bone and drag its ring: the picture bends. Posing does not change the rig itself.
6. Click **Add to project**. The rig is saved as `assets/rigs/<name>.rig.ron` and appears in the library.
7. Open the **Rig clips** panel and click **New clip**. Name it `idle`, set its **Duration** (3 seconds is nice) and its **Loop** to **Forever**.
8. In the **Rig timeline**, put the playhead at 0 s and click **Key pose** (K) to record the rest pose. Move the playhead to 1.5 s, turn the chest bone or the head by a few degrees in pose mode and click **Key pose** again. Go back to 3 s and key the rest pose a third time. You can also add individual **Track** and **Key** entries by hand, or turn on auto-key so that posing a bone records keys by itself.
9. Press play in the viewport to watch the loop. Save the rig again with **Add to project** when you are happy.
10. Open the **Characters** panel, select the character and choose an expression (or **Add an expression**). Set its type to **Rig**, pick the rig, the **Skin** and the **Clip** `idle`, and what happens at the end of the clip (**Loop**, **Play once** or **Ping-pong**). The picture of the expression stays as the poster shown while the rig loads.
11. In the story, use **Show a character** with that expression as usual. The character now breathes on stage. Showing the same character with another expression of the same rig cross-fades the two clips in 0.2 seconds.

### Result

A character that moves in the game, with a drawing you can still repaint, and clips you can reuse for other expressions (a wave, a nod) of the same rig.

### Going further

- Make the character **blink**: add an **Attachment** track on an eyelid part, with keys that show and hide it (see *Characters and animations*).
- Reuse a skeleton for several characters with **Duplicate rig…** and **Copy clips from another rig…** (Tools menu or the wrench menu of the rig editor).
- Cut a flat picture into layers (hair, face, eyes) with the cut tool, and paint corrections directly on the part.
- If the character appears at the wrong place on stage, use **Recenter on origin…**.

### If something goes wrong

- **The picture stays still when I pose a bone**: bind first (**Bind to bones**). If you add bones later, bind again.
- **The expression shows a still picture**: the Problems panel says why, for example a rig with no clip, a clip that was renamed, or a picture that is not a file of the project.
- **The character is cut in pieces or jagged**: the mesh is traced from the transparency; clean up stray pixels in the picture, or use the mesh tool (M) to adjust.
- **Everything moves when I move one bone**: bones need to form chains with the right parents. Draw a new chain from an empty spot rather than from the selected bone.
- **I changed a picture and another character changed too**: pictures can be shared between rigs; the editor lists what else uses a file before the first pixel change and offers to make a copy.
