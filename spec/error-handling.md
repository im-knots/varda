# Error Handling & Resilience

## Status: IMPLEMENTED

Relates to: [/intent/why.md](/intent/why.md) (stability is a core requirement for live performance)

## Guiding Principle

**Varda must never crash during a performance.** Any error that can be anticipated must be caught, reported to the user via the UI, and handled gracefully. The show must go on.

## Shader Errors (Handled)

### Problem
A malformed or incompatible ISF shader that fails to compile currently panics the application. This is the most dangerous failure mode — a VJ drops a new .fs file into their library, it has a typo, and the entire app dies mid-set.

### Required Behavior
1. **Shader compilation failure**: Log the error, show a notification in the UI with the error message and line number, skip the shader. The deck/effect slot shows a fallback (black, or the last successfully compiled output).
2. **Shader runtime error** (GPU hang/crash): Harder to catch, but at minimum detect the frame timeout and disable the offending shader. Show notification.
3. **Hot-reload failure**: If a watched shader file is saved with errors, keep the previous working version active. Show a notification that the reload failed and why.
4. **Registry scan errors**: Invalid ISF files found during library scan are logged and marked as broken in the shader browser (visible to user with error details), but scanning continues for other shaders.

### Notification System
- Non-modal notifications (toast/banner) — must not block the render loop or require user interaction
- Notifications auto-dismiss after a few seconds, or can be dismissed manually
- Severity levels: Info, Warning, Error
- Recent notifications accessible in a log/history panel

### Fallback Rendering
When a shader fails, the deck should not go blank without explanation:
- Show a **fallback pattern** (solid color, checkerboard, or "error" text overlay) so the VJ knows something is wrong
- Alternatively, **freeze the last good frame** — keep showing the last successfully rendered output
- User preference for which fallback behavior to use (settings)

## Audio Device Errors

- Audio device disconnected mid-session: Show notification, continue rendering without audio reactivity
- Audio device not found on startup: Show notification, audio features disabled, everything else works
- Device reconnection: Auto-detect and resume audio input without restart

## Video Decode Errors

- Corrupt video file: Show notification, deck falls back to black or fallback pattern
- Unsupported codec: Show notification with codec name, suggest installing codec support
- End of file (non-looping): Hold last frame or go to black, per deck setting

## GPU / Rendering Errors

- Surface texture acquisition failure: Skip frame, retry next frame (already handled in current code)
- Out of GPU memory: Detect and report, suggest reducing deck count or resolution
- Driver crash: Hardest to handle — may require process restart. Document known driver issues.

## File System Errors

- Shader library path doesn't exist: Log warning, skip path, continue with other paths
- Permission denied on shader file: Log warning, skip file, show in browser as inaccessible
- Scene file corrupt/invalid: Show error with details, don't load, keep current state

## Open Questions

- Should there be a "safe mode" that loads with minimal config if the last session crashed?
- Should Varda auto-save session state periodically so work isn't lost on crash?
- How to handle GPU hangs specifically on Linux (Vulkan driver recovery)?

