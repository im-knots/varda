# Streaming, Recording & Network I/O

## 10-bit SDR Delivery

Set **SDR precision** to **10-bit SDR** on an output to request a high-precision codec or transport.
Varda probes the installed GPU, FFmpeg encoders, NDI runtime, and endpoint contract before starting.
If the complete path is unavailable, it keeps the request, delivers 8-bit SDR, and reports why.

| Destination | 10-bit path | Explicit fallback |
|---|---|---|
| Recording | HEVC Main10, AV1 10-bit, ProRes 422, ProRes 4444 | H.264 and HAP remain 8-bit |
| NDI send | NDI 6 P216 | Older or incomplete runtimes use UYVY |
| SRT | HEVC Main10 in MPEG-TS | H.264 remains 8-bit |
| HLS / DASH | HEVC Main10 or AV1 10-bit in fMP4 | Unsupported encoders use 8-bit |
| RTMP / RTMPS | HEVC Main10 or AV1 with an Enhanced endpoint contract | Legacy RTMP uses H.264 8-bit |
| Syphon | No interoperable 10-bit path | BGRA8 |

Ten-bit delivery is still SDR. It preserves more code values after Varda's existing tonemap and LUT;
it does not enable HDR metadata or extended brightness.

## NDI

### Sending

Each output can send video over NDI to other applications and machines on the network.

1. In the output panel, click **"+ Stream"**
2. Select **NDI** from the protocol dropdown
3. Enter a sender name (e.g., "Varda Main")
4. The NDI stream is discoverable by any NDI-compatible application on the LAN

With a loaded NDI 6 runtime, a 10-bit request sends P216 with Rec.709 limited-range conversion.
Older runtimes use UYVY and show a fallback reason. Receiver software should request its best or
highest-quality color mode to avoid converting P216 back to 8-bit.

### Receiving

1. In the Library panel, open the **📡 NDI Sources** section
2. Click **Rescan** to discover NDI sources on the network
3. **Drag** a source into a channel and it becomes a live deck source

NDI uses dynamic SDK loading (`libloading`). If the SDK is not installed, NDI features are gracefully unavailable.

---

## SRT (Secure Reliable Transport)

### Output (Streaming)

SRT output uses **listener mode**. Varda acts as an SRT server that clients connect to.

1. Click **"+ Stream"** → select **SRT**
2. Enter a URL (default: `srt://0.0.0.0:9001?mode=listener`)
3. Start the output — Varda begins listening for SRT clients

When a client disconnects, the SRT listener automatically restarts so new clients can connect. Frame delivery is non-blocking.

### Input (Receiving)

SRT input uses **caller mode**. Varda connects to a remote SRT listener.

1. In the Library, open the **📺 SRT Sources** section
2. Add a URL (e.g., `srt://192.168.1.50:9001`)
3. **Drag** the source into a channel to create a live deck

SRT input supports receiver deduplication such that the same URL used by multiple decks shares a single connection.

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

1. In the output panel, click **+ Recording** to create a recording output (repeat for each simultaneous recording, each runs its own ffmpeg subprocess).
2. Set the **File:** path (plain text input; default `output.mp4`, relative to the working directory). Paths are literal there is **no automatic timestamping**, so give each recording a distinct name.
3. Pick a **Codec:** from the table above.
4. Click **▶ Start** to begin; the button becomes **⏹ Stop** and a red elapsed-time counter shows while recording.

Each recording starts and stops independently, and ffmpeg writes directly to the path you specify. Recording uses non-blocking frame delivery such that if the encoder can't keep up, frames are dropped rather than stalling the render thread, and the previous frame is repeated in the file so the recording keeps constant frame rate and stays the right length.

> **Add audio with passthrough.** To include sound, pick a device in the output's **Audio:** dropdown — see [Audio Passthrough](#audio-passthrough) below.

---

## Audio Passthrough

Every ffmpeg-backed output (Recording, SRT, HLS, DASH, RTMP) can mux audio from a capture device alongside the video. This is the **same physical device** that drives Varda's modulation engine. One device feeds analysis, the live monitor, and every output at once, all off one hardware clock so audio and visuals stay in sync.

### Selecting a device

1. Configure an ffmpeg output (Recording or any streaming target) and leave it **stopped**.
2. In the output's **Audio:** dropdown, pick a capture device, or **None (silent)** for video-only (the default).
3. Click **▶ Start**. The output now carries that device's audio.


### What you get

- **Recording** muxes AAC at the device's **native sample rate** for faithful, edit-ready captures.
- **Streaming targets** (SRT, HLS, DASH, RTMP) normalize to **48 kHz AAC** for platform compatibility (Twitch/YouTube expect 48k).
- Audio is **downmixed to stereo**.
- **Sync holds even when the renderer stumbles.** Timing comes from the capture device's own sample clock, which runs at a steady rate no matter what the GPU is doing. 

