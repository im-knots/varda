# Library Panel

The Library is the left sidebar and the primary content browser. It lists everything you can drop onto a channel to create a deck — shaders, media, cameras, and live network streams. Toggle it with the **`L`** key (or the sidebar button; its hover text reads "Open library (L)" / "Close library (L)").

## Sections

The panel is a stack of collapsible sections, in this order:

| Section | Contents |
|---------|----------|
| **🎨 Generators** | ISF generator shaders. Count shown in the header. |
| **🔮 Effects** | ISF filter shaders for effect chains. |
| **🖼 Images** | Per-channel **📁 Load to [Channel]** button (opens a file dialog). |
| **🎬 Video** | Per-channel **📁 Load to [Channel]** button (opens a file dialog). |
| **📹 Cameras** | Detected camera devices, with a **🔄 Rescan** button. |
| **📡 Stream Sources** | NDI, SRT, HLS, DASH, and RTMP sources (see below). |
| **💾 Deck Presets** | Saved deck presets (shown only when presets exist). |
| **💾 Channel Presets** | Saved channel presets (shown only when presets exist). |

> The section count badges (e.g. "Generators (40)") reflect the live number of available items.

## Drag-and-Drop

The Library is built around drag-and-drop. The core gesture is **drag an item onto a channel to create a deck** from it:

| Item | Marker | Action |
|------|--------|--------|
| **Generator** | `◆` | Drag onto a channel → new shader deck. Double-click adds it to Channel 0. |
| **Effect** | `◇` | Drag onto a deck/channel/master **effect chain** → appends the effect. |
| **Camera** | `📹` | Drag onto a channel → new camera deck. |
| **NDI** | `📡` | Drag onto a channel → new NDI deck. |
| **SRT / HLS / DASH / RTMP** | `📺` / `📡` | Drag onto a channel → new stream deck. |
| **Deck Preset** | — | Drag onto a channel (or double-click → Channel 0) to load it. |
| **Channel Preset** | — | Drag (or double-click) to add a channel to the mixer. |

Images and video are loaded via their section's **📁 Load to [Channel]** button rather than by dragging.

Effects in a chain can also be reordered by drag-and-drop, and toggled on/off individually (see [Effect Chains](02-concepts.md#effect-chains)).

## Stream Sources

The **📡 Stream Sources** section groups all live network inputs. Its header count is the total across every protocol. Inside are nested sub-sections, each with its own count:

- **NDI** — discovered network senders. Has a **🔄 Rescan** button to re-search the network.
- **SRT** — each entry shows its **Mode: Listener** or **Mode: Caller**.
- **HLS**
- **DASH**
- **RTMP** — each entry shows its **Mode: Pull** or **Mode: Listen**.

**Connection status** is shown as a colored bullet (`●`) next to each entry:

- **Green** — connected and receiving.
- **Gray** — not connected.

Each entry has a small **✕** button to remove it from the list.

> **Library entries vs. decks.** The stream URLs you add to the sidebar are session-only quick-access entries — they are *not* written to disk. Once you drag a stream onto a channel, the resulting **deck** is persisted in `scene.json` with its protocol, URL, and mode. See [Streaming & I/O](09-streaming-and-io.md) for adding and configuring stream sources.

## Cameras

Camera devices are enumerated automatically at startup (AVFoundation on macOS, V4L2 on Linux) and listed under **📹 Cameras**. Each device is draggable onto a channel to create a camera deck.

- **🔄 Rescan** re-enumerates connected devices. Use it after plugging in a USB camera.
- Camera decks are persisted by device **name** in `scene.json`. On reload, Varda re-opens the camera by name; if it isn't connected, the deck is skipped with a warning.

### Resolution

The **resolution selector** is not in the Library — it lives in the **deck detail panel** (bottom bar) when a camera deck is selected. It is a dropdown of the device's supported resolutions (default = the device's native default), alongside the standard scaling mode (Fill / Fit / Stretch / Center). See [Per-Deck Scaling](10-resolution-and-monitoring.md#per-deck-scaling).

## Presets

When you save deck or channel presets (see [Presets](04-performance.md#presets)), they appear in the **💾 Deck Presets** and **💾 Channel Presets** sections for drag-to-load reuse. These sections are hidden until at least one preset exists.

---

[← Prev: Core Concepts](02-concepts.md) · [Home](README.md) · [Next: Performance & Automation →](04-performance.md)
