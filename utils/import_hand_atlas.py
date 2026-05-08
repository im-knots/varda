#!/usr/bin/env python3
"""
Import a hand-drawn character grid image into a Varda VJ MSDF atlas.

Takes an image containing a grid of hand-drawn characters, auto-traces each
glyph to SVG via potrace CLI, converts to MSDF via msdfgen, then assembles
into an 8-column RGB atlas matching the format used by generate_atlases.py.

Usage:
  python3 utils/import_hand_atlas.py <image> <cols> <rows> <output_name> [--invert] [--dedup 0.92]

Example:
  python3 utils/import_hand_atlas.py secretlanguage.png 10 9 secretlanguage --invert --dedup 0.92

Output goes to shaders/character_atlases/<output_name>_font_atlas.png
"""

from PIL import Image, ImageOps, ImageFilter
import os, sys, math, argparse, subprocess, tempfile

CELL = 64         # MSDF cell size (matches generate_atlases.py)
ATLAS_COLS = 8
PX_RANGE = 4
OUT_DIR = os.path.join(os.path.dirname(__file__), '..', 'shaders', 'character_atlases')

# Resolve binaries
MSDFGEN = os.environ.get('MSDFGEN', os.path.expanduser('~/bin/msdfgen'))
POTRACE = os.environ.get('POTRACE', 'potrace')


def extract_cells(img, cols, rows):
    """Split image into a grid of (cols x rows) cells."""
    w, h = img.size
    cell_w = w / cols
    cell_h = h / rows
    cells = []
    for r in range(rows):
        for c in range(cols):
            x0 = int(c * cell_w)
            y0 = int(r * cell_h)
            x1 = int((c + 1) * cell_w)
            y1 = int((r + 1) * cell_h)
            cell = img.crop((x0, y0, x1, y1))
            cells.append(cell)
    return cells


GLYPH_FILL = 0.70


def _keep_largest_component(gray, threshold=64):
    """Zero out all white regions except the largest connected component."""
    w, h = gray.size
    px = list(gray.tobytes())
    labels = [0] * (w * h)
    component_sizes = {}
    label_id = 0

    for start in range(w * h):
        if labels[start] != 0 or px[start] <= threshold:
            continue
        label_id += 1
        queue = [start]
        labels[start] = label_id
        size = 0
        while queue:
            pos = queue.pop()
            size += 1
            y, x = divmod(pos, w)
            for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1)):
                ny, nx = y + dy, x + dx
                if 0 <= ny < h and 0 <= nx < w:
                    npos = ny * w + nx
                    if labels[npos] == 0 and px[npos] > threshold:
                        labels[npos] = label_id
                        queue.append(npos)
        component_sizes[label_id] = size

    if not component_sizes:
        return gray

    best = max(component_sizes, key=component_sizes.get)
    out = bytearray(w * h)
    for i in range(w * h):
        if labels[i] == best:
            out[i] = px[i]
    return Image.frombytes('L', (w, h), bytes(out))


def cell_to_glyph(cell, ink_max, paper_min, invert):
    """Convert a color cell into a clean binary glyph for tracing.

    Returns (glyph_image, coverage) where glyph_image is a grayscale 'L' image
    with white-on-black content, suitable for PBM conversion and potrace.
    """
    gray = cell.convert('L')

    if invert:
        gray = ImageOps.invert(gray)

    # Remap ink/paper levels
    rng = max(paper_min - ink_max, 1)
    lut = []
    for v in range(256):
        if v <= ink_max:
            lut.append(255)
        elif v >= paper_min:
            lut.append(0)
        else:
            lut.append(int(255 * (paper_min - v) / rng))
    gray = gray.point(lut, 'L')
    gray = gray.filter(ImageFilter.MedianFilter(size=3))
    gray = _keep_largest_component(gray)

    bbox = gray.getbbox()
    if bbox is None:
        return None, 0.0

    cropped = gray.crop(bbox)
    cw, ch = cropped.size
    pixels = cropped.tobytes()
    coverage = sum(pixels) / (cw * ch * 255) if cw * ch > 0 else 0.0

    # Hard threshold for clean tracing input
    cropped = cropped.point(lambda v: 255 if v > 80 else 0, 'L')

    return cropped, coverage


