# Projection Mapping

## Basic Projection

### Single Output — No Surfaces

The simplest setup: one output window displaying the full mixer output on one projector or display.

1. Click **"+ Output"** in the right panel — a floating output window appears
2. Select a **display target** from the dropdown (enumerates connected monitors)
3. Click **Fullscreen** to send the window to that display

No surfaces are needed. The output receives the full master mix directly.

### Single Output — Surfaces for Spatial Mapping

Surfaces let you place content in specific regions of the output. Use them when the physical projection surface isn't a simple rectangle, or when different regions need different content.

1. Open the **Stage Editor** (center panel toggle)
2. Pick a drawing tool from the toolbar (keyboard shortcut in parentheses):
   - **⬚ Select** (S) — select and edit existing surfaces
   - **▭ Rectangle** (R) — drag out a rectangle
   - **⬠ Polygon** (P) — click to place vertices, double-click to finish
   - **⬤ Circle** (C) — circle / N-gon with an interactive radius handle
   - **✒ Bezier** — curve or straighten a surface edge, and drag bezier anchors/handles
3. Set each surface's **content source**:
   - **Master** — the full mixer output
   - **Channel** — a single channel's output
   - **Channels** — a sub-mix of selected channels
   - **Deck** — a single deck's raw output
4. Set the **content mapping** mode:
   - **Fill** — entire source texture scaled to fill the surface
   - **Mapped** — surface position on the canvas determines the UV crop (multiple surfaces show different slices of one continuous image)
5. Assign surfaces to the output (output panel → "+ Assign Surface" dropdown)

### Vertex Editing

