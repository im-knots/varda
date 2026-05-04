# Deck Source Types

## Status: IMPLEMENTED

Relates to: [/intent/target-users.md](/intent/target-users.md), [/spec/resolution-and-scaling.md](/spec/resolution-and-scaling.md)

## Overview

A Deck is an independent render unit that outputs a texture. The **source** is what generates the initial visual content before the deck's effect chain is applied. Varda supports multiple source types.

## Source Types

### 1. ISF/GLSL Shader Generator
- **What**: An ISF shader with no `inputImage` — generates visuals from scratch
- **Resolution**: Renders natively at deck texture resolution (resolution-independent)
- **Parameters**: Extracted from ISF metadata, exposed in UI, controllable via MIDI/OSC
- **Examples**: Plasma, noise fields, audio spectrum visualizers, fractal generators
- **Status**: IMPLEMENTED

### 2. Video File
- **What**: Video file (MP4, MOV, AVI, etc.)
- **Resolution**: Decoded at native resolution, scaled to deck texture (see resolution-and-scaling.md)
- **Codec support — two paths**:
  - **HAP path** (preferred for VJ content): HAP / HAP Alpha / HAP Q / HAP Q Alpha. These encode frames as GPU-compressed textures (S3TC/BCn). The CPU reads compressed blocks from the container and uploads directly to the GPU — near-zero CPU decode cost. This is the standard codec for professional VJ content.
    - **HAP** → `BC1` (DXT1) — no alpha, smallest files
    - **HAP Alpha** → `BC3` (DXT5) — with alpha channel
    - **HAP Q** → `BC7` — higher visual quality, no alpha
    - **HAP Q Alpha** → `BC7` + separate `BC3` alpha plane — best quality with alpha. **This is the target default for Varda.**
  - **ffmpeg path** (fallback for everything else): H.264, ProRes, VP9, etc. Decoded to RGBA on the CPU via ffmpeg-next, then uploaded to a regular `Rgba8Unorm` texture. Higher CPU cost, but handles any codec.
- **HAP implementation notes**:
  - Container: MOV or AVI with HAP FourCC (`Hap1`, `Hap5`, `HapY`, `HapA`)
  - Demux the container to get compressed texture data per frame (not decoded pixels)
  - Upload as `wgpu::TextureFormat::Bc1RgbaUnorm` / `Bc3RgbaUnorm` / `Bc7RgbaUnorm` depending on variant
  - Requires `wgpu::Features::TEXTURE_COMPRESSION_BC` (supported on all desktop GPUs)
  - HAP Q Alpha has two texture planes (BC7 color + BC3 alpha) — combine in a shader pass or sample both in the blit shader
  - The `snappy` crate handles HAP's optional Snappy decompression of the compressed blocks
- **Playback Controls** (all required):
  - **Loop modes**: Loop, ping-pong (forward then reverse), one-shot, hold last frame
  - **Speed control**: Arbitrary speed multiplier (0.5×, 1×, 2×, etc.), reverse playback
  - **Beat-sync**: Lock video playback speed to detected BPM so loops align with music
  - **Scrub/seek**: Seek to arbitrary position, mappable to MIDI knob
  - **In/out points**: Define a sub-range of the video to play (loop within segment)
- **Status**: IMPLEMENTED (HAP GPU-native path with all variants: BC1/BC3/BC7/YCoCg/BC4. HAP Q Alpha dual-plane decode + YCoCg→RGB conversion shader. Playback controls: loop/ping-pong/one-shot/hold-last, speed multiplier, in/out points, seek. Video loading via file dialog in UI. ffmpeg CPU decode fallback for all other codecs. Not yet implemented: beat-sync.)

### 3. Image / Still
- **What**: Static image file (PNG, JPG, BMP, TIFF, etc.)
- **Resolution**: Loaded at native resolution, scaled to deck texture
- **Parameters**: Scaling mode, position offset (future: pan/zoom animation)
- **Use cases**: Logos, backgrounds, texture overlays, photo slideshows
- **Priority**: Implement from the start — simple and immediately useful
- **Status**: IMPLEMENTED

### 4. Webcam / Capture Device
- **What**: Live camera feed (V4L2 on Linux, AVFoundation on macOS)
- **Resolution**: Captured at device resolution, scaled to deck texture
- **Parameters**: Device selection, scaling mode, mirror/flip
- **Status**: NOT IMPLEMENTED

### 5. Solid Color
- **What**: Flat color fill — useful as a base layer or for testing effects
- **Resolution**: N/A (trivial to render at any size)
- **Parameters**: Color picker (RGBA)
- **Status**: IMPLEMENTED

### 6. NDI Stream (Stretch Goal)
- **What**: Network video stream from another application or machine
- **Resolution**: Received at stream resolution, scaled to deck texture
- **Parameters**: Source selection (discovered via NDI), scaling mode
- **Status**: NOT IMPLEMENTED

### 7. Screen Capture (Stretch Goal)
- **What**: Capture another application's window or a screen region
- **Resolution**: Captured at source resolution, scaled to deck texture
- **Parameters**: Window/region selection, capture rate
- **Status**: NOT IMPLEMENTED

## Source → Deck Pipeline

Regardless of source type, the flow is always:

```
[Source] → [Scale to deck resolution] → [Deck FX chain] → [Deck output texture]
```

The deck output texture is always at the stage render resolution. This means:
- Channel compositing doesn't need to handle mixed resolutions
- Effect chains always operate at a known, consistent resolution
- The scaling step is source-type-specific but the rest of the pipeline is uniform

## Effect Shaders (Filters)

Separate from sources, ISF **filter** shaders (those with `inputImage`) are used in effect chains at all three levels:
- Deck FX: applied to the deck's source output
- Channel FX: applied to the composited channel output
- Master FX: applied to the final mixed output

Filter shaders are NOT deck sources — they transform existing textures rather than generating new ones.

## Open Questions

- Should we support "source stacking" within a single deck? (e.g., video + shader overlay before FX chain) Or is that what multiple decks in a channel are for?
- Image slideshow mode — auto-advance through a folder of images on a timer/beat?
- GIF support? (Animated GIFs as a source — decode as video or frame sequence?)

