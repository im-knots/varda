# Outputs

An **output** renders its assigned surfaces (or the master mix directly) onto a destination — a window, a fullscreen display, a network stream, or a recording file. Varda supports many simultaneous, independent outputs. This page covers creating outputs, choosing where they draw from, rotation, and multi-output recording. For projector alignment (surfaces, warp, edge blending) see [Projection Mapping](08-projection.md); for stream protocol details see [Streaming & I/O](09-streaming-and-io.md).

## Creating an Output

The output panel (right side) has three buttons:

- **+ Windowed** — a floating, resizable output window
- **+ Recording** — a headless recording output (defaults to H.264 → `output.mp4`)
- **+ Stream** — a headless network sender (defaults to NDI)

## Output Targets

Any output can be retargeted after creation:

| Target | Notes |
|--------|-------|
| **Windowed** | Floating OS window |
| **Display (Fullscreen)** | Borderless fullscreen on a chosen monitor |
| **Recording** | Video file via ffmpeg |
| **SRT / HLS / DASH / RTMP** | Network streams (video-only) |
| **NDI** | Network Device Interface sender |
| **Syphon** | macOS inter-app texture sharing |

### Display Selection

A windowed output has a **Display:** dropdown listing **Windowed** plus every connected monitor, formatted `Name (W×H)` — e.g. `Built-in Retina Display (1920x1200)`. Selecting a monitor sends the window borderless-fullscreen to it.

Monitors are re-enumerated every frame, so hot-plugging works without restart. If an output is configured for a monitor that isn't connected at startup, it falls back to a window and shows a notice (`Monitor '<name>' not connected — output '<name>' opened as window`); re-select the monitor from the dropdown once it appears.

## Output Rotation

Each output has a **Rotation:** control with four options — **0°**, **90°**, **180°**, **270°** — applied at the final blit. Use 90°/270° for portrait projectors/displays; those values swap width/height for the effective render. Rotation is GPU-only (no mesh recompute).

## Source Routing

Outputs draw the **surfaces** assigned to them, and each surface pulls from a content source chosen in the Stage Editor (**Source:** picker). The options:

| Source | What it is |
|--------|-----------|
| **Master** | Final composited output (all channels + master effects) |
| **Channel** | A single channel's post-FX output |
| **Channels** | A sub-mix of several channels (see below) |
| **Deck** | A single deck's raw output (pre-blend, pre-FX) |
| **Domemaster** | Fisheye equidistant-azimuthal projection |

To assign surfaces to an output, use **+ Assign Surface**; remove one with **x**, or reset its warp with **↺**.

### The "Channels" Sub-Mix

Selecting multiple channels (via checkboxes in the source picker) creates a **Channels** sub-mix: those channels are composited together using each channel's own **opacity and blend mode**, exactly as in the master compositor — but **master effects are not applied**. This lets a surface show, say, only channels 0+1 with a live crossfade between them while another surface shows the full master. Selecting a single channel collapses to **Channel**; selecting none falls back to **Master**. Sub-mix textures are cached per unique channel set and reused across frames.

## Recording Outputs

Click **+ Recording** to add a recording output; each runs its own ffmpeg subprocess, so several feeds can record at once. Codecs, file-path rules, and the **▶ Start / ⏹ Stop** controls are covered in [Recording](09-streaming-and-io.md#recording).

> **Audio passthrough.** Recording and all streaming targets can mux audio from a capture device via the output's **Audio:** dropdown — see [Audio Passthrough](09-streaming-and-io.md#audio-passthrough).

## Persistence

Outputs, their targets, rotation, surface assignments, and warp calibration are saved in `stage.json` (the venue layout), separate from your scene/show. See [Persistence](02-concepts.md#persistence).

---

[← Prev: Macro Controls](15-macro-controls.md) · [Home](README.md) · [Next: Projection Mapping →](08-projection.md)
