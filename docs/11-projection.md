# Projection Mapping

## Basic Projection

### Single Output, No Surfaces

This setup sends the full master mix to one projector or display through one output window.

1. Choose **+ Output → Window** in the right panel's **📺 Outputs** section. A floating output window appears.
2. Select a **display target** from the dropdown, which lists connected monitors.
3. Click **Fullscreen** to send the window to that display.

You need no surfaces. The output receives the full master mix directly.

### Single Output, Surfaces for Spatial Mapping

Surfaces place content in specific regions of the output. Use them when the physical projection surface is not a simple rectangle, or when different regions need different content.

1. Open the **Stage Editor** (center panel toggle).
2. Pick a drawing tool from the toolbar (keyboard shortcut in parentheses):
   - **⬚ Select** (S): select and edit existing surfaces
   - **▭ Rectangle** (R): drag out a rectangle
   - **⬠ Polygon** (P): click to place vertices, double-click to finish
   - **⬤ Circle** (C): circle or N-gon with an interactive radius handle
   - **✒ Bezier**: curve or straighten a surface edge, and drag bezier anchors and handles
3. Set each surface's **content source**:
   - **Master**: the full mixer output
   - **Channel**: a single channel's output
   - **Channels**: a sub-mix of selected channels
   - **Deck**: a single deck's raw output
4. Set the **content mapping** mode:
   - **Fill**: the whole source texture is scaled to fill the surface
   - **Mapped**: the surface's position on the canvas sets the UV crop, so several surfaces show different slices of one continuous image
5. Assign surfaces to the output with the **+ Assign Surface** dropdown in the output panel.

### Vertex Editing

