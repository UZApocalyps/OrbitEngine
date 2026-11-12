# Translations (PO and CSV)

Every text of your story (lines, character names, choices, notifications and the tooltips of clickable zones) can be translated into other languages. The language you write in is the **source language**.

## Adding a language

Open the **Translations** panel, type a language tag (for example `fr` or `pt-BR`) and press **Create**. The panel shows how complete the translation is: how many texts are translated, how many are missing, and how many no longer exist in the story (orphans).

## Translating

You have two ways to work:

- **In the editor**: fill in the translated text next to each source text.
- **In another tool**: export the translation as a **PO** file (used by many translation tools) or a **CSV** file (opens in any spreadsheet), send it to your translator, then **Import** the result. The file type is recognized from its extension (.po or .csv). Import updates the texts and reports how many changed.

## Stale translations

If you rewrite a source text after it was translated, the translation is marked **stale**: it may no longer match. The panel lists them so you can review each one, or press **Mark all as reviewed** when they are still correct. For projects translated before this feature existed, **Adopt as up to date** records the current source texts as the reference.

## Fonts for other languages

Texts are drawn with fonts that support the language, including Chinese, Japanese and Korean characters, with a fallback when a glyph is missing.

The default font only covers Latin scripts. For another language, import a font that has its characters (Library, TTF or OTF), then open the interface editor, **Theme** tab: add the font in **Fonts**, and in **Fonts per language** add it to the language. The list is a fallback chain, preferred font first. It applies to the dialogue box, the choices and every menu. A warning appears there, and in the export log, when some characters of a translation cannot be drawn by any font of its language.

Japanese and Chinese lines wrap between characters without needing spaces, and punctuation such as `。` or `」` is never left alone at the start of a line. Arabic and Hebrew texts are shaped and ordered right to left, and right-aligned.

## In the game

The player chooses the language in the preferences screen; the game starts in the language set in the export profile when none is chosen.