- **Drag vertices** to reposition them
- **Double-click an edge** to insert a new vertex
- **Snap-to-grid** for precise alignment
- **D** — Duplicate selected surface
- **H / V** — Flip horizontal / vertical
- **🔗 Combine** (G) — merge selected surfaces into one
- **⤒ Front / ⤓ Back** — restack the selected surface (see [Stacking Order](#stacking-order-layers))

Hit-testing uses the raw cursor position, so vertices and edges stay grabbable even after they have been moved off the grid (e.g. by a gizmo scale or rotate). Snap-to-grid still applies to where a dragged vertex is placed.

### Transform Gizmo

Selecting one or more surfaces in **Select** mode shows a transform gizmo around the selection:

- **Corner / edge handles** — scale the selection (the opposite handle acts as the pivot)
- **Rotation knob** (above the top edge) — rotate the selection around its centre

The gizmo box is inset from the surface's own corner vertices, so scaling/rotating and vertex editing don't fight over the same clicks.

### Bezier Curve Editing

Switch to the **✒ Bezier** tool to author curved surface edges:

- **Click an edge** to toggle it between a straight line and a cubic bezier (click again to straighten)
- **Drag an anchor** (an edge endpoint) to reshape the outline; adjacent handles follow to preserve curvature
- **Drag a control handle** (the small dots on the connector lines of a curved edge) for fine curve control

Bezier edits work in raw, un-snapped coordinates so handles move with full sub-grid precision. The curve is flattened back into the surface's polygon vertices, which remain the single source of truth for routing and warp. Once a surface has an active curve path, edit its geometry with the Bezier tool rather than by dragging flattened vertices in Select mode.

### Combining Surfaces (Multi-Contour)

Select two or more surfaces and click **🔗 Combine** (G) to merge them into a single surface. Varda computes a polygon union: overlapping regions fuse into one outline, while disjoint regions are kept as **extra contours** on the combined surface. All contours share one content source and color, and every contour shows the slice of that source falling over its area (a bounding-box UV fill across the combined bounds) — useful for treating several separate shapes (e.g. the arms of a mandala, or a row of panels) as one routing target.

A combined (multi-contour) surface does **not** carry a per-surface warp: a single warp mesh can't describe disjoint contours, so warp controls are unavailable while a surface has extra contours. To warp individual pieces, keep them as separate surfaces.

### Stacking Order (Layers)

When surfaces overlap, their **stacking order** decides which draws on top. The order is **global** — identical across the stage canvas and every output — so what you arrange in the editor is exactly what projects. Surfaces draw bottom-to-top.

- In the surface list, use the per-row **▲ / ▼** buttons to nudge a surface toward the **front** (top) or **back** (bottom) one step at a time (disabled at the extremes).
- With a surface selected, use **⤒ Front** / **⤓ Back** in the toolbar to jump it to the top or bottom of the stack.

The surface list is ordered bottom-layer first. Stacking order is saved with the stage and is also available via the HTTP API: `POST /api/surfaces/{uuid}/reorder`.

### Warp Calibration

Warp is a property of the **surface** — one surface owns one warp. Edit it in the
**stage editor's bottom detail bar**: open the Stage Editor and select a single
surface; the bottom bar shows that surface's warp editor.

By default the warp is **🔗 bound to the surface shape** (auto-warp): the grid
conforms to the surface's polygon or circle outline and follows it as you edit
the shape. Uncheck **🔗 Bind to shape** to unbind — this freezes the current
shape-conforming grid as an editable starting point and unlocks the manual
controls below for fine-tuning. Re-checking it re-derives the grid from the
shape (discarding manual edits).

1. Uncheck **🔗 Bind to shape** to enable manual editing
2. Select a surface on the stage; the bottom bar shows its warp grid
3. Drag the 4 corner handles (TL, TR, BR, BL) to align the projected image with
   the physical surface
4. Varda computes a perspective-correct homography (DLT 3×3) for accurate UV mapping
5. Click **↺ Reset** to clear the surface's warp (back to native position)

To align the projectors themselves, use the per-output **🔧 Calibrate** selector:

- **Off** — normal content
- **Projector** — one full-frame test card fills the whole output, bypassing
  surface geometry, for physical projector alignment (focus, lens, keystone)
- **Surfaces** — each surface shows a colored test card through its own warp, for
  verifying surface mapping. Test cards include colored grids, crosshairs, and
  corner/edge markers.

#### Mesh warp (interior control points)

A 4-corner pin is linear — it can keystone a flat surface but cannot correct a
bulge in the middle (a cylinder, a bowed wall, a draped cloth). For that, raise
the surface's grid resolution above 2×2 using the **grid − / +** steppers in the
bottom-bar warp editor. This promotes the corner-pin into an N×M mesh, preserving
the current shape, and adds draggable interior points:

1. Set the grid columns and rows with the steppers (each starts at 2×2; the UI
   allows up to 16×16, the engine up to 64×64)
2. Drag any grid point — corner, edge, or interior — to warp the image locally
3. Interior points let you pull the texture to match non-flat geometry
4. Use **↺ Reset** to clear the warp

#### Bezier (curved) warp

A straight mesh grid approximates a curve with many small facets. For genuinely
smooth deformation (a cylinder, a bowed wall, a draped cloth) click **〰 Curve**
in the bottom-bar warp editor (available while unbound). This converts the warp
into a **bezier patch grid** — the same control layout, but each cell edge is a
cubic bezier with tangent handles:

1. Click **〰 Curve** — the current warp becomes a bezier cage of the same shape
2. Drag an **anchor** (cyan dot) to move a grid point; its tangent handles follow
   so the local curvature is preserved
3. Drag a **handle** (yellow square) to bend the adjacent edges smoothly
4. Use the **cage − / +** steppers to add or remove control points (anchor rows
   and columns); the surface resamples onto the new cage
5. The faint grid shows the tessellated result the projector actually renders
6. Click **⊞ Grid** to convert back to a straight mesh warp, or **↺ Reset** to clear

Under the hood the bezier cage is tessellated into a dense warp mesh, so the GPU
pipeline is unchanged. Fine handles let a few control points describe a curve
that would otherwise need a very dense mesh.

Because warp is per-surface, to correct the same content differently on two
projectors, author two surfaces (the dome slicer already creates one surface per
projector). Legacy `.varda` files that stored warp per assignment are migrated to
the surface automatically on load.

---

## Advanced Projection

### Multi-Output with Edge Blending

For multi-projector setups where projectors overlap:

1. Create an output for each projector (click **"+ Output"** for each)
2. Assign each output to its display target and go fullscreen
3. Draw surfaces in the Stage Editor that match each projector's coverage area
4. Where surfaces overlap, Varda applies **edge blending** — smoothstep alpha ramps that feather the overlap region for a seamless image

**Edge blend modes:**

- **Manual** (default) — per-edge controls for the top/bottom/left/right edges: enable/disable, blend **width** (0.0–0.5), and **gamma** (default 2.2, the smoothstep exponent for the falloff ramp). Applied as a post-process over the whole output. Best for simple side-by-side projectors with straight overlap edges.
- **Auto** — Varda detects overlapping surface regions via precise polygon intersection and computes blend zones automatically (up to 4 overlap zones per surface), applying the ramp per-surface in the fragment shader. Reactive — recomputes when surfaces move. Best for complex stages with arbitrary, circular, or non-rectangular overlaps.

Each output's edge blend settings are independent and saved in `stage.json`.

### Multi-Channel Surface Routing

Different surfaces can show different content simultaneously:

- Surface "Main Screen" → **Master** (full mix)
- Surface "DJ Booth" → **Channel A** (just the A channel)
- Surface "Logo" → **Deck** (a specific deck with your logo shader)
- Surface "Dome" → **Domemaster** (fisheye projection)

This enables independent visual zones — a club might have a main screen, side panels, and a ceiling projection each showing different content from the same engine.

### Mesh Warp

For complex surface geometry beyond 4-point corner-pin, surfaces support **arbitrary UV mesh warp** — a dense grid of XY+UV control points with GPU hardware interpolation. Mesh warp is a strict superset of corner-pin (corner-pin is a 2×2 mesh).

Mesh warp can be hand-edited on-canvas in the stage editor's bottom-bar warp editor (see [Mesh warp](#mesh-warp-interior-control-points) above), is used automatically by the dome slicer, and can be loaded from external calibration tools.

---

## Dome Projection

> **🧪 Experimental.** Dome projection is under active development. The workflow and parameters described below may change, and edge cases (especially multi-projector slicing) are not yet fully hardened. Validate your setup before relying on it for a live show.

Varda includes built-in dome projection: a domemaster renderer, an auto-slicer for 1–8 projectors, and an interactive 3D preview. No external tools needed.

### Domemaster Format

A domemaster is a circular fisheye image using **equidistant azimuthal projection** — the standard for planetarium and dome content. The center maps to the dome's zenith, the edge to the horizon.

| Parameter | Description |
|-----------|-------------|
| **FOV** | Field of view (default: 180° for full hemisphere) |
| **Tilt** | Dome tilt angle — shifts the horizon line |
| **Truncation** | Cut-off angle for truncated domes |
| **Radius** | Dome radius — affects projector coverage calculations |

### Setup Workflow

**1. Switch to Dome 3D mode** — toggle the Stage Editor between **⬡ 2D** and **🔮 3D Dome** at the top of the panel. In 3D mode an interactive hemisphere appears with the domemaster texture mapped onto it.

**2. Configure dome geometry** — **R** (radius, 0.5–5.0), **Trunc** (truncation angle, 30°–90°), and **Tilt** (0°–45°).

**3. Choose a projector preset:**

| Preset | Projectors | Use |
|--------|-----------|-----|
| Single | 1 | Small domes, fisheye lens |
| Dual | 2 | Medium domes |
| Triple | 3 | Medium domes |
| Quad | 4 | Large domes |
| Penta | 5 | Large domes |
| Hexa | 6 | Large domes |
| Octa | 8 | Planetariums |

Or configure projector positions and orientations manually.

**4. Click "Generate Slices."** Varda computes per-projector warp meshes, creates surfaces with Domemaster source and mesh warp applied, and determines polygon shapes via convex hull.

**5. Assign to outputs** — create an output per projector, assign surfaces, fullscreen on each display.

**6. Calibrate** — use calibration mode with test cards to verify alignment.

#### 3D Preview Navigation

The 3D dome view uses an orbit camera:

- **Drag** to rotate (azimuth + elevation; elevation clamps just below the zenith)
- **Scroll** to zoom (distance clamps between 1.5 and 10)
- **Reset/Home** returns to the default view

When a projector preset is active, each projector's coverage is drawn as a semi-transparent colored **wedge overlay** on the hemisphere (colors cycle per projector) so you can see how the slices tile the dome.

### Content Rotation

Real-time rotation applied in the GPU shader — does not recompute meshes:

| Control | Description |
|---------|-------------|
| **Azimuth** | Rotate around the dome's vertical axis |
| **Elevation** | Tilt up/down |
| **Roll** | Roll around the viewing axis |

All three axes are **MIDI-mappable** for live performance. Rotation order: Roll → Elevation → Azimuth.

### Surface Auto-Detection

> **🧪 Experimental.** Auto-detection (both file import and live camera) is under active development. Detection results vary with lighting and source quality — review and refine detected surfaces manually before going live.

Instead of drawing surfaces by hand, Varda can detect them automatically — either from an imported file or from a live camera pointed at the stage.

#### From a File

Import a stage plan and auto-detect surfaces. Three file types are supported:

| Format | Detection Method |
|--------|-----------------|
| **PNG / JPG** | Threshold or Canny edge detection + contour tracing. Best for photos of venues or simple stage plan images. |
| **SVG** | Path flattening — extracts shapes directly from vector paths. Best for designed floor plans. |
| **DXF** | Geometric entity extraction (lines, polylines, circles, arcs, ellipses). Best for CAD venue plans. |

1. In the Stage Editor, click **Import** and select a file (PNG, JPG, SVG, or DXF)
2. Varda detects contours and presents them as candidate surfaces
3. Review and confirm — detected surfaces are added to the stage canvas

#### From a Camera

1. Point a camera at the stage and click **📷 Detect** to enter camera detection mode. A **live** feed appears.
2. Frame the shot, then **freeze** a still (the mode switches from live to a captured preview).
3. Varda traces contours on the frozen frame and presents them as candidate surfaces.
4. Review and confirm — detected surfaces are added to the canvas.

#### Detection Parameters

Both paths share the same tunable contour detector (defaults in parentheses):

| Parameter | Default | Purpose |
|-----------|---------|---------|
| **Method** | Threshold | Binary **Threshold** or **Canny** edge detection |
| **Threshold** | 127 | Binary cutoff (0–255), Threshold mode |
| **Canny Low / High** | 50 / 150 | Edge thresholds, Canny mode |
| **Invert** | off | Swap foreground/background |
| **Blur** | 1 | Gaussian blur radius before detection |
| **Morph Close** | 0 | Morphological close kernel radius (0 = off) |
| **Simplify** | 0.005 | Douglas-Peucker simplification tolerance |
| **Min Area** | 0.001 | Minimum contour area (fraction of image) |
| **Min Vertices** | 3 | Smaller contours are discarded |
| **Hull** | None | Optional convex-hull cleanup |

Small contours are filtered out, near-circular shapes are created as circles, and surfaces are named by position (e.g. "Top-Left", "Center").

This feature is also available via the HTTP API: `POST /api/stage/detect/image`, `/svg`, `/dxf`, and `POST /api/stage/detect/confirm`.

---

### Mesh Import/Export

| Format | Description |
|--------|-------------|
| **Paul Bourke XYUV CSV** | Standard dome mesh format (position + UV) |
| **JSON** | Varda's native mesh format |

Auto-detected by file extension. Load and save from surface warp settings.

---

[← Prev: Outputs](07-outputs.md) · [Home](README.md) · [Next: Streaming, Recording & Network I/O →](09-streaming-and-io.md)