### Graceful fallback

If a scene selects a device that isn't present at load (unplugged, renamed), the output starts **video-only** and a notification explains why. A missing microphone never blocks the visual recording or stream.


> **Not a DJ tool.** Audio passthrough is a clean one-device passthrough for delivery; there is no audio-file playback, mixing, or per-output gain. Audio reactivity is driven by the [modulation system](05-modulation.md).

---

## RTMP / RTMPS

### Output (Streaming to Platforms)

Push video directly to Twitch, YouTube, Kick, or any RTMP/RTMPS ingest endpoint.

1. Click **"+ Stream"** → select **RTMP**
2. Enter the ingest URL (e.g., `rtmp://live.twitch.tv/app/<stream-key>` or `rtmps://a.rtmps.youtube.com/live2/<stream-key>`)
3. Choose a codec: **H.264**, **H.265**, or **AV1** (H.265 and AV1 via Enhanced RTMP)
4. Choose **Enhanced** only when the endpoint explicitly accepts Enhanced RTMP signaling
5. Start the output

Varda uses FLV muxing with auto-scaled CBR bitrate and 2-second keyframe intervals. Frame delivery is non-blocking.

Legacy RTMP always resolves to H.264 8-bit. A 10-bit HEVC or AV1 request requires the persisted
Enhanced endpoint contract and compatible FFmpeg muxer support.

> **Stream keys are credentials.** An ingest URL contains your platform stream key. Treat it as a password. Avoid screen-sharing or recording your screen while the RTMP output field is visible, and never paste full ingest URLs into bug reports.

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

