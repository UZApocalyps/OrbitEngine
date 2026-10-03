# Translations (PO and CSV)

Every text of your story (lines, character names, choices) can be translated into other languages. The language you write in is the **source language**.

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

## In the game

The player chooses the language in the preferences screen; the game starts in the language set in the export profile when none is chosen.
