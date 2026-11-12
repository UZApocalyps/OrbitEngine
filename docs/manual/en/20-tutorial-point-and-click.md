# Tutorial: point and click, explore a house

In a point and click scene the story stops on a picture and waits for the player to click a **zone** of it: a door, an object, a character. This tutorial builds a small house to explore with a wizard, then makes it yours: doors placed on the real picture, a key to find, a locked door that opens once the key is taken, and an object whose shape follows its drawing.

## Goal

A house of three rooms (hall, kitchen, bedroom). Doors lead from room to room, a key lies in the hall, and the bedroom door stays dimmed until the key is in your pocket.

## Before you start

- A project with at least one background picture. Three different pictures (one per room) are better; the wizard proposes the first image of the library for every room and you can change each one.
- A scene open in the **Scene** panel. The wizard inserts the house at the selected step.
- Optional, for the last step: a PNG of an object (a vase, a lamp) with a transparent background.

## Steps

### 1. Run the navigation map wizard

1. Select the step after which the house should start (for example the last line of your introduction).
2. Choose **Edit > Insert > New navigation map (house)…**. The window explains what it writes: one label and one point and click map per room, a door zone to each linked room, a "back" zone, and an example of a door locked by a key.
3. In **Rooms**, name each room. The wizard proposes *Hall*, *Kitchen* and *Bedroom*; use **Add a room** for more (up to twelve) and **Remove the room** to drop one. For each room click **Choose a background**.
4. In **Doors between rooms**, tick the pairs of rooms that are linked: a door is written each way. By default the hall is linked to the kitchen and to the bedroom.
5. **Locked door (key example)** offers one of the doors, or **None**. Keep the door from the hall to the bedroom: the key will lie in the hall.
6. Keep **A "Back" zone in every room** ticked: each room but the first gets a zone that returns to the room you came from.
7. Click **Create the map**. The wizard adds, as one undo step, a label and a **Zones** step per room, and a variable `house_key` (initially false) in the **Variables** panel.

If the wizard says a room has no door, link it to another room; it needs at least one zone.

### 2. Look at what was written

1. Select the **Zones** step of the first room in the Scene panel. The **Inspector** shows its **Map**: **Show its own background** (the picture of the room), the **Look (highlight on hover)**, and the list of **Zones**.
2. Each zone has a **Name**, a **Tooltip** shown on hover (*To Kitchen*, *To Bedroom*, *A key*, *Back*), and **Leads to**: the label of the room it opens.
3. Play the scene with **Preview** (F6): you already navigate between rooms. The doors are only placeholder rectangles for now.

### 3. Put the doors on the real picture

1. With the **Zones** step selected, look at the **Stage View** panel: it shows the room and draws its zones. A **Zones:** toolbar offers **Select**, **Rectangle**, **Ellipse** and **Polygon**.
2. With **Select**, click a zone to select it. Drag it onto the door in the picture; drag its corners to resize it. For a door that is not a rectangle, use **Polygon**: click the points one by one, then click the first one again (or press Enter) to close the shape; Backspace removes the last point and Escape cancels. Double-click an edge to add a point, right-click a point to remove it.
3. To add a zone, pick **Rectangle**, **Ellipse** or **Polygon** and draw on the picture. **Delete zone** (or the Delete key) removes the selected zone. Draw as many as you like and set their **Name**, **Tooltip** and **Leads to** in the Inspector.
4. Do the same for the doors of the other rooms.

### 4. Make the key work

The wizard wrote the key zone for you. Select the zone named `key` in the Inspector to see how it works:

1. **Visible only if…** holds the condition "not `house_key`": the key disappears once taken.
2. **On click** lists three actions, in order: **Change a variable** (`house_key` set to true), **Notification** (*You found a key!*) and **Stay on this map**.
3. **Stay on this map** keeps the player in the room instead of going to **Leads to**, so the click does not change room.
4. To build another object this way, click **Add an action** and choose among **Change a variable**, **Notification**, **Play a sound**, **Stay on this map**, **Go to a label**, **Go to a scene**, **Call a label (then back here)** and **Return (end of a call)**. Use the up and down arrows to reorder actions. Without **Stay on this map**, the zone goes to **Leads to** after its actions.

Move the key zone onto the real drawing of a key in the picture (or draw a new one on the right object).

### 5. The locked door

1. Select the door zone to the bedroom (named `locked_door_…`).
2. **Clickable only if…** contains `house_key`. Until the key is found the zone is dimmed and cannot be clicked; afterwards it works. **Visible only if…** would hide it instead.
3. Add a message for the player: give the zone a **Tooltip** such as *Locked*, or add a second zone on the same door that only appears when the key is missing, using the condition **Visible only if…** with `!house_key` and a **Notification** action *The door is locked*.

### 6. Zones shaped like their picture

A rectangle is rarely the exact outline of an object. A zone can follow the opaque pixels of a PNG instead:

1. Import the PNG of the object (for example a vase on a transparent background) in the library.
2. Select a zone, and set its **Shape** to **Picture (opaque pixels)**. Choose the **Picture**.
3. Press **Fit to the picture** to place it at its own size, centered, or **Like the background** if the PNG is a transparent copy of the whole background where only the object is painted.
4. **Opacity threshold** decides how opaque a pixel must be to react to the mouse. Only the pixels of the object are clickable, not the transparent corners.
5. Tick **Draw the picture over the background** to show the object itself, and set a **Hover image (instead of the glow)** for a highlighted version of it when the cursor is over.

### 7. Leave the house

1. Add a zone on the front door.
2. In **Leads to**, click **Go to a scene…** and choose the next scene of your story. A label and a jump to that scene are created and the zone points at it.
3. Play from the start with Preview and walk through the whole house.

## Result

A navigable house where each room is a picture with clickable zones, a key that is picked up and remembered by a variable, a door that opens only with it, objects that react to the exact shape of their drawing, and a way out towards the rest of the story.

## Going further

- Give a zone a **Hover sound**, an **Own look** (color, glow, pulse) or turn on **Idle hint (shimmer)** to hint at where to click.
- Combine variables: several keys, an inventory counter, a flag that changes what a room contains. The **Visible only if…** and **Clickable only if…** fields take a Simple editor with several tests.
- A zone can call a small piece of story (**Call a label (then back here)**) to show a description of an object, then return to the map.
- The same **Zones** step works for a menu, a map of the world or a puzzle: it is not limited to houses.

## If something goes wrong

- **A zone is not where I drew it**: in the Stage View make sure the **Zones** step (not a line) is selected, then use **Select** to move it.
- **I cannot find the toolbar**: it only appears when a **Zones** step is selected in the Scene panel.
- **The player is stuck in a room**: check that at least one zone of the room is always visible and clickable (no condition) and has a **Leads to**. The **Problems** panel reports invalid zones (an empty shape, a variable action without variable), not a map with no way out.
- **Clicking the key changes room**: the **Stay on this map** action is missing from **On click**.
- **A Picture zone does not react**: the PNG needs a transparent background and the **Opacity threshold** must not be above the opacity of the drawing.
