# Outputs

An **output** sends its assigned surfaces, or the master mix, to a window, a fullscreen display, a network stream, a sender, or a recording file. You can run many outputs at once. For surfaces, warp and edge blending see [Projection Mapping](08-projection.md); for stream protocols see [Streaming & I/O](09-streaming-and-io.md).

## Creating an Output

The **+ Output** menu in the output panel lists the types this build supports:

| Type | Sends to |
|------|----------|
| **Window** | A floating window |
| **Display** | Borderless fullscreen on a chosen monitor |
| **Recording** | A video file, through ffmpeg |
| **SRT / HLS / DASH / RTMP** | A network stream (video only) |
| **NDI** | An NDI sender on the network |
| **Syphon** (macOS) / **Spout** (Windows) | Other apps on this computer, as a shared texture |

A new output starts with its type's defaults (a recording writes H.264 to `output.mp4`, an NDI sender is named `Varda NDI`). Edit its settings on its card. The **Type** dropdown changes an existing output to another type and keeps its surfaces, warp, edge blend and format settings.

Every card has:

- the type's settings
- **▶ Start / ⏹ Stop** for recordings, streams and senders (windows always show)
- **Rotation** and **Calibrate**
- surface assignments and edge blending
- **Showing: Stage / Program**, used when no surfaces are assigned. **Stage** shows the whole stage (the default for windows and displays). **Program** shows the master mix over the whole output (the default for everything else).

If this computer or build cannot drive an output (for example Syphon started with `--no-syphon`), the output is kept with its settings and its card says why.

### Choosing a Monitor

A new **Display** output opens nothing until you pick a monitor from its **Monitor** dropdown, which lists connected monitors as `Name (W×H)`. The output then goes fullscreen on that monitor. Switching between **Window** and **Display** keeps the same window.

Monitors are rescanned continuously, so you can plug one in without restarting. If the saved monitor is missing at startup, the output opens as a window and shows `Monitor '<name>' is not connected, so the output opened as a window`. Pick the monitor again once it is connected.

## Output Format

Each output has a format picker. Formats the output cannot deliver are greyed out, and hovering one shows why (for example `Syphon interoperability is limited to BGRA8` or `H.264 streaming is limited to eight-bit SDR`).

- **8-bit SDR**: the default.
- **10-bit SDR**: more code values, same gamut, brightness range and tonemap.
- **HDR10**: PQ (ST 2084) in a BT.2020 container, with mastering metadata.
- **HLG**: relative HDR (ARIB STD-B67), no mastering metadata.
- **EDR (monitor)**: macOS only, for viewing HDR on this Mac. It is not a delivery format.

**HDR10** adds a **Peak** control (600, 1000, 1500 or 4000 cd/m², default 1000): the brightness the top of the range maps to. Linear 1.0 stays at 203 cd/m² reference white (ITU-R BT.2408) at any peak, so existing content keeps its brightness and values above 1.0 become highlights instead of clipping.

**HLG** has no Peak control, because the display decides how bright the top of the range is. Linear 1.0 maps to 75% signal (BT.2408), which gives about 1.92 stops of headroom, against 2.30 for HDR10 at 1000 cd/m².

**Which to use:** HDR10 for files, HLG for live streams unless a CDN requires HDR10. HDR10's content light levels are meant to be measured over a finished program, which a live stream never is. HLG has no such metadata.

**Dither** is on by default. It adds a fixed pattern before the final integer conversion to reduce banding.

### Why a format is greyed out

The available formats depend on the output type and its codec:

- **Streams** (SRT, HLS, DASH, RTMP) default to H.264, which is 8-bit. Set **Codec** to H.265 to unlock 10-bit SDR, HDR10 and HLG.
- **Recordings** need HEVC or AV1 for HDR. Switching a recording to H.264 greys out HDR10.
- **NDI** has no codec setting, so its formats are fixed.
- **Spout** offers 8-bit and 10-bit SDR. **Syphon** offers 8-bit only. Neither offers HDR. See [Spout](09-streaming-and-io.md#spout-windows) and [Syphon](09-streaming-and-io.md#syphon-macos).
- **EDR** is available on display outputs only.

Varda keeps the format you picked. If the output cannot deliver it, the picker shows what it is sending and the card says why (for example `HDR10 fallback: the configured codec is eight-bit`). Change the codec back and your choice applies again.

### Viewing HDR on a Mac

Set a window or display output to **EDR (monitor)** on an EDR display (a recent MacBook Pro or a Pro Display XDR). Values above white show as highlights. Set Peak to the value your recording uses so the two match.

The card shows the headroom, for example `Display headroom 4.2x · monitoring to 4.9x`, and warns when the top of the range clips. Headroom is 1.0 until EDR engages, and it shrinks as display brightness goes up. Lower the brightness for more headroom.

With the default adaptive display preset, EDR shows that the range exists, not how it will look elsewhere. To judge a grade, use a reference preset such as **Apple XDR Display (P3-1600 nits)**. EDR does not check an HDR10 file's metadata; use `ffprobe` or a grading tool for that.

### HDR is range, not wide gamut

Varda composites in linear **Rec.709**. HDR10 output is labeled BT.2020 because the format requires it, but the colors stay within Rec.709. You get more brightness range, not more saturation. Changing the working space would change how every blend mode, shader and saved scene looks.

### HDR support by output

| Output | HDR10 (PQ) | HLG | EDR |
|--------|------------|-----|-----|
| **Recording** | HEVC and AV1. Checked with `ffprobe`: PQ transfer, BT.2020 primaries, ST 2086 mastering display, CTA-861.3 content light level. | Not yet | No |
| **Streaming** | SRT, HLS, DASH and Enhanced RTMP with HEVC or AV1. Legacy RTMP cannot. | Same protocols | No |
| **Display** | When the GPU offers RGB10A2 with BT.2100 PQ. Experimental: not yet tested against capture hardware or an LED processor. | Not on Windows (DX12 has no HLG surface) | macOS, on an EDR display |
| **NDI, Syphon, Spout** | No | No | No |
| **ProRes, H.264, HAP** | No. HAP is always 8-bit. | No | No |

An output that cannot deliver the requested format sends the best it can and names the reason, for example `H.264 cannot carry HDR10; HEVC or AV1 is required`. See [HDR Delivery](09-streaming-and-io.md#hdr-delivery) for streaming and for checking a stream.

### LUTs on HDR outputs

There are two LUT slots. The **Look LUT** applies before the tonemap, in scene-linear light, and reaches every output. The **Calibration LUT** applies after the tonemap and is built for SDR, so HDR outputs skip it and the card says so. Put the show's look in the Look slot and a display's correction in the Calibration slot.

### Per-output tonemap

Each card has a **Tonemap** picker. The default, `Show (<curve>)`, uses the curve set in the 🎨 Tonemap panel. Pick another curve to change this output only, or **Show default** to go back. Outputs using the same curve share one pass.

Over HTTP: `PUT /api/outputs/{uuid}/tonemap` with `{"mode": "AgX"}`, or `{"mode": null}` to use the show curve. See the [API reference](13-api.md).

On HDR outputs only **Bypass** and **Reinhard Extended** apply. The other curves (ACES, AgX, Hable, Uchimura, Lottes, PBR Neutral, Reinhard) are fitted for SDR, so an HDR output uses Bypass instead and the card says so.

### MaxCLL and MaxFALL

HDR10 recordings declare MaxCLL and MaxFALL from the Peak setting, and the encoder clamps the signal to that peak so the values hold. The card shows `MaxCLL/MaxFALL declared from peak`. They cannot be measured first, because the encoder writes them into the bitstream when it starts.

Varda measures them during the recording and reports the result when it stops:

```
measured content light over 5400 frames: MaxCLL 380 cd/m², MaxFALL 96 cd/m².
```

Set Peak close to the measured MaxCLL for the next recording. Declaring 1000 cd/m² for a show that peaks at 380 makes displays tone-map more than needed. HLG outputs carry no such metadata and show no line.

Recordings and streams apply the chosen format to their codec. See [10-bit delivery](09-streaming-and-io.md#10-bit-sdr-delivery) for supported combinations.

## Rotation

**Rotation** is 0°, 90°, 180° or 270°, applied in the final pass. 90° and 270° swap width and height, for portrait projectors and displays.

## Surface Sources

An output draws the **surfaces** assigned to it. Each surface's **Source** is set in the Stage Editor:

| Source | Content |
|--------|---------|
| **Master** | The final mix, with master effects |
| **Channel** | One channel, after its effects |
| **Channels** | Several channels mixed together (see below) |
| **Deck** | One deck, before blending and effects |
| **Domemaster** | A fisheye (equidistant azimuthal) projection |

Add a surface to an output with **+ Assign Surface**. Remove it with **x**; reset its warp with **↺**.

### Channels sub-mix

Tick several channels in the source picker to mix them using each channel's opacity and blend mode, without master effects. One surface can show channels 1 and 2 crossfading while another shows the master. Ticking one channel gives **Channel**; ticking none gives **Master**.

## Recording

**+ Output → Recording** adds a recording. Each recording runs its own ffmpeg process, so several can record at once. See [Recording](09-streaming-and-io.md#recording) for codecs and file paths.

Recordings and streams can include audio from a capture device, chosen in the output's **Audio** dropdown. See [Audio Passthrough](09-streaming-and-io.md#audio-passthrough).

## Saving

Outputs are saved in `stage.json` (the venue layout), separate from the scene: their type and settings, rotation, format, peak, dither, surface assignments and warp. Stages saved before formats existed load as 8-bit SDR with dither on. See [The Varda workspace](02-concepts.md#the-varda-workspace).

---

[← Prev: Control Surfaces & Macros](06-control-surfaces.md) · [Home](README.md) · [Next: Projection Mapping →](08-projection.md)