- **Drag vertices** to move them
- **Double-click an edge** to insert a new vertex
- **Snap-to-grid** for precise alignment
- **D**: duplicate the selected surface
- **H / V**: flip horizontal / vertical
- **🔗 Combine** (G): merge the selected surfaces into one
- **⤒ Front / ⤓ Back**: restack the selected surface (see [Stacking Order](#stacking-order-layers))

Clicks select vertices and edges at the raw cursor position, so you can grab them after they have moved off the grid (for example after a gizmo scale or rotate). Snap-to-grid still applies to where a dragged vertex lands.

### Transform Gizmo

Select one or more surfaces in **Select** mode to show a transform gizmo around the selection:

- **Corner / edge handles**: scale the selection. The opposite handle is the pivot.
- **Rotation knob** (above the top edge): rotate the selection around its center.

The gizmo box sits inside the surface's corner vertices, so gizmo handles and vertex handles do not overlap.

### Bezier Curve Editing

Use the **✒ Bezier** tool to draw curved surface edges:

- **Click an edge** to switch it between a straight line and a cubic bezier. Click again to straighten it.
- **Drag an anchor** (an edge endpoint) to reshape the outline. The adjacent handles follow to keep the curvature.
- **Drag a control handle** (the small dots on the connector lines of a curved edge) to fine-tune the curve.

Bezier edits ignore the grid snap, so handles move with full sub-grid precision. Varda flattens the curve into the surface's polygon vertices, which routing and warp use. Once a surface has a curve, edit its shape with the Bezier tool, not by dragging the flattened vertices in Select mode.

### Combining Surfaces (Multi-Contour)

Select two or more surfaces and click **🔗 Combine** (G) to merge them into one surface. Overlapping regions fuse into one outline. Separate regions stay as **extra contours** on the combined surface.

All contours share one content source and color. Each contour shows the part of the source that falls over its area (a bounding-box UV fill across the combined bounds). Use this to route several separate shapes as one target, such as the arms of a mandala or a row of panels.

A combined (multi-contour) surface has **no** per-surface warp, because one warp mesh cannot describe separate contours. The warp controls are unavailable while a surface has extra contours. To warp individual pieces, keep them as separate surfaces.

### Stacking Order (Layers)

When surfaces overlap, the **stacking order** sets which one draws on top. The order is **global**, so the projection matches the editor. Surfaces draw bottom to top.

- In the surface list, the **▲ / ▼** buttons on each row move a surface one step toward the **front** (top) or **back** (bottom). They are disabled at the ends of the stack.
- With a surface selected, **⤒ Front** / **⤓ Back** in the toolbar move it to the top or bottom of the stack.

The surface list shows the bottom layer first. Stacking order is saved with the stage. Over HTTP: `POST /api/surfaces/{uuid}/reorder`.

### Warp Calibration

Warp belongs to the **surface**. To edit it, open the Stage Editor and select a single surface. The **bottom detail bar** shows that surface's warp editor.

By default the warp is **🔗 bound to the surface shape** (auto-warp). The grid follows the surface's polygon or circle outline as you edit the shape. Uncheck **🔗 Bind to shape** to unbind: the current grid becomes an editable starting point and the manual controls below unlock. Checking it again rebuilds the grid from the shape and discards manual edits.

1. Uncheck **🔗 Bind to shape** to enable manual editing.
2. Select a surface on the stage. The bottom bar shows its warp grid.
3. Drag the 4 corner handles (TL, TR, BR, BL) to line the projected image up with the physical surface.
4. Varda computes a perspective-correct homography (DLT 3×3) for the UV mapping.
5. Click **↺ Reset** to clear the surface's warp and return it to its native position.

To align the projectors themselves, use the **🔧 Calibrate** selector on each output:

- **Off**: normal content
- **Projector**: one full-frame test card fills the whole output and ignores surface geometry. Use it for physical projector alignment (focus, lens, keystone).
- **Surfaces**: each surface shows a colored test card through its own warp. Use it to check surface mapping.

Test cards include colored grids, crosshairs, and corner and edge markers.

#### Mesh warp (interior control points)

A 4-corner pin is linear. It can keystone a flat surface but cannot correct a bulge in the middle, such as a cylinder, a bowed wall or a draped cloth. For those, raise the surface's grid above 2×2 with the **grid − / +** steppers in the bottom-bar warp editor. This turns the corner-pin into an N×M mesh with the same shape and adds draggable interior points:

1. Set the grid columns and rows with the steppers. Each starts at 2×2. The UI allows up to 16×16, the engine up to 64×64.
2. Drag any grid point (corner, edge or interior) to warp the image locally.
3. Use interior points to pull the texture onto non-flat geometry.
4. Use **↺ Reset** to clear the warp.

#### Bezier (curved) warp

A mesh grid approximates a curve with many small flat facets. For smooth deformation of a cylinder, bowed wall or draped cloth, click **〰 Curve** in the bottom-bar warp editor (available while unbound). The warp becomes a **bezier patch grid**, where each cell edge is a cubic bezier with tangent handles:

1. Click **〰 Curve**. The current warp becomes a bezier cage of the same shape.
2. Drag an **anchor** (cyan dot) to move a grid point. Its tangent handles follow, keeping the local curvature.
3. Drag a **handle** (yellow square) to bend the adjacent edges smoothly.
4. Use the **cage − / +** steppers to add or remove anchor rows and columns. The surface resamples onto the new cage.
5. The faint grid shows the tessellated result the projector renders.
6. Click **⊞ Grid** to convert back to a straight mesh warp, or **↺ Reset** to clear it.

Varda tessellates the bezier cage into a dense warp mesh for rendering. A few control points with handles can describe a curve that would otherwise need a very dense mesh.

Warp is per surface. To correct the same content differently on two projectors, make two surfaces. The dome slicer already creates one surface per projector. Older `.varda` files that stored warp per assignment move the warp to the surface when they load.

---

## Advanced Projection

### Multi-Output with Edge Blending

For setups where projectors overlap:

1. Create an output for each projector (**+ Output → Window** for each).
2. Set each output's display target and go fullscreen.
3. Draw surfaces in the Stage Editor that match each projector's coverage.
4. Where surfaces overlap, Varda applies **edge blending**: smoothstep alpha ramps that feather the overlap so the image looks continuous.

**Edge blend modes:**

- **Manual** (default): controls for each of the top, bottom, left and right edges: on/off, blend **width** (0.0–0.5), and **gamma** (default 2.2, the smoothstep exponent for the falloff ramp). Applied after rendering, across the whole output. Use it for side-by-side projectors with straight overlap edges.
- **Auto**: Varda finds overlapping surface regions by polygon intersection and computes blend zones (up to 4 overlap zones per surface). The ramp is applied per surface in the fragment shader. Blend zones update when surfaces move. Use it for stages with arbitrary, circular or non-rectangular overlaps.

Each output has its own edge blend settings, saved in `stage.json`.

### Multi-Channel Surface Routing

Different surfaces can show different content at the same time:

- Surface "Main Screen" → **Master** (full mix)
- Surface "DJ Booth" → **Channel A** (only the A channel)
- Surface "Logo" → **Deck** (a specific deck with your logo shader)
- Surface "Dome" → **Domemaster** (fisheye projection)

For example, a club can give its main screen, side panels and ceiling projection different content from the same engine.

### Mesh Warp

Surfaces support **arbitrary UV mesh warp** for geometry that a 4-point corner-pin cannot fit. A mesh warp is a dense grid of XY+UV control points, interpolated by the GPU. A corner-pin is a 2×2 mesh, so mesh warp covers everything corner-pin does.

You can edit a mesh warp on the canvas in the stage editor's bottom-bar warp editor (see [Mesh warp](#mesh-warp-interior-control-points) above). The dome slicer creates mesh warps automatically, and you can load them from external calibration tools.

---

## Dome Projection

> **🧪 Experimental.** Dome projection is under active development. The workflow and parameters below may change, and edge cases (especially multi-projector slicing) are not fully tested. Test your setup before using it in a live show.

Varda has built-in dome projection: a domemaster renderer, an auto-slicer for 1–8 projectors, and an interactive 3D preview. You need no external tools.

### Domemaster Format

A domemaster is a circular fisheye image in **equidistant azimuthal projection**. The center maps to the dome's zenith and the edge to the horizon.

| Parameter | Description |
|-----------|-------------|
| **FOV** | Field of view (default 180° for a full hemisphere) |
| **Tilt** | Dome tilt angle. Shifts the horizon line. |
| **Truncation** | Cut-off angle for truncated domes |
| **Radius** | Dome radius. Affects projector coverage calculations. |
| **Res** | Domemaster render size: 1K, 2K (default) or 4K, always square |

### Setup Workflow

**1. Switch to Dome 3D mode.** Toggle the Stage Editor between **⬡ 2D** and **🔮 3D Dome** at the top of the panel. 3D mode shows an interactive hemisphere with the domemaster texture mapped onto it.

**2. Configure dome geometry:** **R** (radius, 0.5–5.0), **Trunc** (truncation angle, 30°–90°) and **Tilt** (0°–45°).

Geometry and preset edits can be undone and are saved with the stage. On a headless install, set them over HTTP with `PUT /api/dome/geometry` and `PUT /api/dome/preset`, and read them with `GET /api/state/dome`.

**Res** sets the domemaster's render size: **1K** (1024×1024), **2K** (2048×2048, the default) or **4K** (4096×4096). It is always square and independent of the master render resolution. Pick the size that matches your projector array. Changing it rebuilds the dome's textures immediately. The setting is saved with the stage.

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

You can also set projector positions and orientations by hand.

**4. Click "Generate Slices."** Varda computes a warp mesh for each projector and creates surfaces with the Domemaster source and that mesh warp. Each surface's polygon is the convex hull of its slice.

**5. Assign to outputs.** Create an output per projector, assign the surfaces, and go fullscreen on each display.

**6. Calibrate.** Use calibration mode with test cards to check alignment.

#### 3D Preview Navigation

The 3D dome view uses an orbit camera:

- **Drag** to rotate (azimuth and elevation; elevation stops just below the zenith)
- **Scroll** to zoom (distance between 1.5 and 10)
- **Reset/Home** returns to the default view

When a projector preset is active, each projector's coverage shows as a semi-transparent colored **wedge overlay** on the hemisphere, one color per projector, so you can see how the slices tile the dome.

### Content Rotation

Content rotation runs in the GPU shader in real time and does not recompute meshes:

| Control | Description |
|---------|-------------|
| **Azimuth** | Rotate around the dome's vertical axis |
| **Elevation** | Tilt up/down |
| **Roll** | Roll around the viewing axis |

All three axes are **MIDI-mappable**. Rotation order: Roll → Elevation → Azimuth.

### Surface Auto-Detection

> **🧪 Experimental.** Auto-detection (file import and live camera) is under active development. Results vary with lighting and source quality. Review and refine detected surfaces by hand before going live.

Varda can detect surfaces from an imported file or from a live camera pointed at the stage, so you do not have to draw them.

#### From a File

Import a stage plan to detect surfaces. Three file types are supported:

| Format | Detection Method |
|--------|-----------------|
| **PNG / JPG** | Threshold or Canny edge detection, then contour tracing. Best for photos of venues or simple stage plan images. |
| **SVG** | Path flattening: shapes come directly from the vector paths. Best for designed floor plans. |
| **DXF** | Geometric entity extraction (lines, polylines, circles, arcs, ellipses). Best for CAD venue plans. |

1. In the Stage Editor, click **Import** and select a PNG, JPG, SVG or DXF file.
2. Varda detects contours and shows them as candidate surfaces.
3. Review and confirm. The detected surfaces are added to the stage canvas.

#### From a Camera

1. Point a camera at the stage and click **📷 Detect** to enter camera detection mode. A **live** feed appears.
2. Frame the shot, then **freeze** a still. The mode switches from live to a captured preview.
3. Varda traces contours on the frozen frame and shows them as candidate surfaces.
4. Review and confirm. The detected surfaces are added to the canvas.

#### Detection Parameters

File and camera detection use the same contour detector:

| Parameter | Default | Purpose |
|-----------|---------|---------|
| **Method** | Threshold | Binary **Threshold** or **Canny** edge detection |
| **Threshold** | 127 | Binary cutoff (0–255), Threshold mode |
| **Canny Low / High** | 50 / 150 | Edge thresholds, Canny mode |
| **Invert** | off | Swap foreground and background |
| **Blur** | 1 | Gaussian blur radius before detection |
| **Morph Close** | 0 | Morphological close kernel radius (0 = off) |
| **Simplify** | 0.005 | Douglas-Peucker simplification tolerance |
| **Min Area** | 0.001 | Minimum contour area (fraction of image) |
| **Min Vertices** | 3 | Smaller contours are discarded |
| **Hull** | None | Optional convex-hull cleanup |

Varda drops small contours, creates near-circular shapes as circles, and names surfaces by position (for example "Top-Left", "Center").

Over HTTP: `POST /api/stage/detect/image`, `/svg`, `/dxf`, and `POST /api/stage/detect/confirm`.

---

### Mesh Import/Export

| Format | Description |
|--------|-------------|
| **Paul Bourke XYUV CSV** | Standard dome mesh format (position + UV) |
| **JSON** | Varda's native mesh format |

The format is detected from the file extension. Load and save meshes from the surface warp settings.

---

[← Prev: Outputs](10-outputs.md) · [Home](README.md) · [Next: Streaming, Recording & Network I/O →](12-streaming-and-io.md)