def glyph_to_msdf(glyph_img, tmpdir, idx):
    """Convert a binary glyph image to MSDF via potrace→SVG→msdfgen.

    Returns an RGB PIL Image of size CELL×CELL, or None on failure.
    """
    # Save as PBM (potrace input format) — white-on-black needs inverting
    # because PBM treats 1 as black ink
    pbm_path = os.path.join(tmpdir, f'glyph_{idx}.pbm')
    svg_path = os.path.join(tmpdir, f'glyph_{idx}.svg')
    msdf_path = os.path.join(tmpdir, f'glyph_{idx}.png')

    # potrace expects black = ink, so invert (our convention is white = ink)
    inverted = ImageOps.invert(glyph_img)
    inverted.save(pbm_path)

    # Trace to SVG
    result = subprocess.run(
        [POTRACE, '-s', '-o', svg_path, pbm_path],
        capture_output=True, text=True
    )
    if result.returncode != 0:
        return None

    # Convert SVG to MSDF
    result = subprocess.run(
        [MSDFGEN, 'msdf', '-svg', svg_path,
         '-size', str(CELL), str(CELL),
         '-pxrange', str(PX_RANGE),
         '-autoframe',
         '-o', msdf_path],
        capture_output=True, text=True
    )
    if result.returncode != 0:
        return None

    if not os.path.exists(msdf_path):
        return None

    return Image.open(msdf_path).convert('RGB')


SHAPE_SIZE = 48    # canonical size for shape normalization
SHAPE_BLUR = 3     # Gaussian blur radius to normalize stroke width
COARSE_GRID = 12   # downsampled grid for coarse shape signature


def _hu_moments(pixels, w, h):
    """Compute 7 Hu invariant moments from raw pixel bytes."""
    m00 = m10 = m01 = 0
    for y in range(h):
        row = y * w
        for x in range(w):
            v = pixels[row + x]
            m00 += v
            m10 += x * v
            m01 += y * v

    if m00 == 0:
        return [0.0] * 7

    cx = m10 / m00
    cy = m01 / m00

    mu20 = mu02 = mu11 = mu30 = mu03 = mu21 = mu12 = 0.0
    for y in range(h):
        row = y * w
        dy = y - cy
        dy2 = dy * dy
        dy3 = dy2 * dy
        for x in range(w):
            v = pixels[row + x]
            if v == 0:
                continue
            dx = x - cx
            dx2 = dx * dx
            mu20 += dx2 * v
            mu02 += dy2 * v
            mu11 += dx * dy * v
            mu30 += dx2 * dx * v
            mu03 += dy3 * v
            mu21 += dx2 * dy * v
            mu12 += dx * dy2 * v

    # Normalize central moments
    def eta(mu_pq, p, q):
        return mu_pq / (m00 ** ((p + q) / 2.0 + 1))

    n20 = eta(mu20, 2, 0)
    n02 = eta(mu02, 0, 2)
    n11 = eta(mu11, 1, 1)
    n30 = eta(mu30, 3, 0)
    n03 = eta(mu03, 0, 3)
    n21 = eta(mu21, 2, 1)
    n12 = eta(mu12, 1, 2)

    # 7 Hu invariants
    h1 = n20 + n02
    h2 = (n20 - n02) ** 2 + 4 * n11 ** 2
    h3 = (n30 - 3 * n12) ** 2 + (3 * n21 - n03) ** 2
    h4 = (n30 + n12) ** 2 + (n21 + n03) ** 2
    h5 = ((n30 - 3 * n12) * (n30 + n12) *
          ((n30 + n12) ** 2 - 3 * (n21 + n03) ** 2) +
          (3 * n21 - n03) * (n21 + n03) *
          (3 * (n30 + n12) ** 2 - (n21 + n03) ** 2))
    h6 = ((n20 - n02) * ((n30 + n12) ** 2 - (n21 + n03) ** 2) +
          4 * n11 * (n30 + n12) * (n21 + n03))
    h7 = ((3 * n21 - n03) * (n30 + n12) *
          ((n30 + n12) ** 2 - 3 * (n21 + n03) ** 2) -
          (n30 - 3 * n12) * (n21 + n03) *
          (3 * (n30 + n12) ** 2 - (n21 + n03) ** 2))

    return [h1, h2, h3, h4, h5, h6, h7]


