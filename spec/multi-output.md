# Multi-Output & Projection Mapping

## Status: DRAFT

## Overview

Varda's output system uses a **three-layer abstraction** to model how visual content reaches physical displays:

```
Content (channels, master mix)
    ↓  routing
Surfaces (named regions in a 2D/3D stage model)
    ↓  mapping
Outputs (physical displays, projectors, LED controllers)
```

This separation lets the performer think spatially ("put Channel A on the left LED panel") without coupling to the physical output hardware. The same stage layout can be recalibrated for different venues without changing content routing.

### Why Three Layers (not two)

A simpler "content → output" model (route Channel A directly to Projector 1) breaks down when:
- One projector covers multiple surfaces (main screen + floor projection)
- Multiple LED panels show different content but connect through one controller
- The performer pre-produces a show at home and calibrates at the venue

The **surface** layer is the bridge: it captures the venue's geometry (what screens/panels exist and where), while outputs capture the physical delivery (which projector/controller sends pixels there).

### Quick Output Shortcut

For simple setups (one laptop, one projector), the surface layer should be invisible. A "Send to Output" action on a channel should auto-create a surface if needed, so the user doesn't have to think about surfaces for basic rigs.

---

## Layer 1: Content Routing

Any of these can be routed to any surface:
- **Master mix** — the final composited output
- **Channel** — a specific channel's composited output (post channel-FX)
- **Deck** — a specific deck's raw output (pre-blend, pre-FX)

Multiple surfaces can share the same content source. A surface with no content assigned shows black.

---

## Layer 2: Stage Model & Surfaces

### 2D Surface Editor (Phase 8b)

The primary interface for defining venue geometry. A 2D canvas where the user places, names, and arranges rectangular surfaces representing physical screens, LED panels, and projection areas.

**Surface properties:**
- Name (e.g., "Main Screen", "Left LED", "DJ Booth Front")
- Position and size on the canvas (in arbitrary units — the canvas represents the venue from a top-down or front-facing view)
- Aspect ratio (locked or free)
- Content source assignment (channel, master, deck, or none)
- Output type: **Projection** or **LED Direct** (see Layer 3)

**Editor features:**
- Drag to place, resize, and reposition surfaces
- Snap-to-grid optional
- Named surfaces persist with the scene file
- Live preview: each surface thumbnail shows what content is currently routed to it
- The editor is a UI panel (likely a dedicated tab or central area), not a separate window

### 3D Stage Model Import (Phase 8e — future)

Import a 3D model (OBJ or glTF) representing the venue. Named meshes in the model become surfaces. The 2D editor remains available as a fallback/override. The 3D model enables:
- Accurate perspective correction for angled/curved surfaces
- Visual pre-production with a 3D preview of the mapped result
- Camera placement matching physical projector positions

### Open Questions — Stage Model
- Should surfaces support non-rectangular shapes (polygons, curves) in the 2D editor, or only rectangles?
- For the 3D model path: OBJ (simpler) vs glTF (richer metadata, named meshes)?
- Should the 2D canvas orientation be configurable (top-down vs front-facing)?

---

## Layer 3: Physical Outputs

Two distinct output types, because the rendering strategy is fundamentally different:

### Projection Output

A projector connected via display output (HDMI, DisplayPort, etc.). Implemented as a separate OS window (`winit::Window` + `wgpu::Surface`) that the user drags to the projector display and fullscreens.

**Projection rendering pipeline:**
1. Render all content sources to offscreen textures (already done by the mixer)
2. Build a virtual 3D scene: surfaces as textured quads, positioned per the stage model
3. Place a virtual camera matching the physical projector's position/orientation/FOV
4. Render the scene from that camera's perspective
5. Output the result to the projector's window/surface

**Calibration — corner-pin method:**
Rather than requiring the user to manually input camera position/rotation/FOV, use a **corner-pin calibration** workflow:
1. The output window goes fullscreen on the projector
2. Display a test pattern with known reference points (e.g., the 4 corners of each surface)
3. User drags the projected corners to align with the physical surface corners
4. System solves for the camera/homography matrix from the correspondences

This is intuitive — it's the same "drag corners to match" gesture VJs already understand — and mathematically derives the camera parameters automatically.

**Warp settings** are stored per-output in the scene file and persist across sessions.

### LED Direct Output

An LED wall/panel connected via a pixel-mapping controller (or direct video output). Content is cropped and scaled to match the panel's exact pixel dimensions — no perspective warp needed.

**LED rendering pipeline:**
1. Render the content source to a texture
2. Crop/scale the region corresponding to the surface's area
3. Output the pixel data to the LED controller

**LED output protocols (future — Phase 8f+):**
- Direct video output (same as projection, but no warp — just crop/scale to the panel resolution)
- NDI (network video)
- Art-Net / sACN (for LED pixel controllers) — stretch goal

### Output Management

Each output has:
- A unique name
- Type: Projection or LED Direct
- One or more assigned surfaces (which surfaces this output is responsible for displaying)
- Fullscreen toggle + target monitor selection
- Calibration state (corner-pin data for projection, crop region for LED)

---

## GPU Architecture Changes

### Current (single output)

```
RenderContext { device, queue, surface, surface_config, size }
```

The device, queue, and surface are bundled together. Only one window exists.

### Target (multi-output)

```
SharedGPU { device, queue, adapter }          ← created once at startup
MainWindow { surface, surface_config, size }  ← the UI window (also hosts egui)
OutputWindow { surface, surface_config, size, warp_state, source }  ← per output, 0..N
```

The `SharedGPU` is created from the main window's adapter (ensuring compatibility) and shared across all windows. Each output window creates its own `wgpu::Surface` but reuses the same device/queue.

**winit integration:** `ApplicationHandler::window_event` already receives `WindowId` — we dispatch events to the correct window. Each output window handles `Resized` and `RedrawRequested`. The main window additionally handles egui input.

### Warp Pipeline

A dedicated warp shader that takes:
- Input: the content texture (from mixer)
- Uniform: a 4x4 homography matrix (computed from corner-pin calibration)
- Output: the warped result on the output surface

For simple quad warp, this is a single fullscreen pass with UV remapping. For multi-surface per output (Phase 8d), it renders multiple warped quads in one pass.

---

## Phased Delivery

| Phase | Scope | Size | Depends On |
|-------|-------|------|------------|
| **8a** | Multi-window outputs + source routing (channel/master → output window) + fullscreen | M | Phase 1 |
| **8b** | 2D surface editor (place/name rectangular surfaces, route content to surfaces) | M | 8a |
| **8c** | Quad warp per output (corner-pin calibration, homography-based warp shader) | M | 8a |
| **8d** | Multi-surface per output (one projector covers multiple surfaces with individual warp) | L | 8b, 8c |
| **8e** | 3D stage model import (OBJ/glTF) + 3D preview + camera-based projection mapping | XL | 8b, 8d |
| **8f** | LED direct output (pixel-accurate crop/scale, no warp) | M | 8a, 8b |
| **8g** | Edge blending for overlapping projectors | L | 8d |

**Phase 8a** is the foundation — it generalizes the render context and proves multi-window works.
**Phases 8b + 8c** together deliver "usable at a gig with projectors."
**Phase 8d+** is the full spatial mapping vision.

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

