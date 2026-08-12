//! Arrangement mode: the mixer rotated ninety degrees.
//!
//! A channel is a group, a deck is a lane, and a region is a stretch of that
//! deck's opacity envelope. Only the central area changes; the library, bottom
//! bar, and right panel stay exactly as they are in Performance mode. See
//! /spec/arrangement.md § UI.
//!
//! The whole timeline is painted from a single allocated rect rather than from
//! nested egui layouts. Headers and tracks must agree on every row's vertical
//! position to the pixel, and two independent layouts drift the moment either
//! one's spacing changes.

mod automation;
mod cues;
mod regions;

/// The colour a cue is drawn in, wherever it is drawn: the ruler here, and the
/// bank of pads in Performance mode.
pub(super) const CUE_COLOR: egui::Color32 = cues::COLOR;

pub(super) use automation::a_lane_is_selected;

use super::super::state::{MAX_PIXELS_PER_SECOND, MIN_PIXELS_PER_SECOND};
use super::super::{DeckDrag, ModSourceUI, UIActions, UIData};
use super::clipboard_menu;
use super::utils::channel_color;
use crate::arrangement::RegionConfig;
use crate::engine::EngineCommand;
use crate::modulation::Breakpoint;
use crate::transport::TransportSource;

/// Width of the fixed left column holding group and lane names.
const HEADER_WIDTH: f32 = 168.0;
const RULER_HEIGHT: f32 = 22.0;
const GROUP_HEIGHT: f32 = 20.0;
const LANE_HEIGHT: f32 = 32.0;
const AUTOMATION_HEIGHT: f32 = 46.0;
/// How much of the scale a wheel notch covers when the wheel is being used to
/// zoom. egui's own figure for Cmd + wheel, so the fallback gesture and the
/// pinch move the timeline at the same rate.
const WHEEL_ZOOM_SPEED: f32 = 1.0 / 200.0;
/// Blank time kept past the end of the last region, so there is somewhere to
/// scroll to and somewhere to drop the next region.
const TRAILING_SECONDS: f64 = 30.0;

/// One row of the timeline. Groups, lanes, and automation rows share a vertical
/// rhythm so the fixed headers and the scrolling tracks cannot drift apart.
enum Row<'a> {
    Group {
        ch_idx: usize,
        name: &'a str,
    },
    /// The mixer's own row, holding master effect automation. Sits below every
    /// channel, where the mixer box sits in Performance mode.
    Master,
    Lane(LaneRow<'a>),
    Automation(AutomationRow<'a>),
}

impl Row<'_> {
    fn height(&self) -> f32 {
        match self {
            Row::Group { .. } | Row::Master => GROUP_HEIGHT,
            Row::Lane(_) => LANE_HEIGHT,
            Row::Automation(_) => AUTOMATION_HEIGHT,
        }
    }
}

/// What clicking an automation row's header selects, so the bottom bar shows the
/// thing whose curve is being drawn.
#[derive(Clone, Copy)]
enum Owner {
    Deck(usize, usize),
    Channel(usize),
    Master,
}

/// Everything one deck row needs, so the row renderer does not take a dozen
/// positional arguments.
struct LaneRow<'a> {
    ch_idx: usize,
    deck_idx: usize,
    uuid: &'a str,
    name: &'a str,
    regions: &'a [RegionConfig],
    overridden: bool,
    collapsed: bool,
    /// Whether there is anything to unfold, so the caret only appears where it
    /// would do something.
    has_automation: bool,
}

/// One automated parameter, beneath whatever owns it.
struct AutomationRow<'a> {
    /// Channel the row takes its colour from. The master row borrows the last
    /// channel's colour rather than inventing one.
    ch_idx: usize,
    owner: Owner,
    /// Display name, already stripped of the `deck_<uuid>:` addressing prefix
    /// and qualified by the effect it belongs to where that is ambiguous.
    label: String,
    param_key: &'a str,
    envelope_uuid: &'a str,
    breakpoints: &'a [Breakpoint],
}

impl AutomationRow<'_> {
    fn label(&self) -> &str {
        &self.label
    }
}

/// Maps show seconds to screen x for the current scroll and zoom.
#[derive(Clone, Copy)]
struct TimeAxis {
    left: f32,
    scroll: f64,
    pps: f32,
}

impl TimeAxis {
    fn x(self, seconds: f64) -> f32 {
        self.left + ((seconds - self.scroll) * f64::from(self.pps)) as f32
    }

    fn seconds(self, x: f32) -> f64 {
        self.scroll + f64::from(x - self.left) / f64::from(self.pps)
    }
}

/// Where one row sits and how it maps time to pixels. Bundled so the row
/// renderers take a context rather than a parameter list.
#[derive(Clone, Copy)]
struct RowGeometry {
    header: egui::Rect,
    track: egui::Rect,
    axis: TimeAxis,
    idx: usize,
}

/// The timeline's two columns and how far the rows have been scrolled past the
/// top of them. Bundled for the same reason as [`RowGeometry`].
#[derive(Clone, Copy)]
struct Layout {
    header: egui::Rect,
    lanes: egui::Rect,
    axis: TimeAxis,
    scroll_y: f32,
}

/// Round an edit to the nearest frame when the snap preference is on.
///
/// Snapping is a property of the gesture, never of the stored value: positions
/// stay continuous `f64` so changing the ruler's frame rate re-labels the
/// timeline without moving anything. See /spec/arrangement.md § Does the ruler
/// own a frame rate?
fn snap_seconds(data: &UIData, seconds: f64) -> f64 {
    if !data.arrangement_snap {
        return seconds.max(0.0);
    }
    let fps = data.transport.timecode_rate.fps();
    ((seconds * fps).round() / fps).max(0.0)
}

/// Shortest region an edit may produce: one frame at the ruler's rate.
fn min_span(data: &UIData) -> f64 {
    1.0 / data.transport.timecode_rate.fps()
}

pub(super) fn render_arrangement(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    render_transport_strip(ui, data, actions);
    ui.separator();

    let rows = build_rows(data);
    let area = ui.available_rect_before_wrap();
    ui.advance_cursor_after_rect(area);
    if area.width() <= HEADER_WIDTH || area.height() <= RULER_HEIGHT {
        return;
    }

    let header_rect = egui::Rect::from_min_max(
        area.min,
        egui::pos2(area.left() + HEADER_WIDTH, area.bottom()),
    );
    let track_rect =
        egui::Rect::from_min_max(egui::pos2(header_rect.right(), area.top()), area.max);
    let ruler_rect = egui::Rect::from_min_max(
        track_rect.min,
        egui::pos2(track_rect.right(), track_rect.top() + RULER_HEIGHT),
    );
    let lanes_rect = egui::Rect::from_min_max(
        egui::pos2(track_rect.left(), ruler_rect.bottom()),
        track_rect.max,
    );

    let pps = data
        .arrangement_pixels_per_second
        .clamp(MIN_PIXELS_PER_SECOND, MAX_PIXELS_PER_SECOND);
    let axis = TimeAxis {
        left: track_rect.left(),
        scroll: data.arrangement_scroll,
        pps,
    };

    // Clamped here rather than where it is stored, because the limit is this
    // frame's row count against this frame's panel height.
    let scroll_y = data
        .arrangement_scroll_y
        .clamp(0.0, max_scroll_y(&rows, lanes_rect));

    handle_pan_and_zoom(ui, data, actions, track_rect, axis, &rows, lanes_rect);
    render_ruler(ui, data, actions, ruler_rect, axis);
    let layout = Layout {
        header: header_rect,
        lanes: lanes_rect,
        axis,
        scroll_y,
    };
    render_rows(ui, &rows, data, actions, layout);
    render_vertical_scrollbar(ui, actions, &rows, lanes_rect, scroll_y);
    // After the rows, so a cue's line reads over the regions it marks, and after
    // the ruler, so its handle wins the press that would otherwise scrub.
    cues::render(ui, data, actions, ruler_rect, lanes_rect, axis);
    draw_playhead(ui, data, track_rect, axis);
    automation::handle_clipboard_shortcuts(ui, data, actions);
    offer_channel_drop_targets(ui, &rows, layout);
}

