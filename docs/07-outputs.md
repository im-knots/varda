# Outputs

An **output** renders its assigned surfaces (or the master mix directly) onto a destination such as a window, a fullscreen display, a network stream, or a recording file. Varda supports many simultaneous, independent outputs. This page covers creating outputs, choosing where they draw from, rotation, and multi-output recording. For projector alignment (surfaces, warp, edge blending) see [Projection Mapping](08-projection.md); for stream protocol details see [Streaming & I/O](09-streaming-and-io.md).

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

## Output Format

Every output picks one contract, and **you can only select the ones that output can actually
deliver.** The rest stay in the list greyed out, and hovering one tells you why it is unavailable:
a Syphon output says `Syphon interoperability is limited to BGRA8`, an H.264 stream says
`H.264 streaming is limited to eight-bit SDR`. There is no need to select a mode, read a warning,
and undo it to find out what an output can do.

The full set of contracts:

- **8-bit SDR** is the compatibility default.
- **10-bit SDR** requests more code values without changing gamut, brightness range, or tonemapping.
- **HDR10** requests a PQ (ST 2084) transfer in a BT.2020 container, with mastering metadata.
- **HLG** requests a relative HDR transfer (ARIB STD-B67) that needs no mastering metadata.
- **EDR (monitor)** is macOS only and is for *looking at* HDR on this Mac, not for delivering
  it. It writes linear values straight to an extended-range display, so you can see the
  highlights an HDR10 recording contains without a mastering suite.

Selecting HDR10 reveals a **Peak** control (600 / 1000 / 1500 / 4000 cd/m², default 1000). This is
the mastering target: the brightness the top of the range maps to. It does not change where existing
content sits, because linear 1.0 stays anchored at 203 cd/m² reference white (ITU-R BT.2408) whatever
peak you choose. Everything your shaders and blend modes already produce above 1.0, which used to be
clipped away, becomes the headroom above white.

HLG has no Peak control, and that is the point rather than an omission. HLG is *relative*: the top
of its range means "as bright as this display gets", so the display decides rather than you. Linear
1.0 lands at the same 203 cd/m² reference white, at 75% signal per BT.2408, giving about 1.92 stops
of headroom against 2.30 for HDR10 at a 1000 cd/m² peak.

**Which to pick.** For a file, HDR10. For a live stream, HLG, unless a CDN spec names HDR10. The
reason is metadata: HDR10 carries content light levels that are supposed to be measured across a
finished programme, and a live stream never finishes. HLG needs none, which is why broadcast uses it
for live.

The **Dither** checkbox is enabled by default. It applies a stable, destination-aware pattern before
the final integer conversion, which reduces visible banding without temporal noise.

### When the picker changes under you

Capability follows the target **and the codec**, so the list changes when you change either.
Switching a recording from HEVC to H.264 removes HDR10 from its picker.

This catches people out most often on streams, because **SRT, HLS, DASH and RTMP all default to
H.264**, which is eight-bit. A newly created or newly switched stream output therefore has only
8-bit SDR selectable, and that is correct rather than stale. The greyed-out HDR10 entry will say
`H.264 streaming is limited to eight-bit SDR`. Set the **Codec** to H.265 and 10-bit SDR, HDR10 and
HLG all become selectable.

NDI is the exception that makes this look inconsistent: it has no codec to set, so it shows its real
capability straight away.

EDR is never selectable on a recording or a stream. It is a way of *looking at* HDR on this Mac, not
a format anything can carry, so it is deliverable on display outputs only.

Your choice is **kept, not discarded**. Switch that recording back to HEVC and HDR10 returns as the
selection. While an output cannot carry what you asked for, the picker shows what it is actually
delivering and the card explains the difference, for example
`HDR10 fallback: the configured codec is eight-bit`.

That warning is now the only thing it was ever meant to be: a note that something changed underneath
a choice you had already made.

### Watching HDR on a Mac

Set a windowed output to **EDR (monitor)** on a display that supports it (any recent MacBook
Pro or Pro Display XDR) and content above display white shows as highlights rather than
clipping at white. Set the Peak control to the same value your recording uses, so what you see
matches what the file will contain.

The card reports `Display headroom 4.2x · monitoring to 4.9x`, and warns when the top of your
range is being clipped. Headroom starts at 1.0 and climbs once EDR actually engages, and it
shrinks as you turn display brightness up, because SDR white rises toward the panel's ceiling.
Turning brightness *down* gives you more headroom.

In the default adaptive display preset this shows you that range exists, not what it will look
like on someone else's screen. To judge a grade, switch the display to a reference preset such
as **Apple XDR Display (P3-1600 nits)**, which pins SDR white and gives a fixed known headroom.

EDR is not proof that an HDR10 file is correct. A file can look right here and still carry
wrong metadata, which is what `ffprobe` and a grading tool are for.

