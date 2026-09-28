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
| **T Text** | A text deck (see below). |
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

## Text

A text deck draws words, lyrics or captions in any font installed on the machine.

- Drag **Text** onto a channel to create a deck showing `TEXT`. Type your own text in the deck detail panel. The editor saves when you click away or press **Cmd/Ctrl+Enter**. **Enter** starts a new line.
- **📁 Load** next to the format in the deck detail panel replaces the deck's text with a file: plain text (`.txt`), lyrics (`.lrc`, including word timing tags), or captions (`.vtt` WebVTT, `.srt` SubRip). The file's contents are copied into the deck; later changes to the file do not show up.
- The deck detail panel groups the controls into **Text**, **Style**, **Layout**, **Motion** and **Transport** columns. Each column scrolls on its own when the bar is short. Click a column's title to fold it into a narrow strip, and click the strip to open it again.
- Problems in a timed file (a line without a timestamp, a bad cue timing) are listed under the editor with their line numbers. The rest of the file still loads.

### Modes

| Mode | What it shows |
|------|---------------|
| **Static** | All lines as one block. |
| **Crawl** | All lines, moving up. |
| **Ticker** | All lines in one row, moving left. |
| **Step** | One unit at a time. **Unit** picks a line (a whole cue in a caption file) or a single word. |

**Speed** is in units per second with the **Rate** clock, or units per beat with the **Beat** clock (0.25 steps once per bar in 4/4). A negative speed runs backwards. With **Loop** on, the text starts again after the end; with it off, Step holds the last unit and Crawl and Ticker end empty. **Next**, **Previous** and **Restart** step by hand. Map them to MIDI pads to cue lyrics live with speed at 0.

### Transitions

In Step, **Transition** sets how one unit replaces the next: **Cut**, **Fade**, **Roll-up** (a window of 1 to 4 lines, the newest at the bottom, moving up as each line arrives) or **Paint-on** (revealed left to right). **Transition time** is in seconds with the Rate clock and beats with the Beat clock. A caption with an end time leaves the same way. Crawl and Ticker move continuously and ignore the transition.

### Timecode

A text deck follows the show transport like a video deck does (**Chase**: Auto, Always, Never; **Chase offset**; **Chase delay**). Plain text moves at its speed from the offset, so the same show position always shows the same line. LRC, WebVTT and SubRip files cue each line at its timestamp, and each word where the file has word timing. Word timing only lines up with the singer when the transport is locked to the track, from LTC or MTC sent by the playback rig or from an arrangement. See [Arrangement](15-arrangement.md).

### Placement and speakers

A WebVTT file whose cues set `position`, `line` or `align` places every cue itself in Step. The deck's **Position**, **Align** and **Vertical** controls are then grayed out; hover one to see why. `<v Name>` speaker tags color each cue: the first four speakers use **Speaker 1** to **Speaker 4 color**, and a fifth reuses the first.

### Fonts and colors

**Font** lists the fonts installed on this machine. A scene that names a font this machine lacks draws in the default sans-serif and says so under the Font control; it goes back to the named font on a machine that has it. **Weight** runs from 100 (thin) to 900 (black); variable fonts draw every weight in between. **Size** is the height of a line as a fraction of the deck, so text looks the same at any resolution.

The deck draws **Color** text over the **Background** color, white on black by default. Lower the background's alpha in its color picker for a transparent background. For a knockout over lower decks, keep the black background and set the deck's blend mode to **Subtract**.

Size, weight, speed, scroll, transition time, line spacing, position and every color can be modulated, automated and mapped to MIDI. Colors and position are mapped one channel or axis at a time (see [Per-Component Modulation](05-modulation.md#per-component-modulation)).

### Live text over OSC

Send an OSC string to replace the text, or to add a line at the bottom:

```
/varda/deck/<uuid>/text  "Whole new lyric sheet"
/varda/deck/<uuid>/line  "next caption line"
```

`line` works on plain text only. With Step and Roll-up it shows a live caption window driven by another application. The oldest lines are dropped when the deck's text reaches 64 KiB.

## Presets

Deck and channel presets you save (see [Presets](04-performance.md#presets)) appear in the **💾 Deck Presets** and **💾 Channel Presets** sections. Drag one to load it. These sections are hidden until at least one preset exists.

---

[← Prev: Core Concepts](02-concepts.md) · [Home](README.md) · [Next: Performance & Automation →](04-performance.md)