/// Every deck gets a lane, whether or not the arrangement has claimed it yet.
///
/// Showing only arranged decks would make an empty arrangement look like an
/// empty scene, and would leave nowhere to drop a first region.
fn build_rows(data: &UIData) -> Vec<Row<'_>> {
    let arrangement = data.arrangement.as_ref();
    let mut rows = Vec::new();
    for ch in &data.channels {
        rows.push(Row::Group {
            ch_idx: ch.ch_idx,
            name: &ch.name,
        });
        for deck in &ch.decks {
            let lane = arrangement.and_then(|a| a.config.lane(&deck.uuid));
            let key = crate::arrangement::opacity_param_key(&deck.uuid);
            let curves = deck_automation_rows(data, ch.ch_idx, deck);
            let collapsed = lane.is_some_and(|l| l.collapsed);
            rows.push(Row::Lane(LaneRow {
                ch_idx: ch.ch_idx,
                deck_idx: deck.deck_idx,
                uuid: &deck.uuid,
                name: &deck.name,
                regions: lane.map_or(&[], |l| l.regions.as_slice()),
                overridden: arrangement.is_some_and(|a| a.overridden_params.contains(&key)),
                collapsed,
                has_automation: !curves.is_empty(),
            }));
            if !collapsed {
                rows.extend(curves.into_iter().map(Row::Automation));
            }
        }
        // A channel effect belongs to the channel, not to any one deck, so its
        // curve sits directly under the group header.
        rows.extend(
            automation_rows(
                data,
                ch.ch_idx,
                Owner::Channel(ch.ch_idx),
                &effect_sources(&ch.effects),
                None,
            )
            .into_iter()
            .map(Row::Automation),
        );
    }

    // The master row is drawn whether or not it holds anything, because a curve
    // authored on a master effect otherwise has nowhere to be edited.
    let master_color = data.channels.len().saturating_sub(1);
    rows.push(Row::Master);
    rows.extend(
        automation_rows(
            data,
            master_color,
            Owner::Master,
            &effect_sources(&data.master_effect_info),
            None,
        )
        .into_iter()
        .map(Row::Automation),
    );
    rows
}

/// Modulation key prefixes for an effect chain, each labelled by its effect.
///
/// Two effects can carry the same parameter name, so the effect's own name is
/// carried alongside the prefix rather than being recovered from the key.
fn effect_sources(effects: &[super::super::EffectInfo]) -> Vec<(String, Option<String>)> {
    effects
        .iter()
        .map(|(uuid, name, _, _)| (format!("fx_{uuid}:"), Some(name.clone())))
        .collect()
}

/// A deck's own parameters plus its effect chain.
///
/// The region-compiled opacity curve is excluded: it is authored by dragging
/// regions, and hand-editing its breakpoints would be undone by the next region
/// edit.
fn deck_automation_rows<'a>(
    data: &'a UIData,
    ch_idx: usize,
    deck: &'a super::super::DeckUIInfo,
) -> Vec<AutomationRow<'a>> {
    let region_envelope = data
        .arrangement
        .as_ref()
        .and_then(|a| a.config.lane(&deck.uuid))
        .and_then(|l| {
            l.envelopes
                .get(&crate::arrangement::opacity_param_key(&deck.uuid))
        });

    let mut sources = vec![(format!("deck_{}:", deck.uuid), None)];
    sources.extend(effect_sources(&deck.effects));
    automation_rows(
        data,
        ch_idx,
        Owner::Deck(ch_idx, deck.deck_idx),
        &sources,
        region_envelope,
    )
}

/// The envelopes assigned to any parameter under `sources`, one row each.
///
/// Derived from the modulation graph rather than from the lane, because an
/// envelope can also be created from a parameter's modulation menu and would
/// otherwise have no editor anywhere.
fn automation_rows<'a>(
    data: &'a UIData,
    ch_idx: usize,
    owner: Owner,
    sources: &[(String, Option<String>)],
    exclude: Option<&String>,
) -> Vec<AutomationRow<'a>> {
    let mut rows: Vec<AutomationRow<'a>> = data
        .modulation_assignments
        .iter()
        .filter_map(|(key, assignments)| {
            let (_, owner_label) = sources.iter().find(|(prefix, _)| key.starts_with(prefix))?;
            Some((key.as_str(), owner_label.as_deref(), assignments))
        })
        .flat_map(|(key, owner_label, assignments)| {
            assignments.iter().map(move |a| (key, owner_label, a))
        })
        .filter(|(_, _, a)| Some(&a.source_id) != exclude)
        .filter_map(|(param_key, owner_label, assignment)| {
            let entry = data
                .modulation_sources
                .iter()
                .find(|e| e.uuid == assignment.source_id)?;
            let ModSourceUI::Envelope { breakpoints } = &entry.source else {
                return None;
            };
            let name = param_key.rsplit_once(':').map_or(param_key, |(_, n)| n);
            Some(AutomationRow {
                ch_idx,
                owner,
                label: match owner_label {
                    Some(effect) => format!("{effect} · {name}"),
                    None => name.to_string(),
                },
                param_key,
                envelope_uuid: &entry.uuid,
                breakpoints,
            })
        })
        .collect();
    // Assignments live in a hash map, so without this the rows would reshuffle
    // between frames.
    rows.sort_by(|a, b| a.param_key.cmp(b.param_key));
    rows
}

/// Play, stop, zero, position, snap, and zoom, inline above the ruler.
///
/// Duplicates the top bar popover rather than replacing it: the popover is what
/// Performance mode has, and a mode switch should not move the controls a
/// performer already learned. See /spec/arrangement.md § Transport controls are
/// inline here.
fn render_transport_strip(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let t = &data.transport;
    // Position is read-only while chasing, so offering the controls would be
    // offering a lie.
    let scrubbable = t.source == TransportSource::Internal;

    ui.horizontal(|ui| {
        ui.add_enabled_ui(scrubbable, |ui| {
            let play_label = if t.running { "⏸" } else { "▶" };
            if ui
                .button(play_label)
                .on_hover_text(if t.running { "Pause" } else { "Play" })
                .clicked()
            {
                actions.commands.push(if t.running {
                    EngineCommand::TransportStop
                } else {
                    EngineCommand::TransportPlay
                });
            }
            if ui
                .button("⏹")
                .on_hover_text(if t.running {
                    "Stop. Press again to return to 00:00:00:00"
                } else {
                    "Return to 00:00:00:00"
                })
                .clicked()
            {
                actions.commands.push(EngineCommand::TransportStop);
            }
            if ui
                .button("⏮")
                .on_hover_text("Previous cue. Double-click the ruler to add one")
                .clicked()
            {
                actions.commands.push(EngineCommand::TransportPrevCue);
            }
            if ui
                .button("⏭")
                .on_hover_text("Next cue. Double-click the ruler to add one")
                .clicked()
            {
                actions.commands.push(EngineCommand::TransportNextCue);
            }
        });

        ui.label(
            egui::RichText::new(&t.timecode)
                .monospace()
                .size(18.0)
                .color(super::popovers::transport_color(data)),
        );
        ui.label(egui::RichText::new(&t.status_label).small().weak());

        if !scrubbable {
            ui.label(
                egui::RichText::new("chasing")
                    .small()
                    .color(egui::Color32::from_rgb(120, 180, 255)),
            )
            .on_hover_text(
                "Position is owned by the timecode master. Switch the transport source to \
                 Internal to scrub.",
            );
        }

        render_rearm_all(ui, data, actions);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let pps = data.arrangement_pixels_per_second;
            if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                actions.session.set_arrangement_zoom = Some(pps * 1.5);
            }
            if ui.small_button("−").on_hover_text("Zoom out").clicked() {
                actions.session.set_arrangement_zoom = Some(pps / 1.5);
            }
            ui.label(egui::RichText::new("Zoom").small().weak());

            ui.separator();
            let mut snap = data.arrangement_snap;
            if ui
                .checkbox(&mut snap, "Snap")
                .on_hover_text(format!(
                    "Round edits to whole frames at {} fps. Positions themselves stay continuous.",
                    t.timecode_rate.label()
                ))
                .changed()
            {
                actions.session.toggle_arrangement_snap = true;
            }

            ui.separator();
            render_idle_picker(ui, data, actions);
        });
    });
}

