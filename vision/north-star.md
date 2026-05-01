# Varda — North Star Vision

## One-Liner

A Linux-native, open-source VJ performance tool that a working VJ could use instead of Resolume.

## What "Done" Looks Like

A VJ walks into a venue with a Linux laptop. They open Varda. They:

1. Load a scene with multiple channels — each loaded with generative shaders, video clips, a webcam feed
2. Add channels on the fly as the set evolves — the mixer grows to match, deck grids scale with the window
3. Mix between channels with per-channel opacity, blend modes, and a crossfader (for 2-channel setups) or compositor-style mixing (for 3+)
4. Drop ISF shaders from the community library onto decks as effects
5. Audio reactivity kicks in automatically — bass drives one deck, treble drives another
6. They tweak parameters live via MIDI controller, mapped through the UI
7. Output goes to a projector via NDI or direct display output
8. They save the scene, pack up, do it again tomorrow at a different venue

That's the target. Everything else is in service of this workflow.

## Primary Inspirations

| Software | What We Take | What We Leave |
|---|---|---|
| **Synesthesia** | Shader-first workflow, ISF ecosystem, audio reactivity model | Closed source, macOS-only |
| **Resolume** | Deck/layer/stage mental model, blend modes, MIDI/OSC control | Windows/macOS only, proprietary formats, price |
| **TouchDesigner** | Node flexibility, real-time performance | Complexity, not VJ-focused, proprietary |
| **MadMapper** | Projection mapping, clean UI | Narrow focus, closed source |

## Non-Goals (For Now)

- **Node-based programming** — Varda is a performance tool, not a creative coding environment
- **Timeline editing** — this is live, not post-production
- **3D scene rendering** — shaders operate on 2D textures (for now)
- **Windows support** — Linux and macOS first; Windows can come later via wgpu/Vulkan
- **Mobile** — desktop performance tool only

## Success Criteria

- A VJ who currently uses Resolume can switch to Varda for a shader-heavy set
- ISF shaders from the community work without modification
- 60fps at 1080p with 4+ active decks across multiple channels on mid-range hardware
- Audio reactivity feels responsive and musical, not laggy
- MIDI/OSC control works with standard VJ controllers
- The UI adapts fluidly to the performer's needs — add channels, resize the window, and everything reflows

