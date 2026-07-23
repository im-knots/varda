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
| ProRes 4444 | Professional edit-ready with alpha channel |
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

## HTML / Web Content

Render live web pages — dashboards, SVG/Canvas/WebGL, lyric and lower-third overlays, animated HTML/CSS — as a deck source. Pages are rendered by an embedded [Servo](https://servo.org) browser engine and composite alongside every other source.

### Usage

1. In the Library, open the **🌐 HTML Sources** section and click **+ Add HTML**
2. Enter a source in the **URL:** field:
   - a remote URL — `https://example.com/overlay.html`
   - a local file — `file:///Users/you/show/lyrics.html`
   - an inline document — `data:text/html,<h1>Hello</h1>`
3. Click **✓ Add**, then **drag** the entry onto a channel to create a live HTML deck

You can also add one directly over the HTTP API:

```sh
curl -X POST http://localhost:8080/api/channels/0/decks/html \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/overlay.html"}'
```

HTML decks are persisted in `scene.json` by URL and reload automatically. The source is **render-only** in this release — mouse, scroll, and keyboard input are not forwarded into the page, and engine state (audio/clock/modulation) is not yet exposed to page JavaScript.

### Rendering & performance

HTML is rasterized on the **CPU** (Servo's software renderer) and uploaded to a GPU texture each frame. It is heavier than the GPU-native deck types.

> **Platform support.** HTML decks are available on **Apple Silicon macOS** (arm64) and **Linux** (x86_64). They are **not** available on Intel (x86_64) macOS: Servo deck-creation hangs under Rosetta, so the macOS DMG ships HTML in the Apple Silicon slice only. It comfortably handles HTML/CSS/SVG, dashboards, and text overlays; heavy WebGL or full-screen Canvas animation at high resolution may not sustain 60 fps. Profile your pages with the `html_render` benchmark (see [Benchmarking](../CONTRIBUTING.md#benchmarking)) if frame rate matters.

> **Non-blocking, like the stream sources.** HTML rendering runs on a dedicated background thread (a shared Servo engine), the same way NDI/SRT/HLS/DASH/RTMP decode off the render loop. Finished frames are handed to the render thread and uploaded without blocking, so even a heavy page can't stall the 60 fps loop — it simply updates at whatever rate it can render.

> **Feature flag.** HTML decks require the `html` build feature, which is **on by default**. Disable rendering for a session with `--no-html`, or build without it via `--no-default-features`.

### Transparency

HTML decks can keep their transparent regions transparent so the page composites over lower channels and passes through to alpha-capable outputs. Enable the **Transparent BG** toggle in the HTML deck's detail panel (off by default). With it on, anything the page doesn't paint (or paints with `alpha < 1`) stays see-through instead of being filled with black.

> **Where transparency reaches.** The on-screen display is always opaque — the program is composited over black, so a transparent deck shows through to whatever is behind it in the channel stack, not through the app window. Alpha reaches **Syphon** and alpha-capable **recording** codecs (**ProRes 4444**, **HAP Alpha** — see [Recording](#recording)). **NDI** output stays opaque.

> **Default background is now black.** The embedded browser's page background is transparent globally. For a deck with **Transparent BG off** (the default), the page is flattened over black, so a page that set no `html`/`body` background — and previously relied on the browser's implicit **white** — now shows **black**. Set an explicit background in your page's CSS (e.g. `body { background: white; }`) to control it. Pages that already set a background are unaffected.

> **Known limitations.**
> - With multiple visible channels where a *lower* channel is transparent, colors in overlapping **semi-transparent** regions are approximate. A single transparent channel, or a transparent program sent to Syphon/recording, is exact.
> - An active 2-channel transition **shader** won't carry alpha unless the shader itself preserves it.

---

## Syphon (macOS)

Syphon enables inter-application GPU texture sharing on macOS. Varda works both ways: as a Syphon **client** (receiving other apps' frames as live sources) and as a Syphon **server** (publishing a Varda output for other apps to consume).

**Receive (client):**

1. Open the Library and look under **Syphon Sources** for discovered servers
2. **Drag** a server into a channel to create a live deck

Frames are pulled per-frame from the Syphon server's `MTLTexture` and uploaded into Varda's wgpu texture path via CPU readback — a cheap same-memory copy on Apple-silicon unified memory. A zero-copy receive path (wrapping the IOSurface texture directly as a `wgpu::Texture`) is a possible follow-on.

**Publish (server):**

1. In the output panel, click **+ Stream**
2. Select **Syphon** from the protocol dropdown
3. Enter a server name (e.g., "Varda Main")
4. Start the output — other Syphon apps then see it in their source list

Publishing is **zero-copy on the GPU**: the rendered output is converted (RGBA→BGRA) by a GPU pass and shared directly via Metal — no CPU readback.

### Installing Syphon.framework

Syphon support needs **nothing special at build time** — `Syphon.framework` is *not* linked, it is loaded at runtime via `dlopen`. A normal macOS build (`cargo build` / `cargo run`) works whether or not Syphon is installed; if it is missing, Syphon features simply stay disabled and the rest of Varda runs normally.

To *use* Syphon, install the framework system-wide at:

```
/Library/Frameworks/Syphon.framework
```

This is the standard, verified location — it is also where other Syphon apps on the system expect to find the framework, so a single system-wide install serves all of them. To install it:

1. Get `Syphon.framework` — download it from the [official Syphon-Framework releases](https://github.com/Syphon/Syphon-Framework/releases), or copy it out of any Syphon-enabled app bundle (e.g. Simple Syphon, Resolume, VDMX, MadMapper).
2. Copy the `Syphon.framework` folder into `/Library/Frameworks/` (requires admin):
   ```sh
   sudo cp -R /path/to/Syphon.framework /Library/Frameworks/
   ```
3. Launch Varda. On startup the log shows `Syphon.framework found` when it loaded successfully, or `Syphon.framework not found — Syphon features disabled` otherwise.

> Varda also checks `~/Library/Frameworks/Syphon.framework` (per-user, no admin) as a fallback. The system-wide `/Library/Frameworks/` path above is the recommended and verified one.

Pass `--no-syphon` to disable Syphon explicitly even when the framework is installed.

> **Note:** Varda is both a Syphon client (receive) and a Syphon server (publish a `SyphonServer` output).

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