/// One click to hand every held parameter back, with a count so the performer
/// knows how much is off the rails.
///
/// Per-lane badges alone would mean hunting for the lanes that are held, which
/// is the wrong thing to be doing while the show runs.
fn render_rearm_all(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let held = data
        .arrangement
        .as_ref()
        .map_or(0, |a| a.overridden_params.len());
    if held == 0 {
        return;
    }
    if ui
        .button(
            egui::RichText::new(format!("↻ Re-arm all ({held})"))
                .small()
                .color(egui::Color32::from_rgb(255, 170, 60)),
        )
        .on_hover_text(
            "Hand every held parameter back to the arrangement. Each one ramps to its \
             automated value rather than jumping.",
        )
        .clicked()
    {
        actions
            .commands
            .push(EngineCommand::RearmAll { seconds: None });
    }
}

/// What plays before the show reaches the arranged range.
///
/// Reachable from the timeline because the alternative to choosing is a black
/// screen that looks exactly like a broken rig. See /spec/transport.md § Idle
/// behaviour.
fn render_idle_picker(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    use crate::arrangement::IdleBehaviour;

    let current = data
        .arrangement
        .as_ref()
        .map_or(IdleBehaviour::HoldPerformance, |a| a.config.idle.clone());
    let deck_name = |uuid: &str| {
        data.channels
            .iter()
            .flat_map(|c| &c.decks)
            .find(|d| d.uuid == uuid)
            .map_or("(missing deck)", |d| d.name.as_str())
    };
    let label = match &current {
        IdleBehaviour::HoldPerformance => "Hold performance".to_string(),
        IdleBehaviour::ShowDeck { deck_uuid } => format!("Show {}", deck_name(deck_uuid)),
    };

    let mut chosen: Option<IdleBehaviour> = None;
    egui::ComboBox::from_id_salt("arrangement_idle")
        .selected_text(egui::RichText::new(label).small())
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(
                    current == IdleBehaviour::HoldPerformance,
                    "Hold performance",
                )
                .clicked()
            {
                chosen = Some(IdleBehaviour::HoldPerformance);
            }
            for deck in data.channels.iter().flat_map(|c| &c.decks) {
                let this = IdleBehaviour::ShowDeck {
                    deck_uuid: deck.uuid.clone(),
                };
                if ui
                    .selectable_label(current == this, format!("Show {}", deck.name))
                    .clicked()
                {
                    chosen = Some(this);
                }
            }
        })
        .response
        .on_hover_text("What renders before the transport reaches the arranged range");
    ui.label(egui::RichText::new("Idle").small().weak());

    if let Some(idle) = chosen {
        actions
            .commands
            .push(EngineCommand::SetIdleBehaviour { idle });
    }
}

/// A pinch zooms the timescale, the wheel moves the rows, and Shift or a
/// horizontal wheel pans along time.
///
/// Zooming about the pointer rather than the left edge is what makes a long show
/// navigable: the thing being looked at stays put.
fn handle_pan_and_zoom(
    ui: &egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    track_rect: egui::Rect,
    axis: TimeAxis,
    rows: &[Row<'_>],
    lanes_rect: egui::Rect,
) {
    let Some(pointer) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    if !track_rect.contains(pointer) {
        return;
    }

    let (scroll_delta, zoom_delta, zoom_modifier, pan_modifier) = ui.ctx().input(|i| {
        (
            i.smooth_scroll_delta,
            i.zoom_delta(),
            i.modifiers.alt,
            i.modifiers.shift,
        )
    });

    // A trackpad pinch and Cmd + wheel are one gesture by the time they reach
    // here: egui turns both into a zoom factor and withholds the scroll they
    // came from. Alt keeps a zoom on the wheel for a mouse that has no pinch to
    // give, on the same multiplicative scale so both feel alike.
    let zoom = if (zoom_delta - 1.0).abs() > f32::EPSILON {
        zoom_delta
    } else if zoom_modifier && scroll_delta.y.abs() > 0.0 {
        (scroll_delta.y * WHEEL_ZOOM_SPEED).exp()
    } else {
        1.0
    };
    if (zoom - 1.0).abs() > f32::EPSILON {
        let (pps, scroll) = zoomed(axis, pointer.x, zoom);
        actions.session.set_arrangement_zoom = Some(pps);
        actions.session.set_arrangement_scroll = Some(scroll.clamp(0.0, max_scroll(data)));
        return;
    }

    // A horizontal wheel pans, and so does the vertical one held with Shift, for
    // the single-wheel mouse that has no other way across a long show.
    let pan_px = if scroll_delta.x.abs() > 0.0 {
        scroll_delta.x
    } else if pan_modifier {
        scroll_delta.y
    } else {
        0.0
    };
    if pan_px.abs() > 0.0 {
        let seconds = data.arrangement_scroll - f64::from(pan_px) / f64::from(axis.pps);
        actions.session.set_arrangement_scroll = Some(seconds.clamp(0.0, max_scroll(data)));
        return;
    }

    // Everything else the wheel does is what it does in every other list: move
    // the rows. A show with more channels than fit on screen is otherwise
    // unreachable below the fold.
    if scroll_delta.y.abs() > 0.0 {
        let limit = max_scroll_y(rows, lanes_rect);
        let next = (data.arrangement_scroll_y - scroll_delta.y).clamp(0.0, limit);
        actions.session.set_arrangement_scroll_y = Some(next);
    }
}

/// The strip down the right edge of the lanes, present only when the rows
/// overrun their area. Drag it, or use the wheel.
fn render_vertical_scrollbar(
    ui: &egui::Ui,
    actions: &mut UIActions,
    rows: &[Row<'_>],
    lanes_rect: egui::Rect,
    scroll_y: f32,
) {
    const WIDTH: f32 = 8.0;
    /// Short enough to stay grabbable on a show with a hundred lanes.
    const MIN_THUMB: f32 = 24.0;

    let limit = max_scroll_y(rows, lanes_rect);
    if limit <= 0.0 {
        return;
    }

    let track = egui::Rect::from_min_max(
        egui::pos2(lanes_rect.right() - WIDTH, lanes_rect.top()),
        lanes_rect.max,
    );
    let content = content_height(rows);
    let thumb_height = (track.height() * (lanes_rect.height() / content)).max(MIN_THUMB);
    let travel = track.height() - thumb_height;
    let thumb_top = track.top() + travel * (scroll_y / limit);
    let thumb = egui::Rect::from_min_size(
        egui::pos2(track.left(), thumb_top),
        egui::vec2(WIDTH, thumb_height),
    );

    let response = ui.interact(
        track,
        ui.id().with("arrangement_vscroll"),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter_at(track);
    painter.rect_filled(track, 4.0, ui.visuals().extreme_bg_color);
    let colors = ui.visuals().widgets.style(&response);
    painter.rect_filled(thumb.shrink(1.0), 4.0, colors.bg_fill);

    // Both a drag on the thumb and a click on the track put the pointer where
    // it asked to be, which is the same arithmetic either way.
    if response.is_pointer_button_down_on() {
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            let wanted = pointer.y - track.top() - thumb_height / 2.0;
            let fraction = if travel > 0.0 { wanted / travel } else { 0.0 };
            actions.session.set_arrangement_scroll_y = Some((fraction * limit).clamp(0.0, limit));
        }
    }
}

/// The zoom and horizontal scroll a gesture of `factor` lands on, holding the
/// instant under `pointer_x` where it is.
///
/// Multiplicative, so a pinch out and back leaves the view where it started
/// however many frames it took, and so the same gesture covers the same
/// proportion of the scale at every zoom. Near the top of the show the anchor is
/// given up rather than scrolling to before it starts, since there is nothing
/// there to show.
fn zoomed(axis: TimeAxis, pointer_x: f32, factor: f32) -> (f32, f64) {
    let anchor = axis.seconds(pointer_x);
    let pps = (axis.pps * factor).clamp(MIN_PIXELS_PER_SECOND, MAX_PIXELS_PER_SECOND);
    let offset = f64::from(pointer_x - axis.left) / f64::from(pps);
    (pps, (anchor - offset).max(0.0))
}

/// Furthest the timeline can be panned.
///
/// Bounded so a flick of the wheel cannot strand the view in empty time hours
/// past anything authored, with no landmark to navigate back by.
fn max_scroll(data: &UIData) -> f64 {
    let authored = data.arrangement.as_ref().map_or(0.0, |a| a.duration);
    authored.max(data.transport.position) + TRAILING_SECONDS
}

fn render_ruler(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    rect: egui::Rect,
    axis: TimeAxis,
) {
    // Scrubbing is refused rather than swallowed while chasing: a ruler that
    // accepts clicks and does nothing is the worst of both.
    let scrubbable = data.transport.source == TransportSource::Internal;
    let sense = if scrubbable {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::hover()
    };
    let response = ui.interact(rect, ui.id().with("arrangement_ruler"), sense);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, scrubbable, "arrangement ruler")
    });

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, ui.visuals().faint_bg_color);

    let step = tick_step(axis.pps);
    let first = (axis.scroll / step).floor() * step;
    let last = axis.seconds(rect.right());
    let mut t = first.max(0.0);
    while t <= last {
        let x = axis.x(t);
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom() - 5.0),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color()),
        );
        painter.text(
            egui::pos2(x + 3.0, rect.top() + 1.0),
            egui::Align2::LEFT_TOP,
            data.transport.timecode_rate.format(t),
            egui::FontId::monospace(9.0),
            ui.visuals().weak_text_color(),
        );
        t += step;
    }

    if !scrubbable {
        response.on_hover_text(
            "Position is owned by the timecode master. Switch the transport source to Internal \
             to scrub.",
        );
        return;
    }

    // Click and drag both locate, because scrubbing is the same gesture held.
    if response.clicked() || response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            actions.commands.push(EngineCommand::TransportLocate {
                position: axis.seconds(pos.x).max(0.0),
            });
        }
    }

    // The double-click also located, on its first click. Landing the playhead on
    // the cue it just made is what someone marking a moment wanted anyway.
    if response.double_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            actions.commands.push(EngineCommand::AddCue {
                at: snap_seconds(data, axis.seconds(pos.x)),
                name: String::new(),
            });
        }
    }
}

