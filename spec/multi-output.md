# Multi-Output & Projection Mapping

## Status: DRAFT (Phase 8a, 8b, 8c, 8d IMPLEMENTED)

## Overview

Varda's output system models the full path from content generation to physical display:

```
Deck → Channel → Mixer/Master
                      ↓  content routing
                  Surfaces (named polygon regions, each picks a source)
                      ↓  spatial mapping + warp
                  Outputs (render assigned surfaces with per-surface calibration)
                      ↓  display targeting
                  Display/Projector (windowed or fullscreen on a specific monitor)
```

This five-layer hierarchy scales from simple to complex:

| Use Case | Layers Involved |
|---|---|
| 1 projector, no mapping | Decks → Channels → Output → Projector (surfaces auto-created, master mix goes fullscreen) |
| 1 projector + projection mapping | Add surfaces with warp calibration, assign to output, pick display |
| Multi-projector installation | Multiple outputs, each with different surface subsets, each on a different display |

The key insight: **each layer is optional until you need it.** A VJ doing a simple club gig never touches surfaces. An installation artist uses all five layers.

### Why Three Layers (not two)

A simpler "content → output" model (route Channel A directly to Projector 1) breaks down when:
- One projector covers multiple surfaces (main screen + floor projection)
- Multiple LED panels show different content but connect through one controller
- The performer pre-produces a show at home and calibrates at the venue

The **surface** layer is the bridge: it captures the venue's geometry (what screens/panels exist and where), while outputs capture the physical delivery (which projector/controller sends pixels there).

### The Single-Projector, Multiple-Surfaces Problem

The most common real-world case that demands the three-layer model: **one projector throws onto multiple distinct surfaces** showing different content. Examples:

- A projector covers a main screen center-stage AND a floor area — different channels on each
- A wide-throw projector illuminates three separate LED-style panels at different heights
- A ceiling projector maps content onto a DJ booth face, tabletop, and a backdrop behind

In a two-layer model, you'd need to manually composite the surfaces into a single output texture and apply a single warp — impossible without spatial awareness. With the three-layer model:

1. **Surfaces** define _what_ and _where_: "Main Screen at center shows Master", "Floor shows Channel B"
2. **Output** defines _how_: "Projector 1 is responsible for both surfaces"
3. The rendering pipeline composites each surface's content at its spatial position, then applies per-surface warp so the projector's output aligns with the physical geometry

The output window effectively renders a virtual camera view of the stage model, where each surface is a textured quad showing its routed content. The corner-pin calibration aligns each surface independently within that single projector's throw.

**Data model implications:**
- An `Output` has a `Vec<SurfaceAssignment>` — an ordered list of surfaces it renders
- Each `SurfaceAssignment` carries per-surface warp/calibration data (corner-pin offsets relative to this output)
- A `Surface` can be assigned to exactly one output (no overlapping ownership; edge blending in 8g relaxes this)
- Surfaces not assigned to any output are "unrouted" — visible in the editor but not sent to any physical display

This is the core of Phase 8d. Phases 8a-8c work with the simpler 1:1 case (one surface per output) to get the foundation in place.

### Quick Output Shortcut

For simple setups (one laptop, one projector), the surface layer should be invisible. A "Send to Output" action on a channel should auto-create a surface if needed, so the user doesn't have to think about surfaces for basic rigs.

---

## Layer 1: Content Routing

Any of these can be routed to any surface:
- **Master mix** — the final composited output
- **Channel** — a specific channel's composited output (post channel-FX)
- **Deck** — a specific deck's raw output (pre-blend, pre-FX)

Multiple surfaces can share the same content source. A surface with no content assigned shows black.

### Content Mapping Modes

Each surface has a **content mapping mode** that controls how the source texture maps onto it:

- **Fill** (default): The entire source texture is scaled to fill this surface. Each surface with the same source gets an independent, full copy of the content. Use this for surfaces that should each show the full image (e.g., mirrored screens, IMAG feeds).

- **Mapped**: The surface's position on the 2D canvas determines which region of the source it displays. The canvas IS the content coordinate space — a surface at normalized position (0.2, 0.3) with size (0.1, 0.1) shows source UVs from (0.2, 0.3) to (0.3, 0.4). This is a spatial crop, not a scale.

  Surfaces with the same source in Mapped mode **implicitly form a group** — each shows its slice of one continuous image. No explicit grouping UI needed.

  **Example — sun stage design:**
  - Center circle surface (Mapped, source: Channel A) → shows the center portion of Channel A
  - Six triangle "sunbeam" surfaces around it (Mapped, source: Channel A) → each shows its spatial slice of Channel A
  - Result: Channel A's content is spatially distributed across the whole sun shape
  - The triangles could also be switched to a different source or to Fill mode independently

