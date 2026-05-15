#!/usr/bin/env python3
"""
MSDF character atlas generator for Varda VJ.
Uses msdf-atlas-gen to produce 48px/cell RGB MSDF atlases (8 cols, uniform grid).
Filters out combining marks, punctuation, control chars via category filtering.

Usage:
  python3 utils/generate_atlases.py

Output goes to shaders/character_atlases/
"""

import unicodedata, os, math, subprocess, tempfile, json

CELL = 64           # default cell size
CELL_COMPLEX = 128  # for scripts with detailed/thin-stroke glyphs
COLS = 8
MAX_CHARS = 128
PX_RANGE = 4
OUT_DIR = os.path.join(os.path.dirname(__file__), '..', 'shaders', 'character_atlases')

# Unicode categories to exclude
BAD_CATEGORIES = {
    'Mn', 'Mc', 'Me',                              # combining marks
    'Cc', 'Cf', 'Cn',                              # control, format, unassigned
    'Zs', 'Zl', 'Zp', 'Cs', 'Co',                 # whitespace, surrogates, private use
    'Pc', 'Pd', 'Ps', 'Pe', 'Pi', 'Pf', 'Po',     # punctuation
    'Nd', 'Nl', 'No',                              # numbers (decimal, letter, other)
    'Sc', 'Sk',                                     # currency, modifier symbols
}

# Resolve msdf-atlas-gen binary
MSDF_ATLAS_GEN = os.environ.get('MSDF_ATLAS_GEN',
    os.path.expanduser('~/bin/msdf-atlas-gen'))


def filter_codepoints(codepoints, filter_cats=True):
    """Filter codepoints by Unicode category."""
    if not filter_cats:
        return codepoints
    return [cp for cp in codepoints if unicodedata.category(chr(cp)) not in BAD_CATEGORIES]


def subsample(codepoints, max_n):
    """Evenly subsample a list down to max_n items."""
    if len(codepoints) <= max_n:
        return codepoints
    step = len(codepoints) / max_n
    picked = [codepoints[min(int(i * step), len(codepoints) - 1)] for i in range(max_n)]
    return picked


def write_charset_file(codepoints, path):
    """Write codepoints as hex values for msdf-atlas-gen -charset."""
    with open(path, 'w') as f:
        for cp in codepoints:
            f.write(f"0x{cp:04X}\n")


def generate_msdf_atlas(font_path, codepoints, output_png, name,
                        filter_cats=True, cell_size=None):
    """Generate an MSDF atlas using msdf-atlas-gen CLI."""
    cell = cell_size or CELL
    # Filter and subsample
    cps = filter_codepoints(codepoints, filter_cats)
    if len(cps) > 1000:
        cps = subsample(cps, 500)
    cps = subsample(cps, MAX_CHARS)

    if not cps:
        print(f"  {name:25s}: EMPTY — skipped")
        return 0

    with tempfile.TemporaryDirectory() as tmpdir:
        charset_file = os.path.join(tmpdir, 'charset.txt')
        write_charset_file(cps, charset_file)

        json_file = os.path.join(tmpdir, 'atlas.json')
        img_file = os.path.join(tmpdir, 'atlas.png')

        cmd = [
            MSDF_ATLAS_GEN,
            '-font', font_path,
            '-type', 'msdf',
            '-format', 'png',
            '-imageout', img_file,
            '-json', json_file,
            '-pxrange', str(PX_RANGE),
            '-uniformgrid',
            '-uniformcols', str(COLS),
            '-uniformcell', str(cell), str(cell),
            '-charset', charset_file,
        ]

        # Variable fonts need -varfont instead of -font
        if '[' in font_path:
            cmd[1] = '-varfont'

        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode != 0:
            print(f"  {name:25s}: FAILED")
            print(f"    stderr: {result.stderr.strip()}")
            return 0

        # Read JSON to get actual glyph count
        with open(json_file) as f:
            meta = json.load(f)
        n_glyphs = len(meta.get('glyphs', []))

        # Copy output to final location
        import shutil
        out_path = os.path.join(OUT_DIR, output_png)
        shutil.copy2(img_file, out_path)

        # Get file size
        size_kb = os.path.getsize(out_path) / 1024
        print(f"  {name:25s}: {n_glyphs:3d} chars, {size_kb:.1f}KB")

    return n_glyphs


