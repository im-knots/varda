# Arrangement Mode

Arrangement mode shows the scene as a timeline, with show time running left to right. Each channel is a group and each deck is a lane. You place regions on a lane to set when that deck is visible, instead of bringing it up on a fader.

Performance mode and Arrangement mode are two views of the same scene. Nothing is imported, copied, or converted. A change you make in either view shows up in the other immediately.

Click **▤ Arrange** in the top bar to switch to Arrangement mode, and **🎛 Perform** to switch back. The arrangement keeps driving decks in both views, so you can switch views mid-show.

## Shared Panels

Arrangement mode replaces only the **central mixing area**. The library on the left, the detail bar along the bottom, and the right panel stay where they are:

- Selecting a lane selects its deck. The bottom bar then edits generator parameters, effect chains, and playback for that deck.
- Dragging a generator from the library onto a **group row** creates a deck in that channel, and a lane for it, as dropping onto a channel column does.
- Dragging an **effect** onto a lane, group, Master row, or any automation row under them appends it to that owner's effect chain and selects the owner. These are the same drop targets as in Performance mode; see [Library → Drag-and-Drop](03-library-panel.md#drag-and-drop).
- Modulators, tonemapping, surfaces, and outputs stay reachable, so you can build an LFO while looking at the timeline.

For a wider timeline, collapse the library with **L** and the right panel with **«**.

## Anatomy

| Part | What it is |
|------|------------|
| **Transport strip** | Play/pause, stop, the cue arrows, record, the position readout, snap, zoom, and the idle picker. |
| **Focus strip** | The thin band above the ruler. Drag out a range there to mark the stretch you are working on, then loop it. |
| **Ruler** | Show position in timecode. Click or drag it to locate, double-click to drop a cue. |
| **Playhead** | The vertical line at the current position, color-coded by transport status. |
| **Cue point** | A yellow dot on the ruler with a dashed line down the lanes, marking a moment you want to return to. |
| **Group row** | One channel. Click it to select the channel, right-click to copy or delete it. It is also the library drop target. |
| **Lane** | One deck, with its regions. Click it to select the deck, drag its header to reorder, right-click to copy or delete it. |
| **Automation row** | One automated parameter, drawn as a curve under whatever owns it. |
| **Master row** | The mixer, below every channel, holding master effect automation. |