### HDR range, not wide gamut

Varda composites in linear **Rec.709**. An HDR10 output signals BT.2020 because the format requires
it, but the content inside is Rec.709-gamut: you get the extra brightness range, not extra
saturation. This is deliberate. Changing the working space would alter what every existing blend
mode, shader, and saved scene means, which is a much larger change than HDR delivery needs.

Do not read BT.2020 in the file's metadata as a wide-gamut claim.

### Where HDR works

| Output | HDR10 (PQ) | HLG | EDR |
|--------|------------|-----|-----|
| **Recording** | HEVC and AV1. Verified with `ffprobe`: PQ transfer, BT.2020 primaries, ST 2086 mastering display, and CTA-861.3 content light level. | Not yet | No, and never: EDR is not a delivery format |
| **Streaming** | SRT, HLS, DASH, and Enhanced RTMP on HEVC or AV1. Legacy RTMP cannot. | Same protocols | No |
| **Display** | When the GPU surface exposes RGB10A2 with the BT.2100 PQ color space. Implemented but **not yet qualified against capture hardware or an LED processor**, so treat it as experimental. | Not on Windows (DX12 exposes no HLG surface) | macOS, on an EDR-capable display |
| **NDI, Syphon** | Not supported. These fall back and say so. | Not supported | No |
| **ProRes, H.264, HAP** | Not supported. HAP is block texture compression and is 8-bit by construction. | Not supported | No |

See [HDR Delivery](09-streaming-and-io.md#hdr-delivery) for the streaming detail and how to verify
a stream.

An output that cannot carry what you asked for keeps your request, delivers the best it can, and
shows the reason. `H.264 cannot carry HDR10; HEVC or AV1 is required` names the obstacle rather than
blaming bit depth.

### Limitations worth knowing

**There are two LUT slots** The **Look LUT** is graded before the
tonemap, on scene-linear light, so it reaches every output including HDR ones. The **Calibration
LUT** is graded after the tonemap and is calibrated against the SDR output transform, so applying it
after a PQ transform would put its midtones in the wrong place; HDR outputs skip it and the output
card says which LUT was skipped.

Put your show's look in the Look slot. Use the Calibration slot for a specific display's correction.

### Per-output tonemap

Each output card has a **Tonemap** picker. It defaults to `Show (<curve>)`, meaning it inherits the
show-wide curve from the 🎨 Tonemap panel, and naming which curve that currently is. Pick any other
curve to grade that one output differently, or pick **Show default** to go back to inheriting.

This exists because a tonemap *is* an output transform: a projector and a master recording are
different mediums and can want different curves from the same program. Outputs sharing a curve share
the work, so overriding one output costs one extra pass, not one per output.

The same control is available over HTTP: `PUT /api/outputs/{uuid}/tonemap` with `{"mode": "AgX"}`,
or `{"mode": null}` to inherit. See the [API reference](13-api.md).

**Tonemap curves are substituted on HDR outputs.** Only Bypass and Reinhard Extended have defined HDR
forms. The others (ACES, AgX, Hable, Uchimura, Lottes, PBR Neutral, Reinhard) have shoulders fitted
against an SDR target and would produce a plausible but wrong picture if stretched. Selecting one on
an HDR output falls back to Bypass, and the card tells you.

**MaxCLL and MaxFALL are declared from your configured peak**, not measured across the recording.
The encoder clamps the signal to that peak so the declaration is true, but it is not the measured
value the standard asks for, and the output card says `MaxCLL/MaxFALL declared from peak`. This is
not a shortcut: FFmpeg cannot rewrite content light level after encode, because the values are
written into the video bitstream by the encoder at the moment it starts.

Varda does measure them anyway, and reports the result when a recording stops:

```
measured content light over 5400 frames: MaxCLL 380 cd/m², MaxFALL 96 cd/m².
```

Use it to pick your peak. A show that only reaches 380 cd/m² declared against a 1000 cd/m² peak
over-declares, and a display will tone-map more conservatively than it needs to. Set the peak near
the measured MaxCLL and the declaration becomes accurate.

HLG sidesteps all of this by carrying no such metadata, so an HLG output shows no line at all.

Recording and network outputs apply the same request to their codec or protocol. See
[10-bit delivery](09-streaming-and-io.md#10-bit-sdr-delivery) for the supported combinations.

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

Outputs, their targets, rotation, output format request, peak luminance, dithering preference,
surface assignments, and warp calibration are saved in `stage.json` (the venue layout), separate from
your scene/show. Existing stages load as 8-bit SDR with dithering enabled. See
[Persistence](02-concepts.md#persistence).

---

[← Prev: Control Surfaces & Macros](06-control-surfaces.md) · [Home](README.md) · [Next: Projection Mapping →](08-projection.md)
