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

> Skeleton-based character animation is not available yet.
