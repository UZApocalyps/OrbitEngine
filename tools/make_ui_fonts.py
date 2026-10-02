"""Builds the editor's UI fonts from Inter.

Inter defines 745 glyphs in the Unicode private-use area (U+E000..U+F8FF),
exactly where the Phosphor icon font keeps its icons. In egui the first font
of a family that has a glyph wins, so Inter would shadow part of the icons.
The editor therefore embeds copies of Inter whose private-use code points were
removed from the character map (glyph shapes are untouched).

Inter is licensed under the SIL OFL 1.1 without a Reserved Font Name, which
allows modified versions; assets/fonts/OFL.txt applies to the copies too.

Usage (needs `pip install fonttools`):  python tools/make_ui_fonts.py
"""
from pathlib import Path

from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
PUA = range(0xE000, 0xF900)

for style in ("Regular", "Bold"):
    font = TTFont(ROOT / "assets" / "fonts" / f"Inter-{style}.ttf")
    removed = 0
    for table in font["cmap"].tables:
        for cp in [cp for cp in table.cmap if cp in PUA]:
            del table.cmap[cp]
            removed += 1
    out = ROOT / "assets" / "fonts" / "ui" / f"InterUI-{style}.ttf"
    font.save(out)
    print(f"{out.relative_to(ROOT)}: {removed} private-use mappings removed")