# ── Font paths ────────────────────────────────────────────────────────
HOME = os.path.expanduser('~')
SYMBOLS_PATH = '/System/Library/Fonts/Apple Symbols.ttf'
ARIAL_PATH = '/System/Library/Fonts/Supplemental/Arial Unicode.ttf'
if not os.path.exists(ARIAL_PATH):
    ARIAL_PATH = SYMBOLS_PATH

# Noto fonts (installed via brew)
NOTO_CUNEIFORM  = os.path.join(HOME, 'Library/Fonts/NotoSansCuneiform-Regular.ttf')
NOTO_ETHIOPIC   = os.path.join(HOME, 'Library/Fonts/NotoSansEthiopic[wdth,wght].ttf')
NOTO_HIERO      = os.path.join(HOME, 'Library/Fonts/NotoSansEgyptianHieroglyphs-Regular.ttf')
NOTO_LINEARB    = os.path.join(HOME, 'Library/Fonts/NotoSansLinearB-Regular.ttf')
NOTO_PHOENICIAN = os.path.join(HOME, 'Library/Fonts/NotoSansPhoenician-Regular.ttf')

# Handwritten font for physics/math (Bradley Hand has Greek + Latin + basic math)
BRADLEY_HAND_PATH = '/System/Library/Fonts/Supplemental/Bradley Hand Bold.ttf'

# ── Script definitions ────────────────────────────────────────────────
# (name, font_path, codepoints, filename, filter_cats, cell_size)
# Complex/thin-stroke scripts use CELL_COMPLEX for adequate MSDF resolution.
C = CELL
CC = CELL_COMPLEX
SCRIPTS = [
    ("Arabic",      ARIAL_PATH,      list(range(0x0621, 0x064B)),   "arabic_font_atlas.png",    True,  C),
    ("ASCII",       SYMBOLS_PATH,    list(range(0x0041, 0x005B)),   "ascii_font_atlas.png",     True,  C),
    ("Binary",      SYMBOLS_PATH,    [0x0030, 0x0031],              "binary_font_atlas.png",    False, C),
    ("Chinese",     ARIAL_PATH,      list(range(0x4E00, 0x9FFF)),   "chinese_font_atlas.png",   True,  C),
    ("Cuneiform",   NOTO_CUNEIFORM,  list(range(0x12000, 0x1237F)), "cuneiform_font_atlas.png", True,  CC),
    ("Devanagari",  ARIAL_PATH,      list(range(0x0900, 0x097F)),   "devanagari_font_atlas.png",True,  C),
    ("Ethiopic",    NOTO_ETHIOPIC,   list(range(0x1200, 0x137F)),   "ethiopic_font_atlas.png",  True,  C),
    ("Hangul",      ARIAL_PATH,      list(range(0xAC00, 0xD7AF)),   "hangul_font_atlas.png",    True,  C),
    ("Hieroglyphs", NOTO_HIERO,      list(range(0x13000, 0x1342F)), "hiero_font_atlas.png",     True,  CC),
    ("Katakana",    ARIAL_PATH,      list(range(0x30A0, 0x30FF)),   "katakana_font_atlas.png",  True,  C),
    ("LinearB",     NOTO_LINEARB,    list(range(0x10000, 0x1007F)), "linearb_font_atlas.png",   True,  CC),
    ("Phoenician",  NOTO_PHOENICIAN, list(range(0x10900, 0x1091F)), "phoenician_font_atlas.png",True,  C),
    ("Sanskrit",    ARIAL_PATH,      list(range(0x0900, 0x097F)) + list(range(0x1CD0, 0x1CFF)) + list(range(0xA8E0, 0xA8FF)), "sanskrit_font_atlas.png", True, C),
]

