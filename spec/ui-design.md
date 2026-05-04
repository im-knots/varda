# UI Design & UX Philosophy

## Status: IMPLEMENTED

Relates to: [/spec/channel-routing.md](/spec/channel-routing.md), [/spec/multi-output.md](/spec/multi-output.md)

## Guiding Principles

1. **Performance-first**: The UI is for live performance. Everything the VJ needs is visible at a glance. No hunting through menus mid-set.
2. **Dark theme only**: VJs work in dark venues. One dark theme, no light mode.
3. **Dense but organized**: High information density is fine — panels are full of controls. But each panel has a clear purpose and fixed position.

## Color Language

Dark background with accent colors for interactive elements:

| Color | Usage |
|---|---|
| **Purple** | Ch 1 highlights, primary selection |
| **Blue** | Ch 2 highlights, secondary elements |
| **Orange** | Active/hot states, warnings |
| **Modulator palette** | Each modulation source gets a unique color (cyan, magenta, yellow, lime, orange, pink, sky blue, coral) — used on modulator cards and modulated parameter sliders |
| **Green** | Audio levels, beat indicators, "go" states |
| **Red** | Errors, mute state, stop/remove actions |
| **White/Light gray** | Text, labels, borders |
| **Dark gray** | Panel backgrounds, inactive elements |

Channels beyond A/B can cycle through the accent palette or use user-assigned colors.

## Layout

Fixed layout with resizable panels. Default window size is 1920×1080.

```
┌──────────────┬────────────┬──────────┬────────────┬──────────────────────┐
│ 📚 Library   │ Left Chs   │ 🎚 Mixer │ Right Chs  │ Right Sidebar        │
│ (collapsible)│ ┌────┬───┐ │ A▮ B▮ C▮ │ ┌────┬───┐ │ Main Output          │
│              │ │ D1 │D2 │ │  ➕ Ch   │ │ D4 │D5 │ │ ┌────────────────┐   │
│ 🎨 Generators│ ├────┼───┤ │ Crossfdr │ ├────┼───┤ │ │ [Live Preview] │   │
│  ◆ Plasma    │ │ D3 │   │ │(2ch only)│ │ D6 │   │ │ └────────────────┘   │
│  ◆ Tunnel    │ └────┘   │ │ ⏮A  B⏭  │ └────┘   │ │                      │
│ 🔮 Effects   │          │ │ →B 1s 2s │          │ │ 🎛 Modulation        │
│  ◇ Bloom     │          │ │ 4s 4beat │          │ │ [➕LFO][➕Audio]...  │
│  ◇ Invert    │          │ │ 🔀 Trans │          │ │ [mod1][mod2][mod3]→  │
│ 🖼 Images    │          │ │ Blend    │          │ │                      │
│ 🎬 Video     │          │ │          │          │ │ 🎹 MIDI             │
│ 🎨 Solid     │          │ │          │          │ │ 🗺 Surfaces          │
│ 🔊 Audio     │          │ │          │          │ │ 🖥 Outputs           │
├──────────────┴──────────┴──────────┴────────────┴──────────────────────┤
│ Bottom Bar (resizable, context-sensitive):                              │
│ Selected Deck: [Preview] | [Generator] | [⠿FX1] | [⠿FX2] | [drop→]  │
│ Selected Ch:   [⠿Ch FX1] | [⠿Ch FX2] | [drag effects here →]        │
│ Master:        [⠿Mstr FX1] | [⠿Mstr FX2] | [drag effects here →]    │
└─────────────────────────────────────────────────────────────────────────┘
```

### Panel Descriptions

**Channel Columns** (central area, split across both sides of the mixer):
- Channels are distributed DJ-style: first half on the left, second half on the right (e.g., A,B | Mixer | C,D)
- Each channel column includes: name header (clickable to select), deck grid with dynamically wrapping thumbnails
- Deck grid: live preview thumbnails with per-deck opacity slider, name label, M/S/✕ buttons — columns scale with window width
- Click a deck thumbnail → selects it, shows deck detail in the bottom bar
- Click a channel header → selects it, shows channel effect chain in the bottom bar

**Mixer Box** (center column, between channels):
- Vertical opacity faders for all channels, side by side, with per-channel accent colors
- ➕ Ch button to add new channels at runtime; ✕ button per fader to remove (visible when 3+ channels, minimum 2 enforced)
- **2-channel mode**: Horizontal crossfader with A/B labels, snap buttons (⏮ A / B ⏭), auto-transition buttons (→B 1s / 2s / 4s / 4beat), transition shader selector
- **3+ channel mode**: Crossfader hides; channels mix via per-channel opacity and blend modes
- Per-channel blend mode selectors

