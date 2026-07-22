# HTTP API & Headless Mode

## Overview

Varda's GUI and HTTP API are co-equal consumers of the same engine. The GUI reads state snapshots and emits actions; the API reads the same snapshots and sends the same commands. Neither is an afterthought — they share identical engine contracts.

The API runs on **port 8080** by default (configurable with `--port`).

## Security & Network Trust Model

**Varda trusts the network.** The HTTP API has **no authentication** and binds to
**all interfaces** (`0.0.0.0`), and the OSC input (default port 9000) does the
same. CORS is intentionally permissive (see [CORS](#cors)). This is a deliberate
design choice for the live-performance and installation use cases: a dedicated
front-of-house or show/installation network where controllers, control panels,
and automation scripts talk to the engine without credential friction.

The practical consequence: **anyone who can reach the port has full control of the
engine** — creating/removing decks, loading local media and LUT files by path,
starting streams and recordings, and shutting the process down (`POST /api/shutdown`).

Run Varda only on a network you control. If you need it reachable from a wider or
untrusted network, put it behind your own boundary. Bind the machine to a private
interface, use a firewall or VPN, or front it with an authenticating reverse proxy.


## Swagger UI

Browse all routes interactively at:

```
http://localhost:8080/api/docs
```

Every parameter, path variable, and request body field is documented with descriptions and examples in the OpenAPI 3.0 spec.

## Headless Mode

Run Varda without a UI window — the engine renders on a timer-driven loop, controlled entirely via the API:

```sh
varda --headless --port 8080 --fps 60
```

In headless mode:
- No main window is created (output windows for projectors can still be created via API)
- The render loop runs at `--fps` rate using sleep-based throttling
- All outputs defined in `stage.json` auto-start on launch — NDI sends, SRT streams, HLS/DASH outputs, recordings, and display outputs (fullscreen on connected monitors) all activate automatically
- Graceful shutdown on SIGTERM/SIGINT or `POST /api/shutdown`

This enables the installation use case: configure in windowed mode, save, then deploy headless. All streaming, recording, and network I/O features work identically with or without the UI.

## WebSocket

Connect to the WebSocket endpoint for real-time state streaming:

```
ws://localhost:8080/api/ws
```

**On connect:** Full `EngineState` JSON snapshot.

**Subsequent frames (~30fps):** JSON Patch (RFC 6902) deltas — only changes since the last update:

```json
[
  { "op": "replace", "path": "/mixer/crossfader", "value": 0.75 },
  { "op": "replace", "path": "/mixer/channels/0/decks/0/opacity", "value": 0.5 }
]
```

**Client → Server:** Send `EngineCommand` JSON messages with an optional `"id"` field for response correlation:

```json
{ "id": "req-1", "command": "SetCrossfader", "position": 0.5 }
```

## Common Patterns

### Get engine state

```sh
curl http://localhost:8080/api/state
```

### Get scene structure (channels, decks, effects, UUIDs)

```sh
curl http://localhost:8080/api/scene
```

### Set crossfader position

```sh
curl -X PUT http://localhost:8080/api/mixer/crossfader \
  -H "Content-Type: application/json" \
  -d '{"position": 0.75}'
```

### Add a shader deck to a channel

```sh
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks/shader \
  -H "Content-Type: application/json" \
  -d '{"shader_name": "Sine"}'
```

### Add an HTML deck to a channel

```sh
curl -X POST http://localhost:8080/api/channels/<ch_idx>/decks/html \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/overlay.html"}'
```

### Start an auto-crossfade

```sh
curl -X POST http://localhost:8080/api/mixer/auto-crossfade \
  -H "Content-Type: application/json" \
  -d '{"target": 1.0, "duration_secs": 2.0, "easing": "Linear"}'
```

### Set tonemap mode

```sh
curl -X PUT http://localhost:8080/api/mixer/tonemap \
  -H "Content-Type: application/json" \
  -d '{"mode": "Aces"}'
```

Modes: `Bypass`, `Aces`, `Reinhard`, `ReinhardExtended`, `HableFilmic`, `Uchimura`, `Lottes`, `AgX`, `KhronosPbrNeutral`

### Load a 3D LUT

```sh
curl -X PUT http://localhost:8080/api/mixer/lut \
  -H "Content-Type: application/json" \
  -d '{"filename": "my-look.cube"}'
```

Place `.cube` or `.3dl` files in `.varda/luts/`. The filename is relative to that directory.

### Unload the active LUT

```sh
curl -X DELETE http://localhost:8080/api/mixer/lut
```

### Create a macro and bind a target

A macro drives many parameters from one control. Create it, add a target, then drive it live (or map `macro/<uuid>/value` to MIDI/OSC). See [Macro Controls](15-macro-controls.md).

```sh
# Create a knob macro (returns its uuid)
curl -X POST http://localhost:8080/api/macros \
  -H "Content-Type: application/json" \
  -d '{"kind": "Knob"}'

# Add a target parameter
curl -X POST http://localhost:8080/api/macros/<uuid>/targets \
  -H "Content-Type: application/json" \
  -d '{"path": "deck/<deck_uuid>/effect/<fx_uuid>/param/scale"}'

# Drive the macro (fans out to all targets)
curl -X PUT http://localhost:8080/api/macros/<uuid>/value \
  -H "Content-Type: application/json" \
  -d '{"value": 0.75}'
```

### Send any engine command

```sh
curl -X POST http://localhost:8080/api/command \
  -H "Content-Type: application/json" \
  -d '{"SetCrossfader": {"position": 0.5}}'
```

### Save the workspace

```sh
curl -X POST http://localhost:8080/api/scene/save
```

### Shut down (headless)

```sh
curl -X POST http://localhost:8080/api/shutdown
```

### Curve a surface edge (Bezier)

Toggle an edge between a straight line and a cubic bezier (`to_cubic: false` straightens it again):

```sh
curl -X PUT http://localhost:8080/api/surfaces/<uuid>/edge/convert \
  -H "Content-Type: application/json" \
  -d '{"edge_idx": 0, "to_cubic": true}'
```

### Move a curve-path anchor

```sh
curl -X PUT http://localhost:8080/api/surfaces/<uuid>/path/anchor \
  -H "Content-Type: application/json" \
  -d '{"anchor_idx": 1, "pos": [0.3, 0.4]}'
```

### Move a cubic control handle

`handle` is `C1` or `C2` (the two control points of the cubic segment):

```sh
curl -X PUT http://localhost:8080/api/surfaces/<uuid>/path/handle \
  -H "Content-Type: application/json" \
  -d '{"segment_idx": 0, "handle": "C1", "pos": [0.6, 0.7]}'
```

### Warp a surface (per-surface)

Warp is a property of the surface, keyed by its UUID. Move a corner-pin corner:

```sh
curl -X PUT http://localhost:8080/api/surfaces/{uuid}/warp/corner \
  -H "Content-Type: application/json" \
  -d '{"corner_idx": 0, "position": [0.1, 0.1]}'
```

Clear a surface's warp (back to native position):

```sh
curl -X POST http://localhost:8080/api/surfaces/{uuid}/warp/reset
```

### Subdivide a surface's warp into a mesh

Converts the surface's warp to a `cols` × `rows` grid, preserving the current
deformation (a corner-pin becomes a bilinear grid). Dimensions clamp to `[2, 64]`.

```sh
curl -X PUT http://localhost:8080/api/surfaces/{uuid}/warp/subdivisions \
  -H "Content-Type: application/json" \
  -d '{"cols": 3, "rows": 3}'
```

### Move a mesh warp point

Moves a single grid point (row-major) of the surface's mesh warp. No-op if the
surface's warp is not currently a mesh.

```sh
curl -X PUT http://localhost:8080/api/surfaces/{uuid}/warp/mesh-point \
  -H "Content-Type: application/json" \
  -d '{"row": 1, "col": 1, "position": [0.6, 0.4]}'
```

### Bind/unbind the warp to the surface shape (auto-warp)

When `bound` is `true` the warp auto-conforms to the surface outline; setting it
`false` unbinds and materialises the conforming warp for manual fine-tuning.

```sh
curl -X POST http://localhost:8080/api/surfaces/{uuid}/warp/bind \
  -H "Content-Type: application/json" \
  -d '{"bound": false}'
```

### Bezier (curved) warp

Convert the surface's warp into a smooth bezier patch grid (seeded from the
current warp so the shape is preserved), then edit anchors and tangent handles or
resize the control cage. Bezier editing is meaningful only while the warp is
unbound.

```sh
# Convert to a bezier patch grid
curl -X POST http://localhost:8080/api/surfaces/{uuid}/warp/bezier

# Move a control anchor (row-major grid coords)
curl -X PUT http://localhost:8080/api/surfaces/{uuid}/warp/anchor \
  -H "Content-Type: application/json" \
  -d '{"row": 0, "col": 0, "position": [0.15, 0.25]}'

# Move a tangent handle. horizontal=true → edge (r,c)→(r,c+1); false → (r,c)→(r+1,c).
# which=0 near the start anchor, 1 near the end anchor.
curl -X PUT http://localhost:8080/api/surfaces/{uuid}/warp/handle \
  -H "Content-Type: application/json" \
  -d '{"horizontal": true, "row": 0, "col": 0, "which": 0, "position": [0.33, 0.05]}'

# Resize the anchor cage (adds/removes control points; dims clamp to [2, 64])
curl -X PUT http://localhost:8080/api/surfaces/{uuid}/warp/cage \
  -H "Content-Type: application/json" \
  -d '{"cols": 3, "rows": 3}'
```

### Set an output's calibration mode

Switches an output between `Off`, `Projector` (full-frame test card), and
`Surfaces` (per-surface test cards through each warp).

```sh
curl -X PUT http://localhost:8080/api/outputs/0/calibration \
  -H "Content-Type: application/json" \
  -d '{"mode": "Projector"}'
```

## Route Groups

The API is organized into 16 OpenAPI tags:

| Tag | Examples |
|-----|----------|
| **System** | `GET /api/health`, `POST /api/shutdown` |
| **Mixer** | `PUT /api/mixer/crossfader`, `POST /api/mixer/auto-crossfade`, `PUT /api/mixer/tonemap`, `PUT /api/mixer/lut`, `DELETE /api/mixer/lut` |
| **Channels** | `POST /api/channels`, `PUT /api/channels/:uuid/opacity` |
| **Decks** | `POST /api/channels/:uuid/decks/shader`, `POST /api/channels/:idx/decks/html`, `PUT /api/decks/:uuid/opacity` |
| **Video** | `POST /api/decks/:uuid/video/toggle-play`, `PUT /api/decks/:uuid/video/speed` |
| **Effects** | `POST /api/effects`, `POST /api/effects/toggle` |
| **Modulation** | `POST /api/modulation/lfo`, `POST /api/modulation/assign` |
| **Macros** | `POST /api/macros`, `POST /api/macros/:uuid/targets`, `PUT /api/macros/:uuid/value`, `PUT /api/macros/:uuid/button/behavior` |
| **Params** | `PUT /api/params` (set any parameter by path) |
| **Surfaces** | `POST /api/surfaces/rect`, `PUT /api/surfaces/:uuid/source`, `PUT /api/surfaces/:uuid/path/handle`, `PUT /api/surfaces/:uuid/warp/corner`, `POST /api/surfaces/:uuid/warp/reset`, `PUT /api/surfaces/:uuid/warp/subdivisions`, `PUT /api/surfaces/:uuid/warp/mesh-point`, `POST /api/surfaces/:uuid/warp/bind`, `POST /api/surfaces/:uuid/warp/bezier`, `PUT /api/surfaces/:uuid/warp/anchor`, `PUT /api/surfaces/:uuid/warp/handle`, `PUT /api/surfaces/:uuid/warp/cage` |
| **Outputs** | `POST /api/outputs/windowed`, `POST /api/outputs/headless`, `PUT /api/outputs/:idx/calibration` |
| **Sequences** | `POST /api/sequences`, `POST /api/sequences/:idx/play` |
| **Audio** | `POST /api/audio/scan`, `POST /api/audio/open` |
| **Streams** | `POST /api/streams/library` |
| **Devices** | `POST /api/devices/ndi/scan`, `POST /api/devices/midi/scan` |
| **Auto Transitions** | `PUT /api/decks/:uuid/auto-transition/enabled` |

For the complete list of all routes with request/response schemas, see the Swagger UI at `/api/docs`.

## CORS

Permissive CORS is enabled on all routes:

```
Access-Control-Allow-Origin: *
Access-Control-Allow-Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS
Access-Control-Allow-Headers: Content-Type, Authorization
```

Browser-based control panels work from any origin without configuration. This
pairs with the [trusted-network model](#security--network-trust-model): there is
no auth, so origin restrictions would add friction without a security benefit on
a trusted LAN. Do not expose the port to untrusted networks.

---

[← Prev: ISF Shader Authoring](12-isf-authoring.md) · [Home](README.md) · [Next: Benchmarking →](14-benchmarking.md)
