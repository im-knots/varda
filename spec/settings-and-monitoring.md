# Settings & Performance Monitoring

## Status: NEEDS DESIGN

## Settings Panel

A dedicated settings UI (modal or side panel) for global application configuration. Not per-scene — these are app-level preferences.

### Render Settings
- **Output resolution**: Dropdown or custom input (720p, 1080p, 1440p, 4K, custom)
- **Target framerate**: 30, 60, 120, or uncapped
- **VSync**: On/off
- **GPU backend**: Auto, Vulkan, Metal (informational — may not be switchable at runtime)

### Audio Settings
- **Input device**: Dropdown of available audio devices
- **FFT size**: 512, 1024, 2048, 4096 (tradeoff: frequency resolution vs. latency)
- **BPM detection sensitivity**: Slider
- **Audio reactivity smoothing**: Global smoothing factor

### MIDI Settings
- **MIDI device selection**: Dropdown of available MIDI devices
- **Controller preset**: Akai APC Mini, Novation Launchpad, Generic, Custom
- **MIDI mapping editor**: (see [/spec/midi-control.md](/spec/midi-control.md))

### OSC Settings
- **OSC receive port**: Default 9000
- **OSC send port/host**: For outbound audio data, etc.

### Shader Library
- **Library paths**: List of directories to scan, add/remove
- **Hot-reload**: Enable/disable file watching

### Error Handling Preferences
- **Shader fallback mode**: Black, checkerboard, freeze last frame
- **Auto-save interval**: Off, 1min, 5min, 10min

## Performance Monitoring

### FPS Overlay
- Real-time FPS counter displayed in the UI (toggleable)
- Frame time graph (last N frames) — shows spikes and consistency
- Color-coded: green (>55fps), yellow (30-55fps), red (<30fps)

### Per-Deck Stats
- Render time per deck (how much GPU time each deck consumes)
- Helps identify which deck/shader is the bottleneck

### GPU Info
- GPU name, driver version, VRAM usage (if available via wgpu)
- Current backend (Vulkan/Metal)

### Audio Latency
- Measured latency from audio input to visual response
- Buffer underrun count

## Open Questions

- Should settings be stored in a config file? (XDG on Linux, ~/Library on macOS)
- Should some settings be overridable per-scene? (e.g., output resolution)
- Is there value in a "performance mode" that disables UI previews to save GPU?

