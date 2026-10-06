# Streaming, Recording & Network I/O

## 10-bit SDR Delivery

Set an output's format to **10-bit SDR** to request a high-precision codec or transport.
Before starting, Varda checks the installed GPU, FFmpeg encoders, NDI runtime, and endpoint contract.
If any part of the path is missing, Varda keeps your request, delivers 8-bit SDR, and shows the reason.

| Destination | 10-bit path | Fallback |
|---|---|---|
| Recording | HEVC Main10, AV1 10-bit, ProRes 422, ProRes 4444 | H.264 and HAP remain 8-bit |
| NDI send | NDI 6 P216 | Older or incomplete runtimes use UYVY |
| SRT | HEVC Main10 in MPEG-TS | H.264 remains 8-bit |
| HLS / DASH | HEVC Main10 or AV1 10-bit in fMP4 | Unsupported encoders use 8-bit |
| RTMP / RTMPS | HEVC Main10 or AV1 with an Enhanced endpoint contract | Legacy RTMP uses H.264 8-bit |
| Syphon | No interoperable 10-bit path | BGRA8 |
| Spout | `R10G10B10A2` | BGRA8 |

10-bit SDR keeps more code values after Varda's tonemap and LUT. It adds no HDR metadata and no
extra brightness. For HDR, choose **HDR10** or **HLG**.

## HDR Delivery

Recordings and streams can both carry HDR. Choose the transfer by what you are producing: a file or a live feed.

| | HDR10 (PQ) | HLG |
|---|---|---|
| Standard | SMPTE ST 2084, absolute brightness | ARIB STD-B67 / BT.2100, relative |
| Mastering metadata | ST 2086 + content light level | **None needed** |
| Peak setting | Yes, per output | No; the display sets it |
| Best for | Files, and CDN specs that name HDR10 | Live streams |

**Live streams default to HLG.** HDR10 carries MaxCLL and MaxFALL, which describe how bright the
content gets and are meant to be measured across the whole program. A live stream never ends, so
the values can't be measured and must be declared up front. HLG has no such metadata, which is why
broadcast uses it for live.

If a CDN ingest spec names HDR10, pick HDR10. Varda declares the metadata from your peak setting.

### HDR support by destination

| Destination | HDR10 (PQ) | HLG |
|---|---|---|
| Recording | HEVC, AV1 | Not yet; falls back to PQ |
| SRT | HEVC Main10 in MPEG-TS | Yes |
| HLS (either segment length) | HEVC Main10 or AV1 10-bit in fMP4 | Yes |
| DASH | HEVC Main10 or AV1 10-bit | Yes |
| RTMP / RTMPS | HEVC or AV1 on an **Enhanced RTMP** endpoint | Yes |
| NDI, Syphon, Spout | No | No |
| H.264, ProRes, HAP | No | No |

Legacy RTMP cannot carry HDR. It falls back with `the endpoint contract is legacy RTMP`.
HDR over NDI is not planned, because the public NDI SDK has no standard HDR transfer signaling.

### Verifying a stream or file

```bash
ffprobe -show_entries stream=color_transfer,color_primaries,color_space yourfile.mp4
```

HDR10 reports `smpte2084` / `bt2020` / `bt2020nc`. HLG reports `arib-std-b67` / `bt2020` /
`bt2020nc`. The same command works on an HLS playlist URL.

### Short segments

An HLS output has a **Short segments (lower latency)** option. It writes one-second segments
instead of two-second ones, so a player can start a segment sooner.

This is standard HLS, not RFC low-latency HLS. FFmpeg's HLS muxer cannot write partial segments
(`EXT-X-PART`), so latency cannot go below one segment. Expect 2–5 s of latency.
Ordinary HLS players work with it.

### Segment duration and keyframes

HLS and DASH can only start a segment on a keyframe, so Varda sets the encoder's keyframe interval
to the segment duration: one second for short-segment HLS, two seconds for standard HLS and DASH.
You don't configure this. It explains the `#EXTINF` values in a playlist.

### HDR signaling in HLS playlists

