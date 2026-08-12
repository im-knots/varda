# Frame Analysis & Preprocessors

Varda can **look at a picture and turn what it sees into data** — the average brightness of a deck, the position of a performer's face, a live depth silhouette. One subsystem does all of this: the **analyzer engine**. It feeds two very different workflows from a single running analysis:

| Path | What it produces | Who uses it | Set up in |
|------|------------------|-------------|-----------|
| **Analysis → modulation** | normalized **scalars** (`brightness`, `face_x`, …) that drive *any* parameter | **performers** | the deck's analyzer setup → [Modulation](05-modulation.md) |
| **Preprocessors → shaders** | **data textures** (face landmarks, depth, mask, motion) injected into a shader | **shader authors** | a shader's ISF `PREPROCESSORS` block → [Shader Authoring](12-isf-authoring.md#analyzer-preprocessors) |

The important thing to understand: **the `brightness` modulation source and the `face_detect` preprocessor are the same engine.** An analyzer runs once per deck; whether its output becomes a modulation scalar or a shader texture is just a matter of who asked for it. Because analyzers are **reference-counted per deck**, wiring several outputs — or several shaders — to one analyzer costs a single analysis pass.

Analysis always runs **off the render thread**: each analyzer owns a background worker fed a downscaled copy of the deck's frame, and publishes results through a lock-free snapshot. If analysis is slower than the frame rate, consumers simply read the most recent result — the render loop never blocks or stutters.

## What's implemented

| Analyzer | Type id | Kind | Availability |
|----------|---------|------|--------------|
| **Brightness** | `brightness` | CPU, no ML | Always available |
| **Face detection** | `face_detect` | CPU, ONNX (BlazeFace → 478-point mesh) | Builds with the `face-detection` feature (default; off on macOS Intel / Windows) |
| **Depth sensor** | `depth_sensor` | GPU, physical device (Kinect v1) | Builds with the `depth` feature (default; off on macOS Intel / Windows) |

Analyzer types the specs describe but that are **not yet implemented** — `depth_estimate`, `segmentation`, `optical_flow`, `edge_detect`, `motion`, `color_dominant`, `hand_gesture` — are **planned**. They do not appear in the pickers and shaders that request them fall back to black (except `depth_sensor`, which is required — see below).

---

## Analysis as Modulation (performers)

Any analyzer scalar can drive any parameter, exactly like an LFO or an audio band. This turns the visuals themselves into a controller: brighten one deck and another deck's blur opens; move your face left and a generator rotates.

You add an **Analyzer** modulation source from the deck's analyzer setup (not the Modulation panel's `➕` row). It then behaves like every other source — assign it with a slider's `〰` button, stack it, smooth it. See [Modulation → Analyzer](05-modulation.md#analyzer) for the assignment workflow.

### `brightness` outputs (always available)

| Output | Meaning |
|--------|---------|
| `brightness` | Average luminance (Rec.709) |
| `contrast` | Standard deviation of luminance |
| `red` / `green` / `blue` | Average per-channel value |

### `face_detect` outputs

Available when the build includes the `face-detection` feature. A two-stage pipeline (BlazeFace detection → a 478-point face mesh) exposes the primary face as scalars:

| Output | Meaning | Range |
|--------|---------|-------|
| `face_count` | Number of faces detected (normalized — one face reads `0.1`) | 0–1 |
| `face_x` | Primary face centre, horizontal | 0–1 |
| `face_y` | Primary face centre, vertical | 0–1 |
| `face_size` | Primary face bounding-box area | 0–1 |
| `face_rotation` | Primary face tilt (from the eye-line angle) | 0–1 |

Each analyzer source has a **Smoothing** control (0.0–0.99, default `0.3`) that damps jitter — essential for face outputs, which are noisier than `brightness`.

---

## Depth Sensor (performers)

`depth_sensor` is different from the other analyzers: it reads a **physical depth camera** (Kinect v1) rather than a deck's own frame, and runs entirely on the GPU — the sensor's pixels never touch host memory. It surfaces four live streams a shader can consume: a normalized **depth** map, a subject **mask**, screen-space **motion**, and the sensor's **rgb** stream.