**Decision**: Implicit grouping via same-source + Mapped mode, rather than explicit group objects. Rationale: simpler mental model, fewer UI concepts, and the canvas position already encodes the spatial relationship. If a user wants two independent mapped groups, they use different sources.

---

## Layer 2: Stage Model & Surfaces

### Surface Data Model

Surfaces are **polygons** — an ordered list of vertices in normalized canvas coordinates [0..1]. Rectangles are just 4-vertex polygons. This lets the user model triangles, circles, and arbitrary shapes for projection mapping.

```
Surface {
    name: String,
    vertices: Vec<[f32; 2]>,     // ordered polygon vertices, normalized [0..1]
    source: OutputSource,
    content_mapping: ContentMapping,
    output_type: SurfaceOutputType,
    circle_hint: Option<CircleHint>,  // present if surface was created as a circle
}

CircleHint {
    center: [f32; 2],   // center in normalized coords
    radius: f32,         // radius in normalized coords
    sides: u32,          // number of polygon sides (3–128)
    aspect_ratio: f32,   // canvas width/height used for vertex generation
}
```

**Circle support**: Surfaces created with the Circle tool carry a `CircleHint` that enables radius and side-count editing. Vertices are regenerated from the hint when these properties change. "Convert to Polygon" drops the circle identity, keeping vertices as a plain polygon. Vertex insertion or individual vertex dragging on a circle also auto-converts to polygon. Moving a circle surface (select tool drag) keeps the hint center in sync with the vertices — the radius handle, ring, and center dot follow the moved shape.

**Derived properties:**
- Bounding box: min/max of all vertices — used for Mapped mode UV calculation and hit testing
- Center: average of all vertices — used for labeling and selection

### 2D Surface Editor

Two UI modes for defining venue geometry:

**Simple view** (right panel): A read-only mini canvas preview showing all surfaces as colored polygon outlines. An "Open Editor" button launches the full editor. Surface property controls (name, source, mapping mode, output type) remain in the right panel.

**Full editor** (replaces deck view): Activated by clicking "Open Editor" in the right panel. A large dark canvas with configurable grid. Drawing tools and vertex editing. Close button returns to the normal deck view.

**Surface properties:**
- Name (e.g., "Main Screen", "Left LED", "DJ Booth Front")
- Vertices: ordered polygon points on the canvas (normalized [0..1])
- Content source assignment (channel, master, deck, or none)
- Content mapping mode: **Fill** or **Mapped** (see Layer 1 above)
- Output type: **Projection** or **LED Direct** (see Layer 3)

**Full editor features:**
- **Rectangle tool**: Click-drag creates a 4-vertex rectangle surface
- **Polygon tool**: Click to place vertices, double-click or close-to-start to finish. Creates arbitrary polygon surfaces (triangles, pentagons, etc.)
- **Circle/N-gon tool**: Click-drag creates a circle surface with `CircleHint` (configurable vertex count, 3–128). Circle surfaces show a radius ring and handle instead of vertex handles. Dragging the radius handle adjusts radius interactively. Toolbar shows radius DragValue, sides DragValue, and "Convert to Polygon" button when a circle is selected.
- **Edit mode**: Select a surface, then drag individual vertices to reshape. Visual handles on vertices. Dragging a vertex on a circle auto-converts it to a polygon first.
- **Click-to-select**: Click inside a surface to select it (ray-casting point-in-polygon hit test)
- **Multi-select**: Shift+click to toggle individual surfaces in/out of the selection. Click-drag on empty space to draw a marquee rectangle — all surfaces whose bounding box intersects the rectangle are selected on release. Shift+marquee adds to the existing selection.
- **Multi-move**: When multiple surfaces are selected, dragging any one of them moves all selected surfaces together by the same delta.
- **Double-click edge**: Insert a new vertex on an edge at the click point (point-to-segment projection, snaps to grid)
- **Duplicate** (D key or toolbar): Clone selected surface(s) with grid-aligned offset. Works on multi-selection — each selected surface is duplicated independently.
- **Flip Horizontal/Vertical** (H/V keys or toolbar): Mirror surface vertices around each surface's bounding box center. Works on multi-selection — each selected surface is flipped independently.
- **Delete**: Select one or more surfaces + Delete/Backspace key removes all selected (indices removed in reverse order to preserve correctness)
- **Auto-tool switching**: If a drawing tool is active and you click/drag inside an existing surface, automatically switch to Select mode (prevents accidental overlapping draws)
- **Grid**: Configurable grid size (10%, 5%, 2.5%, 1.25%) with snap-to-grid toggle
- **Keyboard shortcuts**: S (Select), R (Rectangle), P (Polygon), C (Circle), Escape (cancel in-progress draw)
- Named surfaces persist with the scene file (not yet implemented)