/// Tick spacing that keeps labels legible at any zoom.
fn tick_step(pps: f32) -> f64 {
    const CANDIDATES: [f64; 12] = [
        0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 300.0, 600.0,
    ];
    // Roughly the width of an `HH:MM:SS:FF` label plus breathing room.
    const MIN_LABEL_SPACING: f64 = 72.0;
    for step in CANDIDATES {
        if step * f64::from(pps) >= MIN_LABEL_SPACING {
            return step;
        }
    }
    CANDIDATES[CANDIDATES.len() - 1]
}

/// Paint headers and tracks together, one row at a time.
fn render_rows(
    ui: &mut egui::Ui,
    rows: &[Row<'_>],
    data: &UIData,
    actions: &mut UIActions,
    layout: Layout,
) {
    let Layout {
        header: header_rect,
        lanes: lanes_rect,
        axis,
        scroll_y,
    } = layout;
    // A row straddling either edge is drawn and clipped rather than dropped, so
    // scrolling reveals a row gradually instead of snapping it into place.
    let visible = egui::Rect::from_min_max(
        egui::pos2(header_rect.left(), lanes_rect.top()),
        lanes_rect.max,
    );
    let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(visible));
    ui.set_clip_rect(visible);

    for (i, (row, (top, height))) in rows
        .iter()
        .zip(row_spans(rows, lanes_rect, scroll_y))
        .enumerate()
    {
        if top + height < lanes_rect.top() {
            continue;
        }
        if top > lanes_rect.bottom() {
            break;
        }
        let geom = RowGeometry {
            header: egui::Rect::from_min_size(
                egui::pos2(header_rect.left(), top),
                egui::vec2(header_rect.width(), height),
            ),
            track: egui::Rect::from_min_size(
                egui::pos2(lanes_rect.left(), top),
                egui::vec2(lanes_rect.width(), height),
            ),
            axis,
            idx: i,
        };
        match row {
            Row::Group { ch_idx, name } => render_group_row(ui, data, actions, geom, *ch_idx, name),
            Row::Master => render_master_row(ui, data, actions, geom),
            Row::Lane(lane) => render_lane_row(ui, data, actions, geom, lane),
            Row::Automation(curve) => {
                automation::render_automation_row(ui, data, actions, geom, curve);
            }
        }
    }
}

/// Top and height of each row, tiled from the top of the lane area with
/// `scroll_y` pixels of rows already past it.
fn row_spans(rows: &[Row<'_>], lanes_rect: egui::Rect, scroll_y: f32) -> Vec<(f32, f32)> {
    let mut y = lanes_rect.top() - scroll_y;
    rows.iter()
        .map(|row| {
            let span = (y, row.height());
            y += row.height();
            span
        })
        .collect()
}

/// Height of every row stacked, whether or not it fits.
fn content_height(rows: &[Row<'_>]) -> f32 {
    rows.iter().map(Row::height).sum()
}

/// Furthest the rows can be scrolled: enough to bring the last one into view and
/// no further, so a flick cannot leave the timeline showing nothing.
fn max_scroll_y(rows: &[Row<'_>], lanes_rect: egui::Rect) -> f32 {
    (content_height(rows) - lanes_rect.height()).max(0.0)
}

/// Publish each group's row as a channel drop target.
///
/// The deferred library drop handler resolves targets by looking up a rect per
/// channel index, so dropping a generator onto a group creates the deck (and
/// therefore its lane) with no arrangement-specific drop code at all. See
/// /spec/arrangement.md § UI.
fn offer_channel_drop_targets(ui: &egui::Ui, rows: &[Row<'_>], layout: Layout) {
    let Layout {
        header: header_rect,
        lanes: lanes_rect,
        scroll_y,
        ..
    } = layout;
    for (row, (top, height)) in rows.iter().zip(row_spans(rows, lanes_rect, scroll_y)) {
        let Row::Group { ch_idx, .. } = row else {
            continue;
        };
        let rect = egui::Rect::from_min_max(
            egui::pos2(header_rect.left(), top),
            egui::pos2(lanes_rect.right(), top + height),
        );
        ui.ctx().memory_mut(|mem| {
            mem.data
                .insert_temp(egui::Id::new("ch_drop_rect").with(*ch_idx), rect);
        });
    }
}

fn render_group_row(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    geom: RowGeometry,
    ch_idx: usize,
    name: &str,
) {
    let RowGeometry {
        header,
        track,
        idx: row_idx,
        ..
    } = geom;
    let color = channel_color(ch_idx);
    let selected = data.selected_channel == Some(ch_idx);
    let painter = ui.painter();
    painter.rect_filled(
        header,
        2.0,
        color.gamma_multiply(if selected { 0.55 } else { 0.3 }),
    );
    painter.rect_filled(track, 0.0, color.gamma_multiply(0.12));
    painter.text(
        header.left_center() + egui::vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(12.0),
        ui.visuals().strong_text_color(),
    );

    let response = ui.interact(
        header,
        ui.id().with(("arrangement_group", row_idx)),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, name)
    });
    if response.clicked() {
        actions.session.select_channel = Some(ch_idx);
    }
}

/// The mixer's row, holding master effect automation.
///
/// Drawn even when it holds nothing: a curve authored on a master effect from
/// the bottom bar has to land somewhere, and an absent row would leave it
/// running with no editor. The crossfader is not here because it is not a
/// modulation target yet. See /spec/arrangement.md § Inside the central area.
fn render_master_row(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions, geom: RowGeometry) {
    let RowGeometry {
        header,
        track,
        idx: row_idx,
        ..
    } = geom;
    let selected = data.selected_master;
    let painter = ui.painter();
    let color = ui.visuals().weak_text_color();
    painter.rect_filled(
        header,
        2.0,
        color.gamma_multiply(if selected { 0.35 } else { 0.18 }),
    );
    painter.rect_filled(track, 0.0, color.gamma_multiply(0.07));
    painter.text(
        header.left_center() + egui::vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "Master",
        egui::FontId::proportional(12.0),
        ui.visuals().strong_text_color(),
    );

    let response = ui.interact(
        header,
        ui.id().with(("arrangement_master", row_idx)),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, "Master")
    });
    if response.clicked() {
        actions.session.select_master = true;
    }
}