def _extract_shape_features(glyph):
    """Extract scale-invariant shape features for deduplication.

    Returns (hu_moments, coarse_bytes, aspect_ratio).
    """
    bbox = glyph.getbbox()
    if not bbox:
        return [0.0] * 7, bytes(COARSE_GRID * COARSE_GRID), 1.0

    cropped = glyph.crop(bbox)
    bw, bh = cropped.size
    aspect = bw / bh if bh > 0 else 1.0

    # Resize to canonical size (removes scale dependency)
    canonical = cropped.resize((SHAPE_SIZE, SHAPE_SIZE), Image.BILINEAR)
    pixels = canonical.tobytes()

    # Binarize
    binary = bytes(255 if v > 128 else 0 for v in pixels)

    # Hu moments from binary image
    hu = _hu_moments(binary, SHAPE_SIZE, SHAPE_SIZE)

    # Coarse shape signature: blur then downsample
    bin_img = Image.frombytes('L', (SHAPE_SIZE, SHAPE_SIZE), binary)
    blurred = bin_img.filter(ImageFilter.GaussianBlur(SHAPE_BLUR))
    coarse = blurred.resize((COARSE_GRID, COARSE_GRID), Image.BILINEAR)
    coarse_bytes = coarse.tobytes()

    return hu, coarse_bytes, aspect


def _shape_similarity(feat_a, feat_b):
    """Scale-invariant similarity using Hu moments, coarse shape, and aspect ratio."""
    hu_a, coarse_a, aspect_a = feat_a
    hu_b, coarse_b, aspect_b = feat_b

    # 1. Hu moment similarity (40%) — log-space L1
    hu_dist = sum(abs(math.log10(abs(a) + 1e-30) - math.log10(abs(b) + 1e-30))
                  for a, b in zip(hu_a, hu_b))
    hu_sim = max(0.0, 1.0 - hu_dist / 20.0)

    # 2. Coarse shape cosine similarity (45%)
    dot = sum(a * b for a, b in zip(coarse_a, coarse_b))
    na = sum(a * a for a in coarse_a) ** 0.5
    nb = sum(b * b for b in coarse_b) ** 0.5
    coarse_sim = dot / (na * nb) if na > 0 and nb > 0 else 0.0

    # 3. Aspect ratio similarity (15%)
    ar_diff = abs(aspect_a - aspect_b) / max(aspect_a, aspect_b, 0.01)
    aspect_sim = 1.0 - min(ar_diff, 1.0)

    return 0.40 * hu_sim + 0.45 * coarse_sim + 0.15 * aspect_sim


def deduplicate(glyphs, similarity_threshold):
    """Remove near-duplicate glyphs using multi-signal similarity.

    Combines pixel comparison, structural shape features, and aspect ratio
    to catch visually similar glyphs even with slight positional shifts.
    """
    features = [_extract_shape_features(g) for g, _ in glyphs]
    keep = [True] * len(glyphs)

    for i in range(len(glyphs)):
        if not keep[i]:
            continue
        for j in range(i + 1, len(glyphs)):
            if not keep[j]:
                continue
            sim = _shape_similarity(features[i], features[j])
            if sim >= similarity_threshold:
                if glyphs[i][1] >= glyphs[j][1]:
                    keep[j] = False
                else:
                    keep[i] = False
                    break

    result = [g for g, k in zip(glyphs, keep) if k]
    removed = len(glyphs) - len(result)
    if removed:
        print(f"  Dedup: removed {removed} near-duplicates (threshold {similarity_threshold:.0%}), {len(result)} unique")
    return result