**Polygon rendering in outputs**: Output windows render actual polygon shapes using a fan-triangulated vertex pipeline (`PolygonBlitPipeline`). UVs are computed relative to each polygon's bounding box. Triangles render as triangles, circles as circles — whatever shape is drawn in the stage editor appears in the output.

**Decision**: Surfaces support arbitrary polygons (not just rectangles). Rationale: real venues have triangular LED panels, circular projection areas, and irregular shapes. The bounding-box-based Mapped mode UV mapping works naturally with polygons.

**Decision**: No freehand/curve drawing for now. Straight-edge polygons cover the vast majority of real venue geometry.

### 3D Stage Model Import (Phase 8e — future)

Import a 3D model (OBJ or glTF) representing the venue. Named meshes in the model become surfaces. The 2D editor remains available as a fallback/override. The 3D model enables:
- Accurate perspective correction for angled/curved surfaces
- Visual pre-production with a 3D preview of the mapped result
- Camera placement matching physical projector positions

### Open Questions — Stage Model
- For the 3D model path: OBJ (simpler) vs glTF (richer metadata, named meshes)?
- Should the 2D canvas orientation be configurable (top-down vs front-facing)?

---

## Layer 3: Outputs

An output renders its assigned surfaces and sends the result to a **display target**. Outputs do NOT have a content source — they get content exclusively through surfaces.

### Output Data Model

```
OutputWindow {
    name: String,
    target: OutputTarget,                      // Windowed or Display { name, monitor_index }
    surface_assignments: Vec<SurfaceAssignment>, // what to render
    calibration_mode: bool,                      // whether calibration UI is active
}

SurfaceAssignment {
    surface_idx: usize,
    warp_corners: [[f32; 2]; 4],  // per-surface corner-pin in output-normalized [0..1]
    enabled: bool,
}
```

### Display Targeting

Each output has a **display target** — where its window appears:

- **Windowed**: A floating OS window the user can position and resize. Good for previewing, testing, and setups where you don't need fullscreen.
- **Display** (e.g., "HDMI-1 (1920x1080)", "Built-in Retina Display"): Borderless fullscreen on a specific connected monitor. Available monitors are enumerated via `winit::ActiveEventLoop::available_monitors()` and listed in a dropdown. When a projector is plugged in, it appears automatically.

This replaces the old "drag window to projector + click fullscreen" workflow with a single dropdown selection.

### Rendering Rules

- **Output has surface assignments** → renders only those surfaces with per-surface warp (homography)
- **Output has no surface assignments but surfaces exist** → renders all surfaces without warp (fallback)
- **No surfaces exist at all** → renders the master mix as a fullscreen quad (basic setup)

### Calibration — Corner-Pin Method

Each surface assignment carries 4 warp corners (TL, TR, BR, BL) in output-normalized coordinates [0..1]. The calibration workflow:
1. Assign surfaces to an output
2. Select a display target (projector)
3. Enter calibration mode (🔧 button)
4. **Calibration test cards** appear on the output — each surface shows a distinct colored card with grid lines, crosshairs, corner markers, and border. This replaces the live content on the projector so you can see alignment clearly. ✅ Implemented.
5. Drag corner handles (in the mini warp canvas) to align with physical surfaces
6. DLT homography solver computes a 3×3 perspective matrix per surface
7. The polygon vertex shader applies the homography with perspective-correct UV interpolation
8. Click "Done" to exit calibration and return to live content

**Calibration card details**: 8 distinct colors (red, green, blue, yellow, purple, cyan, orange, pink) cycle across surfaces. Cards include an 8×8 grid, center crosshair + circle, corner bracket markers, and edge midpoint markers. Cards always use Fill UV mapping regardless of the surface's content mapping mode, so the full grid is visible per surface.

**Warp settings** persist per-output in the scene file across sessions.

### LED Direct Output (Phase 8f — future)

An LED wall/panel connected via a pixel-mapping controller. Content is cropped and scaled to match the panel's exact pixel dimensions — no perspective warp needed.

