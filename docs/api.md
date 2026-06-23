# HTTP API & Headless Mode

## Overview

Varda's GUI and HTTP API are co-equal consumers of the same engine. The GUI reads state snapshots and emits actions; the API reads the same snapshots and sends the same commands. Neither is an afterthought — they share identical engine contracts.

The API runs on **port 8080** by default (configurable with `--port`).

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
- All outputs from `stage.json` auto-start on launch
- Graceful shutdown on SIGTERM/SIGINT or `POST /api/shutdown`

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

## Route Groups

The API is organized into 15 OpenAPI tags:

| Tag | Examples |
|-----|----------|
| **System** | `GET /api/health`, `POST /api/shutdown` |
| **Mixer** | `PUT /api/mixer/crossfader`, `POST /api/mixer/auto-crossfade`, `PUT /api/mixer/tonemap`, `PUT /api/mixer/lut`, `DELETE /api/mixer/lut` |
| **Channels** | `POST /api/channels`, `PUT /api/channels/:uuid/opacity` |
| **Decks** | `POST /api/channels/:uuid/decks/shader`, `PUT /api/decks/:uuid/opacity` |
| **Video** | `POST /api/decks/:uuid/video/toggle-play`, `PUT /api/decks/:uuid/video/speed` |
| **Effects** | `POST /api/effects`, `POST /api/effects/toggle` |
| **Modulation** | `POST /api/modulation/lfo`, `POST /api/modulation/assign` |
| **Params** | `PUT /api/params` (set any parameter by path) |
| **Surfaces** | `POST /api/surfaces/rect`, `PUT /api/surfaces/:uuid/source` |
| **Outputs** | `POST /api/outputs/windowed`, `POST /api/outputs/headless` |
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

Browser-based control panels work from any origin without configuration.
