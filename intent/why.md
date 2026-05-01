# Why Varda Exists

## The Problem

Professional VJ software (Resolume, TouchDesigner, Synesthesia, MadMapper) is:

- **Closed source** — no ability to extend, audit, or fix
- **Paid / limited free tiers** — barrier to entry for emerging VJs
- **Not Linux native** — VJs who want to run Linux are locked out
- **Vendor-locked** — proprietary formats, no interop

VJs are technical people. Many would prefer Linux-native, open-source tools if they existed. They don't — yet.

## The Intent

Varda is a **Linux and macOS native VJ performance tool** written in Rust. It aims to be a credible Resolume replacement for shader-driven, audio-reactive visual performance.

It is **not** trying to be TouchDesigner (node-based creative coding) or After Effects (timeline editing). It is a **live performance tool** — decks, mixing, effects, audio reactivity, output.

## Core Beliefs

1. **ISF/GLSL is the lingua franca of VJ shaders** — Varda uses ISF natively, not a proprietary format
2. **The GPU should do the work** — CPU stays out of the render path
3. **Open source VJ tools should exist** — the community deserves them
4. **Rust is the right language** — memory safety, performance, no GC pauses during live performance
5. **Linux deserves first-class creative tools** — not ports, not afterthoughts

## Name

Varda — the Vala of light and stars in Tolkien's legendarium. She who kindled the stars.