**LED output protocols (future):**
- Direct video output (no warp — just crop/scale to panel resolution)
- NDI (network video)
- Art-Net / sACN (for LED pixel controllers)

### Surface Source Routing Note

Surfaces pull content from a source (Master, Channel, or Deck). This interacts with the crossfader: if Surface A pulls from Channel A and Surface B pulls from Channel B, the crossfader fades the *master mix* but doesn't affect these surfaces (they bypass master). Both surfaces need to be set to "Master" source if the crossfader should affect them. This is correct behavior — pulling from a specific channel is for installations where surfaces show independent content.

---

## GPU Architecture — IMPLEMENTED

```
RenderContext { instance, adapter, device, queue, surface, surface_config, size }  ← main UI window
OutputWindow { window, surface, surface_config, size, target, surface_assignments }  ← per output, 0..N
```

All output windows share the main `RenderContext`'s device/queue. Each creates its own `wgpu::Surface`.

**Display targeting:** `OutputTarget::Windowed` or `OutputTarget::Display { name, monitor_index }`. Display selection uses `winit::ActiveEventLoop::available_monitors()` to enumerate connected displays. Selecting a display calls `window.set_fullscreen(Fullscreen::Borderless(Some(monitor)))`.

**winit integration:** `ApplicationHandler::window_event` receives `WindowId` — events are dispatched to the correct window. Each output window handles `Resized` and `RedrawRequested`. The main window additionally handles egui input.

### Warp Pipeline — IMPLEMENTED

The polygon vertex shader (`polygon.wgsl`) applies a 3×3 homography matrix per surface:
- **Forward homography**: Maps from surface bounding box corners → warp corners (DLT solver in `warp.rs`)
- **Vertex transform**: `H * [x, y, 1]` → clip coords with projective `w = h_pos.z`
- **Perspective-correct UVs**: GPU hardware interpolates UVs correctly via the projective divide
- **Per-surface**: Each `SurfaceAssignment` gets its own homography uniform via `PolygonBlitPipeline`

---

## Phased Delivery

| Phase | Scope | Size | Depends On |
|-------|-------|------|------------|
| **8a** | Multi-window outputs + source routing (channel/master → output window) + fullscreen | M | Phase 1 |
| **8b** | 2D surface editor: polygon surfaces, advanced stage editor with drawing tools, grid | M | 8a |
| **8c** | Output-level warp (4-corner whole-frame) + assign surfaces to outputs | M | 8a, 8b |
| **8d** | Per-surface warp within an output (individual corner-pin per surface) | L | 8c |
| **8e** | 3D stage model import (OBJ/glTF) + 3D preview + camera-based projection mapping | XL | 8b, 8d |
| **8f** | LED direct output (pixel-accurate crop/scale, no warp) | M | 8a, 8b |
| **8g** | Edge blending for overlapping projectors | L | 8d |

**Phase 8a** is the foundation — it generalizes the render context and proves multi-window works.
**Phase 8b** delivers the stage editor with polygon drawing tools for pre-production.
**Phase 8c** delivers output warp + surface-to-output assignment — "usable at a gig with projectors."
**Phase 8d+** is the full per-surface spatial mapping vision.

---

## Scene Serialization

All mapping state saves with the scene file:

```json
{
  "surfaces": [
    { "name": "Main Screen", "rect": [0.1, 0.2, 0.6, 0.4], "content": { "type": "master" } },
    { "name": "Left LED", "rect": [0.0, 0.3, 0.1, 0.3], "content": { "type": "channel", "index": 0 } }
  ],
  "outputs": [
    {
      "name": "Projector 1",
      "type": "projection",
      "surfaces": ["Main Screen"],
      "calibration": { "corners": [[0.1, 0.1], [0.9, 0.05], [0.95, 0.95], [0.05, 0.9]] },
      "fullscreen_monitor": 1
    },
    {
      "name": "LED Controller",
      "type": "led_direct",
      "surfaces": ["Left LED"],
      "resolution": [384, 768]
    }
  ]
}
```

---

## Stretch Goals
- **NDI output**: Each output can optionally send via NDI instead of/in addition to a display window
- **Syphon** (macOS) / **Spout** (Windows) for inter-app sharing
- **Art-Net / sACN**: Direct LED pixel protocol output
- **Screen capture as input**: Capture another app's window as a deck source
- **Mask/stencil per surface**: Arbitrary alpha masks for non-rectangular projection surfaces