The transport strip duplicates the top bar readout. Both send the same commands, and both are visible in both modes. See [Transport](07-control-surfaces.md#transport).

### Navigating

| Gesture | Result |
|---------|--------|
| **Scroll** | Move up and down the rows. If the scene has more channels than fit on screen, a scrollbar appears down the right edge of the tracks. You can drag it. |
| **Shift + scroll**, or a horizontal wheel | Pan along the timeline. |
| **Pinch**, or **Cmd/Alt + scroll** | Zoom the timescale around the pointer. The point under the pointer stays put. |
| **+ / −** | Zoom in and out from the transport strip. |
| **Click or drag the ruler** | Locate the transport. |
| **⏮ / ⏭** | Jump to the previous or next cue point. |
| **⏹** | Stop, holding the position. Press it again to return to the start. |

While the transport is chasing external timecode, the timecode master controls position. The ruler and the transport buttons are disabled, and their tooltips say why.

### The focus area

Use the focus area to loop a stretch of the show while you work on it. Drag across the thin strip above the ruler, and a blue bar marks that range.

| Gesture | Result |
|---------|--------|
| **Drag empty strip** | Mark a range. Dragging right to left marks the same range as left to right. |
| **Drag the bar's body** | Move the range, keeping its length. |
| **Drag either edge** | Resize it. |
| **Right-click the bar** | **Loop this range**, **Zoom to range**, or **Clear**. |

**Loop this range** sends the range to the transport, which wraps playback inside it. The bar fills in while it is looping. If you move or resize the bar while it is looping, the loop follows, so you can adjust a loop point without stopping.

Clearing the range also stops the loop. Turning the loop off keeps the range marked, so you can keep working on the same stretch without wrapping. **Zoom to range** fills the view with the range. A scene saved with a loop opens with that loop shown as the focus area.

### Cue points

A **cue point** marks a moment you want to return to, such as a drop or an encore. Double-click the ruler to drop one where you clicked. The arrows on either side of stop step through the cues.

| Gesture | Result |
|---------|--------|
| **Double-click the ruler** | Drop a cue there, named `Cue 1`, `Cue 2`, and so on. The playhead moves to it. |
| **Drag a cue's dot** | Move it. The move snaps like every other edit. |
| **Right-click a cue** | Rename it (Enter commits) or delete it. |
| **⏮ / ⏭** | Jump backwards or forwards through the cues. |

⏮ with no earlier cue returns to the start. ⏭ past the last cue does nothing.

Repeated presses step through the list, including while the show is playing. Each press steps from where the previous press landed, not from where playback has moved the playhead since. The next press starts from the playhead again once playback reaches the next cue, or once you move the playhead yourself by scrubbing, locating, or stopping back to the start.

Cues are saved with the scene. The arrows are engine commands, so a MIDI foot switch or `POST /api/transport/cue/next` steps through cues the same way.

#### Cue pads in Performance mode

Every cue also appears as a pad in Performance mode, in a bank two buttons wide under the mixer and the macros, in ruler order. Pressing a pad moves the show to that cue and leaves the transport running or stopped, as it was. The bank appears when you add the first cue.

The pads are the cues themselves. Rename a cue on the ruler and its pad is renamed; delete it and the pad goes. To map a pad to a controller, turn on MIDI learn (right-click empty space), click the pad, then move the control, as you would map a fader. The mapping is stored against the cue, so it survives moving the cue. `POST /api/transport/cue/{uuid}`, `/varda/cue/<uuid>/fire`, and a mapped note all do the same thing.

While the transport is chasing timecode, the pads are greyed out, because the timecode master controls position.

### Reordering decks

Drag a lane by its header (the name at the left, not the track) to move that deck up or down inside its channel. A line shows where it will land. The order matches the mixer, so a move here also shows in Performance mode, and the other way around. Deck order is composite order within a channel, so reordering changes which deck draws on top.

You cannot drag a lane into a different channel; no drop line appears. To move a deck to another channel, drag it onto the channel in Performance mode.

### Copying decks and channels

Right-click a lane header for **Copy**, **Duplicate**, and **Paste**, as in the mixer. A copy made in Arrangement mode includes the deck's regions, so the copy plays at the same times as the original and you can drag it from there. A deck copied in Performance mode pastes as a bare deck with no lane. See [Copy and Paste](02-concepts.md#copy-and-paste).

Right-click a group row to copy, duplicate, or paste the channel.

### Deleting from the timeline

The row menus delete from the scene itself:

| Item | On | What goes |
|------|----|-----------|
| **Remove lane** | A lane header | The row and its curves. The deck stays in the mixer, unarranged. |
| **Delete deck** | A lane header | The deck itself, here and in Performance mode, with its lane and curves. |
| **Delete channel** | A group row | The channel, its decks, and all of their lanes and curves. |

**Delete channel** is greyed out when only two channels are left, because the mixer keeps channels A and B. Cmd+Z undoes any of these in one step. Deleting a deck from the mixer also removes its lane.

## Regions

A **region** is a span of time during which a deck is visible. The deck exists in the scene whether or not a region covers it.

Each region compiles to breakpoints on the deck's opacity curve: fade in, full, fade out, zero. Two regions overlapping in sibling lanes make a crossfade, using the blend mode already set on those decks.

| Gesture | Result |
|---------|--------|
| **Drag across empty track** | Create a region between where you pressed and where you released. |
| **Double-click empty track** | Drop a four-second region at that position. |
| **Drag a region** | Move it, keeping its length. |
| **Drag either edge** | Resize it. |
| **Drag a fade handle** (top corners) | Set the fade in or fade out. |
| **Right-click a region** | Delete region, or clear fades. |

A single click selects the lane's deck and creates nothing, so the bottom bar follows you around the timeline.

An edge can be grabbed from a few pixels outside the region; pressing there resizes the region instead of starting a new one. The pointer changes to horizontal arrows when you are about to grab an edge. When two regions are closer together than that margin, the gap is split between them, so you grab the nearer edge.

### Snapping

**Snap** rounds every edit to a whole frame at the show's timecode rate. It is on by default. Turn it off for continuous positions.

Snapping applies to the edit gesture only. Varda stores continuous positions, so changing the show's frame rate relabels the ruler without moving any region. See [Frame rates](07-control-surfaces.md#frame-rates-and-drop-frame).

## Selecting a Slice

Copying a whole deck takes every region on it, and copying a whole curve takes every breakpoint. To copy part of a region or curve, mark a **selection** and copy that slice.

| Gesture | Result |
|---------|--------|
| **Click a region** | Selects that region, ready to copy or delete. The bottom bar still follows the deck. |
| **Shift+drag on the tracks** | Draws a marquee: a time span across the lanes it covers. Everything inside is selected. |
| **Esc** | Clears the selection. Clicking empty track or a curve also clears it. |

A marquee can cover one lane or several deck and automation lanes, across channels. Drag down past a channel's rows and it keeps adding the rows it reaches, so you can select everything between two timecodes.

A region is included if it overlaps the marquee's time span at all, but only the part inside the time span is selected. If the marquee cuts through the middle of a region:

- Copy takes the cropped middle.
- Delete removes the middle and leaves the ends.
- Dragging moves the cropped middle and leaves the ends in place.

New cut edges are hard edges. Original fades stay with the fragment that keeps the original region edge. A marquee that covers no regions or points still shows its highlight.

A bare drag creates or edits. Hold Shift to make the same drag a selection.

With a selection marked:

- **Cmd+C** copies the slice. Cropped region pieces and curve pieces are copied together, with times measured from the start of the selection.
- **Delete** (or Backspace) removes only the selected part of each region and leaves the unselected fragments. On a curve, it clears the marked stretch and keeps the curve continuous on either side. The whole delete is one undo entry.
- **Cmd+V** pastes at the pointer when it is over a lane. Otherwise it pastes at the playhead onto the selected deck or curve. A pasted automation slice adds edge points so it keeps the shape it had under the marquee, and it replaces whatever it covers.

A deck lane takes only the region parts of a slice, and an automation lane takes only the curve parts. The other parts stay on the clipboard, so you can paste them onto a lane that takes them. You can also right-click empty track and pick **Paste slice here** to paste the region parts at that spot.

### Dragging a selection

**Drag from inside the highlight** to move everything in the selection together: regions, curve pieces, and the gaps between them.

| Gesture | Result |
|---------|--------|
| **Drag inside the highlight** | Moves the whole selection. |
| **Alt/Option + drag** | Leaves the original in place and moves a copy. |
| **Drag up or down** | Moves regions onto another deck lane, across channels if you drag that far. Curves stay on their own parameter row. |

While you drag, an outline shows where the slice will land. The edit happens on release, as one undo entry. The selection stays active at the new position, so you can move it again. Snap rounds the landing to a whole frame, and the selection stops at the start of the show instead of moving anything to a negative position.

A selected region's edge and fade handles still work, so you can resize and fade a single clicked region as usual. On an automation lane, a drag inside the selection moves the selection. Press **Esc** first to edit a point inside the marked stretch.

## Automation Lanes

To create a curve, open the **`〰`** dropdown on any modulatable parameter and pick **＋ Automation lane**. The curve appears as a row under whatever owns that parameter:

| Automated parameter | Row appears |
|---------------------|-------------|
| A deck's own parameters, or one of its effects | Under that deck's lane, folded until you unfold it |
| A channel effect | Under that channel's group header |
| A master effect | Under the **Master** row at the bottom |

Effect parameters are labeled `effect · parameter`, so two effects with the same parameter name stay separate. See [Automation Curves](06-modulation.md#automation-curves) for how a curve sets a value.

You can bend the segment between two breakpoints. Grab the line itself (the pointer turns into a vertical arrow) and drag toward the side you want it to bulge. A bent segment can start slowly and finish fast, or the reverse, on rising and falling segments alike. To straighten it, right-click the breakpoint at the start of the segment and pick **Linear**. **Smooth** and **Hold** replace the bend with those shapes.

Dragging a flat line raises or lowers it. The whole flat run moves together, including every breakpoint on it and the held stretches before the first breakpoint and after the last. To set a level on an unshaped lane, drop one breakpoint and drag the line on either side of it to the value you want.

The crossfader cannot be automated. It is mappable and can be driven by a macro, but it is not a modulation target. To crossfade in an arrangement, overlap two regions in sibling lanes.

| Gesture | Result |
|---------|--------|
| **Drag a breakpoint** | Move it in time and value. It cannot cross its neighbors. |
| **Drag a sloped line** | Bend that segment. Drag toward the side you want it to bulge. |
| **Drag a flat line** | Raise or lower it, along with every breakpoint on it. |
| **Double-click empty curve** | Add a breakpoint there. |
| **Double-click a breakpoint** | Remove it. |
| **Right-click a breakpoint** | Choose Linear, Smooth, or Hold, or delete it. |
| **Right-click a lane, or its header** | Copy this shape, or paste the copied one onto it. |
| **Cmd+C / Cmd+V** | The same copy and paste, on the lane you last clicked. |
| **Right-click the lane header** | Also removes the automation lane entirely. |
| **▾ caret** on a deck | Fold or unfold that deck's curves. |

The deck's own opacity curve has no editable row. Regions write it, and the next region edit would overwrite any hand edits.

Curves do not appear as cards in the right panel's modulation list.

### Reusing a shape

A curve drives only the parameter it was drawn for, and the `〰` dropdown does not list it as a source for other parameters. To put the same shape on a second parameter:

1. Right-click the lane you want to copy and pick **Copy curve**.
2. Right-click the other parameter's lane and pick **Paste curve**.

Both items are in the menu wherever you right-click the lane, on a breakpoint or on bare curve. The shape lands where you right-clicked, keeps its own length, and replaces whatever it covers. Pasting from the lane header or with the keyboard lands the shape at the playhead. To use the keyboard, click a lane to select it, then press Cmd+C and Cmd+V.

The two curves are independent after pasting. Editing one does not change the other.

## Recording a Pass

**⏺** in the transport strip (and in the top bar, so it is also in Performance mode) arms automation recording. While armed, any control you move is written into the arrangement as a curve at the current show position.

1. Press **⏺**. From a stop, this also starts playback. While chasing timecode it only arms, and the pass starts when the master rolls.
2. Play the show with the mouse, MIDI, OSC, macros, or the API. Every automatable control records.
3. Press **⏺** again to end the pass.

The button is grey when idle, dark red when armed, and bright red while it is writing. Its tooltip shows the number of parameters being written.

Anything with a `〰` dropdown records, including deck opacity, deck and effect parameters, and channel faders. If a parameter has no curve yet, Varda creates a lane for it.

**A pass replaces only the stretch it covered.** If you punch in at bar 9, move a knob, and punch out at bar 17, the curve before bar 9 and after bar 17 is unchanged. Pasting a curve follows the same rule.

Recording writes your movements, not a point per frame. A control you held still keeps its value with no ramp, and points are thinned to the shape you played so the curve stays editable. A jump in position ends the take, so a loop wrap starts a new take over the same bars.

While you hold a control, it is overridden as usual. When the pass ends, the control follows its new curve. **The whole pass is one undo entry**, so Cmd+Z removes the whole take.

Recording is also available as `PUT /api/transport/record` and the `action/record` binding, so a foot switch or a show controller can punch in.

## Authority and Override

The arrangement takes control **per lane**, and only after the transport has run. Before you press Play, the scene renders as you saved it, so an arrangement you have not started cannot black out your output.

A lane with regions or curves is controlled by the arrangement. Everything else in the scene stays live, so a show can be part arranged and part performed.

### Overriding a parameter

**Move a control the arrangement is driving and your input takes over immediately.** A fader drag, a MIDI knob, an OSC message, or an API write suspends the arrangement's control of *that parameter only*. There is no confirmation.

An overridden lane shows an amber dot in its header, in both views.

### Re-arming

Click the amber dot to re-arm that parameter, or click **↻ Re-arm all** in the transport strip to re-arm everything. The button appears only while something is overridden, and shows a count.

Re-armed parameters **ramp** back to their automated value instead of jumping to it, to avoid a visible glitch.

Overrides are session state and are **never saved**. Reloading the scene restores full arrangement control.

### Video chase

A video deck can lock its playhead to the transport, the same clock the arrangement, automation, and cues follow. It chases the **transport**, not the LTC/MTC input directly, so it behaves the same whether the transport runs internally or follows a house clock.

In the deck detail bar, **Chase** has three settings:

| Setting | Behavior |
|---------|----------|
| **Auto** (default) | Chases while the transport is running. Free-runs, with loop modes, when it is stopped. |
| **Always** | Chases, and holds the mapped frame while the transport is stopped. |
| **Never** | Plays on wall-clock time, ignoring the transport. |

**Offset** is the transport time at which the clip's in-point sits. It is independent of regions: a region sets when the deck is visible, and offset sets which frame shows. **Delay** is a signed offset in frames at the transport's displayed rate, for correcting sound-to-light latency.

While chasing, loop mode is ignored. If the mapped time is before the in-point or after the out-point, the clip holds that frame. The speed fader sets the clip's rate relative to the transport.

Older scenes default to Auto. They play as before until you press Play on the transport. From then on, video decks chase unless you set Never.

### Performance sequencers while the arrangement runs

- **Deck auto-transitions** are per deck. A deck under arrangement control has its auto-transition suspended. A deck without regions keeps it.
- **Transition sequences** span channels. While the arrangement has control, starting a free-running sequence is refused with a reason, and a running one is stopped.

## Idle Behavior

**Idle** in the transport strip sets what renders before the transport reaches the arranged range:

| Setting | Behavior |
|---------|----------|
| **Hold performance** | The mixer holds. The arrangement does nothing until the show reaches it. Default. |
| **Show `<deck>`** | That deck plays until the arranged range starts. |

Use **Show `<deck>`** for an installation that runs a loop until the schedule starts. Choose a setting deliberately: a black pre-show can look like a broken rig.

If the arrangement drives everything to zero, Varda shows a one-time notice.

## What Regions Cannot Do

A lane is a deck, and a deck holds one source. Loading a different video into a deck, recalling a preset, or firing a sequence are **events**, not spans, so they cannot be regions. To play shader A then shader B, put them in two lanes and overlap the regions. The overlap is also a crossfade between them.

## Undo, Saving, and Load

- Every timeline drag is **one** undo entry. Cmd+Z returns the region or breakpoint to where it was before the drag.
- The arrangement is saved in `scene.json` with the rest of the scene. Each lane is a deck and each curve is a modulation source in the scene, so there is no separate arrangement file.
- This is **scene version 7**. Older scenes open unchanged, but a scene containing an arrangement will not open on an older build.
- Every deck stays in memory for the whole show. The monitoring cluster at the bottom of the right panel shows the deck count and an estimate of the color-target memory they hold. See [Performance Monitoring](13-resolution-and-monitoring.md#performance-monitoring).

## Sleeping Clips

When a video deck's next region is minutes away, Varda **stops decoding it** and wakes it one second before the region starts. This saves decoder load in long shows with many video decks.

**A sleeping clip freezes.** When its region arrives, it resumes from where it paused, not from where wall-clock time would have put it. The same show position looks the same on every run. This differs from a clip free-running behind a closed fader in Performance mode. A sleeping clip's transport still reads as playing. Re-arming does not restart a clip you paused by hand.

A deck keeps decoding in these cases:

- Performance mode, or an arrangement whose transport has not run.
- An LFO, audio band, or other live modulator is on its opacity.
- You have overridden it by hand. It keeps decoding until you re-arm it.
- It is in a cued channel, or feeds a program tap, so its off-air view stays live.
- The transport is outside the arranged range with **Hold performance** set.

Cameras and screen captures sleep on the same schedule, and their frames come up ahead of the region that needs them. Live network sources (NDI, SRT, Syphon), HTML decks, and depth sensors never sleep, because reconnecting or reloading costs more than sleeping saves.

A sleeping deck keeps its render targets in memory, so the VRAM readout does not change when a deck sleeps.

## HTTP API

Everything above is available over HTTP, so you can build an arrangement from a script or drive one from a show controller:

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

Transport control (`/api/transport/play`, `/locate`, `/loop`, `/rate`, `/source`) and curve editing (`PUT /api/modulation/<uuid>/breakpoints`) are documented with their own subsystems. See [HTTP API](15-api.md#route-reference).

---

[← Prev: Performance & Automation](04-performance.md) · [Home](README.md) · [Next: Modulation & Audio Reactivity →](06-modulation.md)