An HLS master playlist can carry a `VIDEO-RANGE` attribute that announces HDR. FFmpeg's HLS muxer
does not write it, so Varda's playlists have none. The HDR information is in the video bitstream,
which is where players read it. A player that requires `VIDEO-RANGE` will not detect HDR; this is a
known gap, not a setting you missed.

See [Output Format](10-outputs.md#output-format) for the peak-luminance control, the gamut
caveat, and the current limits on LUTs, tonemap curves, and mastering metadata.

## NDI

### Sending

Each output can send video over NDI to other applications and machines on the network.

1. In the output panel, choose **+ Output → NDI**
2. Set the **Sender name** on its card
3. Enter a sender name (e.g., "Varda Main")
4. Any NDI application on the LAN can discover the stream

With an NDI 6 runtime loaded, a 10-bit request sends P216. An 8-bit request, or an older runtime,
sends UYVY; an older runtime also shows a fallback reason. Both are converted on the GPU to Rec.709
at limited range (video levels) and tagged that way, so receivers show the same colors and levels
either way. Set the receiving software to its best or highest-quality color mode so it does not
convert P216 back to 8-bit.

### Receiving

1. In the Library panel, open the **📡 NDI Sources** section
2. Click **Rescan** to find NDI sources on the network
3. **Drag** a source into a channel to create a live deck

Varda requests UYVY from each source, or RGBA when the source has alpha, so the NDI runtime does
no conversion. Varda reads UYVY as Rec.709 at limited range (the same format it sends) and converts
it to RGB on the GPU. RGBA is used as received.

Varda loads the NDI SDK at runtime (`libloading`). If the SDK is not installed, NDI features are unavailable and the rest of Varda runs normally.

---

## SRT (Secure Reliable Transport)

### Output (Streaming)

SRT output uses **listener mode**: Varda is an SRT server that clients connect to.

1. Choose **+ Output → SRT stream**
2. Set the **URL** on its card (default: `srt://0.0.0.0:9001`)
3. Start the output. Varda starts listening for SRT clients. Until one connects, the card shows **Waiting for a client on** the URL, and no frames are sent or counted as dropped.

When a client disconnects, the listener restarts so new clients can connect. Frame delivery is non-blocking.

### Input (Receiving)

SRT input uses **caller mode**: Varda connects to a remote SRT listener.

1. In the Library, open the **📺 SRT Sources** section
2. Add a URL (e.g., `srt://192.168.1.50:9001`)
3. **Drag** the source into a channel to create a live deck

Decks that use the same SRT URL share one connection.

> **Note:** Native SRT support requires ffmpeg built with `--enable-libsrt`.

---

## HLS & DASH

### Output

1. Choose **+ Output → HLS stream** or **DASH stream**
2. Choose a codec: **H.264**, **H.265**, or **AV1**
3. For HLS, optionally enable **Short segments** for 2 to 5 second end-to-end latency
4. Start the output

Varda writes segments and manifests to `.varda/streams/<name>/` and serves them from its built-in HTTP server:

```
http://<your-ip>:8080/streams/<name>/playlist.m3u8   (HLS)
http://<your-ip>:8080/streams/<name>/manifest.mpd     (DASH)
http://<your-ip>:8080/streams/<name>/player.html      (auto-generated HTML5 player)
```

Click any URL in the output panel to **copy it to the clipboard**.

The generated `player.html` uses hls.js or dash.js and works in any modern browser. Share its URL with anyone on your network.

| Mode | Latency | Use Case |
|------|---------|----------|
| Standard HLS | 15–25s | Reliable delivery, CDN-friendly |
| HLS, short segments | 2–5s | Near-real-time web viewing |
| DASH | 10–20s | Cross-platform, multi-codec |

### Input

1. In the Library, open **📡 HLS Sources** or **📡 DASH Sources**
2. Add a stream URL (`.m3u8` for HLS, `.mpd` for DASH)
3. **Drag** it into a channel to create a live deck

Input streams detect stalls and reconnect on failure (see [Stream Input Reliability](#stream-input-reliability)).

---

## Recording

Each output can record to its own video file. You can run several recordings at once.

### Codecs

| Codec | Use Case |
|-------|----------|
| H.264 | Quick recording, small files |
| H.265 | Better compression, smaller files |
| AV1 | Best compression, slower encoding |
| ProRes 422 | Edit-ready |
| ProRes 4444 | Edit-ready, with alpha channel |
| HAP | VJ content reuse, GPU-native playback |
| HAP Alpha | HAP with alpha channel |
| HAP Q | Higher-quality HAP (YCoCg compression) |

### Usage

1. In the output panel, choose **+ Output → Recording**. Add one recording output per simultaneous recording; each runs its own ffmpeg process.
2. Set the **File:** path (plain text; default `output.mp4`, relative to the working directory). Varda uses the path exactly as typed and adds **no timestamp**, so give each recording its own name.
3. Pick a **Codec:** from the table above.
4. Click **▶ Start**. The button changes to **⏹ Stop**, and a red elapsed-time counter runs while recording.

Each recording starts and stops on its own, and ffmpeg writes straight to your path. If the encoder falls behind, Varda drops frames instead of stalling the render thread. It repeats the previous frame in the file, so the recording keeps a constant frame rate and the correct length.

> **Adding audio.** Pick a device in the output's **Audio:** dropdown. See [Audio Passthrough](#audio-passthrough) below.

---

## Audio Passthrough

Every ffmpeg output (Recording, SRT, HLS, DASH, RTMP) can mux audio from a capture device with the video. This is the **same physical device** that drives Varda's modulation engine. One device feeds analysis, the live monitor, and every output, all on one hardware clock, so audio and visuals stay in sync.

### Selecting a device

1. Set up an ffmpeg output (Recording or any stream) and leave it **stopped**.
2. In the output's **Audio:** dropdown, pick a capture device. **None (silent)** records video only and is the default.
3. Click **▶ Start**. The output now carries that device's audio.

### Audio format

- **Recording** writes AAC at the device's **native sample rate**.
- **Streams** (SRT, HLS, DASH, RTMP) use **48 kHz AAC**, which Twitch and YouTube expect.
- Audio is **downmixed to stereo**.
- **Sync holds when the renderer drops frames.** Timing comes from the capture device's sample clock, which runs at a steady rate whatever the GPU is doing.

### Missing devices

If a scene selects a device that is missing at load (unplugged or renamed), the output starts **video-only** and a notification says why. The video recording or stream still runs.

> **Scope.** Audio passthrough sends one device's audio to your outputs. There is no audio-file playback, mixing, or per-output gain. For audio reactivity, see the [modulation system](06-modulation.md).

---

## RTMP / RTMPS

### Output (Streaming to Platforms)

Send video to Twitch, YouTube, Kick, or any RTMP/RTMPS ingest endpoint.

1. Choose **+ Output → RTMP stream**
2. Enter the ingest URL (e.g., `rtmp://live.twitch.tv/app/<stream-key>` or `rtmps://a.rtmps.youtube.com/live2/<stream-key>`). The output will not start until the URL names a server.
3. Choose a codec: **H.264**, **H.265**, or **AV1** (H.265 and AV1 need Enhanced RTMP)
4. Choose **Enhanced** only when the endpoint accepts Enhanced RTMP signaling
5. Start the output

Varda muxes FLV with an auto-scaled CBR bitrate and a keyframe every 2 seconds. Frame delivery is non-blocking.

Legacy RTMP always uses H.264 8-bit. A 10-bit HEVC or AV1 request needs the Enhanced endpoint
contract saved on the output and FFmpeg muxer support for it.

> **Stream keys are credentials.** An ingest URL contains your platform stream key. Treat it as a password. Don't share or record your screen while the RTMP output field is visible, and never paste full ingest URLs into bug reports.

### Input (Receiving RTMP Streams)

RTMP input has two modes.

**Pull mode** connects to a remote RTMP stream:

1. In the Library, open **📡 RTMP Sources** (under Stream Sources)
2. Add a stream URL (e.g., `rtmp://192.168.1.50/live/stream`)
3. **Drag** it into a channel to create a live deck

**Listen mode** accepts pushes from OBS, vMix, or other RTMP senders:

1. In the Library, add an RTMP source and select **Listen** mode
2. Varda generates a listen URL (starting at port 1935, one port higher for each additional listener)
3. Set OBS or other software to push to that URL
4. **Drag** the source into a channel

The Library groups stream sources under one **Stream Sources** header. Every stream source type (NDI, SRT, HLS, DASH, RTMP) works the same way: drag it onto a channel.

---

## HTML / Web Content

An HTML deck renders a live web page as a deck source: dashboards, SVG/Canvas/WebGL, lyric and lower-third overlays, animated HTML/CSS. Varda renders pages with an embedded [Servo](https://servo.org) browser engine and composites them like any other source.

### Usage

1. In the Library, open the **🌐 HTML Sources** section and click **+ Add HTML**
2. Enter a source in the **URL:** field:
   - a remote URL: `https://example.com/overlay.html`
   - a local file: `file:///Users/you/show/lyrics.html`
   - an inline document: `data:text/html,<h1>Hello</h1>`
3. Click **✓ Add**, then **drag** the entry onto a channel to create a live HTML deck

You can also add one over the HTTP API:

```sh
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks \
  -H "Content-Type: application/json" \
  -d '{"type": "Html", "url": "https://example.com/overlay.html"}'
```

HTML decks are saved in `scene.json` by URL and reload with the scene.

### Rendering & performance

Servo renders HTML on the **CPU** (software renderer), and Varda uploads the result to a GPU texture each frame. HTML decks cost more than the GPU-native deck types.

> **Platform support.** HTML decks are available on **Apple Silicon macOS** (arm64) and **Linux** (x86_64). They are **not** available on Intel (x86_64) macOS: creating a Servo deck hangs under Rosetta, so the macOS DMG includes HTML in the Apple Silicon slice only. HTML/CSS/SVG, dashboards, and text overlays run well. Heavy WebGL or full-screen Canvas animation at high resolution may not hold 60 fps. If frame rate matters, profile your pages with the `html_render` benchmark (see [Benchmarking](../CONTRIBUTING.md#benchmarking)).

> **Background rendering.** HTML renders on a dedicated background thread (one shared Servo engine), the same way NDI/SRT/HLS/DASH/RTMP decode off the render loop. Finished frames go to the render thread and upload without blocking.

> **Feature flag.** HTML decks need the `html` build feature, which is **on by default**. Turn HTML off for a session with `--no-html`, or build without it using `--no-default-features`.

---

## Syphon (macOS)

Syphon shares GPU textures between applications on macOS. Varda can be a Syphon **client** (receiving other apps' frames as live sources) and a Syphon **server** (publishing a Varda output to other apps).

**Receive (client):**

1. Open the Library and look under **Syphon Sources** for discovered servers
2. **Drag** a server into a channel to create a live deck

**Publish (server):**

1. In the output panel, choose **+ Output → Syphon**
2. Set its **Name** on its card
3. Enter a server name (e.g., "Varda Main")
4. Start the output. Other Syphon apps now list it as a source.

### Color

Syphon carries display-encoded 8-bit BGRA. Varda publishes and expects that format. Syphon has no
interoperable 10-bit or HDR path: a Syphon output uses BGRA8, and the output card says so (see the
tables at the top of this page).

> **Behavior change.** Earlier versions published frames that were too dark: a mid grey of 128
> reached other applications as 55, and only pure black and pure white were correct. Published
> frames now match the source exactly. If you corrected for this downstream, in a receiving app's
> color controls or a projector LUT, remove that correction.

### Performance

Both directions share GPU memory with the other application. Frames are not copied through the CPU.

- **Receiving** binds the publishing app's shared surface directly, so a Syphon source has no
  per-frame transfer cost at any resolution. Varda re-binds only when the other application resizes or
  restarts.
- **Publishing** costs one conversion pass, plus Syphon's own copy into the surface it shares with
  clients. Every Syphon publisher pays that copy.

### Installing Syphon.framework

You don't need anything at build time. Varda does not link `Syphon.framework`; it loads it at runtime with `dlopen`. A normal macOS build (`cargo build` / `cargo run`) works with or without Syphon installed. Without it, Syphon features are disabled and the rest of Varda runs normally.

To use Syphon, install the framework system-wide at:

```
/Library/Frameworks/Syphon.framework
```

This is the standard, tested location, and other Syphon apps look for the framework there too, so one install serves all of them. To install it:

1. Get `Syphon.framework`. Download it from the [official Syphon-Framework releases](https://github.com/Syphon/Syphon-Framework/releases), or copy it out of any Syphon-enabled app bundle (e.g. Simple Syphon, Resolume, VDMX, MadMapper).
2. Copy the `Syphon.framework` folder into `/Library/Frameworks/` (requires admin):
   ```sh
   sudo cp -R /path/to/Syphon.framework /Library/Frameworks/
   ```
3. Launch Varda. At startup the log shows `Syphon.framework found` if it loaded, or `Syphon.framework not found; Syphon features disabled` if not.

> If the framework is not in `/Library/Frameworks/`, Varda also checks `~/Library/Frameworks/Syphon.framework` (per-user, no admin needed). The system-wide path is the recommended and tested one.

Pass `--no-syphon` to turn Syphon off even when the framework is installed.

---

## Spout (Windows)

Spout shares GPU textures between applications on Windows, like Syphon on macOS. Varda can be a Spout **receiver** (taking other applications' frames as live sources) and a Spout **sender** (publishing a Varda output to applications such as Resolume, OBS, TouchDesigner or HeavyM).

You don't need to install anything. Spout sharing is built into Windows, so there is no framework or runtime to add (unlike Syphon on macOS or the NDI SDK).

**Receive:**

1. Open the Library and look under **Spout Senders** for the applications currently publishing
2. **Drag** one into a channel to create a live deck

**Publish:**

1. In the output panel, choose **+ Output → Spout**
2. Set its **Name** on its card
3. Enter a sender name (e.g., "Varda Main")
4. Start the output. Other Spout applications now list it as a source.

Start order doesn't matter in either direction. A deck bound to a sender that is not running yet connects as soon as that application starts publishing, so a saved show reopens correctly whichever app you launch first.

### Requirements

Spout needs Varda on the **DirectX 12** graphics backend, the Windows default. If Varda selects Vulkan instead, the Spout library section and the Spout output option are hidden.

### Color

Spout carries display-encoded pixels. Varda publishes 8-bit BGRA by default, because that is Spout's default and every receiving application reads it.

You can choose **10-bit** per output, like any other format. Use it when the receiving application reads 10-bit. Most Spout applications expect 8-bit BGRA, so check the receiver before switching a show to 10-bit.

Spout has no HDR. For HDR, use a recording or a streaming protocol (see [HDR Delivery](#hdr-delivery)).

---

## Screen & Window Capture

A capture deck shows an OS display or a single application window as a live source: a browser, a game, a DAW, a slide deck, another VJ app, or Varda itself. The source application needs no Syphon support or plugin.

### Platform support

| Platform | Backend | Details |
|---|---|---|
| **macOS** 12.3+ | [ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit) | Needs the Screen Recording permission (see below). Varda leaves its own windows out of a display capture. The display is scaled down before it leaves the system, so a 4K screen costs no more than a 1080p one |
| **Windows** 10 (1903+) | [Windows.Graphics.Capture](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture) | No permission prompt. Some Windows builds draw a yellow border around the captured surface; Varda hides it where the OS allows |
| **Linux, Wayland** | XDG Desktop Portal plus PipeWire | Your compositor shows the picker. See the note below |
| **Linux, X11** | `GetImage` polling | Works everywhere. Varda cannot leave itself out of a display capture, and the cursor is never included |

Varda picks the Linux backend at startup from your session type, so one build serves both. An XWayland session uses the portal, because X11 there can only see other XWayland clients.

**On Wayland the Library shows one entry, "Pick a window or display…", instead of a list.** Drag it onto a channel and your desktop's share dialog opens so you can choose what to capture. The dialog opens again for each new capture, including captures in a scene you load.

### Usage

1. In the Library, open the **🖥 Screen Capture** section and click **🔄 Rescan**
2. Targets are grouped under **Displays** and **Windows**. Hover a row to see its pixel size
3. **Drag** a target onto a channel to create a live capture deck

The list refreshes only when you press Rescan. Varda does not poll it in the background, because window lists change every time you switch apps.

Varda's own windows are labeled `(Varda)` and tinted, so you can see when you are about to capture Varda.

### Permissions (macOS)

macOS controls screen capture through the Screen Recording privacy setting. The Library section shows the current state:

- **Not yet determined.** A **Grant Screen Recording access** button appears. Click it and macOS shows its prompt.
- **Denied.** The panel directs you to **System Settings → Privacy & Security → Screen Recording**.

In both cases, **restart Varda after granting access**. macOS does not apply a new grant to a running process. Varda never requests the permission at startup, and never requests it when capture is disabled.

Windows and Linux have no such permission, so the Library shows no permission banner there. On Wayland the portal dialog asks when you drag a capture onto a channel.

### Deck controls

Select a capture deck to see its controls in the deck detail panel (bottom bar):

| Control | Parameter path | Notes |
|---|---|---|
| **Rate** | `deck/<deck_uuid>/capture/rate` | 1 to 120 fps. Default 30 |
| **Crop** X / Y / W / H | `deck/<deck_uuid>/capture/crop_x` (`_y`, `_w`, `_h`) | Normalized 0 to 1 within the target, with a **Reset** button |
| **Cursor** | `deck/<deck_uuid>/capture/cursor` | Include the mouse pointer. Fixed once the capture opens on Wayland. Not available on X11 |
| **Exclude Varda** | `deck/<deck_uuid>/capture/exclude_varda` | Leave out Varda's own windows. Display targets only. macOS only (see below) |

Each control has a parameter path, so you can MIDI-learn it, address it over OSC, and drive it from a macro. You cannot modulate these controls directly: a modulation target is re-evaluated every frame, and only shader inputs, opacity, macro values, and [video playback](06-modulation.md#video-playback) support that so far. To modulate a capture control, assign the modulator to a macro and point the macro at the capture path. See [Parameter Paths](07-control-surfaces.md#parameter-paths).

### Capturing Varda itself

You can capture one of Varda's own windows, for example to record Varda for content. Varda's UI window contains deck and channel previews, so a capture of it contains a nested preview of itself. This is a video feedback loop, which is often the effect you want.

For a clean recording of the program without the interface, use a [Program Tap](#program-tap) to read the program directly, capture a windowed [output](10-outputs.md) instead of the main window, or use a [recording output](#recording).

For a full-display capture, **Exclude Varda** is on by default, so pointing a deck at your main monitor does not create an infinite mirror.

**On Linux under X11, Exclude Varda does nothing.** X11 cannot leave one application out of a screen capture, so a display capture always includes Varda. The mirror stays stable because the capture rate is below the render rate. For a clean desktop capture on X11, move Varda to another monitor or capture individual windows. On Wayland you pick the target in the compositor's dialog, so this does not apply.

### Persistence

Capture decks are saved in `scene.json` by **name**: a display's name, or a window's application and title. Platform handles (CoreGraphics display ids, window numbers) change across reboots and are never saved.

If the target is missing when a scene loads, the deck is **restored unbound**. It renders black, logs a warning, and the deck detail panel shows the problem. The deck keeps its effect chain, opacity, and MIDI mappings, so you only need to repoint it.

### API

```sh
# Refresh the target list; the answer lists the targets found
curl -X POST http://localhost:8080/api/sources/ScreenCapture/actions/rescan

# Capture a whole display
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks \
  -H "Content-Type: application/json" \
  -d '{"type": "ScreenCapture", "target": {"kind": "display", "name": "Built-in Retina Display"}}'

# Capture one window, cropped to its top-left quadrant at 24 fps
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks \
  -H "Content-Type: application/json" \
  -d '{"type": "ScreenCapture",
       "target": {"kind": "window", "app": "Safari", "title": "Dashboard"},
       "rate": 24,
       "crop": {"x": 0.0, "y": 0.0, "w": 0.5, "h": 0.5},
       "show_cursor": true}'
```

The `ScreenCapture` entry in `GET /api/library/sources` reports whether capture is available (and if not, why), the targets from the last scan, and a notice when Screen Recording access is missing. `POST /api/sources/ScreenCapture/actions/grant_permission` asks the OS for access.

> **Feature flag.** Screen capture needs the `screen-capture` build feature, which is **on by default**. Turn it off for a session with `--no-screen-capture`, which skips OS capture entirely so Varda never requests Screen Recording permission. Or build without it using `--no-default-features`.

---

## Program Tap

A **tap** feeds Varda's own output back in as a deck source, read from GPU memory. Use it for video feedback, picture-in-picture of the program, and effect chains that process the whole mix.

There are two tap points:

| Tap | Reads |
|---|---|
| **Master Program** | The full mix, **before** tonemap and LUT |
| **Channel** | One channel's composite, after its own effect chain |

### Frame delay

A tap always shows the **previous** frame. A tap never reads a texture that is still being written, and the delay does not depend on where the tapping deck sits in the mixer. Moving the deck to another channel, reordering channels, or adding more taps leaves the latency at exactly one frame.

Master taps read before tonemapping so that a feedback loop does not apply the transfer curve again on every pass. If it did, the image would crush toward the roll-off within a second or two.

### Usage

1. In the Library, open the **🔁 Taps** section
2. **Drag** either **Master Program** or a channel onto a channel to create a tap deck

The list comes from the live channel list, so there is nothing to rescan. Select a tap deck to get a **Source** dropdown in the deck detail panel. Use it to switch the tap between Master Program and any channel without recreating the deck.

### Feedback

A deck that taps the master while also being part of the master creates a feedback loop. The one-frame delay keeps the loop stable. Before you turn one up:

- **Gain above 1.0 grows without limit.** A tap deck with opacity above 1.0 on an additive blend multiplies its own output every frame. The tonemap rolls the result off but does not clamp it, so the image blooms to white and stays there.
- **Add a transform.** A tap composited exactly over its own source reproduces the same image. Scale, rotate, offset, or run it through an effect chain to get tunnels and trails.

### API

```sh
# Tap the master program
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks \
  -H "Content-Type: application/json" \
  -d '{"type": "Tap", "source": {"kind": "master_program"}}'

# Tap another channel
curl -X POST http://localhost:8080/api/channels/<ch_uuid>/decks \
  -H "Content-Type: application/json" \
  -d '{"type": "Tap", "source": {"kind": "channel", "uuid": "<other_ch_uuid>"}}'

# Repoint an existing tap deck, keeping its effects and modulation
curl -X PUT http://localhost:8080/api/decks/<deck_uuid>/source \
  -H "Content-Type: application/json" \
  -d '{"type": "Tap", "source": {"kind": "master_program"}}'
```

---

## Stream Input Reliability

All stream **input** protocols (SRT, HLS, DASH, and RTMP) behave the same way:

- **Deduplication**: decks that use the same URL share one connection, so adding a stream to several channels costs one receive.
- **Stall detection**: if no frames arrive within the timeout, the receiver reconnects. The timeout is **5 s** for live protocols (SRT, RTMP) and **15 s** for segment protocols (HLS, DASH), which buffer in larger chunks.
- **Auto-reconnect**: after a failure or stall, the receiver retries with **exponential backoff** (500 ms up to 10 s) until the source returns.

---

## Headless Mode

All streaming, recording, and network I/O features work the same in headless mode. Outputs defined in `stage.json` start at launch. See [HTTP API & Headless Mode](15-api.md#headless-mode).

---

[← Prev: Projection Mapping](11-projection.md) · [Home](README.md) · [Next: Resolution, Settings & Monitoring →](13-resolution-and-monitoring.md)