fn render_lane_row(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    geom: RowGeometry,
    lane: &LaneRow<'_>,
) {
    let RowGeometry {
        header,
        track,
        idx: row_idx,
        ..
    } = geom;
    let selected = data.selected_deck == Some((lane.ch_idx, lane.deck_idx));
    {
        let painter = ui.painter();
        if selected {
            let tint = channel_color(lane.ch_idx).gamma_multiply(0.35);
            painter.rect_filled(header, 2.0, tint);
            painter.rect_filled(track, 0.0, tint.gamma_multiply(0.4));
        }
        painter.text(
            header.left_center() + egui::vec2(24.0, 0.0),
            egui::Align2::LEFT_CENTER,
            lane.name,
            egui::FontId::proportional(12.0),
            ui.visuals().text_color(),
        );
        painter.line_segment(
            [track.left_bottom(), track.right_bottom()],
            egui::Stroke::new(0.5_f32, ui.visuals().weak_text_color().gamma_multiply(0.25)),
        );
    }

    regions::render_lane_track(ui, data, actions, geom, lane);

    let response = ui.interact(
        header,
        ui.id().with(("arrangement_lane", row_idx)),
        egui::Sense::click_and_drag(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, lane.name)
    });
    if response.clicked() {
        actions.session.select_deck = Some((lane.ch_idx, lane.deck_idx));
    }
    response.context_menu(|ui| {
        // The same menu the mixer's deck card shows, so a copy made here also
        // carries the deck's regions. See /spec/clipboard.md.
        let subject = clipboard_menu::Subject::deck(lane.uuid, lane.name);
        clipboard_menu::items(ui, data, actions, &subject);
        ui.separator();
        if ui.button("Remove lane").clicked() {
            actions.commands.push(EngineCommand::RemoveLane {
                deck_uuid: lane.uuid.to_string(),
            });
            ui.close();
        }
    });
    handle_lane_reorder(ui, data, actions, geom, lane, &response);

    if lane.has_automation {
        draw_collapse_caret(ui, header, lane, actions);
    }
    if lane.overridden {
        draw_override_badge(ui, header, lane.uuid, actions);
    }
}

/// Drag a lane by its header to reorder the deck within its channel.
///
/// Deliberately the mixer's `DeckDrag` payload and the mixer's `ReorderDeck`
/// command rather than an arrangement-side notion of lane order. Row order is
/// read from the channel's deck list, so there is one order and both views edit
/// it; anything else would let the two drift apart.
///
/// A deck dragged onto another channel's lanes does nothing. Moving a deck
/// between channels is a different operation whose target is a channel rather
/// than a position between two lanes, and it stays in the mixer, which has
/// somewhere to drop it.
fn handle_lane_reorder(
    ui: &egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    geom: RowGeometry,
    lane: &LaneRow<'_>,
    response: &egui::Response,
) {
    if response.drag_started() {
        egui::DragAndDrop::set_payload(
            ui.ctx(),
            DeckDrag {
                deck_uuid: lane.uuid.to_string(),
            },
        );
    }

    let Some(dragged) = egui::DragAndDrop::payload::<DeckDrag>(ui.ctx()) else {
        return;
    };
    // Resolved at drop time rather than at drag start, so a reorder part way
    // through a gesture cannot send a stale ordinal.
    let Some(channel) = data.channels.get(lane.ch_idx) else {
        return;
    };
    let Some(from) = channel
        .decks
        .iter()
        .position(|d| d.uuid == dragged.deck_uuid)
    else {
        return;
    };

    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    if !geom.header.contains(pointer) {
        return;
    }

    let above = pointer.y < geom.header.center().y;
    let gap = if above {
        lane.deck_idx
    } else {
        lane.deck_idx + 1
    };
    let to = reorder_target(from, gap);
    if to == from {
        return;
    }

    let y = if above {
        geom.header.top()
    } else {
        geom.header.bottom()
    };
    ui.painter().line_segment(
        [
            egui::pos2(geom.header.left(), y),
            egui::pos2(geom.track.right(), y),
        ],
        egui::Stroke::new(2.0_f32, channel_color(lane.ch_idx)),
    );

    if response.dnd_release_payload::<DeckDrag>().is_some() {
        actions.commands.push(EngineCommand::ReorderDeck {
            channel_uuid: channel.uuid.clone(),
            from_idx: from,
            to_idx: to,
        });
    }
}

/// Where a deck ends up after being dropped into the gap before `gap`.
///
/// `ReorderDeck` removes before it inserts, so a deck moving down lands one
/// short of the gap it was aimed at. Getting this wrong is off by one in only
/// one of the two directions, which is exactly the kind of bug that survives a
/// quick try in the app.
fn reorder_target(from: usize, gap: usize) -> usize {
    if from < gap {
        gap - 1
    } else {
        gap
    }
}

/// The fold control for a lane's automation rows.
fn draw_collapse_caret(
    ui: &mut egui::Ui,
    header: egui::Rect,
    lane: &LaneRow<'_>,
    actions: &mut UIActions,
) {
    let caret = egui::Rect::from_center_size(
        header.left_center() + egui::vec2(12.0, 0.0),
        egui::vec2(16.0, 16.0),
    );
    let response = ui.interact(
        caret,
        ui.id().with(("arrangement_caret", lane.uuid)),
        egui::Sense::click(),
    );
    let label = if lane.collapsed {
        "Show automation"
    } else {
        "Hide automation"
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    ui.painter().text(
        caret.center(),
        egui::Align2::CENTER_CENTER,
        if lane.collapsed { "▸" } else { "▾" },
        egui::FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
    );
    if response.on_hover_text(label).clicked() {
        actions.commands.push(EngineCommand::SetLaneCollapsed {
            deck_uuid: lane.uuid.to_string(),
            collapsed: !lane.collapsed,
        });
    }
}

/// The "held by hand" marker, and the way back.
///
/// A performer who grabbed a fader needs to see that the arrangement is no
/// longer driving it, and needs somewhere to hand it back; without both, the
/// only route back to automation is to quit and reload.
fn draw_override_badge(
    ui: &mut egui::Ui,
    header: egui::Rect,
    deck_uuid: &str,
    actions: &mut UIActions,
) {
    let dot = egui::Rect::from_center_size(
        header.right_center() - egui::vec2(12.0, 0.0),
        egui::vec2(14.0, 14.0),
    );
    let response = ui.interact(
        dot,
        ui.id().with(("rearm", deck_uuid)),
        egui::Sense::click(),
    );
    // Custom-painted, so it needs an accessible name of its own or it is a dot
    // that only a mouse can find.
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            "Hand back to the arrangement",
        )
    });
    ui.painter()
        .circle_filled(dot.center(), 5.0, egui::Color32::from_rgb(255, 170, 60));
    if response
        .on_hover_text("Held by hand. Click to hand it back to the arrangement.")
        .clicked()
    {
        actions.commands.push(EngineCommand::RearmParam {
            param_key: crate::arrangement::opacity_param_key(deck_uuid),
            seconds: None,
        });
    }
}

