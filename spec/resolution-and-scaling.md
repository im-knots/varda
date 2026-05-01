# Resolution & Scaling

## Status: IMPLEMENTED

## The Problem

Varda needs to handle mixed-resolution content gracefully. A single stage might contain:

- An ISF shader that renders at any resolution natively (resolution-independent)
- A 1080p video clip
- A 720p webcam feed
- A 4K image

These all need to composite together into a single output at the user's chosen resolution.

## User-Configurable Output Resolution

The user sets the **stage render resolution** (the final output size). Common choices:
- 1280×720 (720p)
- 1920×1080 (1080p) — default
- 2560×1440 (1440p)
- 3840×2160 (4K)
- Custom (arbitrary width × height, for LED walls, unusual aspect ratios)

This is a global setting, not per-channel or per-deck.

## Per-Deck Resolution Strategy

### Decision: Scale at the deck level

Each deck renders to a texture at the **stage resolution**, regardless of source content resolution. Scaling happens when the source content is loaded into the deck:

```
Source (any resolution) → [Scale/Fit to deck texture] → Deck FX → Channel mix → Master FX → Output
```

### Scaling Modes (per-deck setting)

When source content doesn't match the stage resolution:

1. **Fill** (default) — scale to fill the entire deck texture, cropping edges if aspect ratio differs
2. **Fit** — scale to fit within the deck texture, letterboxing/pillarboxing if aspect ratio differs
3. **Stretch** — stretch to exactly match deck texture dimensions (distorts if aspect ratio differs)
4. **Center** — no scaling, center the source at native resolution, black borders if smaller, crop if larger

### Why Scale at Deck Level

- **Shaders are resolution-independent**: ISF generators render directly at stage resolution — no scaling needed
- **Video/images need scaling once**: Scale on load/decode, not every frame during compositing
- **Consistent pipeline**: Every deck outputs the same resolution texture, simplifying channel compositing and effect chains
- **Effect chains work at consistent resolution**: Deck FX, Channel FX, and Master FX all operate at stage resolution

### Shader Sources

ISF/GLSL shaders receive `RENDERSIZE` as a uniform and render at whatever resolution the deck texture is. No scaling needed — they're inherently resolution-independent. This is one of the advantages of a shader-first VJ tool.

### Video Sources

Video frames are decoded at their native resolution and then scaled to the deck texture size using the selected scaling mode. This happens on the GPU via a blit/sample operation — not CPU-side scaling.

### Image Sources

Same as video — loaded at native resolution, scaled to deck texture via GPU blit.

## Open Questions

- Should we support per-deck render resolution overrides? (e.g., render a complex shader at half-res for performance, then upscale)
- What filtering mode for scaling? Bilinear (fast) vs. Lanczos (quality)? User-selectable?
- How does resolution interact with multi-output? (Different outputs at different resolutions?)

