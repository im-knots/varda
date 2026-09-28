# Library Panel

The Library is the left sidebar. It lists everything you can drop onto a channel to create a deck. Toggle it with the **`L`** key or the sidebar button (hover text "Open library (L)" / "Close library (L)").

## Sections

The panel is a stack of collapsible sections, in this order:

| Section | Contents |
|---------|----------|
| **🎨 Generators** | ISF generator shaders. Count shown in the header. |
| **🖼 Images** | Per-channel **📁 Load to [Channel]** button (opens a file dialog). |
| **🎬 Video** | Per-channel **📁 Load to [Channel]** button (opens a file dialog). |
| **📹 Cameras** | Detected camera devices, with a **🔄 Rescan** button. |
| **🛰 Depth Sensors** | Connected depth sensors, with a **🔄 Rescan** button. |
| **🖥 Screen Capture** | Capturable displays and windows, with a **🔄 Rescan** button (see below). |
| **🔁 Taps** | Varda's own master program and each channel, as sources (see below). |
| **📡 NDI** | Discovered NDI senders, with a **🔄 Rescan** button. |
| **📺 SRT**, **📡 HLS**, **📡 DASH**, **📺 RTMP** | Saved stream URLs, one section per protocol (see below). |
| **🌐 HTML Sources** | Web pages (HTML/CSS/JS) rendered by Servo (see below). |
| **🔗 Syphon** / **🔗 Spout** | Discovered Syphon servers (macOS) or Spout senders (Windows), with a **🔄 Rescan** button. Shown only on the platform that supports them. |
| **🔮 Effects** | ISF filter shaders for effect chains. |
| **💾 Deck Presets** | Saved deck presets (shown only when presets exist). |
| **💾 Channel Presets** | Saved channel presets (shown only when presets exist). |

Each kind of source has its own section.

> Each section header shows a live count of its items (e.g. "Generators (40)").

## Drag-and-Drop

To create a deck, **drag an item onto a channel**:

| Item | Marker | Action |
|------|--------|--------|
| **Generator** | `◆` | Drag onto a channel → new shader deck. Double-click adds it to Channel 0. |
| **Effect** | `◇` | Drag onto a deck/channel/master **effect chain**, or onto the owner itself (mixer deck card, channel empty space, Main Output preview, arrangement lane / automation) → appends the effect and selects the owner. |
| **Camera** | `📹` | Drag onto a channel → new camera deck. |
| **Capture target** | `🖥` | Drag onto a channel → new screen or window capture deck. |
| **Tap** | `🔁` | Drag onto a channel → new deck reading Varda's own output. |
| **NDI** | `📡` | Drag onto a channel → new NDI deck. |
| **SRT / HLS / DASH / RTMP** | `📺` / `📡` | Drag onto a channel → new stream deck. |
| **HTML** | `🌐` | Drag onto a channel → new HTML deck. |
| **Deck Preset** | (none) | Drag onto a channel (or double-click → Channel 0) to load it. |
| **Channel Preset** | (none) | Drag (or double-click) to add a channel to the mixer. |

