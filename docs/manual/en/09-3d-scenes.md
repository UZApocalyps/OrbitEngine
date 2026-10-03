# 3D scenes

A visual novel can use a 3D scene as its background: a room you can look around in, a street with a moving camera. The 2D characters and the dialogue box are drawn on top of it.

## Building a 3D scene

Open the **3D scene editor**. It has three panels:

- **3D viewport**: the scene as the camera sees it. Orbit, move and zoom to look around.
- **3D hierarchy**: the list of objects in the scene (models, lights, cameras).
- **3D inspector**: the settings of the selected object (position, rotation, size, color, light strength).

Create a new scene, import 3D models, place lights, and add one or more named cameras. Save the scene: it becomes a file in your project (`.scene3d.ron`) that you can reuse in the story.

## Using it in the story

Three steps drive 3D in the story:

- **Show 3D scene**: displays a 3D scene as the stage background, with a camera and an optional transition.
- **Camera**: switches to another named camera, for example to cut or move to a close-up.
- **Hide 3D scene**: goes back to a normal 2D background.

Going from 2D to 3D and back uses the same transitions as any background.

## Tips

- Start with one light and one camera, then add details.
- Test in the Preview panel: it uses the same renderer as the exported game.
