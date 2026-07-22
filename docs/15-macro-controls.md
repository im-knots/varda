# Macro Controls

A **macro** is a performance control you build yourself — a **knob**, **fader**, or **button** — that drives many parameters at once. Turn one knob and two effect parameters on two different decks move together; press one button and a whole look snaps into place. Each macro is also a mappable parameter in its own right, so a single hardware knob mapped by MIDI learn drives the macro, and the macro drives everything wired to it.

Macros live in the **central mixer column**, stacked directly below the mixer box and the transition sequence builder — the same place you reach for the crossfader. Each macro shows as a small **live control** (knob, fader, or button) you play with right there in the column. This works just like the sequence builder: **play the control in place, and click anywhere around it** (its card) to open the macro's full configuration in the bottom bar.

---

## Creating a Macro

Below the macro controls in the central column are three centered add buttons:

- **＋ Knob** — a rotary knob (drag up/down to sweep 0–1)
- **＋ Fader** — a linear 0–1 slider (identical behavior to a knob; pick whichever matches your mental model)
- **＋ Button** — an on/off control with three press behaviors (see [Buttons](#buttons))

Each new macro is named `Macro 1`, `Macro 2`, … with an accent color from the shared palette (the same colors used by modulation sources). Its compact widget shows a color dot, the name, an **x** delete button, and the interactive live control. Clicking **x** removes the macro immediately (like deleting a transition sequence).

The central column — and every channel column — scrolls vertically when its stack of decks, sequences, and macros is taller than the panel, so adding many macros never pushes content off-screen.

**Click the card around the control** to select it — the bottom bar switches to that macro's detail editor, showing a larger control, a color dot, an editable **name** field, a **kind** selector, a **🗑 Delete** button, an **x Close** button, and the target (or trigger) editor. This mirrors how clicking a deck, channel, or sequence fills the bottom bar. (Dragging the knob/fader or pressing the button plays it live and does *not* open the editor.)

Rename a macro by typing in its name field in the detail editor — the new name is committed when the field loses focus. Change a macro's kind at any time with the kind dropdown; switching to **Button** adds button behavior options, switching away removes them.

---

## Binding Targets

A **target** is any mappable parameter the macro should drive. Select the macro (click its name) to open its detail editor in the bottom bar, then:

1. Open the **＋ Add target** dropdown in the detail editor.
2. Pick a parameter from the list. It is grouped and labeled by location — for example `Deck 1 · Blur · radius`, `Ch 0 · opacity`, `Master · Glow · intensity`, `Crossfader`, or a **modulator parameter** such as `LFO 1 · frequency`, `ADSR 2 · release`, or `StepSeq 1 · rate`. Driving a modulator param lets a macro reshape a modulation source (e.g. sweep an LFO's rate) as well as deck/channel/effect params.
3. The target is added instantly with defaults (`min 0.0`, `max 1.0`, `Linear` curve, not inverted) and starts following the macro immediately.

Every target has its own mapping row so a single macro can push each parameter through a different range and shape:

| Control | Meaning |
|---------|---------|
| **min** / **max** | The slice of the parameter's full range the macro sweeps. `min 0.2, max 0.9` uses only that portion. Setting **min greater than max** inverts the response. |
| **inv** | Invert the response (equivalent to swapping min/max) — the target *falls* as the macro *rises*. Use it to open one effect while closing another from one gesture. |
| **curve** | The response shape applied before mapping into `[min, max]`: **Linear**, **Exp** (ease-in, slow start), **Log** (ease-out, fast start), **S-Curve** (ease-in-out), or **Stepped** (quantize into discrete levels — great for stutter/enum-like params). |
| **x** | Remove the target. |

There is no limit on targets per macro, and the same parameter can be a target of more than one macro (last gesture wins, exactly like two MIDI CCs mapped to one parameter).

> **The motivating example.** To control two effect parameters on two decks with one knob: add a Knob macro, add target `Deck A · FX1 · scale`, then add target `Deck B · FX2 · warp` and tick **inv** on the second. Now one knob turn opens the first effect while closing the second.

### Macros and modulation compose

A macro sets a parameter's **base** value; the modulation engine adds its offset **on top** every frame. So a parameter can be both macro-driven and modulated at once — the macro sets where the value sits, modulation animates around it. They never conflict.

A macro **cannot** target another macro (loop prevention). Target the underlying parameters directly instead.

---

## Modulating a Macro

Beyond driving targets *by hand*, a **Knob** or **Fader** macro can itself be driven by a **modulation source** (LFO, ADSR, audio, step sequencer). Assign one and the modulator sweeps the whole macro — and through it every mapped target — automatically. This is the "one gesture animates everything" move: bind an LFO to a macro that drives five parameters, and all five breathe together.

In the macro's detail editor (bottom bar), the **Mod** section lists each assigned source on its own row (color dot + name + an **x** to remove just that source) with an **＋ Modulate** dropdown below to add more:

1. Pick a source from **＋ Modulate** to assign it — it appears as a new row in the source's color. Assign several to stack them (their offsets sum).
2. The value label then reads `value 0.50 → 0.73` — the first number is your manual set point (the **base**), the second is the live **effective** value being fanned out. The **control itself** also shows a colored **ghost** marker at the effective value (the knob's ghost pointer / the fader's ghost line), so a modulated macro visibly tracks its source — just like a modulated param slider. The base pointer stays where you set it.
3. Click a row's **x** to remove that one source (the others keep driving the macro).

Modulation rides *on top* of the base: turn the knob to move where the sweep is centered, exactly like a modulated effect parameter. Each mapped target still applies its own min/max/curve/invert to the modulated value, so one LFO can open one effect while closing another.

- Only **Knob/Fader** macros can be modulated; **Button** macros cannot (they're discrete).
- Modulators are created in the **Modulation** panel (see [Modulation](06-control-surfaces.md)); any source there is assignable to a macro.
- Macro modulation assignments are **per-scene** (saved in `scene.json`) and **undoable**.

> Tip: you can *also* modulate a macro's individual target parameters directly from their own deck/effect panels. Modulating the macro animates all targets together; modulating a target animates just that one.

---

## Buttons

A Button macro has a behavior selector with three modes:

| Behavior | Press | Release |
|----------|-------|---------|
| **Momentary** | drive all targets to their **max** | drive all targets back to **min** |
| **Toggle** | latch on/off — each press flips targets between **max** and **min** | (ignored) |
| **Trigger** | fire one-shot **actions** once, on the press | (ignored) |

The behavior selector lives in the macro's detail editor (bottom bar). Momentary and Toggle buttons use the same **target** list as knobs and faders. A **Trigger** button instead shows an **On press** editor:

- **Undo / Redo / Save** checkboxes — fire the corresponding global app action on press.
- **＋ Add param** — add a parameter action that writes a fixed value (`1.0`) to a path on press, e.g. `deck/<uuid>/trigger` to snap a deck to full opacity. Remove one with **x**.

> Trigger buttons are fire-and-forget — perfect for mapping a pad to Undo, a "reset" snapshot, or a deck slam.

---

## Mapping a Macro to MIDI / OSC / Keyboard

Because a macro is addressable as `macro/<uuid>/value`, it inherits Varda's whole control plane with no extra setup:

- **MIDI** — enter **MIDI Learn** (right-click → *Enter MIDI Learn*), then click a macro's live control in the central column (it glows purple like any other control) and move a hardware control. A button macro maps naturally to a pad: note-on drives `1.0`, note-off drives `0.0`. See [MIDI](06-control-surfaces.md#midi).
- **OSC** — send `/varda/macro/<uuid>/value <0..1>`. Discover the UUID via `GET /api/scene/macros` or `GET /api/state`.
- **Keyboard** — keyboard learn can bind a key to a macro (especially buttons) via the same value path.

---

## Persistence

Macros are **per-scene** and saved in `scene.json` alongside channels, effects, and modulation. Loading a scene restores its macros, values, targets, and button configuration. Older scenes saved before macros existed load normally with no macros.

MIDI mappings to `macro/<uuid>/value` are stored in `.varda/midi.json` like any other mapping. As with deck/effect mappings, a mapping saved against one scene's macro resolves to nothing (logged, no crash) if a different scene is loaded, since macro UUIDs are per-scene.

## Undo / Redo

Macro **configuration** changes — add, remove, rename, change kind, edit a target, change button behavior or triggers — are undoable. **Turning a macro** (its live value) is performance input and is deliberately *not* undoable, consistent with the crossfader, channel opacity, and MIDI live control.

---

## HTTP API

Every macro operation available in the UI is exposed over the [HTTP API](13-api.md) under the **Macros** tag. Read state via `GET /api/state/macros` or `GET /api/scene/macros`.

### Create a macro

```sh
curl -X POST http://localhost:8080/api/macros \
  -H "Content-Type: application/json" \
  -d '{"kind": "Knob"}'
```

Returns the new macro's `uuid`. `kind` is `Knob`, `Fader`, or `Button`.

### Add a target

```sh
curl -X POST http://localhost:8080/api/macros/<uuid>/targets \
  -H "Content-Type: application/json" \
  -d '{"path": "deck/<deck_uuid>/effect/<fx_uuid>/param/scale"}'
```

`path` is any mappable parameter-router path, including **modulator params** such as `mod/<mod_uuid>/frequency`, `mod/<mod_uuid>/rate`, or `mod/<mod_uuid>/step/3` — so the API can bind a macro to a modulator exactly like the UI target picker. (Only `macro/*` paths are rejected, to prevent loops.)

### Shape a target (range / curve / invert)

Targets are addressed by zero-based index in the order they were added:

```sh
curl -X PUT http://localhost:8080/api/macros/<uuid>/targets/0 \
  -H "Content-Type: application/json" \
  -d '{"min": 0.2, "max": 0.9, "curve": "SCurve", "invert": true}'
```

`curve` is `"Linear"`, `"Exponential"`, `"Logarithmic"`, `"SCurve"`, or `{"Stepped": 4}`.

### Drive a macro (live)

```sh
curl -X PUT http://localhost:8080/api/macros/<uuid>/value \
  -H "Content-Type: application/json" \
  -d '{"value": 0.75}'
```

This is equivalent to the shared parameter route `PUT /api/params` with `{"path": "macro/<uuid>/value", "value": 0.75}` — both fan out to every target.

### Modulate a macro's value

Drive a Knob/Fader macro from a modulation source (the modulator's offset rides on top of the manual set point and re-fans to every target each frame):

```sh
# Assign a modulation source to the macro's value
curl -X PUT http://localhost:8080/api/macros/<uuid>/modulation \
  -H "Content-Type: application/json" \
  -d '{"source_id": "<mod_uuid>", "amount": 0.5}'

# Clear ALL modulation on the macro
curl -X DELETE http://localhost:8080/api/macros/<uuid>/modulation

# Clear only one source (leave any others intact)
curl -X DELETE http://localhost:8080/api/macros/<uuid>/modulation/<mod_uuid>
```

### Configure a Button macro

```sh
# Set behavior: Momentary | Toggle | Trigger
curl -X PUT http://localhost:8080/api/macros/<uuid>/button/behavior \
  -H "Content-Type: application/json" -d '{"behavior": "Trigger"}'

# Set trigger actions (fired on press)
curl -X PUT http://localhost:8080/api/macros/<uuid>/button/triggers \
  -H "Content-Type: application/json" \
  -d '{"triggers": [{"Global": "Save"}, {"Param": {"path": "crossfader", "value": 0.0}}]}'
```

### Other operations

| Operation | Request |
|-----------|---------|
| Rename | `PUT /api/macros/<uuid>/name` `{"name": "Sweep"}` |
| Change kind | `PUT /api/macros/<uuid>/kind` `{"kind": "Fader"}` |
| Remove target | `DELETE /api/macros/<uuid>/targets/<idx>` |
| Delete macro | `DELETE /api/macros/<uuid>` |

---

## Parameter Path

| Path | Description |
|------|-------------|
| `macro/<uuid>/value` | Drive a macro (0.0–1.0); fans out to all its targets. MIDI/OSC/keyboard/API mappable. |

See [Parameter Paths](06-control-surfaces.md#parameter-paths) for the full addressing reference.

---

[← Prev: Control Surfaces](06-control-surfaces.md) · [Home](README.md) · [Next: Outputs →](07-outputs.md)
