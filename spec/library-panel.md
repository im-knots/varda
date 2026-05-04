# Library Panel (Left Sidebar)

## Status: IMPLEMENTED

Relates to: [/spec/ui-design.md](/spec/ui-design.md), [/spec/deck-sources.md](/spec/deck-sources.md)

## Intent

The right sidebar is overloaded with output preview, modulation, shader library, MIDI, stage layout, and outputs. Split content browsing into a dedicated **left sidebar** so VJs can quickly find and add generators, effects, and media without scrolling through unrelated controls.

Traces to: [/vision/parity-gap.md](/vision/parity-gap.md) — "Massive built-in effects/source library" is listed as a Resolume advantage. A dedicated library panel is step one toward closing that gap.

## Design

### Layout

Collapsible left sidebar. Defaults to **open**. Toggle via a button or keyboard shortcut.

```
┌─────────────┬────────────┬──────────┬────────────┬──────────────────────┐
│ 📚 Library  │ Left Chs   │ 🎚 Mixer │ Right Chs  │ Right Sidebar        │
│             │            │          │            │                      │
│ ▸ Generators│ ┌────┬───┐ │ Ch1▮Ch2▮ │ ┌────┬───┐ │ Main Output          │
│ ▸ Effects   │ │ D1 │D2 │ │ Crossfdr │ │ D4 │D5 │ │ 🎛 Modulation        │
│ ▸ Images    │ ├────┼───┤ │          │ ├────┼───┤ │ 🗺 Stage Layout      │
│ ▸ Video     │ │ D3 │   │ │          │ │ D6 │   │ │ 📺 Outputs           │
│ ▸ Audio     │ └────┘   │ │          │ └────┘   │ │ 🎹 MIDI              │
│             │          │ │          │          │ │                      │
└─────────────┴──────────┴──────────┴────────────┴──────────────────────┘
```

### Sections

Each section is a collapsible header. Items within are a vertical scrollable list.

**Generators** — ISF generator shaders from the registry. Each item shows shader name. Drag to a channel column to create a new deck.

**Effects** — ISF filter shaders from the registry. Each item shows shader name. Drag to an effect chain in the bottom bar to add the effect. Drag within the chain to reorder.

**Images** — File browser / persistent collection of image files. Click to open file dialog and add to collection. Drag image to a channel to create an image deck.

**Video** — File browser / persistent collection of video files. Same drag-to-channel pattern.

**Audio** — Reserved for audio-reactive sources and audio file playback (future).

**Solid Color** — Quick access to add a solid color deck to any channel.

### Drag-and-Drop Interactions

Uses `egui::DragAndDrop` payload system with a **deferred drop pattern** to work around egui's tooltip-layer drag ghost blocking drop targets.

#### Payload Types

| Drag Source | Payload | Drop Target | Action |
|---|---|---|---|
| Library Generator | `LibraryDrag::Generator(registry_idx)` | Channel column | Create new deck with generator |
| Library Effect | `LibraryDrag::Effect(registry_idx)` | Effect chain in bottom bar | Add effect to deck/channel/master |
| Library Image | `LibraryDrag::Image(path)` | Channel column | Create new image deck |
| Library Video | `LibraryDrag::Video(path)` | Channel column | Create new video deck |
| Effect grip ⠿ (bottom bar) | `EffectDrag::Deck/Channel/Master` | Drop zone in same chain | Reorder effect |

#### Deferred Drop Pattern — IMPLEMENTED

Standard `dnd_release_payload()` doesn't work across egui panels because `dnd_drag_source` renders on a tooltip layer that covers drop targets. The solution:

1. **During drag**: Each frame, drop targets store their screen rects in `egui::Memory` temp storage (keyed by target identity). The drag source stores its payload data in temp memory too.
2. **Detect drop**: In `render_ui` (after all panels render), check if `has_payload` transitioned from `true` to `false` — meaning the user just released. Read the last-known hovered target rect and stored payload data to determine what was dropped where.
3. **Apply action**: Emit the appropriate action (e.g., `shader_to_add`, `effect_to_add`, `effect_to_move`).

This pattern is used for:
- Library generators → channel columns
- Library effects → deck/channel/master effect chains
- Effect grip reordering within chains

#### Visual Feedback

- **Drag ghost**: egui's built-in `dnd_drag_source` ghost for library items (translucent label follows cursor)
- **Drop highlight**: Target channel/chain highlights with cyan border when a valid payload hovers. Effect chains show "🔮 Drag effects here" placeholder when empty, with highlighted border during drag. Remaining space after existing effects is always a valid drop target.
- **Reorder drop zones**: Thin (8px) vertical zones between effect cards highlight cyan when the pointer is over them during an `EffectDrag`
- **Invalid drop**: No highlight — item returns to library on release

### Effect Chain Reordering — IMPLEMENTED

Only the ⠿ grip handle on each effect card initiates a drag (using `Label::sense(Sense::drag())` + manual `DragAndDrop::set_payload`). The rest of the card (delete button, checkbox, sliders) remains freely interactive — no drag cursor on the full card.

Actions emitted:
- `effect_to_move: (ch_idx, deck_idx, from_idx, to_idx)` — deck effect reorder
- `ch_effect_to_move: (ch_idx, from_idx, to_idx)` — channel effect reorder
- `master_effect_to_move: (from_idx, to_idx)` — master effect reorder

### Right Sidebar (After Move)

With generators/effects/media moved to the left, the right sidebar becomes:
- Main Output Preview
- Master Effects (label, editing in bottom bar)
- 🎛 Modulation (stays — global control, not content)
- 🗺 Stage Layout + Surface Editor
- 📺 Outputs
- 🎹 MIDI

### State

- `library_panel_open: bool` — defaults to `true`, persisted in scene save
- No library panel when stage editor is open? **Open question** — maybe collapse automatically or keep visible

### Keyboard Shortcut

- `L` — toggle library panel open/closed (only when not typing in a text field)

## Decisions

- **Modulation stays on right**: Modulation sources are global control, not content. They belong with MIDI/outputs/routing. ✅ User decision.
- **Default open**: Library panel starts open since it's a primary workflow tool. ✅ User decision.
- **Drag-and-drop over buttons**: Current shader browser uses per-channel `+` buttons. Drag-and-drop is more natural for a library workflow and matches Resolume's UX. Buttons can coexist as fallback but drag is primary.

## Open Questions

- Should the library panel remember which sections are expanded/collapsed across sessions?
- Should there be a search/filter box at the top of the library panel?
- What does the "Audio" section contain? Audio files for playback? Audio-reactive source presets?
- Should the library persist a collection of user media (images/videos) or always use file dialogs?
