# 3D scenes

> Experimental. The 3D features are hidden by default and may change between versions. Turn them on with **View > Options > Experimental 3D features** (or start the editor with the environment variable `ORBIT_EXPERIMENTAL_3D=1`). This chapter of the manual only shows when they are on. Projects that already use 3D keep loading and playing with the switch off.

A visual novel can use a 3D scene as its background: a room you can look around in, a street with a moving camera. The 2D characters and the dialogue box are drawn on top of it.

## Building a 3D scene

Open the **3D scene editor** (it opens three panels):

- **3D viewport**: the scene as the camera sees it. Right-drag orbits, middle-drag pans, the wheel zooms.
- **3D hierarchy**: the list of objects in the scene (models, lights, cameras).
- **3D inspector**: the settings of the selected object (position, rotation, size, color, light strength).

Create a new scene, import 3D models, place lights, and add one or more named cameras. A new scene starts with a sun and a camera named `main`. Save the scene (it is a `.scene3d.ron` file); imported models are stored in the `models` folder next to it, so save the scene before importing.

### What you can add

From the **Add** menu of the 3D hierarchy (or **Add a child** on a right click):

- **Shape**: a cube, sphere, plane, cylinder, cone, capsule or torus, made by the engine. Shapes need no file. You can use them for floors, walls, furniture placeholders or simple props. A new shape stands on the ground and gets the scene's **Default** material (white), which you can edit or swap in the inspector. The inspector also sets its size, radius, height and number of segments (more segments make it rounder).
- **Lights** (directional, point, spot), **cameras** and **empty nodes** (to group objects).
- **Models** with **Import glTF**: `.glb` files, or `.gltf` files with their `.bin` and texture files next to them, which is how Sketchfab exports models. The model and its files are copied into the `models` folder next to the scene. Materials that use the older "specular-glossiness" workflow are converted, so their colors stay close to the original. A model keeps its **skeleton, skin weights and animations**: see *Posing a character* below. If an import fails, a message tells you why (for example a missing `.bin` file).

You can also drop `.glb` / `.gltf` files on the viewport, or drag a model from the **Models** list of the hierarchy onto the 3D view to place it.

### Sky and textures

With nothing selected, the 3D inspector shows the **environment**: pick a plain color or a **skybox** (any image of the scene, see below), the ambient light and the fog.

A material has four texture slots (base color, occlusion / roughness / metal, normal, emission). Each slot offers the textures of the scene: the ones that come with your imported models, and the images you add with **Import...** (PNG or JPEG). Imported images are stored in the `models` folder next to the scene file, so save the scene first. For a skybox, use a wide panorama (an equirectangular image) or a cube map.

## Viewport tools

- **Move / Rotate / Scale** with **W**, **E**, **R** (the gizmo is on the selection). **F** frames the selection. **Delete** removes it and **Ctrl+D** duplicates it.
- **Snapping** (toolbar; hold **Ctrl** to invert it): 0.25 m for moves, 15 degrees for rotations, 0.1 steps for scale.
- **Preset views**: **1** front, **3** right, **7** top; with **Ctrl**, the opposite side (back, left, bottom). The toolbar has a **Viewpoint** menu with the six views.
- **G** puts the selection on the ground.
- The viewport draws the **frustum of every camera** and a **gizmo for every light**, so you can see what points where.
- **Show what the game camera sees** adds a small picture-in-picture preview of the game camera (the selected camera, otherwise the first). **Look through this camera** (camera inspector) views the scene from it.
- **Align to the view** moves the game camera (the selected one, otherwise the first) to the current view: orbit to a nice angle, then align.

## Posing a character

A glTF character that has a skeleton shows a **Skeleton** section in the inspector with its bones count.

- Click **Pose** (or press **P**) to enter **pose mode**. The bone tree lists the bones. Click a bone in the viewport or the tree, then drag the rings of the gizmo to turn it (on the root bone, **W** moves it). **Delete** resets the selected bone, **Reset the pose** resets all of them, **Escape** leaves pose mode. Every change can be undone.
- **Saved poses**: type a name and **Save the pose**; **Apply** brings a saved pose back.
- **Animations**: the animations that came with the glTF file are listed. **Play**, **Pause** and **Stop the preview** play one in the viewport, and you can scrub through it. **Use this frame as the pose** turns the pose shown at that time into the pose of the character in the scene. **New animation** creates a frame-by-frame animation, and **Key the pose here** stores the current pose as a key of it at this time.

## Adding the scene to your project

A 3D scene is only available to the story once it is part of the project. Click **Add to the project** (the folder icon in the viewport toolbar): the scene is saved as `assets/3d/<name>.scene3d.ron` and appears in the asset library, in the "3D scene" category. The models are copied with it. You can also drop an existing `.scene3d.ron` file on the library. Later saves update the same asset, and a 3D scene imported in the library keeps its name and its `models` folder.

The project's unsaved-changes marker also counts the open 3D scene: saving the project saves it.

## Using it in the story

Three steps drive 3D in the story. You find them in the **3D** submenu of the insert menu, next to the background and character steps:

- **Show 3D scene**: displays a 3D scene as the stage background, with a camera and an optional transition. Pick the scene in the inspector, then the camera in the list (the cameras are read from the scene file; "first camera of the scene" is the default).
- **3D camera**: switches to another named camera of the scene that is on screen, for example to cut or move to a close-up.
- **Hide 3D scene**: goes back to a normal 2D background.

Going from 2D to 3D and back uses the same transitions as any background. In the Stage View and the Preview, a scene that shows a 3D scene draws its models, not only the sky.

**Show in story.** In the 3D viewport, **Show the 3D scene in the story** adds the scene to the project if needed, selects the step that shows it (or inserts one after the selected line) and plays the preview from there. It needs a scene in the story to insert into.

**Stale copy banner.** The story shows the file that is in the project. If the scene you edit is another file, a banner says "The story shows a copy of this scene" and that your changes do not reach the story. Click **Use the story's file**: it saves your scene over the story's file, copies its models next to it and edits that file from then on.

If a step names a camera the scene does not have (for example after you renamed it), if a scene file is invalid, or if the scene places a model that is not in its `models` folder, the **Problems** panel says so.

## In the exported game

When you export, the game ships the models, textures and shapes that its 3D scenes use. Models and images in `models` that no scene uses are left out. If a scene refers to a model that is missing, the export log says so and the scene is shown without it.

The **3D quality (desktop)** setting of the export profile (Low or High) decides the rendering quality of 3D scenes in the Windows and Linux games; the web and Android games always use the web profile. The Export panel measures your scenes (lights, shadows, triangles, textures) and tells whether they fit the profile; over it the game still works, with extra lights and shadows dropped.

A game with 3D models or shapes needs a player from this version of OrbitEngine or a later one. An older player refuses the game with a message instead of showing empty scenes. 2D games are not affected.

## Working with an AI assistant

The assistant (MCP) can read and edit your 3D scenes: list them, read their nodes, lights, cameras and materials, add or move objects (including shapes), change the sky, create a new scene, and pose the skeleton of a character (set bone rotations, save, apply and remove poses). Every change is checked, and refused as a whole if it would break the scene. With the editor open and a graphics card, the assistant can also take a picture of a scene from any of its cameras to check its own work. These edits rewrite the scene file; they are not part of the undo history, so close and reopen the scene in the 3D editor to see them.

## Tips

- Start with one light and one camera, then add details.
- Test in the Preview panel: it uses the same renderer as the exported game.
- If a model does not show, check that its file is in the `models` folder next to the scene and read the Problems panel.
