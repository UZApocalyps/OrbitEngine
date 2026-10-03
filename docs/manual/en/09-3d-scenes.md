# 3D scenes

A visual novel can use a 3D scene as its background: a room you can look around in, a street with a moving camera. The 2D characters and the dialogue box are drawn on top of it.

## Building a 3D scene

Open the **3D scene editor**. It has three panels:

- **3D viewport**: the scene as the camera sees it. Orbit, move and zoom to look around.
- **3D hierarchy**: the list of objects in the scene (models, lights, cameras).
- **3D inspector**: the settings of the selected object (position, rotation, size, color, light strength).

Create a new scene, import 3D models, place lights, and add one or more named cameras. A new scene starts with a sun and a camera named `main`.

### Sky and textures

With nothing selected, the 3D inspector shows the **environment**: pick a plain color or a **skybox** (any image of the scene, see below), the ambient light and the fog.

A material has four texture slots (base color, occlusion / roughness / metal, normal, emission). Each slot offers the textures of the scene: the ones that come with your imported models, and the images you add with **Import…** (PNG or JPEG). Imported images are stored in the `models` folder next to the scene file, so save the scene first. For a skybox, use a wide panorama (an equirectangular image) or a cube map.

### Adding the scene to your project

A 3D scene is only available to the story once it is part of the project. Click **Add to the project** (the folder icon in the viewport toolbar): the scene is saved as `assets/3d/<name>.scene3d.ron` and appears in the asset library, in the "3D scene" category. You can also drop an existing `.scene3d.ron` file on the library. Later saves update the same asset.

The project's unsaved-changes marker also counts the open 3D scene: saving the project saves it.

## Using it in the story

Three steps drive 3D in the story. You find them in the "add a step" menu, next to the background and character steps:

- **Show 3D scene**: displays a 3D scene as the stage background, with a camera and an optional transition. Pick the scene in the inspector, then the camera in the list (the cameras are read from the scene file; "first camera of the scene" is the default).
- **3D camera**: switches to another named camera of the scene that is on screen, for example to cut or move to a close-up.
- **Hide 3D scene**: goes back to a normal 2D background.

Going from 2D to 3D and back uses the same transitions as any background.

If a step names a camera the scene does not have (for example after you renamed it), or if a scene file is invalid, the **Problems** panel says so.

## Working with an AI assistant

The assistant (MCP) can read and edit your 3D scenes: list them, read their nodes, lights, cameras and materials, add or move objects, change the sky, create a new scene. Every change is checked, and refused as a whole if it would break the scene. With the editor open and a graphics card, the assistant can also take a picture of a scene from any of its cameras to check its own work. These edits rewrite the scene file; they are not part of the undo history, so close and reopen the scene in the 3D editor to see them.

## Tips

- Start with one light and one camera, then add details.
- Test in the Preview panel: it uses the same renderer as the exported game.
