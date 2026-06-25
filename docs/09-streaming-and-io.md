# Streaming, Recording & Network I/O

## NDI

### Sending

Each output can send video over NDI to other applications and machines on the network.

1. In the output panel, click **"+ Stream"**
2. Select **NDI** from the protocol dropdown
3. Enter a sender name (e.g., "Varda Main")
4. The NDI stream is discoverable by any NDI-compatible application on the LAN

### Receiving

1. In the Library panel, open the **📡 NDI Sources** section
2. Click **Rescan** to discover NDI sources on the network
3. **Drag** a source into a channel — it becomes a live deck source

NDI uses dynamic SDK loading (`libloading`) — the NDI SDK does not need to be present at compile time. If the SDK is not installed, NDI features are gracefully unavailable.

---

## SRT (Secure Reliable Transport)

### Output (Streaming)

SRT output uses **listener mode** — Varda acts as an SRT server that clients connect to.

1. Click **"+ Stream"** → select **SRT**
2. Enter a URL (default: `srt://0.0.0.0:9001?mode=listener`)
3. Start the output — Varda begins listening for SRT clients

When a client disconnects, the SRT listener automatically restarts so new clients can connect. Frame delivery is non-blocking — the render thread continues at full speed even when no client is connected.

### Input (Receiving)

SRT input uses **caller mode** — Varda connects to a remote SRT listener.

1. In the Library, open the **📺 SRT Sources** section
2. Add a URL (e.g., `srt://192.168.1.50:9001`)
3. **Drag** the source into a channel to create a live deck

SRT input supports receiver deduplication — the same URL used by multiple decks shares a single connection.

> **Note:** Requires ffmpeg built with `--enable-libsrt` for native SRT protocol support.

---

## HLS & DASH

### Output

1. Click **"+ Stream"** → select **HLS** or **DASH**
2. Choose a codec: **H.264**, **H.265**, or **AV1**
3. For HLS, optionally enable **Low Latency** (LL-HLS) for 2–5 second end-to-end latency
4. Start the output

Varda writes segments and manifests to `.varda/streams/<name>/` and serves them via the built-in HTTP server:

```
http://<your-ip>:8080/streams/<name>/playlist.m3u8   (HLS)
http://<your-ip>:8080/streams/<name>/manifest.mpd     (DASH)
http://<your-ip>:8080/streams/<name>/player.html      (auto-generated HTML5 player)
```

Click any URL in the output panel to **copy it to the clipboard**.

The auto-generated `player.html` uses hls.js or dash.js and works in any modern browser — share the URL with anyone on your network.

| Mode | Latency | Use Case |
|------|---------|----------|
| Standard HLS | 15–25s | Reliable delivery, CDN-friendly |
| LL-HLS | 2–5s | Near-real-time web viewing |
| DASH | 10–20s | Cross-platform, multi-codec |

### Input

1. In the Library, open **📡 HLS Sources** or **📡 DASH Sources**
2. Add a stream URL (`.m3u8` for HLS, `.mpd` for DASH)
3. **Drag** into a channel to create a live deck