fn draw_playhead(ui: &egui::Ui, data: &UIData, track_rect: egui::Rect, axis: TimeAxis) {
    let x = axis.x(data.transport.position);
    if x < track_rect.left() || x > track_rect.right() {
        return;
    }
    ui.painter_at(track_rect).line_segment(
        [
            egui::pos2(x, track_rect.top()),
            egui::pos2(x, track_rect.bottom()),
        ],
        egui::Stroke::new(1.5_f32, super::popovers::transport_color(data)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    pub(super) fn fixture_with_arrangement() -> UIData {
        let mut data = UIData::test_fixture();
        let deck_uuid = data.channels[0].decks[0].uuid.clone();
        let mut lane = crate::arrangement::LaneConfig::new(&deck_uuid);
        lane.regions.push(RegionConfig {
            start: 4.0,
            end: 12.0,
            fade_in: 1.0,
            fade_out: 2.0,
        });
        let config = crate::arrangement::ArrangementConfig {
            lanes: vec![lane],
            ..Default::default()
        };
        data.arrangement = Some(crate::engine::types::ArrangementSnapshot {
            duration: config.duration(),
            config,
            engaged: true,
            overridden_params: vec![],
        });
        data.arrangement_mode_open = true;
        data
    }

    /// Assign a two-point envelope to `param_key`.
    fn push_envelope(data: &mut UIData, uuid: &str, param_key: &str) {
        data.modulation_sources
            .push(super::super::super::ModSourceUIEntry {
                uuid: uuid.to_string(),
                source: ModSourceUI::Envelope {
                    breakpoints: vec![
                        Breakpoint {
                            position: 0.0,
                            value: 0.0,
                            curve: crate::modulation::CurveKind::default(),
                        },
                        Breakpoint {
                            position: 8.0,
                            value: 1.0,
                            curve: crate::modulation::CurveKind::default(),
                        },
                    ],
                },
                timebase: crate::timebase::Timebase::Transport,
            });
        data.modulation_assignments.insert(
            param_key.to_string(),
            vec![super::super::super::ModAssignmentUI {
                source_id: uuid.to_string(),
                amount: 1.0,
            }],
        );
    }

    /// A deck with one hand-drawn curve on a parameter that is not opacity.
    pub(super) fn fixture_with_automation() -> UIData {
        let mut data = fixture_with_arrangement();
        let deck_uuid = data.channels[0].decks[0].uuid.clone();
        push_envelope(&mut data, "env-speed", &format!("deck_{deck_uuid}:speed"));
        data
    }

    fn axis() -> TimeAxis {
        TimeAxis {
            left: 100.0,
            scroll: 0.0,
            pps: 40.0,
        }
    }

    /// Dropping into a gap is not the same as landing on an index, because the
    /// deck is removed before it is inserted. Both directions are checked here
    /// because only one of them is off by one.
    #[test]
    fn a_deck_lands_in_the_gap_it_was_dropped_into() {
        // Moving down: dropping deck 0 below deck 2 (gap 3) leaves it last of
        // three, at index 2.
        assert_eq!(reorder_target(0, 3), 2);
        assert_eq!(reorder_target(0, 1), 0, "the gap just below is a no-op");

        // Moving up: nothing is removed from above it first, so the gap is the
        // destination.
        assert_eq!(reorder_target(2, 0), 0);
        assert_eq!(reorder_target(2, 2), 2, "the gap just above is a no-op");
    }

    #[test]
    fn render_arrangement_smoke() {
        let data = fixture_with_arrangement();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_arrangement(ui, &data, &mut actions);
        });
    }

    /// A scene with no arrangement still opens the mode: that is where the first
    /// region gets made.
    #[test]
    fn render_arrangement_without_one_is_still_a_timeline() {
        let mut data = UIData::test_fixture();
        data.arrangement_mode_open = true;
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_arrangement(ui, &data, &mut actions);
        });
    }

    /// Every deck gets a row whether or not the arrangement claimed it, so an
    /// empty arrangement does not look like an empty scene.
    #[test]
    fn every_deck_gets_a_lane() {
        let data = fixture_with_arrangement();
        let rows = build_rows(&data);
        let lanes = rows.iter().filter(|r| matches!(r, Row::Lane(_))).count();
        let groups = rows
            .iter()
            .filter(|r| matches!(r, Row::Group { .. }))
            .count();
        let decks: usize = data.channels.iter().map(|c| c.decks.len()).sum();

        assert_eq!(lanes, decks);
        assert_eq!(groups, data.channels.len());
    }

    /// Only the lane whose deck the arrangement drives carries regions.
    #[test]
    fn regions_land_on_the_lane_that_owns_them() {
        let data = fixture_with_arrangement();
        let arranged = data.channels[0].decks[0].uuid.clone();
        for row in build_rows(&data) {
            if let Row::Lane(lane) = row {
                if lane.uuid == arranged {
                    assert_eq!(lane.regions.len(), 1, "the arranged lane keeps its region");
                } else {
                    assert!(
                        lane.regions.is_empty(),
                        "lane {} should be empty",
                        lane.uuid
                    );
                }
            }
        }
    }

    #[test]
    fn the_override_badge_follows_the_held_parameter() {
        let mut data = fixture_with_arrangement();
        let held = data.channels[0].decks[0].uuid.clone();
        data.arrangement.as_mut().unwrap().overridden_params =
            vec![crate::arrangement::opacity_param_key(&held)];

        let flagged: Vec<&str> = build_rows(&data)
            .into_iter()
            .filter_map(|r| match r {
                Row::Lane(lane) if lane.overridden => Some(lane.uuid),
                _ => None,
            })
            .collect();
        assert_eq!(flagged, vec![held.as_str()]);
    }

    /// An automated parameter gets its own row under the deck it belongs to.
    #[test]
    fn an_automated_parameter_gets_a_row() {
        let data = fixture_with_automation();
        let curves: Vec<&str> = build_rows(&data)
            .into_iter()
            .filter_map(|r| match r {
                Row::Automation(curve) => Some(curve.envelope_uuid),
                _ => None,
            })
            .collect();
        assert_eq!(curves, vec!["env-speed"]);
    }

    /// The region-compiled opacity curve is authored by dragging regions, so it
    /// must not also appear as a hand-editable row.
    #[test]
    fn the_region_curve_is_not_offered_for_hand_editing() {
        let mut data = fixture_with_automation();
        let deck_uuid = data.channels[0].decks[0].uuid.clone();
        let key = crate::arrangement::opacity_param_key(&deck_uuid);
        data.modulation_sources
            .push(super::super::super::ModSourceUIEntry {
                uuid: "env-opacity".to_string(),
                source: ModSourceUI::Envelope {
                    breakpoints: vec![],
                },
                timebase: crate::timebase::Timebase::Transport,
            });
        data.modulation_assignments.insert(
            key.clone(),
            vec![super::super::super::ModAssignmentUI {
                source_id: "env-opacity".to_string(),
                amount: 1.0,
            }],
        );
        let arrangement = data.arrangement.as_mut().unwrap();
        arrangement.config.lanes[0]
            .envelopes
            .insert(key, "env-opacity".to_string());

        let curves: Vec<&str> = build_rows(&data)
            .into_iter()
            .filter_map(|r| match r {
                Row::Automation(curve) => Some(curve.envelope_uuid),
                _ => None,
            })
            .collect();
        assert_eq!(curves, vec!["env-speed"]);
    }

    /// Folding a lane hides its curves but keeps the lane itself.
    #[test]
    fn collapsing_a_lane_folds_its_automation_away() {
        let mut data = fixture_with_automation();
        data.arrangement.as_mut().unwrap().config.lanes[0].collapsed = true;

        let rows = build_rows(&data);
        assert!(!rows.iter().any(|r| matches!(r, Row::Automation(_))));
        assert!(rows.iter().any(|r| matches!(r, Row::Lane(_))));
    }

    /// Position and pixel must round-trip, or click-to-locate lands somewhere
    /// other than where it was clicked.
    #[test]
    fn the_time_axis_round_trips() {
        let axis = axis();
        for seconds in [0.0, 1.5, 90.0, 3600.0] {
            let back = axis.seconds(axis.x(seconds));
            assert!(
                (back - seconds).abs() < 0.05,
                "{seconds} came back as {back}"
            );
        }
    }

    #[test]
    fn scrolling_moves_the_window_not_the_content() {
        let scrolled = TimeAxis {
            scroll: 10.0,
            ..axis()
        };
        // With ten seconds scrolled off the left, t=10 sits at the left edge.
        assert!((scrolled.x(10.0) - axis().left).abs() < 0.001);
        // And the same instant has moved left by exactly ten seconds of pixels.
        assert!((axis().x(10.0) - scrolled.x(10.0) - 400.0).abs() < 0.001);
    }

    /// Labels must not collide at any zoom, and must not thin out to nothing
    /// when zoomed in.
    #[test]
    fn tick_spacing_stays_legible_at_every_zoom() {
        for pps in [
            MIN_PIXELS_PER_SECOND,
            1.0,
            40.0,
            200.0,
            MAX_PIXELS_PER_SECOND,
        ] {
            let step = tick_step(pps);
            assert!(step > 0.0, "zoom {pps} produced a non-positive step");
            // The coarsest step is allowed to fall short at the very lowest
            // zoom; everything else must clear the label width.
            if pps > MIN_PIXELS_PER_SECOND {
                assert!(
                    step * f64::from(pps) >= 72.0,
                    "labels would overlap at {pps} px/s"
                );
            }
        }
    }

    /// Panning is bounded past the end of the show, so a flick of the wheel
    /// cannot strand the view in empty time with nothing to navigate back by.
    #[test]
    fn panning_stops_past_the_end_of_the_show() {
        let data = fixture_with_arrangement();
        let authored = data.arrangement.as_ref().unwrap().duration;
        let limit = max_scroll(&data);
        assert!(limit > authored, "there must be room past the last region");
        assert!(limit < authored + 60.0, "but not unbounded room");
    }

    /// Zooming about the pointer is what makes a long show navigable: the frame
    /// being looked at has to stay under the finger while the scale changes
    /// around it.
    #[test]
    fn a_pinch_holds_the_instant_under_the_pointer() {
        // Well into the show, so a zoom out has somewhere to go: an anchor near
        // zero cannot be held, because holding it would put the view before the
        // start of the show.
        let axis = TimeAxis {
            scroll: 600.0,
            ..axis()
        };
        let pointer_x = axis.left + 300.0;
        let looked_at = axis.seconds(pointer_x);

        for factor in [1.1, 1.9, 0.9, 0.5] {
            let (pps, scroll) = zoomed(axis, pointer_x, factor);
            let after = TimeAxis {
                scroll,
                pps,
                ..axis
            };
            assert!(
                (after.seconds(pointer_x) - looked_at).abs() < 0.001,
                "a pinch of {factor} moved the instant under the pointer"
            );
        }
    }

    /// A pinch out and back has to leave the view where it started, which it
    /// only does if the gesture scales the timebase rather than adding to it.
    #[test]
    fn pinching_out_and_back_returns_to_the_same_view() {
        let axis = TimeAxis {
            scroll: 600.0,
            ..axis()
        };
        let pointer_x = axis.left + 200.0;

        let (pps, scroll) = zoomed(axis, pointer_x, 1.4);
        let (back, scroll_back) = zoomed(
            TimeAxis {
                scroll,
                pps,
                ..axis
            },
            pointer_x,
            1.0 / 1.4,
        );

        assert!(
            (back - axis.pps).abs() < 0.001,
            "{back} is not {}",
            axis.pps
        );
        assert!((scroll_back - axis.scroll).abs() < 0.001);
    }

    /// The scale is bounded at both ends, and a gesture that runs into a bound
    /// still has to leave the pointer somewhere sensible rather than at a
    /// negative position.
    #[test]
    fn a_pinch_stops_at_the_ends_of_the_scale() {
        let axis = axis();
        let pointer_x = axis.left + 50.0;

        let (widest, scroll) = zoomed(axis, pointer_x, 0.000_1);
        assert!((widest - MIN_PIXELS_PER_SECOND).abs() < f32::EPSILON);
        assert!(scroll >= 0.0, "{scroll} is before the start of the show");

        let (tightest, scroll) = zoomed(axis, pointer_x, 10_000.0);
        assert!((tightest - MAX_PIXELS_PER_SECOND).abs() < f32::EPSILON);
        assert!(scroll >= 0.0);
    }

    /// The gesture arrives from egui as a zoom factor whether it was a trackpad
    /// pinch or Cmd held on the wheel, so this covers both entry points.
    #[test]
    fn a_pinch_over_the_timeline_zooms_the_timescale() {
        let data = fixture_with_arrangement();
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_arrangement(ui, &data, &mut actions);
        });
        // Over the tracks rather than the headers, and past the ruler.
        harness.event(egui::Event::PointerMoved(egui::pos2(400.0, 200.0)));
        harness.event(egui::Event::Zoom(1.5));
        harness.run();
        drop(harness);

        let zoom = actions
            .session
            .set_arrangement_zoom
            .expect("a pinch must change the timescale");
        assert!(
            zoom > data.arrangement_pixels_per_second,
            "pinching out has to widen the scale, got {zoom}"
        );
    }

    /// Rows are laid out in one pass so the header and the track for a lane are
    /// the same strip of screen. Drift here is the bug the single-rect layout
    /// exists to prevent.
    #[test]
    fn a_lane_header_and_its_track_share_a_row() {
        let data = fixture_with_automation();
        let rows = build_rows(&data);
        let lanes = egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(600.0, 800.0));
        let spans = row_spans(&rows, lanes, 0.0);

        // Rows tile without gaps or overlaps, which is what keeps the two
        // columns aligned however the row heights change.
        for pair in spans.windows(2) {
            assert!(
                (pair[0].0 + pair[0].1 - pair[1].0).abs() < f32::EPSILON,
                "rows {pair:?} do not tile"
            );
        }
        assert!((spans[0].0 - lanes.top()).abs() < f32::EPSILON);
    }

    /// Scrolling moves every row by the same amount, headers included. Anything
    /// else would slide a lane's name away from its regions.
    #[test]
    fn scrolling_the_rows_moves_all_of_them_together() {
        let data = fixture_with_automation();
        let rows = build_rows(&data);
        let lanes = egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(600.0, 800.0));

        let still = row_spans(&rows, lanes, 0.0);
        let scrolled = row_spans(&rows, lanes, 50.0);

        for (a, b) in still.iter().zip(&scrolled) {
            assert!((a.0 - b.0 - 50.0).abs() < f32::EPSILON, "{a:?} vs {b:?}");
            assert!((a.1 - b.1).abs() < f32::EPSILON, "heights do not scroll");
        }
    }

    /// A show with more channels than fit has to be reachable to the last row,
    /// and no further: scrolling past the end would leave the timeline blank
    /// with nothing to navigate back by.
    #[test]
    fn the_rows_scroll_exactly_far_enough_to_reach_the_last_one() {
        let data = fixture_with_automation();
        let rows = build_rows(&data);
        let content = content_height(&rows);
        assert!(content > 0.0);

        let roomy = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, content));
        assert!(
            max_scroll_y(&rows, roomy).abs() < f32::EPSILON,
            "rows that all fit do not scroll at all"
        );

        let cramped =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, content / 2.0));
        let limit = max_scroll_y(&rows, cramped);
        let last = *row_spans(&rows, cramped, limit)
            .last()
            .expect("a fixture with rows");
        assert!(
            (last.0 + last.1 - cramped.bottom()).abs() < 0.001,
            "the last row sits exactly on the bottom edge at full scroll"
        );
    }

    /// The bug this fixes: rows below the fold used to stop being drawn, so a
    /// scene with more channels than fit could not be managed at all.
    #[test]
    fn a_row_below_the_fold_is_reachable_by_scrolling() {
        let data = fixture_with_automation();
        let rows = build_rows(&data);
        // A viewport too short for even the first two rows.
        let lanes = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 24.0));
        let limit = max_scroll_y(&rows, lanes);
        assert!(
            limit > 0.0,
            "the fixture must overflow for this to mean anything"
        );

        let visible = |scroll_y: f32| {
            row_spans(&rows, lanes, scroll_y)
                .into_iter()
                .enumerate()
                .filter(|(_, (top, height))| *top + *height > lanes.top() && *top < lanes.bottom())
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        };

        assert!(!visible(0.0).contains(&(rows.len() - 1)));
        assert!(
            visible(limit).contains(&(rows.len() - 1)),
            "the last row must be on screen once scrolled to the end"
        );
    }

    /// Dropping a generator on a group has to create the deck, which it does by
    /// publishing the same drop rect the mixer's channel columns publish. If
    /// this stops being written the library drag silently does nothing in
    /// Arrangement mode.
    #[test]
    fn every_group_offers_itself_as_a_library_drop_target() {
        let data = fixture_with_arrangement();
        let mut actions = UIActions::new();
        let harness = egui_kittest::Harness::new_ui(|ui| {
            render_arrangement(ui, &data, &mut actions);
        });

        for ch in &data.channels {
            let key = egui::Id::new("ch_drop_rect").with(ch.ch_idx);
            let rect: Option<egui::Rect> = harness
                .ctx
                .memory(|mem| mem.data.get_temp::<egui::Rect>(key));
            assert!(
                rect.is_some_and(|r| r.width() > 0.0),
                "channel {} published no drop target",
                ch.ch_idx
            );
        }
    }

    /// Master effect parameters are automatable from the bottom bar, so their
    /// curves need a home. Without the master row they would run with no editor.
    #[test]
    fn a_master_effect_curve_lands_on_the_master_row() {
        let mut data = fixture_with_arrangement();
        let fx = data.master_effect_info[0].0.clone();
        push_envelope(&mut data, "env-master", &format!("fx_{fx}:intensity"));

        let rows = build_rows(&data);
        assert!(
            rows.iter().any(|r| matches!(r, Row::Master)),
            "the master row is drawn whether or not it holds anything"
        );
        let master = rows
            .iter()
            .filter_map(|r| match r {
                Row::Automation(curve) if matches!(curve.owner, Owner::Master) => Some(curve),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(master.len(), 1);
        assert_eq!(master[0].envelope_uuid, "env-master");
        assert_eq!(master[0].label(), "master_effect · intensity");
    }

    /// A channel effect belongs to the channel rather than to any one deck, so
    /// its curve sits under the group rather than under an arbitrary lane.
    #[test]
    fn a_channel_effect_curve_lands_under_its_group() {
        let mut data = fixture_with_arrangement();
        let fx = data.channels[0].effects[0].0.clone();
        push_envelope(&mut data, "env-channel", &format!("fx_{fx}:mix"));

        let owners: Vec<usize> = build_rows(&data)
            .into_iter()
            .filter_map(|r| match r {
                Row::Automation(curve) => match curve.owner {
                    Owner::Channel(ch_idx) => Some(ch_idx),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(owners, vec![data.channels[0].ch_idx]);
    }

    /// A deck effect's curve belongs to the deck's lane, named for the effect so
    /// two effects sharing a parameter name stay distinguishable.
    #[test]
    fn a_deck_effect_curve_is_named_for_its_effect() {
        let mut data = fixture_with_arrangement();
        let fx = data.channels[0].decks[0].effects[0].0.clone();
        push_envelope(&mut data, "env-fx", &format!("fx_{fx}:amount"));

        let labels: Vec<String> = build_rows(&data)
            .into_iter()
            .filter_map(|r| match r {
                Row::Automation(curve) => Some(curve.label().to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["test_effect · amount"]);
    }

    /// Nothing held means nothing to hand back, so the button stays out of the
    /// way until it means something.
    #[test]
    fn re_arm_all_appears_only_while_something_is_held() {
        let data = fixture_with_arrangement();
        let mut actions = UIActions::new();
        let quiet = egui_kittest::Harness::new_ui(|ui| {
            render_transport_strip(ui, &data, &mut actions);
        });
        assert!(
            quiet.query_by_label_contains("Re-arm all").is_none(),
            "an unheld arrangement should not offer a re-arm"
        );

        let mut held = fixture_with_arrangement();
        let uuid = held.channels[0].decks[0].uuid.clone();
        held.arrangement.as_mut().unwrap().overridden_params =
            vec![crate::arrangement::opacity_param_key(&uuid)];
        let mut actions = UIActions::new();
        {
            let mut harness = egui_kittest::Harness::new_ui(|ui| {
                render_transport_strip(ui, &held, &mut actions);
            });
            harness.run();
            harness.get_by_label_contains("Re-arm all").click();
            harness.run();
        }
        assert!(
            actions
                .commands
                .iter()
                .any(|c| matches!(c, EngineCommand::RearmAll { .. })),
            "clicking it must hand everything back"
        );
    }

    /// Idle behaviour is the black-screen safeguard, so it has to be reachable
    /// without the API.
    #[test]
    fn the_idle_behaviour_is_pickable_from_the_timeline() {
        let data = fixture_with_arrangement();
        let deck_name = data.channels[0].decks[0].name.clone();
        let mut actions = UIActions::new();
        {
            let mut harness = egui_kittest::Harness::new_ui(|ui| {
                render_transport_strip(ui, &data, &mut actions);
            });
            harness.run();
            harness.get_by_value("Hold performance").click();
            harness.run();
            harness
                .get_by_label_contains(&format!("Show {deck_name}"))
                .click();
            harness.run();
        }
        assert!(actions.commands.iter().any(|c| matches!(
            c,
            EngineCommand::SetIdleBehaviour {
                idle: crate::arrangement::IdleBehaviour::ShowDeck { .. }
            }
        )));
    }

    /// Snapping rounds to whole frames, and only when it is asked to.
    #[test]
    fn snapping_rounds_to_the_rulers_frame() {
        let mut data = fixture_with_arrangement();
        data.transport.timecode_rate = crate::transport::TimecodeRate::Fps25;

        data.arrangement_snap = true;
        assert!((snap_seconds(&data, 1.031) - 1.04).abs() < 1e-9);
        assert!((snap_seconds(&data, -3.0)).abs() < f64::EPSILON);

        data.arrangement_snap = false;
        assert!((snap_seconds(&data, 1.031) - 1.031).abs() < 1e-9);
    }
}
