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
2. Draw surfaces using the drawing tools:
   - **R** — Rectangle
   - **P** — Polygon (click vertices, close the shape)
   - **C** — Circle (with interactive radius handle)
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
- **G** — Combine multiple selected surfaces

### Corner-Pin Warp

Each surface assigned to an output gets independent corner-pin warp calibration:

1. Enable **Calibration Mode** on the output (shows test cards)
2. Drag the 4 corner handles to align the projected image with the physical surface
3. Varda computes a perspective-correct homography (DLT) for accurate UV mapping

Test cards include: 8 colors, 8×8 grid, corner brackets, and gradient bars.

---

## Advanced Projection

### Multi-Output with Edge Blending

For multi-projector setups where projectors overlap:

1. Create an output for each projector (click **"+ Output"** for each)
2. Assign each output to its display target and go fullscreen
3. Draw surfaces in the Stage Editor that match each projector's coverage area
4. Where surfaces overlap, Varda applies **edge blending** — smoothstep alpha ramps that feather the overlap region for a seamless image

**Edge blend modes:**

- **Auto** — Varda detects overlapping surface regions using polygon intersection analysis and computes blend zones automatically. Reactive — recomputes when surfaces move.
- **Manual** — per-edge controls: enable/disable, blend width, and gamma per edge

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

Mesh warp is used automatically by the dome slicer and can be loaded from external calibration tools.

---

## Dome Projection

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

**1. Switch to Dome 3D mode** in the Stage Editor. An interactive hemisphere appears with the domemaster texture mapped onto it.

**2. Configure dome geometry** — radius, truncation angle, and tilt.

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

### Content Rotation

Real-time rotation applied in the GPU shader — does not recompute meshes:

| Control | Description |
|---------|-------------|
| **Azimuth** | Rotate around the dome's vertical axis |
| **Elevation** | Tilt up/down |
| **Roll** | Roll around the viewing axis |

All three axes are **MIDI-mappable** for live performance. Rotation order: Roll → Elevation → Azimuth.

### Mesh Import/Export

| Format | Description |
|--------|-------------|
| **Paul Bourke XYUV CSV** | Standard dome mesh format (position + UV) |
| **JSON** | Varda's native mesh format |

Auto-detected by file extension. Load and save from surface warp settings.
