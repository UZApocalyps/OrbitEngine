# Tutorial: achievements, notifications and translations

Three finishing touches that make a game feel complete: rewards the player collects, short messages that pop up without stopping the story, and a second language. They are independent: follow only the parts you need.

## Part 1. Achievements

### Goal

Define two achievements, one visible and one hidden, unlock them from the story, and see the notification and the Achievements screen of the game.

### Before you start

- A project with a story that has at least one scene and a choice (the one from *Tutorial: your first visual novel* is perfect).
- Two small square PNG icons, if you want your own (a default medal is used otherwise).

### Steps

1. Open **View > Panels > Editing > Show: Achievements**. The **Achievements** panel opens.
2. Click **Add**. A **New achievement** appears in the list. Fill in the **Title** (for example `First words`), the **Description** (what the player did) and, if you have one, an **Icon**.
3. Add a second one, `Generous`, for the player who shared their lunch. Tick **Hidden until unlocked**: the list will show a placeholder instead of its title, description and icon until it is unlocked. You can also set a **Locked icon**, shown while the achievement is still locked.
4. Now unlock them. In the **Scene** panel, select the step after which the player earns the first one (for example the first line of the story), then insert an **Unlock achievement** step: press Ctrl+I and type `unlock`, or use the button **Add an unlock node here** of the Achievements panel, which inserts it for you.
5. In the **Inspector**, pick the **Achievement** to unlock. Do the same under the label of the generous branch for `Generous`.
6. The panel shows, under each achievement, **Unlocked by** the steps that unlock it. A message **No node unlocks this achievement yet.** warns you of a forgotten one; the **Problems** panel reports it too.
7. Play with **Preview** (F6). When the story reaches the step, a toast announces the achievement. The preview never records unlocks, so you can replay as often as you like.

### Result

Achievements that unlock once, are remembered between playthroughs (even if the player deletes their saves), and are listed on the **Achievements** screen opened from the main menu (it only appears when the story defines achievements).

### Going further

- The achievement titles and descriptions are ordinary texts: they are translated with the rest (see Part 3).
- Restyle the toast with the `achievement` style (Part 2) and the Achievements screen in the **Interface** panel.
- Build your own screen with a generated list whose **List** is **Achievements** (see *Interface editor and splash screens*).

### If something goes wrong

- **Nothing happens at the step**: the step is only counted the first time in a real playthrough. In a scene replay it does nothing.
- **The step says "Unknown achievement"**: the achievement it pointed at was deleted. Pick another one.
- **The icon is refused**: it must be an image declared in the project; import it in the library first.

## Part 2. Notifications (toasts)

### Goal

Show a short message in a corner of the screen, in a style of your own, without ever stopping the story.

### Steps

1. In the Scene panel, select a step and insert a **Notification** step (press Ctrl+I and type `notif`).
2. In the **Inspector**, write the text. You can use markup such as `{b}important{/b}` and put a variable in square brackets, for example `Friendship: [friendship]`: its value is shown when the toast appears.
3. Optionally pick an **Icon** (an image), a **Sound**, an **Own duration** in seconds, and a **Style**. The list of styles comes from the theme; **or a style of the theme…** accepts the name of a style you created.
4. To design the look, open **Tools > Interface editor**, tab **Notifications**. Set the anchor, width, animation and the maximum number of toasts. In the style list type a name in **new style name** and click **Add a notification style** (for example `hint`). Style its **Background**, **Border**, **Corner radius**, **Text size** and **Text color**, then click **Show test notification** to see it over the preview.
5. Back in the Notification step, choose your style. Achievements use the `achievement` style and the engine's own messages the `system` style, so restyling those changes them everywhere.

### Result

Short messages that never block the story. They are never saved: going back in the story or loading a save removes the toasts that were on screen.

### Going further

- A zone in a point and click map or a button in a screen can show one too, with the action **Notification** (see *Tutorial: point and click, explore a house*).
- In **While skipping**, decide whether toasts are shown, shortened or hidden while the player skips text.
- A text written in a notification is translated like any other text.

## Part 3. Translate your game

### Goal

Add a second language and bring it to 100 percent, with the help of a spreadsheet.

### Steps

1. Open **View > Panels > Editing > Show: Translations**. The **Translations** panel opens.
2. In the field **new language**, type a language tag (`fr`, `es`, `pt-BR`...) and click **Create**. The file `i18n/story.<tag>.ron` is created next to your story. The language you write in is the source language and cannot be created as a translation.
3. The panel shows the coverage, for example *0/42 translated, 42 missing, 0 orphaned*. Every text counts: lines, choices, character names, notification texts, achievement titles and descriptions.
4. To translate in a spreadsheet, click **Export CSV...** (or **Export PO...** for translation tools such as Poedit, or use **File > Import, export and translations**). The panel confirms how many strings were exported and where.
5. Send the file to your translator, or fill it in yourself. Keep the file format and the identifiers.
6. When it is back, click **Import PO/CSV...**. The panel tells you how many strings were updated and how many are stale. The file type is recognized from its extension (`.po` or `.csv`).
7. Check the coverage: *missing* should be 0. If you later rewrite a source text, its translation is flagged in **Stale translations**; review each one, or click **Mark all as reviewed** if they are still correct. **Adopt as up to date** is for projects translated before this check existed.
8. For a language with its own alphabet (Chinese, Japanese, Korean, Cyrillic...) import a font that has the characters (TTF or OTF in the library), then open **Tools > Interface editor**, tab **Theme**: add the font under **Fonts** and add it to the language under **Fonts per language**.
9. Test it in an exported game: in the Export panel set **Default language** to your new tag, export, and play. The player can also switch language in the **Preferences** screen of the game.

### Result

A game that starts in the language of your choice and offers the others in its preferences, with a translation you can update whenever the story changes.

### Going further

- In the **Interface** panel, the language selector at the top shows the menu texts (`ui.*`) in English, French or the story language, to check how your screens look in another language.
- Add voice files per language with **Import voices…** on a line (see *Characters and animations* and the Inspector).
- Keep the CSV under version control: its diff shows exactly which texts changed.

### If something goes wrong

- **"Unknown file type"**: only `.po` and `.csv` files can be imported.
- **Characters shown as squares**: no font of the language has them. The Interface editor and the export log tell you which characters are missing.
- **Some texts stay in the source language**: they are listed as *missing* in the panel; fill them in and import again.
- **The panel does not let me type translations**: editing is done outside the editor (spreadsheet or PO tool) and imported back.