Render live web pages such as dashboards, SVG/Canvas/WebGL, lyric and lower-third overlays, animated HTML/CSS as a deck source. Pages are rendered by an embedded [Servo](https://servo.org) browser engine and composite alongside every other source.

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

HTML decks are persisted in `scene.json` by URL and reload automatically.

### Rendering & performance

HTML is rasterized on the **CPU** (Servo's software renderer) and uploaded to a GPU texture each frame. It is heavier than the GPU-native deck types.

> **Platform support.** HTML decks are available on **Apple Silicon macOS** (arm64) and **Linux** (x86_64). They are **not** available on Intel (x86_64) macOS: Servo deck-creation hangs under Rosetta, so the macOS DMG ships HTML in the Apple Silicon slice only. It comfortably handles HTML/CSS/SVG, dashboards, and text overlays; heavy WebGL or full-screen Canvas animation at high resolution may not sustain 60 fps. Profile your pages with the `html_render` benchmark (see [Benchmarking](../CONTRIBUTING.md#benchmarking)) if frame rate matters.

> **Non-blocking, like the stream sources.** HTML rendering runs on a dedicated background thread (a shared Servo engine), the same way NDI/SRT/HLS/DASH/RTMP decode off the render loop. Finished frames are handed to the render thread and uploaded without blocking.

> **Feature flag.** HTML decks require the `html` build feature, which is **on by default**. Disable rendering for a session with `--no-html`, or build without it via `--no-default-features`.

---

## Syphon (macOS)

Syphon enables inter-application GPU texture sharing on macOS. Varda works both ways: as a Syphon **client** (receiving other apps' frames as live sources) and as a Syphon **server** (publishing a Varda output for other apps to consume).

**Receive (client):**

1. Open the Library and look under **Syphon Sources** for discovered servers
2. **Drag** a server into a channel to create a live deck

**Publish (server):**

1. In the output panel, click **+ Stream**
2. Select **Syphon** from the protocol dropdown
3. Enter a server name (e.g., "Varda Main")
4. Start the output — other Syphon apps then see it in their source list

### Installing Syphon.framework

Syphon support needs **nothing special at build time** — `Syphon.framework` is *not* linked, it is loaded at runtime via `dlopen`. A normal macOS build (`cargo build` / `cargo run`) works whether or not Syphon is installed; if it is missing, Syphon features simply stay disabled and the rest of Varda runs normally.

To *use* Syphon, install the framework system-wide at:

```
/Library/Frameworks/Syphon.framework
```

This is the standard, verified location. It is also where other Syphon apps on the system expect to find the framework, so a single system-wide install serves all of them. To install it:

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

## Screen & Window Capture

Capture an OS display or a single application window as a live deck source. Anything on screen can become a layer: a browser, a game, a DAW, a slide deck, another VJ app, or Varda itself. Unlike Syphon, the source application needs no support for it and no plugin.

### Platform support

| Platform | How it works | What to know |
|---|---|---|
| **macOS** 12.3+ | [ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit) | Needs the Screen Recording permission, see below. Varda excludes its own windows from a display capture, and the display is scaled down before it leaves the system, so a 4K screen costs no more than a 1080p one |
| **Windows** 10 (1903+) | [Windows.Graphics.Capture](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture) | No permission prompt. Windows draws a yellow capture border around the captured surface on some builds, which Varda suppresses where the OS allows it |
| **Linux, Wayland** | XDG Desktop Portal plus PipeWire | Your compositor owns the picker. See the note below |
| **Linux, X11** | `GetImage` polling | Works everywhere, but Varda cannot exclude itself from a display capture, and the cursor is never included |

Varda picks the Linux backend at startup based on the session you are logged into, so one build serves both. A session running under XWayland uses the portal, not X11, because X11 can only see other XWayland clients there.

**On Wayland the Library shows a single entry, "Pick a window or display…", instead of a list.** Drag it onto a channel and your desktop's own share dialog opens so you can choose what to capture. On Wayland the compositor decides what an application may see. Each new capture opens the dialog again, including when you load a scene that contains one.

### Usage

1. In the Library, open the **🖥 Screen Capture** section and click **🔄 Rescan**
2. Targets are grouped under **Displays** and **Windows**. Hovering a row shows its pixel size
3. **Drag** a target onto a channel to create a live capture deck

The list is only refreshed when you press Rescan. Window lists churn constantly as you switch apps, so Varda never polls them in the background.

Varda's own windows are labeled `(Varda)` and tinted, so capturing yourself is an informed choice rather than an accidental mirror.

### Permissions (macOS)

Screen capture is gated by the system Screen Recording privacy control. The Library section reflects the current state:

- **Not yet determined.** A **Grant Screen Recording access** button appears. Clicking it asks macOS to prompt you.
- **Denied.** The panel points you at **System Settings → Privacy & Security → Screen Recording**.

In both cases **you must restart Varda after granting**. macOS does not apply a new grant to an already running process, so a user who approves the prompt and sees nothing change would otherwise conclude the feature is broken. Varda never requests the permission at startup, and never requests it at all when capture is disabled.

Windows and Linux have no equivalent gate, so the Library shows no permission banner there. On Wayland the portal dialog is the gate, and it appears when you drag a capture onto a channel rather than ahead of time.

### Deck controls

Select a capture deck to get its controls in the deck detail panel (bottom bar):

| Control | Parameter path | Notes |
|---|---|---|
| **Rate** | `deck/<deck_uuid>/capture/rate` | 1 to 120 fps. Defaults to 30 |
| **Crop** X / Y / W / H | `deck/<deck_uuid>/capture/crop_x` (`_y`, `_w`, `_h`) | Normalized 0 to 1 within the target, with a **Reset** button |
| **Cursor** | `deck/<deck_uuid>/capture/cursor` | Include the mouse pointer. Fixed when the capture opens on Wayland, and not available on X11 |
| **Exclude Varda** | `deck/<deck_uuid>/capture/exclude_varda` | Omit Varda's own windows. Offered for display targets only, since for a window target it would do nothing. macOS only, see below |

Every one of these is a real parameter path, so all of them are MIDI-learnable, OSC-addressable, and modulatable like any shader input. See [Parameter Paths](06-control-surfaces.md#parameter-paths).

### Capturing Varda itself

Pointing a capture at one of Varda's own windows is supported, and is the straightforward way to record Varda for content. Be aware of what it implies: Varda's UI window contains deck and channel previews, so a capture of that window contains a preview of itself, nested. That is a genuine video feedback loop, and it is often the effect you want.

If you want a clean recording of the program without the interface nested inside it, you have better options than capturing the UI window. Use a [Program Tap](#program-tap) to read the program directly, capture a windowed [output](07-outputs.md) rather than the main window, or use a [recording output](#recording).

For a full-display capture, **Exclude Varda** is on by default so that pointing a deck at your main monitor does not produce an infinite mirror.

**On Linux under X11 this cannot be honoured.** X11 offers no way to leave one application out of a screen capture, so a display capture always includes Varda. The mirror stays stable rather than running away, because the capture rate sits below the render rate, but if you want a clean desktop capture on X11 you need to move Varda to another monitor or capture individual windows instead. Wayland does not have this problem, because you pick the target in your compositor's dialog.

### Persistence

Capture decks are saved in `scene.json` by **name**: a display's name, or a window's application and title. Platform handles such as CoreGraphics display ids and window numbers are ephemeral across reboots and are never written to disk.

If the target is missing when a scene loads, the deck is **restored unbound** rather than dropped. It renders black and logs a warning, and the deck detail panel tells you so. Losing a deck along with its effect chain, opacity, and MIDI mappings because an app happened to be closed would be a bad way to start a show, so the deck survives and you repoint it.

### API

```sh
# Refresh the target list, then read it
curl -X POST http://localhost:8080/api/devices/screen/scan
curl http://localhost:8080/api/library/screen

# Capture a whole display
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks/screen \
  -H "Content-Type: application/json" \
  -d '{"target": {"kind": "display", "name": "Built-in Retina Display"}}'

# Capture one window, cropped to its top-left quadrant at 24 fps
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks/screen \
  -H "Content-Type: application/json" \
  -d '{"target": {"kind": "window", "app": "Safari", "title": "Dashboard"},
       "rate": 24,
       "crop": {"x": 0.0, "y": 0.0, "w": 0.5, "h": 0.5},
       "show_cursor": true}'
```

`GET /api/state/screen_capture` reports the permission state, the backend in use, whether capture is available at all, and the number of live sessions.

> **Feature flag.** Screen capture requires the `screen-capture` build feature, which is **on by default**. Disable it for a session with `--no-screen-capture`, which skips OS capture entirely so no Screen Recording permission is ever requested, or build without it via `--no-default-features`.

---

## Program Tap

A **tap** re-enters Varda's own output as a deck source. The deck reads the program out of GPU memory. This is how you build video feedback, picture-in-picture of the program, and effect chains that process the whole mix.

There are two tap points:

| Tap | Reads |
|---|---|
| **Master Program** | The full mix, **before** tonemap and LUT |
| **Channel** | One channel's composite, after its own effect chain |

### One frame behind

A tap always shows the **previous** frame, never the current one. It means a tap can never read a texture that is still being written, and it means the delay does not depend on where the tapping deck sits in the mixer. Move the deck to another channel, reorder the channels, add more taps, and the latency stays at exactly one frame.

Master taps read before tonemapping on purpose. A feedback loop that read the tonemapped output and fed it back into the linear pipeline would apply the transfer curve again on every trip around the loop, and the image would crush toward the roll-off within a second or two.

### Usage

1. In the Library, open the **🔁 Taps** section
2. **Drag** either **Master Program** or a channel onto a channel to create a tap deck

The list is built from the live channel list rather than a device scan, so there is nothing to rescan. Select a tap deck to get a **Source** dropdown in the deck detail panel, which repoints it between Master Program and any channel without recreating the deck.

### Feedback

Tapping the master from a deck that is itself part of the master is a deliberate feedback loop, and the one frame of delay is what keeps it stable rather than deadlocked. Two things are worth knowing before you turn one up:

- **Gain above unity grows without bound.** A tap deck at opacity above 1.0 on an additive blend multiplies its own output every frame. The tonemap rolls the result off, it does not clamp it, so the image will bloom to white and stay there.
- **Add a transform to see anything interesting.** A tap composited exactly over its own source just reproduces the image. Scale, rotate, offset, or run it through an effect chain, and you get the tunnels and trails that make the technique worth using.


### API

```sh
# Tap the master program
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks/tap \
  -H "Content-Type: application/json" \
  -d '{"source": {"kind": "master_program"}}'

# Tap another channel
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks/tap \
  -H "Content-Type: application/json" \
  -d '{"source": {"kind": "channel", "uuid": "<other_ch_uuid>"}}'

# Repoint an existing tap deck
curl -X PUT http://localhost:8080/api/decks/<deck_uuid>/tap/source \
  -H "Content-Type: application/json" \
  -d '{"source": {"kind": "master_program"}}'
```

---

## Stream Input Reliability

All stream **input** protocols (SRT, HLS, DASH, and RTMP) share the same resilience layer:

- **Deduplication** — the same URL used by multiple decks shares one underlying connection, so adding a stream to several channels costs a single receive.
- **Stall detection** — if no frames arrive for a timeout window, the receiver reconnects. The window is **5 s** for the live protocols (SRT, RTMP) and **15 s** for segment protocols (HLS, DASH), which buffer in larger chunks.
- **Auto-reconnect** — on failure or stall, the receiver retries with **exponential backoff** (500 ms up to 10 s) until the source returns.

---

## Headless Mode

All streaming, recording, and network I/O features work identically in headless mode. Outputs defined in `stage.json` auto-start on launch. See [HTTP API & Headless Mode](13-api.md#headless-mode).

---

[← Prev: Projection Mapping](08-projection.md) · [Home](README.md) · [Next: Resolution, Settings & Monitoring →](10-resolution-and-monitoring.md)
