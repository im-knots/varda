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

Input streams include stall detection (5-second timeout) and auto-reconnect on failure.

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
| HAP Q | Higher quality HAP (BC7 compression) |

### Usage

1. In the output panel, create a recording output or use an existing output
2. Select a codec and output path
3. Start/stop recording with the inline controls

Recording uses non-blocking frame delivery — if the encoder can't keep up, frames are silently dropped rather than stalling the render thread.

---

## RTMP / RTMPS

### Output (Streaming to Platforms)

Push video directly to Twitch, YouTube, Kick, or any RTMP/RTMPS ingest endpoint — no OBS relay needed.

1. Click **"+ Stream"** → select **RTMP**
2. Enter the ingest URL (e.g., `rtmp://live.twitch.tv/app/<stream-key>` or `rtmps://a.rtmps.youtube.com/live2/<stream-key>`)
3. Choose a codec: **H.264**, **H.265**, or **AV1** (H.265 and AV1 via Enhanced RTMP)
4. Start the output

Varda uses FLV muxing with auto-scaled CBR bitrate and 2-second keyframe intervals. Frame delivery is non-blocking.

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

## Headless Mode

All I/O features work identically in headless mode — the engine renders without any UI window, controlled entirely via the [HTTP API](api.md).

```sh
varda --headless --port 8080 --fps 60
```

Outputs defined in `stage.json` auto-start on launch: NDI sends, SRT streams, HLS/DASH outputs, recordings, and display outputs (fullscreen on connected monitors) all activate automatically.

This enables the installation use case: configure in windowed mode, save, deploy headless.
