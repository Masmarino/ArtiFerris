"""Subsets the site's typefaces to what its five languages need, into public/fonts/ and scripts/assets/fonts/.

The sources are the @fontsource devDependencies, so run `npm ci` first. Needs `pip install fonttools brotli`. Both fonts
are under the SIL Open Font License (public/fonts/OFL-ibm-plex.txt). The output is committed and a normal build does not run this.

    python3 scripts/subset-fonts.py
"""

from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "public" / "fonts"
ASSETS = ROOT / "scripts" / "assets" / "fonts"

# Basic Latin and Latin-1 (every accented letter of fr, it, es, de), the oe ligature, typographic quotes and dashes, the ellipsis, a bullet, the minus sign and the euro.
UNICODES = (
    list(range(0x20, 0x7F))
    + list(range(0xA0, 0x100))
    + [0x152, 0x153, 0x2013, 0x2014, 0x2018, 0x2019, 0x201A, 0x201C, 0x201D, 0x201E, 0x2022, 0x2026, 0x2212, 0x20AC]
)

# fontsource splits each font by script; its "latin" file already holds Latin-1.
PLEX = ROOT / "node_modules/@fontsource/{family}/files/{family}-latin-{weight}-normal.woff2"


def subset_font(font: TTFont, name: str) -> None:
    options = subset.Options()
    options.flavor = "woff2"
    options.layout_features = ["kern", "liga", "calt", "ccmp", "locl", "mark", "mkmk", "zero"]
    options.drop_tables += ["DSIG"]
    options.name_IDs = [1, 2, 3, 4, 6]
    options.notdef_outline = True
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=UNICODES)
    subsetter.subset(font)
    font.flavor = "woff2"
    font.save(str(OUT / name))
    print(name, (OUT / name).stat().st_size)


# IBM Plex, one family for the whole site: Sans for the text and the interface, Sans Condensed for headings, Mono for
# code, paths, hashes and the figures.
FACES = [
    ("ibm-plex-sans", 400),
    ("ibm-plex-sans", 500),
    ("ibm-plex-sans", 600),
    ("ibm-plex-sans-condensed", 600),
    ("ibm-plex-mono", 400),
    ("ibm-plex-mono", 500),
]
for family, weight in FACES:
    subset_font(TTFont(str(PLEX).format(family=family, weight=weight)), f"{family}-{weight}.woff2")

# scripts/build-images.mjs draws the social image's text with opentype.js, which reads TrueType, not WOFF2: write the
# condensed semibold and the regular Plex Mono as TTF in scripts/assets/fonts. Only kerning is kept in these copies, the
# one lookup opentype.js applies.
ASSETS.mkdir(parents=True, exist_ok=True)
for name in ("ibm-plex-sans-condensed-600", "ibm-plex-mono-400"):
    font = TTFont(str(OUT / f"{name}.woff2"))
    kern_only = subset.Options()
    kern_only.layout_features = ["kern"]
    subsetter = subset.Subsetter(kern_only)
    subsetter.populate(unicodes=UNICODES)
    subsetter.subset(font)
    font.flavor = None
    font.save(str(ASSETS / f"{name}.ttf"))