# Physics / math for SM Lagrangian (Bradley Hand Bold coverage only)
PHYSICS_CPS = (
    # Latin uppercase
    [0x0041, 0x0042, 0x0043, 0x0044, 0x0046, 0x0047, 0x0048, 0x004C,
     0x004D, 0x004E, 0x0050, 0x0051, 0x0052, 0x0053, 0x0054, 0x0055,
     0x0056, 0x0057, 0x0059] +
    # Latin lowercase
    [0x0062, 0x0063, 0x0064, 0x0065, 0x0067, 0x0068, 0x0069, 0x006C,
     0x006E, 0x0071, 0x0072, 0x0073, 0x0074, 0x0075] +
    # Greek uppercase
    [0x0391, 0x0392, 0x0393, 0x0394, 0x0395, 0x0396, 0x0397, 0x0398,
     0x039A, 0x039B, 0x039C, 0x039D, 0x039E, 0x03A0, 0x03A1, 0x03A3,
     0x03A4, 0x03A5, 0x03A6, 0x03A7, 0x03A8, 0x03A9] +
    # Greek lowercase α-ω
    list(range(0x03B1, 0x03CA)) +
    # Digits 0-9
    list(range(0x0030, 0x003A)) +
    [
        0x2211,  # ∑ summation
        0x222B,  # ∫ integral
        0x2202,  # ∂ partial derivative
        0x2207,  # ∇ nabla
        0x221E,  # ∞ infinity
        0x00D7,  # × multiplication
        0x2260,  # ≠ not equal
        0x2264,  # ≤ less-equal
        0x2265,  # ≥ greater-equal
        0x2248,  # ≈ approximately
        0x2020,  # † dagger (Hermitian conjugate)
        0x221A,  # √ square root
        0x0028,  # ( left paren
        0x0029,  # ) right paren
        0x005B,  # [ left bracket
        0x005D,  # ] right bracket
        0x003D,  # = equals
        0x002B,  # + plus
        0x002D,  # - minus
        0x002F,  # / solidus
    ]
)

WITCHY_CPS = list(range(0x1F700, 0x1F774)) + [
    0x2609, 0x260A, 0x260B,
    0x2640, 0x2642, 0x2643, 0x2644, 0x2645, 0x2646, 0x2647,
    0x2648, 0x2649, 0x264A, 0x264B, 0x264C, 0x264D,
    0x264E, 0x264F, 0x2650, 0x2651, 0x2652, 0x2653,
    0x26B3, 0x26B4, 0x26B5, 0x26B6, 0x26B7, 0x26B8,
]

# ── Main ──────────────────────────────────────────────────────────────
if __name__ == '__main__':
    os.makedirs(OUT_DIR, exist_ok=True)
    print(f"Generating MSDF atlases ({CELL}px/cell, {COLS} cols, pxRange={PX_RANGE})\n")

    results = {}
    for name, font_path, cps, filename, fc, cs in SCRIPTS:
        print(f"  Processing {name} ({len(cps)} codepoints, {cs}px cell)...")
        n = generate_msdf_atlas(font_path, cps, filename, name,
                                filter_cats=fc, cell_size=cs)
        results[name] = n

    # Physics (handwritten Bradley Hand) — skip category filtering, thin strokes
    print(f"  Processing Physics ({len(PHYSICS_CPS)} codepoints, {CELL_COMPLEX}px cell)...")
    n = generate_msdf_atlas(BRADLEY_HAND_PATH, PHYSICS_CPS, "physics_font_atlas.png",
                            "Physics", filter_cats=False, cell_size=CELL_COMPLEX)
    results["Physics"] = n

    # Witchy (alchemical + astrology) — skip category filtering, thin strokes
    print(f"  Processing Witchy ({len(WITCHY_CPS)} codepoints, {CELL_COMPLEX}px cell)...")
    n = generate_msdf_atlas(SYMBOLS_PATH, WITCHY_CPS, "witchy_font_atlas.png",
                            "Witchy", filter_cats=False, cell_size=CELL_COMPLEX)
    results["Witchy"] = n

    print(f"\nDone: {sum(1 for v in results.values() if v > 0)}/{len(results)} atlases")
    print("\nChar counts:")
    for name in sorted(results):
        print(f"  {name}: {results[name]}")
