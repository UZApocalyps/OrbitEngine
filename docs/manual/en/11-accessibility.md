# Accessibility

OrbitEngine games are built so that more people can play them. This chapter lists what is available today.

## Screen reader

Players who use a screen reader can turn on the **screen reader** option in the preferences screen. The game then describes its screens, buttons, sliders and dialogue to the reader (on desktop through the system accessibility service, on the web through a hidden page structure placed next to the game). New dialogue lines are announced as they appear.

For this to work well, give your own screens meaningful texts: a button labelled with words is announced with those words, an image-only button is not. Keep this in mind in the Interface editor.

## Keyboard and game controller

Menus and the game can be played without a mouse: the keyboard and game controllers are supported, and the controller buttons mapped to game actions are kept in the player's preferences. On a phone or tablet, the game answers to touch.

## Comfort settings

The preferences screen lets players adjust the text speed, the auto-advance delay, the skip mode, the volume of each sound category and fullscreen.

## Text size, high contrast and reduced motion

Three more options are in the preferences screen. All of them are off by default, so a game looks and plays exactly as you designed it until a player changes them. They are saved with the player's other preferences.

- **Text size** scales the dialogue, the choices and the menu text from 100% to 160%, in steps of 10%. The dialogue box grows upward and the choice buttons grow with the text, so lines never run out of their box.
- **High contrast** replaces the colors of your theme with white text on black backgrounds, yellow highlights and thick borders. Your sizes, fonts and layout are kept, and so are your style names. The mode is applied on top of your theme: it never changes your files.
- **Reduce motion** skips the animations. Transitions between backgrounds become cuts, and effects such as moves, shakes, zooms and flashes, as well as timelines, jump straight to their end state, so the scene is exactly the same as if the animation had played out. The story does not wait for them. Timed pauses that you wrote with a wait are kept. The blinking "continue" marker stops blinking.

Your own preferences screen can offer the same options: in the Interface editor, bind a slider to **Text size**, and toggles to **High contrast**, **Reduce motion** and **Screen reader**. The stock preferences screen already has them. A screen made before these options existed keeps working; it just does not show them until you add the controls.

### Checking your theme

The Interface editor warns about theme colors that are hard to read: in the Theme tab and in the problems list, each pair of text and background colors below the WCAG AA contrast (4.5:1 for text, 3:1 for the parts of sliders and toggles) is listed with its ratio. A warning does not stop your game from being exported. Players who need more contrast can still switch on the high-contrast mode, which always passes the check.

## What you can do as an author

- Write choices with clear wording, not only colors or positions.
- Do not rely on sound alone to give information; also write it in a line of text.
- Check your custom screens with the keyboard only.