Input streams include stall detection and auto-reconnect on failure (see [Stream Input Reliability](#stream-input-reliability)).

---

## Recording

Each output can record to a video file independently. Multiple simultaneous recordings to different files are supported.

### Codecs

| Codec | Use Case |
|-------|----------|
| H.264 | Quick recording, small files |
| H.265 | Better compression, smaller files |
| AV1 | Best compression, slower encoding |
| ProRes 422 | Professional edit-ready |
| HAP | VJ content re-use, GPU-native playback |
| HAP Alpha | HAP with alpha channel |
| HAP Q | Higher quality HAP (YCoCg compression) |

### Usage

1. In the output panel, click **+ Recording** to create a recording output (repeat for each simultaneous recording — each runs its own ffmpeg subprocess).
2. Set the **File:** path (plain text input; default `output.mp4`, relative to the working directory). Paths are literal — there is **no automatic timestamping**, so give each recording a distinct name.
3. Pick a **Codec:** from the table above.
4. Click **▶ Start** to begin; the button becomes **⏹ Stop** and a red elapsed-time counter shows while recording.

Each recording starts and stops independently, and ffmpeg writes directly to the path you specify. Recording uses non-blocking frame delivery — if the encoder can't keep up, frames are silently dropped rather than stalling the render thread.

> **Add audio with passthrough.** To include sound, pick a device in the output's **Audio:** dropdown — see [Audio Passthrough](#audio-passthrough) below.

---

## Audio Passthrough

Every ffmpeg-backed output (Recording, SRT, HLS, DASH, RTMP) can mux audio from a capture device alongside the video. This is the **same physical device** that drives Varda's modulation engine — one device feeds analysis, the live monitor, and every output at once, all off one hardware clock so audio and visuals stay in sync.

### Selecting a device

1. Configure an ffmpeg output (Recording or any streaming target) and leave it **stopped**.
2. In the output's **Audio:** dropdown, pick a capture device, or **None (silent)** for video-only (the default).
3. Click **▶ Start**. The output now carries that device's audio.

While the output is active, a small readout shows the selected device with live `sent` / `dropped` chunk counts. A non-zero drop count (shown amber) means the encoder briefly couldn't keep up — audio is never allowed to stall the real-time capture thread, so the oldest backlog is dropped instead.

### What you get

- **Recording** muxes AAC at the device's **native sample rate** for faithful, edit-ready captures.
- **Streaming targets** (SRT, HLS, DASH, RTMP) normalize to **48 kHz AAC** for platform compatibility (Twitch/YouTube expect 48k).
- Audio is **downmixed to stereo** and kept in sync (±~1 frame) via asynchronous resampling — good for live-set recordings and streams.

### Graceful fallback

If a scene selects a device that isn't present at load (unplugged, renamed), the output starts **video-only** and a notification explains why. A missing microphone never blocks the visual recording or stream.

### Co-equal control

The selected device persists in your scene and is available from the GUI dropdown, the HTTP API (the `audio_device` field on an output target), and any scene authored elsewhere — it replays with audio on load. The API is a co-equal consumer in both directions: set the device on create/update, and **read it back** — `GET /api/state/outputs` (and `/api/stage/outputs`) returns each output's full `target` plus an `audio_passthrough` block (`device`, `frames_written`, `frames_dropped`) with the same live health the GUI card shows. List the available capture devices via `GET /api/state/audio`.

> **Not a DJ tool.** Audio passthrough is a clean one-device passthrough for delivery; there is no audio-file playback, mixing, or per-output gain. Audio reactivity is driven by the [modulation system](05-modulation.md).

---

## RTMP / RTMPS

### Output (Streaming to Platforms)

Push video directly to Twitch, YouTube, Kick, or any RTMP/RTMPS ingest endpoint — no OBS relay needed.

1. Click **"+ Stream"** → select **RTMP**
2. Enter the ingest URL (e.g., `rtmp://live.twitch.tv/app/<stream-key>` or `rtmps://a.rtmps.youtube.com/live2/<stream-key>`)
3. Choose a codec: **H.264**, **H.265**, or **AV1** (H.265 and AV1 via Enhanced RTMP)
4. Start the output

Varda uses FLV muxing with auto-scaled CBR bitrate and 2-second keyframe intervals. Frame delivery is non-blocking.

> **Stream keys are credentials.** An ingest URL contains your platform stream key. Treat it as a password — avoid screen-sharing or recording your screen while the RTMP output field is visible, and never paste full ingest URLs into bug reports.

### Input (Receiving RTMP Streams)

RTMP input supports two modes:

**Pull mode** — connect to a remote RTMP stream:

1. In the Library, open **📡 RTMP Sources** (under Stream Sources)
2. Add a stream URL (e.g., `rtmp://192.168.1.50/live/stream`)
3. **Drag** into a channel to create a live deck

**Listen mode** — accept pushes from OBS, vMix, or other RTMP senders:

1. In the Library, add an RTMP source and select **Listen** mode
2. Varda generates a listen URL (starting at port 1935, incrementing for additional listeners)
3. Configure OBS or other software to push to the generated URL
4. **Drag** the source into a channel

Stream sources are grouped under a single **Stream Sources** header in the Library panel. All stream source types (NDI, SRT, HLS, DASH, RTMP) share the same drag-to-channel workflow.

---

## Syphon (macOS)

Syphon enables inter-application GPU texture sharing on macOS. Varda includes framework detection and server discovery.

> **Note:** The runtime IOSurface↔wgpu Metal bridge is pending implementation. Syphon sources appear in the library but full texture sharing is not yet functional.

---

## Stream Input Reliability

All stream **input** protocols — SRT, HLS, DASH, and RTMP — share the same resilience layer:

- **Deduplication** — the same URL used by multiple decks shares one underlying connection, so adding a stream to several channels costs a single receive.
- **Stall detection** — if no frames arrive for a timeout window, the receiver reconnects. The window is **5 s** for the live protocols (SRT, RTMP) and **15 s** for segment protocols (HLS, DASH), which buffer in larger chunks.
- **Auto-reconnect** — on failure or stall, the receiver retries with **exponential backoff** (500 ms up to 10 s) until the source returns.

---

## Headless Mode

All streaming, recording, and network I/O features work identically in headless mode — outputs defined in `stage.json` auto-start on launch. See [HTTP API & Headless Mode](13-api.md#headless-mode).

---

[← Prev: Projection Mapping](08-projection.md) · [Home](README.md) · [Next: Resolution, Settings & Monitoring →](10-resolution-and-monitoring.md)