Images and video load through their section's **📁 Load to [Channel]** button; you can't drag them. The image dialog accepts SVG as well as the usual raster formats. See [Per-Deck Scaling](10-resolution-and-monitoring.md#per-deck-scaling) for how Varda redraws vector art at your render resolution.

You can also drag effects within a chain to reorder them, and toggle each one on or off (see [Effect Chains](02-concepts.md#effect-chains)).

## Stream Sources

Each network input protocol has its own section with an item count:

- **📡 NDI**: discovered network senders. **🔄 Rescan** searches the network again.
- **📺 SRT**: each entry shows **Mode: Listener** or **Mode: Caller**.
- **📡 HLS**
- **📡 DASH**
- **📺 RTMP**: each entry shows **Mode: Pull** or **Mode: Listen**.

The SRT, HLS, DASH and RTMP sections each have a **+ Add** form for saving a URL.

A colored bullet (`●`) next to each entry shows **connection status**:

- **Green**: connected and receiving.
- **Gray**: not connected.

Click an entry's **✕** button to remove it from the list.

> **Library entries and decks.** Stream URLs you add to the sidebar last for the current session only and are not saved to disk. When you drag a stream onto a channel, the resulting **deck** is saved in `scene.json` with its protocol, URL, and mode. See [Streaming & I/O](09-streaming-and-io.md) for adding and configuring stream sources.

## HTML Sources

The **🌐 HTML Sources** section holds web pages that the embedded [Servo](https://servo.org) browser engine renders live as decks.

- Click **+ Add HTML**, type a URL into the **URL:** field (default `https://example.com/visuals.html`), then click **✓ Add**.
- Each entry has a colored bullet (`●`): **green** when a deck is rendering it, **gray** otherwise. The **✕** button removes it from the list.
- **Drag** an entry onto a channel to create an HTML deck.

As with streams, URLs you add here last for the current session only and are not saved to disk. The **deck** you create by dragging is saved in `scene.json` by URL. See [HTML / Web Content](09-streaming-and-io.md#html--web-content) for rendering and performance.

## Cameras

Varda lists camera devices under **📹 Cameras** at startup (AVFoundation on macOS, V4L2 on Linux). Drag a device onto a channel to create a camera deck.

- **🔄 Rescan** lists connected devices again. Use it after plugging in a USB camera.
- Camera decks are saved in `scene.json` by device **name**. On reload, Varda opens the camera by name. If the camera isn't connected, the deck stays as a placeholder: it renders black, keeps its settings, and shows a warning.

### Resolution

Set a camera's resolution in the **deck detail panel** (bottom bar) when a camera deck is selected. The **resolution selector** is a dropdown of the device's supported resolutions (default: the device's native default). The standard scaling mode (Fill / Fit / Stretch / Center) is next to it. See [Per-Deck Scaling](10-resolution-and-monitoring.md#per-deck-scaling).

## Screen Capture

The **🖥 Screen Capture** section lists capturable displays and windows in two groups, **Displays** and **Windows**. Drag either onto a channel to create a live capture deck. Hover a row to see the target's pixel size.

- **🔄 Rescan** lists targets again. Varda does not poll the list in the background, because window lists change every time you switch apps. Press Rescan after opening or closing the app you want.
- Varda's own windows are labeled `(Varda)` and tinted, so you can tell when you are capturing Varda.
- On macOS the section also shows the Screen Recording permission state. When access has not been granted, it shows a **Grant Screen Recording access** button. Restart Varda after granting.
- Capture decks are saved in `scene.json` by display name, or by application and window title. If the target is gone at load time, the deck stays and renders black.

The rate, crop, cursor, and Varda-exclusion controls are in the **deck detail panel** (bottom bar) when a capture deck is selected. See [Screen & Window Capture](09-streaming-and-io.md#screen--window-capture) for those controls, the permission flow, and performance.

## Taps

The **🔁 Taps** section feeds Varda's own output back in as a deck source. It lists **Master Program** and every channel. The list comes from the live mixer, so there is nothing to rescan.

- Drag an entry onto a channel to create a tap deck.
- A tap always shows the **previous** frame. This keeps feedback loops stable, and the delay stays the same whatever the deck order.
- Select a tap deck to get a **Source** dropdown in the deck detail panel. Use it to repoint the tap without recreating the deck.

See [Program Tap](09-streaming-and-io.md#program-tap) for frame timing, feedback behavior, and the API.

## Presets

Deck and channel presets you save (see [Presets](04-performance.md#presets)) appear in the **💾 Deck Presets** and **💾 Channel Presets** sections. Drag one to load it. These sections are hidden until at least one preset exists.

---

[← Prev: Core Concepts](02-concepts.md) · [Home](README.md) · [Next: Performance & Automation →](04-performance.md)
