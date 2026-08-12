# Arrangement Mode

Performance mode is a mixer: channels stacked vertically, decks inside them, everything played by hand. **Arrangement mode is the same scene turned ninety degrees**, with show time running left to right. A channel becomes a group, a deck becomes a lane, and you lay out when each deck is visible instead of bringing it up on a fader.

Nothing is imported, copied, or converted. There is one scene, and these are two views of it. A change you make in either shows up in the other immediately.

Click **▤ Arrange** in the top bar to switch, and **🎛 Perform** to switch back. The arrangement keeps driving decks either way, so switching views mid-show is safe.

## What Changes and What Doesn't

Only the **central mixing area** is replaced. The library on the left, the detail bar along the bottom, and the right panel all stay exactly where they were:

- Selecting a lane selects its deck, so the bottom bar edits generator parameters, effect chains, and playback for the deck you clicked in the timeline.
- Dragging a generator from the library onto a **group row** creates a deck in that channel, and therefore a lane, the same way dropping onto a channel column does.
- Modulators, tonemapping, surfaces, and outputs stay reachable, so you can build an LFO while looking at the timeline.

If you want a wider timeline, collapse the library with **L** and the right panel with **«**.

## Anatomy

| Part | What it is |
|------|------------|
| **Transport strip** | Play/pause, stop, the cue arrows, the position readout, snap, zoom, and the idle picker. |
| **Ruler** | Show position in timecode. Click or drag it to locate, double-click to drop a cue. |
| **Playhead** | The vertical line at the current position, colour-coded by transport status. |
| **Cue point** | A yellow dot on the ruler with a dashed line down the lanes, marking a moment worth returning to. |
| **Group row** | One channel. Click it to select the channel; it is also the library drop target. |
| **Lane** | One deck, with its regions. Click it to select the deck, or drag its header to reorder. |
| **Automation row** | One automated parameter, drawn as a curve under whatever owns it. |
| **Master row** | The mixer, below every channel, holding master effect automation. |

