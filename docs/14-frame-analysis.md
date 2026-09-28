# Frame Analysis & Preprocessors

Varda can analyze a picture and turn the result into data: the average brightness of a deck, the position of a performer's face, a live depth silhouette. The **analyzer engine** does all of this and feeds two workflows:

| Path | What it produces | Who uses it | Set up in |
|------|------------------|-------------|-----------|
| **Analysis → modulation** | normalized **scalars** (`brightness`, `face_x`, …) that drive *any* parameter | **performers** | the deck's analyzer setup → [Modulation](05-modulation.md) |
| **Preprocessors → shaders** | **data textures** (face landmarks, depth, mask, motion) injected into a shader | **shader authors** | a shader's ISF `PREPROCESSORS` block → [Shader Authoring](12-isf-authoring.md#analyzer-preprocessors) |

The `brightness` modulation source and the `face_detect` preprocessor use the same engine. An analyzer runs once per deck, and its output can go to modulation, to shaders, or both. Analyzers are **reference-counted per deck**, so connecting several modulation sources or shaders to one analyzer costs one analysis pass.

Analysis runs **off the render thread**. Each analyzer has a background worker that receives a downscaled copy of the deck's frame and publishes results through a lock-free snapshot. If analysis is slower than the frame rate, consumers read the latest result. The render loop does not wait for it.

## What's implemented

| Analyzer | Type id | Kind | Availability |
|----------|---------|------|--------------|
| **Brightness** | `brightness` | CPU, no ML | Always available |
| **Face detection** | `face_detect` | CPU, ONNX (BlazeFace → 478-point mesh) | Builds with the `face-detection` feature (default; off on macOS Intel / Windows) |
| **Depth sensor** | `depth_sensor` | GPU, physical device (Kinect v1) | Builds with the `depth` feature (default; off on macOS Intel / Windows) |

These analyzer types are **planned** and not yet implemented: `depth_estimate`, `segmentation`, `optical_flow`, `edge_detect`, `motion`, `color_dominant`, `hand_gesture`. They do not appear in the pickers. Shaders that request them get black instead. (`depth_sensor` is different: it is required, see below.)

---

## Analysis as Modulation (performers)

Any analyzer scalar can drive any parameter, like an LFO or an audio band. For example, brighten one deck to open another deck's blur, or move your face left to rotate a generator.

Add an **Analyzer** modulation source from the deck's analyzer setup, not from the Modulation panel's `➕` row. It then works like any other source: assign it with a slider's `〰` button, stack it, smooth it. See [Modulation → Analyzer](05-modulation.md#analyzer) for how to assign it.

### `brightness` outputs (always available)

| Output | Meaning |
|--------|---------|
| `brightness` | Average luminance (Rec.709) |
| `contrast` | Standard deviation of luminance |
| `red` / `green` / `blue` | Average per-channel value |

### `face_detect` outputs

Available when the build includes the `face-detection` feature. A two-stage pipeline (BlazeFace detection → a 478-point face mesh) outputs the primary face as scalars:

| Output | Meaning | Range |
|--------|---------|-------|
| `face_count` | Number of faces detected (normalized: one face reads `0.1`) | 0–1 |
| `face_x` | Primary face center, horizontal | 0–1 |
| `face_y` | Primary face center, vertical | 0–1 |
| `face_size` | Primary face bounding-box area | 0–1 |
| `face_rotation` | Primary face tilt (from the eye-line angle) | 0–1 |

Each analyzer source has a **Smoothing** control (0.0–0.99, default `0.3`) that reduces jitter. Face outputs are noisier than `brightness`, so they usually need it.

---

## Depth Sensor (performers)

`depth_sensor` reads a **physical depth camera** (Kinect v1) instead of a deck's frame. It runs entirely on the GPU; the sensor's pixels never go through host memory. It provides four live streams a shader can use: a normalized **depth** map, a subject **mask**, screen-space **motion**, and the sensor's **rgb** stream.

As a performer, you pick a shader built for depth (for example a silhouette or depth-fog look) and set up the sensor for the room. Every depth shader has the same controls in the deck's bottom bar. All are MIDI/OSC-mappable at `deck/<uuid>/depth_prepro/<param>`:

| Control | Path param | Range | Default | What it does |
|---------|-----------|-------|---------|--------------|
| Near clip | `near` | 0–8000 mm | 500 mm | Closest distance mapped into the depth range |
| Far clip | `far` | 0–8000 mm | 4000 mm | Farthest distance (always kept above near) |
| Smoothing | `smoothing` | 0.0–0.99 | 0.5 | Temporal smoothing of the depth stream |
| Hole fill | `hole_fill` | 0–8 texels | 2 | Fills small gaps the sensor can't resolve |
| Mask feather | `mask_feather` | 0–8 texels | 3 | Softens the silhouette edge |
| Motion gain | `motion_gain` | 0–8 | 3.2 | Amplifies the motion stream |
| Mirror | `mirror` | on/off | on | Flips horizontally to match a front-facing camera |

> **Set near and far first in a new room.** They set which slice of space appears in the picture.

**Depth shaders require the hardware.** A shader that declares `depth_sensor` **does not load** if no sensor is attached, and an error toast names the shader. (A black fallback would be useless for a look made entirely of a silhouette.) The `depth` feature is not built on Windows or macOS Intel, so these shaders never load there.

For the shader-author side of depth (texture formats, GLSL access), see [Shader Authoring → Depth Sensor](12-isf-authoring.md#depth_sensor-live-depth-camera).

---

## Preprocessors (concept)

Some effects need data the fragment shader can't compute itself, such as face landmarks, a depth map or a segmentation mask. The shader declares a **preprocessor** in its ISF header. Varda runs the named analyzer and binds its output as a **texture** next to the shader's other inputs. The shader reads it with ordinary texture samples.

As a performer you don't need to do anything: drop the effect on a deck and it runs the analysis it needs. For the `PREPROCESSORS` JSON block, binding order and `texelFetch` access patterns, see [Shader Authoring → Analyzer Preprocessors](12-isf-authoring.md#analyzer-preprocessors).

---

## Analyzer HTTP API

All analyzer operations are in the [HTTP API](13-api.md) under the **Analyzers** and **Modulation** tags.

### List available analyzers

```sh
curl http://localhost:8080/api/library/analyzers
```

Returns each available analyzer type with its `scalar_outputs` (name, description, range, default smoothing) and `texture_outputs`. Types not in the build (for example `face_detect` on macOS Intel) are left out.

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

Assign the returned source to any parameter with `POST /api/modulation/assign`, as with an LFO. See [HTTP API](13-api.md) and [Modulation](05-modulation.md#routing).

---

[← Prev: HTTP API & Headless Mode](13-api.md) · [Home](README.md) · [Next: Arrangement Mode →](15-arrangement.md)
