# Writing the story

A story is made of **scenes**. A scene is a list of **steps** that the game plays one after the other: a line of dialogue, a background change, a choice, and so on.

## The Scene editor

Add steps with **Edit > Insert**, the buttons of the Scene editor or the Explorer, or the quick menu (Ctrl+I). Ctrl+Enter adds a line. The insert menu is organized in submenus by category. Every kind of step:

**Dialogue**

- **Line**: who speaks (a character, or nobody for narration) and the text.
- **Player choice**: two or more options. Each option points to a **label** and can have a condition.
- **Notification (toast)**: a short message shown in a corner of the screen while the story goes on (see *Notifications* below).

**Scene**

- **Background**, **Show a character** / **Hide a character**, **Wait**. Transitions (dissolve, fade) can be set on backgrounds and on showing or hiding characters.

**Effects**

- **Effect** (move, shake, zoom, flash, tint), **Timeline** (tracks of keys on the camera, the screen or a character), **Parallax background**, **Particles** and **Stop particles**. See the *Characters and animations* and *Curves* chapters.

**Screens**

- **Open screen** / **Close screen**: show a screen of the project (an inventory, a HUD) from the story, optionally waiting for the player to close it. See the *Interface* chapter.

**Sound**

- **Play music**, **Stop music**, **Play a sound**.

**Logic**

- **Set a variable** and **Condition** (if): see the *Variables* chapter.
- **Script (Rhai)**: a little code for logic that is too heavy for the graph (see *Scripting*).
- **Unlock achievement** (see *Achievements* in the Interface chapter).
- **Clickable zones (point & click)**: a map where the player clicks zones (see below).

**Navigation**

- **Label**: a named spot in a scene, used as the target of a choice or a jump.
- **Jump to a label**, **Jump to scene** (scenes never chain on their own, so say where the story goes next).
- **Call a routine** / **Return from routine**: reuse a small piece of story from several places.
- **End of story**: the story stops.

**3D** (experimental, only when *View > Options > Experimental 3D features* is on): **Show 3D scene**, **3D camera**, **Hide 3D scene**. See the *3D scenes* chapter.

**Plugins**: the steps added by the plugins of the project (see *Rust plugins*).

Select a step to edit all its settings in the Inspector (right click, **Edit**, to open it directly). Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+D and Alt+Up / Alt+Down copy, cut, paste, duplicate and move steps.

## Clickable zones (point & click)

A **Clickable zones** step turns the background into a map: the story stops there until the player clicks a zone, then goes where the zone leads. Use it for a house to explore, a room full of objects to inspect, a world map.

The step has the **map** settings: an optional background of its own (**Show its own background**; without one, the current background is used) and a **look** shared by all zones (colour, fill, glow width and intensity, pulse speed, idle shimmer, hand cursor). Each zone has:

- a **name**, an optional **tooltip** shown on hover, and **Leads to**: the label the story goes to when the zone is clicked. **Go to a scene...** creates a label and a jump to another scene and points the zone at it.
- a **shape**: rectangle, ellipse, polygon, or **Picture (opaque pixels)**: a picture of your project, placed over the background, of which only the opaque pixels react (an object on the scene). For a picture, **Fit to the picture** gives it its own size, **Like the background** stretches it over the screen (an object painted on a transparent copy of the background), the **opacity threshold** decides which pixels count, and **Draw the picture over the background** shows it. The glow follows the outline of the object.
- **Visible only if...** and **Clickable only if... (otherwise dimmed)**: conditions on variables.
- a **hover image** (replaces the glow) and a **hover sound**, and an **Own look**.
- **On click** actions, run before the story moves on: **Change a variable** (no code), **Notification**, **Play a sound**, then one of **Stay on this map** (for example picking up an object, then the map is shown again), **Go to a label**, **Go to a scene**, **Call a label (then back here)** or **Return (end of a call)**. Without a flow action, the zone goes to **Leads to**.

Draw the zones directly in the **Stage View** panel: choose **Rectangle**, **Ellipse** or **Polygon** in the *Zones* toolbar and drag on the picture (a polygon: click the points, then the first one again, double-click or press Enter). With the *Select* tool, drag a zone to move it, drag its corners or points to reshape it, double-click the edge of a polygon to add a point, right-click a point to remove it. Delete removes the zone. Each gesture is one undo step.

In the game, an idle zone shimmers softly, a hovered zone gets a pulsing halo and the hand cursor, and its tooltip appears. Zones can be chosen with the keyboard (Tab, arrows and digits) and the game controller, and **V** shows every zone at once (the controller has a *show zones* action, Select / View by default, that can be assigned to another button).

In the text format, a map reads:

```
hotspots bg "assets/hall.png"
    spot rect (0.1, 0.2, 0.25, 0.5) "The door" -> outside name door
        do set key = 1
        do notify "You found a key!"
        do stay
```

### The navigation map wizard

**Edit > Insert > New navigation map (house)...** (also in the palette) writes a whole house in one undo step: you list the rooms, pick a background for each, tick which rooms are linked, and the wizard creates one label and one map per room, a door zone to each linked room (a rectangle: move it onto the real door in the Stage View), an optional **Back** zone and an example of a door locked by a key variable.

## Notifications

The **Notification (toast)** step shows a short message in a corner of the screen and the story goes on at once. It has a text (tags and `[variable]` allowed), an optional icon, sound, duration and style. Its look is set in the **Notifications** tab of the Interface editor (see the *Interface* chapter). Scripts can do the same with `notify(...)`.

## The Graph

The Graph shows the same steps as boxes linked by arrows. It is the best place to check that every choice leads somewhere and to spot dead ends. Drag boxes to tidy the layout; this does not change the story.

## Checking your work

The **Problems** panel warns you about, for example, a choice pointing nowhere, a scene that never ends, or a missing image. Click a problem to jump to the faulty step; F8 goes to the next one. Validate again with Ctrl+F7.

## Trying it

The **Preview** panel plays the story as a player would see it (F6). Use **Play from here** (F7) on a step to start with the state a player would have at that point (characters on stage, variables set).

## Writing as text

The **Text view** shows the story as a script you can type (see the *Text view* chapter). The text and the graph always describe the same story.