**Library Panel** (left sidebar, collapsible with `L` key):
- **Generators**: Draggable list of generator shaders. Drag to a channel column to create a new deck. Double-click to add to Ch 1.
- **Effects**: Draggable list of filter shaders. Drag to an effect chain drop zone in the bottom bar to add.
- **Images / Video / Solid Color / Audio**: File loading and source creation organized by type.
- See [spec/library-panel.md](library-panel.md) for full design.

**Right Sidebar** (fixed, scrollable):
- **Main Output Preview**: Live preview of final composited output, always visible. Click heading or preview to select master effects in bottom bar.
- **Master Effects**: Label indicating master FX are edited in bottom bar when selected
- **Modulation**: Add buttons (➕ LFO, Audio, ADSR, StepSeq) + horizontal scrollable columns, one per modulator with full parameter controls. Each modulator card has a color-coded header and waveform/envelope visualization. LFOs show waveform shapes; ADSRs show envelope curves.
- **Library toggle**: When library panel is closed, a "📚 Open Library (L)" button appears here for quick access.

**Bottom Bar** (resizable, context-sensitive — shows one of three views):
- **Deck Detail** (when a deck is clicked): Deck preview (scales with bar height, 16:9 aspect) | Generator params column (blend, scale, shader params) | Effect columns | Remaining drop zone
- **Channel Effects** (when a channel header is clicked): Channel effect columns | Remaining drop zone
- **Master Effects** (when Main Output is clicked): Master effect columns | Remaining drop zone
- **Adding effects**: Drag effects from the Library panel onto any effect chain. When a chain is empty, the entire area shows "🔮 Drag effects here" with a highlighted border during drag. When effects exist, remaining empty space to the right is always a valid drop target. No "Add Effect" button — drag-from-library is the only workflow.
- **Effect reordering**: Each effect card has a ⠿ drag grip (only the grip icon shows the drag cursor, not the full card). Drag between thin drop zones (highlighted cyan on hover) to reorder within the chain. Delete (✕) and toggle buttons remain freely clickable.
- **Deferred drop pattern**: Both library-to-chain drops and within-chain reordering use a deferred detection pattern — drop zone rects are stored per-frame, and the actual drop is resolved in `render_ui` after all panels render. This avoids egui's tooltip-layer drag ghost blocking drop targets.
- All effect parameter sliders include modulation assignment dropdowns and MIDI learn support
- Modulated parameter sliders show a live ghost indicator in the modulator's color, so the performer can see modulation effect in real-time
- Horizontal scroll for long effect chains

## Output Windows

The main Varda window contains the **UI + preview**. Output to projectors/displays uses **separate fullscreen windows**:

- Main window: always shows UI with embedded preview
- Output windows: borderless fullscreen on target displays, showing only the rendered output
- Each output window can be routed to a different source (master output, specific channel, specific deck)
- Output windows are created/managed from Settings or a dedicated Output panel

This means a VJ with a laptop + projector:
1. Laptop screen: Varda UI with preview
2. Projector: Fullscreen output window showing master output

## Interaction

- **Click**: Select deck, toggle effect, trigger transition
- **Drag**: Reorder effects, adjust opacity sliders, move crossfader, resize panels, **drag deck thumbnails between channels** (hot reassignment — preserves opacity, blend mode, solo/mute)
- **Right-click**: Context menus (remove deck, add effect, assign modulation)
- **Keyboard shortcuts**: Essential for performance (defined in separate spec)
- **MIDI/OSC**: Hardware control (defined in [/spec/midi-control.md](/spec/midi-control.md))

## Resolved Questions

- **Deck grid previews**: Live GPU-rendered previews. The GPU cost is acceptable and the live feedback is essential for VJ workflow. ✅ Decided and implemented.
- **Many channels**: Channels distribute across both sides of the mixer in a DJ-style split layout. The mixer shows faders for all channels; crossfader is active only for exactly 2 channels, otherwise per-channel opacity controls are used. ✅ Implemented.
- **Dynamic channel addition**: New channels can be added at runtime via the ➕ Ch button in the mixer box. ✅ Implemented.
- **Dynamic channel removal**: Channels can be removed via ✕ button on each fader (only when 3+ channels; minimum 2 enforced). ✅ Implemented.
- **Responsive deck grid**: Deck thumbnail grid dynamically calculates column count based on available width — no hardcoded 2-per-row limit. ✅ Implemented.

## Open Questions

- Should there be a "performer mode" that hides non-essential UI elements?
- Should the bottom bar support tabs/sections to show modulation assignments alongside effect chains?
- How should the UI adapt when the window is smaller than 1920×1080?