The transport strip duplicates the top bar readout rather than replacing it. Both push the same commands, and both stay visible in both modes. See [Transport](06-control-surfaces.md#transport).

### Navigating

| Gesture | Result |
|---------|--------|
| **Scroll** | Pan along the timeline. |
| **Cmd/Alt + scroll** | Zoom about the pointer, so whatever you are looking at stays under the cursor. |
| **+ / −** | Zoom in and out from the transport strip. |
| **Click or drag the ruler** | Locate the transport. |
| **⏮ / ⏭** | Jump to the previous or next cue point. |
| **⏹** | Stop, holding the position. Press it again to return to the start. |

While the transport is chasing external timecode, position belongs to the master: the ruler and the transport buttons are disabled and say so on hover.

### Cue points

A **cue point** marks a moment worth returning to: the drop, the encore, the bit that never quite lands in rehearsal. Double-click the ruler to drop one where you clicked, and the arrows either side of stop walk the list.

| Gesture | Result |
|---------|--------|
| **Double-click the ruler** | Drop a cue there, named `Cue 1`, `Cue 2`, and so on. The playhead lands on it. |
| **Drag a cue's dot** | Move it, snapped like every other edit. |
| **Right-click a cue** | Rename it (Enter commits) or delete it. |
| **⏮ / ⏭** | Jump backwards or forwards through the cues. |

Back with no earlier cue returns to the start, so there is always a way home. Forward past the last cue stays where it is.

Pressing an arrow repeatedly walks the list, including while the show is playing: each press steps from where the last one landed rather than from wherever playback has carried the playhead since. Let it play on into the next cue, or move the playhead yourself by scrubbing, locating, or stopping back to the start, and the next press picks up from the playhead again.

Cues are saved with the scene, and the arrows are engine commands rather than buttons, so a MIDI foot switch or `POST /api/transport/cue/next` walks a show the same way.

#### Cue pads in Performance mode

Every cue is also a pad in Performance mode, in a bank two buttons wide under the mixer and the macros, in the order the ruler draws them. Pressing one takes the show to that cue and leaves the transport as it was, running or stopped, so a pad is a way to go somewhere rather than a way to start. The bank appears with the first cue and is absent before that.

The pads are the same cues, not copies of them: rename a cue on the ruler and its pad is renamed, delete it and the pad goes. To map one to a controller, turn on MIDI learn (right-click empty space), click the pad, then move the control you want, exactly as you would map a fader. The mapping is stored against the cue, so moving the cue later keeps it. `POST /api/transport/cue/{uuid}`, `/varda/cue/<uuid>/fire`, and a mapped note all do the same thing.

While the transport is chasing timecode the pads are greyed out, because the position belongs to the timecode master.

### Reordering decks

Drag a lane by its header (the name at the left, not the track) to move that deck up or down inside its channel. A line shows where it will land, and the order is the same order the mixer shows, so a deck moved here has moved in Performance mode too, and the other way around. Deck order is composite order within a channel, so this changes what draws on top of what.

Dragging a lane onto a different channel's lanes does nothing, and no drop line appears. Moving a deck to another channel is a Performance mode gesture, because there the target is the channel itself rather than a position between two lanes.

### Copying a deck here copies its placement

Right-click a lane header for the same **Copy**, **Duplicate**, and **Paste** the mixer offers, with one difference that matters: a copy made in Arrangement mode carries the deck's regions, so the copy plays at the same times as the original and can be dragged from there. The same deck copied in Performance mode arrives as a bare deck with no lane, because in the mixer a deck is a source and here it is a source and a placement. See [Copy and Paste](02-concepts.md#copy-and-paste).

## Regions

A **region** is a span during which a deck is visible. It is not a container for content. The deck exists in the scene whether or not a region covers it, and a region only says *when*.

Under the hood a region compiles to breakpoints on that deck's opacity curve: fade in, full, fade out, zero. Two regions overlapping in sibling lanes are therefore a crossfade, using the blend mode already set on those decks. Nothing extra is needed to express one.

| Gesture | Result |
|---------|--------|
| **Drag across empty track** | Author a region between where you pressed and where you released. |
| **Double-click empty track** | Drop a four-second region at that position. |
| **Drag a region** | Move it, keeping its length. |
| **Drag either edge** | Resize it. |
| **Drag a fade handle** (top corners) | Set the fade in or fade out. |
| **Right-click a region** | Delete region, or clear fades. |

A single click selects the lane's deck rather than creating anything, so the bottom bar follows you around the timeline as you work.

The edges are forgiving: a press a few pixels outside a region still grabs its edge rather than starting a new region on the empty track next to it. The pointer tells you which gesture you are about to get before you commit to it, so watch for the horizontal arrows. Two regions closer together than that split the space between them, so you always resize the edge you are nearest.

### Snapping

**Snap** rounds every edit to a whole frame at the show's timecode rate, and is on by default. Turn it off for continuous positions.

Snapping applies to the *gesture*, never to what is stored: positions are continuous everywhere in Varda, so changing the show's frame rate re-labels the ruler without moving a single region. See [Frame rates](06-control-surfaces.md#frame-rates-and-drop-frame).

## Automation Lanes

Curves are created from the parameter, not from the timeline: open the **`〰`** dropdown on any modulatable parameter and pick **＋ Automation lane**. The curve then appears as a row under whatever owns that parameter, so you always know where to look for it:

| Automated parameter | Row appears |
|---------------------|-------------|
| A deck's own parameters, or one of its effects | Under that deck's lane, folded away until you unfold it |
| A channel effect | Under that channel's group header |
| A master effect | Under the **Master** row at the bottom |

Effect parameters are labelled `effect · parameter`, so two effects sharing a parameter name stay apart. See [Automation Curves](05-modulation.md#automation-curves) for what a curve does to a value.

A segment between two breakpoints is a shape, not just a straight line. Grab the line itself (the pointer turns into a vertical arrow) and drag it to bend the segment, so a move can start slowly and arrive fast or the other way around. Drag toward the way you want it to bulge, on rising and falling segments alike. Right-clicking the breakpoint the segment leaves and picking **Linear** straightens it again, and **Smooth** or **Hold** replace the bend with those shapes.

The crossfader cannot be automated yet: it is mappable and macro-drivable, but it is not a modulation target, so there is no curve to draw. Author a crossfade as two overlapping regions in sibling lanes instead, which is the form the arrangement prefers anyway.

| Gesture | Result |
|---------|--------|
| **Drag a breakpoint** | Move it in time and value. It cannot cross its neighbours. |
| **Drag the line between two breakpoints** | Bend that segment. Drag toward the direction you want it to bulge. |
| **Double-click empty curve** | Add a breakpoint there. |
| **Double-click a breakpoint** | Remove it. |
| **Right-click a breakpoint** | Choose Linear, Smooth, or Hold, or delete it. |
| **Right-click a lane, or its header** | Copy this shape, or paste the copied one onto it. |
| **Cmd+C / Cmd+V** | The same copy and paste, on the lane you last clicked. |
| **Right-click the lane header** | Also removes the automation lane entirely. |
| **▾ caret** on a deck | Fold that deck's curves away, or unfold them. |

The deck's own opacity curve is not offered as an editable row, because it is authored by dragging regions and hand edits to it would be overwritten by the next region edit.

Curves do not appear as cards in the right panel's modulation list. A show can have hundreds of them and that list is built for a handful of live modulators.

### Reusing a shape

A curve drives the one parameter it was drawn for, so it is not offered in the `〰` dropdown as a source you can assign somewhere else. To put the same shape on a second parameter, right-click the lane you like and pick **Copy curve**, then right-click the other parameter's lane and pick **Paste curve**. Both items are in the menu wherever you right-click the lane, on a breakpoint or on bare curve. The shape lands where you right-clicked, keeping its own length and replacing whatever it covers. Pasting from the lane header instead, or with the keyboard, lands it at the playhead, since neither of those points at a moment in the show. The keyboard does the same thing: click a lane to select it, then Cmd+C and Cmd+V.

The two lanes are independent from that moment on. Editing one never moves the other, which is the point: a shared source would mean a tweak for one parameter silently rewriting the other, and you would only find out during the show.

## Who Is Driving: Authority and Override

The arrangement takes control **per lane**, and only once the transport has actually run. Before you press Play the scene renders exactly as you saved it, so an arrangement you have not started can never black your output.

A lane with regions or curves is arrangement-controlled. Everything else in the scene stays live, so a show can be half arranged and half performed without choosing between the two.

### Grabbing something back

**Touch a control the arrangement is driving and you win, immediately.** A fader drag, a MIDI knob, an OSC message, or an API write all suspend the arrangement's control of *that parameter only*. There is no confirmation, because there is no time for one.

An overridden lane shows an amber dot in its header, in both views, so you can see from anywhere that it is no longer following the show.

### Handing it back

Click the amber dot to re-arm that one parameter, or **↻ Re-arm all** in the transport strip to hand everything back at once. The button only appears while something is held, and carries a count.

Re-armed parameters **ramp** back to their automated value rather than snapping to it. Jumping to the right value is correct arithmetic and a visible glitch, and this happens in front of an audience.

Overrides are session state and are **never saved**. Reloading the scene restores full arrangement control, because a saved override is an invisible trap that breaks the show the next time the file opens.

### Performance sequencers while the arrangement runs

- **Deck auto-transitions** are per deck, so they partition cleanly. A deck under arrangement control has its auto-transition suspended; a deck without regions keeps it.
- **Transition sequences** cross channels, so they cannot partition. While the arrangement holds authority, starting a free-running sequence is refused with a reason, and one already running is stopped.

## Idle Behaviour

**Idle** in the transport strip decides what renders before the transport reaches the arranged range:

| Setting | Behaviour |
|---------|-----------|
| **Hold performance** | The mixer holds. The arrangement stays inert until the show reaches it. Default. |
| **Show `<deck>`** | That deck plays until the arranged range starts. |

"Run this loop until the schedule starts" is a normal installation requirement, and a pre-show state that is simply black looks exactly like a broken rig on a dark stage. Pick one deliberately.

If the arrangement ever drives everything to zero, Varda tells you once rather than leaving you to wonder whether the output died.

## What Regions Cannot Do

A lane is a deck, and a deck holds one source. Loading a different video into a deck, recalling a preset, or firing a sequence are **events, not spans**, so they are not regions. Today, put "shader A then shader B" in two lanes and overlap them, which is also how you get a crossfade between the two.

## Undo, Saving, and Load

- Every timeline drag is **one** undo entry. Cmd+Z returns the region or breakpoint to where it was before you started dragging.
- The arrangement is saved in `scene.json` with everything else, because every lane is a deck in that scene and every curve is a modulation source in it. There is no separate arrangement file to keep in sync.
- This is **scene version 7**. Older scenes open unchanged, but a scene containing an arrangement will not open on a build older than this one.
- Decks stay in memory for the whole show, so a long arrangement holds all of them at once. The monitoring cluster at the bottom of the right panel shows the deck count and an estimate of the colour-target memory they hold. See [Performance Monitoring](10-resolution-and-monitoring.md#performance-monitoring).

## Sleeping Clips

A video whose next region is minutes away has nothing to show, so Varda **stops decoding it** and wakes it a second before it is needed. Sixty video decks in a two-hour show would otherwise run sixty decoders all evening to serve clips nobody is looking at.

This changes one thing you can see, and it is worth knowing: **a sleeping clip freezes rather than playing on silently.** When its region arrives it resumes from where it paused, instead of being wherever wall-clock time carried it. That is the behaviour an arrangement wants (the same show position looks the same on the second run), but it will surprise you once if you are used to a clip free-running behind a closed fader. Its transport still reads as playing, because sleep is not pause and re-arming will not restart a clip you paused by hand.

Nothing sleeps unless the arrangement is confident:

- Performance mode never sleeps anything, and neither does an arrangement whose transport has not run.
- A deck with an LFO, an audio band, or any other live modulator on its opacity keeps decoding, since it can come up at any moment.
- A deck you have grabbed by hand keeps decoding until you re-arm it.
- A deck in a cued channel, or one feeding a program tap, keeps decoding so its off-air view stays live.
- Outside the arranged range with **Hold performance**, the arrangement has said nothing, so nothing sleeps.

Cameras and screen captures follow the same schedule: their frames come up ahead of the region that needs them. Live network sources (NDI, SRT, Syphon), HTML decks, and depth sensors never sleep, because a dropped connection or a lost page costs more than it saves.

Memory is untouched: a sleeping deck still holds its render targets, which is why the VRAM readout does not move when one goes to sleep.

## HTTP API

Everything above is reachable without the UI, which is how you build an arrangement from a script or drive one from a show controller:

```bash
# Give a deck a lane and a visible span
curl -X POST http://localhost:8080/api/arrangement/lanes/<deck_uuid>/regions \
  -H "Content-Type: application/json" \
  -d '{"start": 12.0, "end": 48.0, "fade_in": 1.0, "fade_out": 2.0}'

# Hand every held parameter back to the arrangement
curl -X POST http://localhost:8080/api/arrangement/rearm \
  -H "Content-Type: application/json" -d '{}'

# Mark a moment, then walk to it
curl -X POST http://localhost:8080/api/arrangement/cues \
  -H "Content-Type: application/json" -d '{"at": 64.5, "name": "Drop"}'
curl -X POST http://localhost:8080/api/transport/cue/next
```

Transport control (`/api/transport/play`, `/locate`, `/loop`, `/rate`, `/source`) and curve editing (`PUT /api/modulation/<uuid>/breakpoints`) live with their own subsystems. See [HTTP API](13-api.md#route-groups).

---

[← Prev: Frame Analysis & Preprocessors](14-frame-analysis.md) · [Home](README.md)