As a performer you don't write the shader — you pick one built around depth (e.g. a silhouette or depth-fog look) and **frame the room**. Every depth shader shares the same runtime controls in the deck's bottom bar, all MIDI/OSC-mappable at `deck/<uuid>/depth_prepro/<param>`:

| Control | Path param | Range | Default | What it does |
|---------|-----------|-------|---------|--------------|
| Near clip | `near` | 0–8000 mm | 500 mm | Closest distance mapped into the depth range |
| Far clip | `far` | 0–8000 mm | 4000 mm | Farthest distance (always kept above near) |
| Smoothing | `smoothing` | 0.0–0.99 | 0.5 | Temporal smoothing of the depth stream |
| Hole fill | `hole_fill` | 0–8 texels | 2 | Fills small gaps the sensor can't resolve |
| Mask feather | `mask_feather` | 0–8 texels | 3 | Softens the silhouette edge |
| Motion gain | `motion_gain` | 0–8 | 3.2 | Amplifies the motion stream |
| Mirror | `mirror` | on/off | on | Flips horizontally to match a front-facing camera |

> **Set near/far first when you move to a new room.** They define which slice of space becomes the picture; everything else is polish.

**Depth shaders are required-hardware shaders.** A shader that declares `depth_sensor` will **refuse to load** if no sensor is attached (a black fallback is useless for a look whose entire content is a silhouette) — you'll get an error toast naming the shader. The `depth` feature is compiled out on Windows and macOS Intel, so these shaders never load there.

The shader-author side of depth (texture formats, GLSL access) lives in [Shader Authoring → Depth Sensor](12-isf-authoring.md#depth_sensor--live-depth-camera).

---

## Preprocessors (concept)

When an effect needs **structured data the fragment shader can't compute itself** — face landmarks, a depth map, a segmentation mask — the shader declares a **preprocessor** in its ISF header. Varda runs the named analyzer and injects its output as a **texture** bound alongside the shader's other inputs; the shader reads it with ordinary texture samples.

From a performer's seat this is invisible: you drop the effect on a deck and it works, drawing on whatever analysis it needs. The full authoring mechanics — the `PREPROCESSORS` JSON block, binding order, and `texelFetch` access patterns — are in [Shader Authoring → Analyzer Preprocessors](12-isf-authoring.md#analyzer-preprocessors).

---

## Analyzer HTTP API

Every analyzer operation is on the [HTTP API](13-api.md) under the **Analyzers** and **Modulation** tags.

### Discover what a deck can run

```sh
curl http://localhost:8080/api/library/analyzers
```

Returns each available analyzer type with its `scalar_outputs` (name, description, range, default smoothing) and `texture_outputs`. Types absent from a build (e.g. `face_detect` on macOS Intel) are omitted.

### Attach / detach an analyzer on a deck

```sh
# Attach (reference-counted — a second attach just shares the running instance)
curl -X POST http://localhost:8080/api/decks/<deck_uuid>/analyzers \
  -H "Content-Type: application/json" \
  -d '{"analyzer_type": "face_detect", "options": {}}'

# Detach (stops the instance when the last consumer releases it)
curl -X DELETE http://localhost:8080/api/decks/<deck_uuid>/analyzers/face_detect
```

### Drive a parameter from an analyzer scalar

```sh
# Create an analyzer modulation source (returns its uuid)
curl -X POST http://localhost:8080/api/modulation/analyzer \
  -H "Content-Type: application/json" \
  -d '{"deck_id": "<deck_uuid>", "analyzer_type": "face_detect", "output_name": "face_x"}'

# Adjust its smoothing (0.0–0.99)
curl -X PUT http://localhost:8080/api/modulation/<source_uuid>/analyzer/smoothing \
  -H "Content-Type: application/json" -d '{"value": 0.4}'
```

Assign the returned source to any parameter with `POST /api/modulation/assign`, exactly like an LFO — see [HTTP API](13-api.md) and [Modulation](05-modulation.md#routing).

---

[← Prev: HTTP API & Headless Mode](13-api.md) · [Home](README.md) · [Next: Arrangement Mode →](15-arrangement.md)