def make_atlas(msdf_images, name, filename):
    """Assemble MSDF glyph images into an 8-column RGB grid atlas."""
    n = len(msdf_images)
    if n == 0:
        print(f"  {name}: EMPTY — skipped")
        return 0
    cols = min(n, ATLAS_COLS)
    rows = math.ceil(n / cols)
    atlas = Image.new('RGB', (cols * CELL, rows * CELL), (0, 0, 0))
    for i, img in enumerate(msdf_images):
        if img.size != (CELL, CELL):
            img = img.resize((CELL, CELL), Image.LANCZOS)
        atlas.paste(img, ((i % cols) * CELL, (i // cols) * CELL))
    os.makedirs(OUT_DIR, exist_ok=True)
    path = os.path.join(OUT_DIR, filename)
    atlas.save(path)
    size_kb = os.path.getsize(path) / 1024
    print(f"  {name}: {n} chars, {cols}x{rows} grid, {atlas.size[0]}x{atlas.size[1]}px, {size_kb:.1f}KB")
    print(f"  Saved to: {path}")
    return n


def main():
    parser = argparse.ArgumentParser(description='Import hand-drawn character grid into MSDF atlas')
    parser.add_argument('image', help='Path to the source grid image')
    parser.add_argument('cols', type=int, help='Number of character columns in the grid')
    parser.add_argument('rows', type=int, help='Number of character rows in the grid')
    parser.add_argument('name', help='Atlas name (output: <name>_font_atlas.png)')
    parser.add_argument('--ink-max', type=int, default=50,
                        help='Darkest end of remap: pixels <= this are solid ink (default: 50)')
    parser.add_argument('--paper-min', type=int, default=140,
                        help='Lightest end of remap: pixels >= this are paper/background (default: 140)')
    parser.add_argument('--min-coverage', type=float, default=0.01,
                        help='Minimum coverage to include a cell (default: 0.01)')
    parser.add_argument('--invert', action='store_true',
                        help='Use if source has light glyphs on dark background')
    parser.add_argument('--max-chars', type=int, default=128,
                        help='Maximum characters in output atlas (default: 128)')
    parser.add_argument('--dedup', type=float, default=0.85,
                        help='Similarity threshold for deduplication (0.0-1.0, default: 0.85). Set to 1.0 to disable.')
    args = parser.parse_args()

    print(f"Loading {args.image}...")
    img = Image.open(args.image)
    print(f"  Source: {img.size[0]}x{img.size[1]}px, {args.cols}x{args.rows} grid")
    print(f"  Cell size: ~{img.size[0]/args.cols:.0f}x{img.size[1]/args.rows:.0f}px")
    print(f"  Ink max: {args.ink_max}, Paper min: {args.paper_min}")

    cells = extract_cells(img, args.cols, args.rows)
    print(f"  Extracted {len(cells)} cells")

    # Extract and clean glyphs (binary images for dedup + tracing)
    glyphs = []
    for i, cell in enumerate(cells):
        glyph, coverage = cell_to_glyph(cell, args.ink_max, args.paper_min, args.invert)
        if glyph is None or coverage < args.min_coverage:
            continue
        glyphs.append((glyph, coverage))

    print(f"  Valid glyphs: {len(glyphs)} (filtered {len(cells) - len(glyphs)} empty/low-coverage)")

    # Remove near-duplicate glyphs
    if args.dedup < 1.0:
        glyphs = deduplicate(glyphs, args.dedup)

    # If too many, subsample evenly sorted by coverage
    if len(glyphs) > args.max_chars:
        glyphs.sort(key=lambda x: x[1])
        step = len(glyphs) / args.max_chars
        glyphs = [glyphs[min(int(i * step), len(glyphs) - 1)] for i in range(args.max_chars)]
        print(f"  Subsampled to {len(glyphs)} chars (max {args.max_chars})")

    # Convert each glyph to MSDF via potrace→SVG→msdfgen
    print(f"  Converting {len(glyphs)} glyphs to MSDF...")
    msdf_images = []
    with tempfile.TemporaryDirectory() as tmpdir:
        for i, (glyph_img, _cov) in enumerate(glyphs):
            msdf = glyph_to_msdf(glyph_img, tmpdir, i)
            if msdf is not None:
                msdf_images.append(msdf)
            else:
                print(f"    Warning: glyph {i} failed MSDF conversion, skipping")

    if not msdf_images:
        print(f"  {args.name}: No glyphs converted successfully")
        return

    filename = f"{args.name}_font_atlas.png"
    make_atlas(msdf_images, args.name, filename)


if __name__ == '__main__':
    main()
