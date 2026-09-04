//! Arrangement selection and the slice clipboard.
//!
//! One active selection at a time: a rectangle in show space scoped to a single
//! channel, a time span crossed with the deck and automation rows it covers. It
//! is what Copy and Delete act on when it is present, and it is drawn as a
//! marquee over the tracks. See /spec/arrangement-selection.md.
//!
//! Selection geometry lives in egui memory, like the focus range and the
//! breakpoint clipboard, rather than in `UIData`: it is a view concern the
//! engine never sees. The mutations it drives (`AddRegion`, `RemoveRegion`,
//! `SetEnvelopeBreakpoints`) are the same undoable engine commands a hand edit
//! uses, so a whole Delete or Paste collapses to one history entry because it is
//! one frame's worth of commands.

use super::super::super::{ModSourceUI, UIData};
use crate::arrangement::RegionConfig;
use crate::engine::EngineCommand;
use crate::modulation::{Breakpoint, CurveKind, evaluate_envelope};

/// The active arrangement selection: a time × lanes rectangle over the
/// timeline's whole stack of rows, channel boundaries included.
///
/// A single clicked region is the same object with one deck lane and the
/// region's own span, so membership needs no special case for it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Selection {
    pub start: f64,
    pub end: f64,
    /// Deck lanes (by deck UUID) the selection covers.
    pub decks: Vec<String>,
    /// Automation rows (by envelope UUID) the selection covers.
    pub envelopes: Vec<String>,
}

impl Selection {
    pub(super) fn includes_deck(&self, uuid: &str) -> bool {
        self.decks.iter().any(|u| u == uuid)
    }

    pub(super) fn includes_envelope(&self, uuid: &str) -> bool {
        self.envelopes.iter().any(|u| u == uuid)
    }

    /// Whether an armed selection owns a press at `at` on this row, so the row's
    /// own bare drag stands down and the whole selection moves instead.
    fn owns(&self, at: f64, member: bool) -> bool {
        member && at >= self.start && at <= self.end
    }

    /// The same selection after a move: shifted in time, with each member deck
    /// swapped for the lane it landed on.
    pub(super) fn moved(&self, delta: f64, lane_map: &[(String, String)]) -> Self {
        Self {
            start: self.start + delta,
            end: self.end + delta,
            decks: self
                .decks
                .iter()
                .map(|uuid| target_lane(lane_map, uuid).to_string())
                .collect(),
            envelopes: self.envelopes.clone(),
        }
    }
}

/// Whether an armed selection owns a press on this deck lane at `at`.
pub(super) fn owns_deck_press(ctx: &egui::Context, deck_uuid: &str, at: f64) -> bool {
    load(ctx).is_some_and(|s| s.owns(at, s.includes_deck(deck_uuid)))
}

/// Whether an armed selection owns a press on this automation row at `at`.
pub(super) fn owns_envelope_press(ctx: &egui::Context, envelope_uuid: &str, at: f64) -> bool {
    load(ctx).is_some_and(|s| s.owns(at, s.includes_envelope(envelope_uuid)))
}

/// A portable arrangement slice: regions and curve pieces rebased so the
/// selection's start sits at time 0. Paste re-bases them onto its anchor.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Slice {
    pub duration: f64,
    /// Regions from every selected deck lane, flattened.
    pub regions: Vec<RegionConfig>,
    /// Breakpoints from every selected envelope, flattened and sorted. Almost
    /// always one curve; merging is harmless when it is more.
    pub curve: Vec<Breakpoint>,
}

impl Slice {
    fn is_empty(&self) -> bool {
        self.regions.is_empty() && self.curve.is_empty()
    }
}

/// Where a paste lands: a deck lane takes region parts, an automation lane takes
/// curve parts. Mixed slices drop the half the target cannot hold.
pub(super) enum PasteTarget {
    Deck(String),
    Envelope(String),
}

// ── Memory ───────────────────────────────────────────────────────────

fn selection_id() -> egui::Id {
    egui::Id::new("__arrangement_selection")
}

fn slice_id() -> egui::Id {
    egui::Id::new("__arrangement_slice_clipboard")
}

pub(super) fn load(ctx: &egui::Context) -> Option<Selection> {
    ctx.memory(|mem| mem.data.get_temp(selection_id()))
}

pub(super) fn store(ctx: &egui::Context, selection: Selection) {
    ctx.memory_mut(|mem| mem.data.insert_temp(selection_id(), selection));
}

pub(super) fn clear(ctx: &egui::Context) {
    ctx.memory_mut(|mem| mem.data.remove::<Selection>(selection_id()));
}

fn load_slice(ctx: &egui::Context) -> Option<Slice> {
    ctx.memory(|mem| mem.data.get_temp(slice_id()))
}

fn store_slice(ctx: &egui::Context, slice: Slice) {
    ctx.memory_mut(|mem| mem.data.insert_temp(slice_id(), slice));
}

// ── Data lookups ─────────────────────────────────────────────────────

/// The regions on a deck's lane, or an empty slice when the deck has none.
fn lane_regions<'a>(data: &'a UIData, deck_uuid: &str) -> &'a [RegionConfig] {
    data.arrangement
        .as_ref()
        .and_then(|a| a.config.lane(deck_uuid))
        .map_or(&[], |lane| lane.regions.as_slice())
}

/// The breakpoints of an envelope source, if it is an envelope.
fn envelope_breakpoints<'a>(data: &'a UIData, envelope_uuid: &str) -> Option<&'a [Breakpoint]> {
    data.modulation_sources
        .iter()
        .find(|entry| entry.uuid == envelope_uuid)
        .and_then(|entry| match &entry.source {
            ModSourceUI::Envelope { breakpoints } => Some(breakpoints.as_slice()),
            _ => None,
        })
}

// ── Membership ───────────────────────────────────────────────────────

/// Whether a region's body overlaps `[start, end]` at all.
///
/// Intersection, not containment: a transition that begins a hair before the
/// drag is still "this clip transition" and has to count.
///
/// An empty span overlaps nothing, and saying so here is what keeps a degenerate
/// selection from reaching the cut. Without the first test a drag that begins and
/// ends on the same beat reads as containing every region it lands inside, and
/// `region_slice` then hands back a selected piece whose start equals its end: a
/// zero-span region, which `RegionConfig::is_valid` rejects and which the cut,
/// copy and move paths would go on to emit as an `AddRegion`. Splitting a region
/// at a point is a coherent thing to want, but it is a different operation from
/// selecting a span, and a zero-width drag is not a request for it.
fn intersects(region: &RegionConfig, start: f64, end: f64) -> bool {
    end > start && region.start < end && region.end > start
}

/// The index of every region on `regions` that the span touches.
fn regions_in_span(regions: &[RegionConfig], start: f64, end: f64) -> Vec<usize> {
    regions
        .iter()
        .enumerate()
        .filter(|(_, r)| intersects(r, start, end))
        .map(|(i, _)| i)
        .collect()
}

/// The part of one region inside a selection and the zero, one, or two pieces
/// outside it.
///
/// A cut boundary is hard: only the fragment that still owns the original start
/// keeps its fade-in, and only the fragment that still owns the original end
/// keeps its fade-out. This avoids inventing fades at selection edges.
#[derive(Debug, PartialEq)]
struct RegionSlice {
    selected: RegionConfig,
    remainders: Vec<RegionConfig>,
}

fn region_slice(region: &RegionConfig, start: f64, end: f64) -> Option<RegionSlice> {
    if !intersects(region, start, end) {
        return None;
    }

    let selected_start = region.start.max(start);
    let selected_end = region.end.min(end);
    let selected = RegionConfig {
        start: selected_start,
        end: selected_end,
        fade_in: if start <= region.start {
            region.fade_in
        } else {
            0.0
        },
        fade_out: if end >= region.end {
            region.fade_out
        } else {
            0.0
        },
    };

    let mut remainders = Vec::with_capacity(2);
    if region.start < selected_start {
        remainders.push(RegionConfig {
            start: region.start,
            end: selected_start,
            fade_in: region.fade_in,
            fade_out: 0.0,
        });
    }
    if selected_end < region.end {
        remainders.push(RegionConfig {
            start: selected_end,
            end: region.end,
            fade_in: 0.0,
            fade_out: region.fade_out,
        });
    }

    Some(RegionSlice {
        selected,
        remainders,
    })
}

/// The curve kind that owns time `t`: the shape leaving the last breakpoint at
/// or before it, or a plain line before the first point.
fn kind_at(points: &[Breakpoint], t: f64) -> CurveKind {
    points
        .iter()
        .rev()
        .find(|p| p.position <= t)
        .map_or_else(CurveKind::default, |p| p.curve)
}

/// The audible shape of a curve over `[start, end]`, with synthesized edge
/// points so the slice matches what sat under the marquee even where no authored
/// breakpoint fell on a boundary. Positions are absolute.
fn curve_slice(points: &[Breakpoint], start: f64, end: f64) -> Vec<Breakpoint> {
    if points.is_empty() || end <= start {
        return Vec::new();
    }
    let mut cursor = 0;
    let start_value = evaluate_envelope(points, start, &mut cursor);
    let mut out = vec![Breakpoint {
        position: start,
        value: start_value,
        curve: kind_at(points, start),
    }];
    out.extend(
        points
            .iter()
            .filter(|p| p.position > start && p.position < end)
            .copied(),
    );
    let end_value = evaluate_envelope(points, end, &mut cursor);
    out.push(Breakpoint {
        position: end,
        // The edge's own kind governs the segment leaving it, which is outside
        // the slice, so a plain line is the least surprising default.
        value: end_value,
        curve: CurveKind::default(),
    });
    out
}

/// A curve with `[start, end]` cleared and continuity kept outside it: the
/// boundary values become lasting breakpoints so the outside shape does not
/// jump, and the interior is filled by a straight line between them.
fn curve_cleared(points: &[Breakpoint], start: f64, end: f64) -> Vec<Breakpoint> {
    if points.is_empty() || end <= start {
        return points.to_vec();
    }
    let mut cursor = 0;
    let start_value = evaluate_envelope(points, start, &mut cursor);
    let end_value = evaluate_envelope(points, end, &mut cursor);
    let mut out: Vec<Breakpoint> = points
        .iter()
        .filter(|p| p.position < start)
        .copied()
        .collect();
    out.push(Breakpoint {
        position: start,
        value: start_value,
        curve: CurveKind::default(),
    });
    out.push(Breakpoint {
        position: end,
        value: end_value,
        // Preserve the shape the curve had leaving `end` into the kept tail.
        curve: kind_at(points, end),
    });
    out.extend(points.iter().filter(|p| p.position > end).copied());
    out.sort_by(|a, b| a.position.total_cmp(&b.position));
    out
}

/// Span-replace paste: clear the target's points under the landing span, then
/// drop the slice in with its relative times rebased onto `anchor`.
fn pasted_curve(
    existing: &[Breakpoint],
    slice: &[Breakpoint],
    anchor: f64,
    duration: f64,
) -> Vec<Breakpoint> {
    let last = anchor + duration;
    let mut out: Vec<Breakpoint> = existing
        .iter()
        .filter(|p| p.position < anchor || p.position > last)
        .copied()
        .collect();
    out.extend(slice.iter().map(|p| Breakpoint {
        position: p.position + anchor,
        ..*p
    }));
    out.sort_by(|a, b| a.position.total_cmp(&b.position));
    out
}

/// A curve with the shape under `[start, end]` picked up and put down `delta`
/// away: the source span is cleared first (unless this is a duplicate, which
/// leaves the original behind) and the landing span is replaced by the slice.
///
/// One pass rather than a clear followed by a paste, so a move whose source and
/// destination overlap cannot clear away what it has just laid down.
fn moved_curve(
    points: &[Breakpoint],
    start: f64,
    end: f64,
    delta: f64,
    duplicate: bool,
) -> Vec<Breakpoint> {
    if points.is_empty() || end <= start {
        return points.to_vec();
    }
    let slice: Vec<Breakpoint> = curve_slice(points, start, end)
        .into_iter()
        .map(|point| Breakpoint {
            position: point.position - start,
            ..point
        })
        .collect();
    let base = if duplicate {
        points.to_vec()
    } else {
        curve_cleared(points, start, end)
    };
    pasted_curve(&base, &slice, start + delta, end - start)
}

// ── Copy / Delete / Paste ────────────────────────────────────────────

/// Build the slice a selection copies: membership rebased so the selection
/// start is time 0.
fn build_slice(data: &UIData, selection: &Selection) -> Slice {
    let start = selection.start;
    let duration = (selection.end - start).max(0.0);

    let mut regions = Vec::new();
    for deck_uuid in &selection.decks {
        for region in lane_regions(data, deck_uuid) {
            if let Some(sliced) = region_slice(region, selection.start, selection.end) {
                regions.push(RegionConfig {
                    start: sliced.selected.start - start,
                    end: sliced.selected.end - start,
                    ..sliced.selected
                });
            }
        }
    }

    let mut curve = Vec::new();
    for envelope_uuid in &selection.envelopes {
        if let Some(points) = envelope_breakpoints(data, envelope_uuid) {
            curve.extend(
                curve_slice(points, selection.start, selection.end)
                    .into_iter()
                    .map(|p| Breakpoint {
                        position: p.position - start,
                        ..p
                    }),
            );
        }
    }
    curve.sort_by(|a, b| a.position.total_cmp(&b.position));

    Slice {
        duration,
        regions,
        curve,
    }
}

/// Copy the current selection to the slice clipboard. An empty selection copies
/// an empty slice, which Paste then treats as nothing to place.
pub(super) fn copy(ctx: &egui::Context, data: &UIData, selection: &Selection) {
    store_slice(ctx, build_slice(data, selection));
}

/// The commands that delete only the selected part of each member region,
/// preserving any unselected fragments, and clear each selected curve span.
///
/// All removals run before fragment additions so snapshot indices remain valid.
/// The whole batch lands in one frame, hence one undo entry.
pub(super) fn delete_commands(data: &UIData, selection: &Selection) -> Vec<EngineCommand> {
    let mut edits = Vec::new();
    let mut adds = Vec::new();
    for deck_uuid in &selection.decks {
        let regions = lane_regions(data, deck_uuid);
        let mut indices = regions_in_span(regions, selection.start, selection.end);
        indices.sort_unstable_by(|a, b| b.cmp(a));
        for index in indices {
            let Some(sliced) = region_slice(&regions[index], selection.start, selection.end) else {
                continue;
            };
            edits.push(EngineCommand::RemoveRegion {
                deck_uuid: deck_uuid.clone(),
                index,
            });
            adds.extend(
                sliced
                    .remainders
                    .into_iter()
                    .map(|region| EngineCommand::AddRegion {
                        deck_uuid: deck_uuid.clone(),
                        region,
                    }),
            );
        }
    }
    for envelope_uuid in &selection.envelopes {
        if let Some(points) = envelope_breakpoints(data, envelope_uuid) {
            let cleared = curve_cleared(points, selection.start, selection.end);
            if cleared != points {
                edits.push(EngineCommand::SetEnvelopeBreakpoints {
                    uuid: envelope_uuid.clone(),
                    breakpoints: cleared,
                });
            }
        }
    }
    edits.extend(adds);
    edits
}

/// The commands that paste the held slice onto a target row at `anchor`.
///
/// A deck lane takes the region parts, an automation lane takes the curve parts.
/// The other half stays on the clipboard rather than being refused.
pub(super) fn paste_commands(
    ctx: &egui::Context,
    data: &UIData,
    anchor: f64,
    target: &PasteTarget,
) -> Vec<EngineCommand> {
    let Some(slice) = load_slice(ctx) else {
        return Vec::new();
    };
    if slice.is_empty() {
        return Vec::new();
    }
    match target {
        PasteTarget::Deck(deck_uuid) => slice
            .regions
            .iter()
            .filter_map(|region| {
                let start = (region.start + anchor).max(0.0);
                let end = region.end + anchor;
                (end > start).then(|| EngineCommand::AddRegion {
                    deck_uuid: deck_uuid.clone(),
                    region: RegionConfig {
                        start,
                        end,
                        ..*region
                    },
                })
            })
            .collect(),
        PasteTarget::Envelope(envelope_uuid) => {
            if slice.curve.is_empty() {
                return Vec::new();
            }
            let existing = envelope_breakpoints(data, envelope_uuid).unwrap_or(&[]);
            vec![EngineCommand::SetEnvelopeBreakpoints {
                uuid: envelope_uuid.clone(),
                breakpoints: pasted_curve(existing, &slice.curve, anchor, slice.duration),
            }]
        }
    }
}

/// Whether anything is on the slice clipboard, for enabling Paste menu items.
pub(super) fn slice_available(ctx: &egui::Context) -> bool {
    load_slice(ctx).is_some_and(|slice| !slice.is_empty())
}

// ── Move ─────────────────────────────────────────────────────────────

/// The lane a source deck's regions land on, which is the deck itself unless the
/// drag carried them to another row.
fn target_lane<'a>(lane_map: &'a [(String, String)], deck_uuid: &'a str) -> &'a str {
    lane_map
        .iter()
        .find(|(source, _)| source == deck_uuid)
        .map_or(deck_uuid, |(_, target)| target.as_str())
}

/// How far back a selection may be dragged before its cropped payload would sit
/// at a negative time.
pub(super) fn move_floor(_data: &UIData, selection: &Selection) -> f64 {
    selection.start
}

/// The commands that move a selection's membership by `delta`, with each member
/// deck's regions landing on the lane `lane_map` sends it to.
///
/// Every in-place update and removal is emitted before any addition. Indices are
/// read from this frame's snapshot, and a lane that is both a source and another
/// lane's target would otherwise have its regions renumbered underneath its own
/// pending edits.
pub(super) fn move_commands(
    data: &UIData,
    selection: &Selection,
    delta: f64,
    lane_map: &[(String, String)],
    duplicate: bool,
) -> Vec<EngineCommand> {
    let mut edits = Vec::new();
    let mut adds = Vec::new();

    for deck_uuid in &selection.decks {
        let target = target_lane(lane_map, deck_uuid);
        let regions = lane_regions(data, deck_uuid);
        let mut indices = regions_in_span(regions, selection.start, selection.end);
        // Descending, so removing one region cannot renumber another that is
        // still waiting for its own command on the same lane.
        indices.sort_unstable_by(|a, b| b.cmp(a));
        for index in indices {
            let Some(sliced) = region_slice(&regions[index], selection.start, selection.end) else {
                continue;
            };
            let landed = RegionConfig {
                start: sliced.selected.start + delta,
                end: sliced.selected.end + delta,
                ..sliced.selected
            };
            if !landed.is_valid() {
                continue;
            }
            if duplicate {
                adds.push(EngineCommand::AddRegion {
                    deck_uuid: target.to_string(),
                    region: landed,
                });
            } else if target == deck_uuid && sliced.remainders.is_empty() {
                edits.push(EngineCommand::UpdateRegion {
                    deck_uuid: deck_uuid.clone(),
                    index,
                    region: landed,
                });
            } else {
                edits.push(EngineCommand::RemoveRegion {
                    deck_uuid: deck_uuid.clone(),
                    index,
                });
                adds.extend(
                    sliced
                        .remainders
                        .into_iter()
                        .map(|region| EngineCommand::AddRegion {
                            deck_uuid: deck_uuid.clone(),
                            region,
                        }),
                );
                adds.push(EngineCommand::AddRegion {
                    deck_uuid: target.to_string(),
                    region: landed,
                });
            }
        }
    }

    for envelope_uuid in &selection.envelopes {
        if let Some(points) = envelope_breakpoints(data, envelope_uuid) {
            let landed = moved_curve(points, selection.start, selection.end, delta, duplicate);
            if landed != points {
                edits.push(EngineCommand::SetEnvelopeBreakpoints {
                    uuid: envelope_uuid.clone(),
                    breakpoints: landed,
                });
            }
        }
    }

    edits.extend(adds);
    edits
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn linear(position: f64, value: f32) -> Breakpoint {
        Breakpoint {
            position,
            value,
            curve: CurveKind::default(),
        }
    }

    fn ramp() -> Vec<Breakpoint> {
        vec![linear(0.0, 0.0), linear(10.0, 1.0)]
    }

    /// Turn arbitrary generated tuples into a non-empty, sorted envelope with
    /// unique finite positions. The input stays deliberately untidy: sorting,
    /// collisions, one-point curves, and every curve kind all occur in the
    /// generated cases.
    fn hostile_curve(raw: &[(u16, u16, u8)]) -> Vec<Breakpoint> {
        let mut points: Vec<Breakpoint> = raw
            .iter()
            .map(|(position, value, kind)| Breakpoint {
                position: f64::from(*position) / 8.0,
                value: f32::from(*value) / f32::from(u16::MAX),
                curve: match kind % 3 {
                    0 => CurveKind::Step,
                    1 => CurveKind::Smooth,
                    _ => CurveKind::Linear {
                        tension: f32::from(*kind % 9) - 4.0,
                    },
                },
            })
            .collect();
        points.sort_by(|a, b| a.position.total_cmp(&b.position));
        points.dedup_by(|a, b| a.position == b.position);
        if points.is_empty() {
            points.push(linear(0.0, 0.0));
        }
        points
    }

    fn assert_curve_sane(points: &[Breakpoint]) -> Result<(), TestCaseError> {
        prop_assert!(
            points
                .windows(2)
                .all(|pair| pair[0].position <= pair[1].position),
            "curve positions lost their order: {points:?}"
        );
        prop_assert!(
            points.iter().all(|point| {
                point.position.is_finite()
                    && point.value.is_finite()
                    && (0.0..=1.0).contains(&point.value)
            }),
            "curve produced a non-finite or out-of-range point: {points:?}"
        );
        Ok(())
    }

    #[test]
    fn a_region_that_only_overlaps_the_edge_is_a_member() {
        let regions = vec![
            RegionConfig::new(0.0, 5.0),
            RegionConfig::new(8.0, 12.0),
            RegionConfig::new(20.0, 24.0),
        ];
        // The span [4, 10] clips the first and the second but not the third.
        assert_eq!(regions_in_span(&regions, 4.0, 10.0), vec![0, 1]);
    }

    /// Touching at a boundary is not overlapping: a region ending exactly where
    /// the selection starts is not inside it.
    #[test]
    fn a_region_touching_the_boundary_is_not_a_member() {
        let regions = vec![RegionConfig::new(0.0, 4.0), RegionConfig::new(4.0, 8.0)];
        assert_eq!(regions_in_span(&regions, 4.0, 8.0), vec![1]);
    }

    /// A drag that begins and ends on the same beat selects nothing, even where
    /// it lands inside a region. Cutting on it used to produce a piece whose
    /// start equalled its end.
    #[test]
    fn a_zero_width_selection_touches_nothing() {
        let regions = vec![RegionConfig::new(0.0, 5.0), RegionConfig::new(8.0, 12.0)];
        assert!(regions_in_span(&regions, 3.0, 3.0).is_empty());
        assert!(region_slice(&regions[0], 3.0, 3.0).is_none());
    }

    #[test]
    fn a_selection_through_a_region_crops_it_and_leaves_both_sides() {
        let region = RegionConfig::new(2.0, 12.0).with_fades(2.0, 3.0);
        let sliced = region_slice(&region, 5.0, 9.0).expect("the selection intersects");

        assert_eq!(sliced.selected, RegionConfig::new(5.0, 9.0));
        assert_eq!(
            sliced.remainders,
            vec![
                RegionConfig::new(2.0, 5.0).with_fades(2.0, 0.0),
                RegionConfig::new(9.0, 12.0).with_fades(0.0, 3.0),
            ]
        );
    }

    #[test]
    fn a_one_sided_crop_keeps_only_the_original_edge_fade() {
        let region = RegionConfig::new(2.0, 12.0).with_fades(2.0, 3.0);

        let left = region_slice(&region, 0.0, 7.0).expect("the selection reaches the left edge");
        assert_eq!(
            left.selected,
            RegionConfig::new(2.0, 7.0).with_fades(2.0, 0.0)
        );
        assert_eq!(
            left.remainders,
            vec![RegionConfig::new(7.0, 12.0).with_fades(0.0, 3.0)]
        );

        let right = region_slice(&region, 7.0, 20.0).expect("the selection reaches the right edge");
        assert_eq!(
            right.selected,
            RegionConfig::new(7.0, 12.0).with_fades(0.0, 3.0)
        );
        assert_eq!(
            right.remainders,
            vec![RegionConfig::new(2.0, 7.0).with_fades(2.0, 0.0)]
        );
    }

    #[test]
    fn selecting_a_whole_region_preserves_it_without_fragments() {
        let region = RegionConfig::new(2.0, 12.0).with_fades(2.0, 3.0);
        let sliced = region_slice(&region, 0.0, 20.0).expect("the selection contains the region");

        assert_eq!(sliced.selected, region);
        assert!(sliced.remainders.is_empty());
    }

    /// The synthesized edges have to read the same value the renderer would draw
    /// at those instants, or a pasted slice would not match what was under the
    /// marquee.
    #[test]
    fn curve_slice_edges_match_the_evaluator() {
        let points = ramp();
        let slice = curve_slice(&points, 2.0, 6.0);
        let mut cursor = 0;
        assert!(
            (slice.first().unwrap().value - evaluate_envelope(&points, 2.0, &mut cursor)).abs()
                < 1e-6
        );
        assert!(
            (slice.last().unwrap().value - evaluate_envelope(&points, 6.0, &mut cursor)).abs()
                < 1e-6
        );
        assert!((slice.first().unwrap().position - 2.0).abs() < 1e-9);
        assert!((slice.last().unwrap().position - 6.0).abs() < 1e-9);
    }

    /// Interior authored points survive the slice with their shapes.
    #[test]
    fn curve_slice_keeps_interior_points() {
        let points = vec![
            linear(0.0, 0.0),
            Breakpoint {
                position: 3.0,
                value: 0.5,
                curve: CurveKind::Step,
            },
            linear(10.0, 1.0),
        ];
        let slice = curve_slice(&points, 1.0, 6.0);
        let interior: Vec<f64> = slice
            .iter()
            .filter(|p| p.position > 1.0 && p.position < 6.0)
            .map(|p| p.position)
            .collect();
        assert_eq!(interior, vec![3.0]);
        assert!(matches!(
            slice
                .iter()
                .find(|p| (p.position - 3.0).abs() < 1e-9)
                .unwrap()
                .curve,
            CurveKind::Step
        ));
    }

    /// Deleting a span leaves the shape outside it exactly where it was, with no
    /// jump at the boundaries.
    #[test]
    fn clearing_a_span_keeps_the_outside_continuous() {
        let points = ramp();
        let mut cursor = 0;
        let at_two = evaluate_envelope(&points, 2.0, &mut cursor);
        let at_six = evaluate_envelope(&points, 6.0, &mut cursor);

        let cleared = curve_cleared(&points, 2.0, 6.0);
        // The boundary values are pinned so the tails do not move.
        let boundary_start = cleared
            .iter()
            .find(|p| (p.position - 2.0).abs() < 1e-9)
            .unwrap();
        let boundary_end = cleared
            .iter()
            .find(|p| (p.position - 6.0).abs() < 1e-9)
            .unwrap();
        assert!((boundary_start.value - at_two).abs() < 1e-6);
        assert!((boundary_end.value - at_six).abs() < 1e-6);

        // Evaluating the cleared curve outside the span matches the original.
        let mut c = 0;
        assert!(
            (evaluate_envelope(&cleared, 0.0, &mut c) - evaluate_envelope(&points, 0.0, &mut c))
                .abs()
                < 1e-6
        );
        assert!((evaluate_envelope(&cleared, 10.0, &mut c) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_slice_is_rebased_to_zero_and_paste_puts_it_back() {
        let existing: Vec<Breakpoint> = Vec::new();
        let slice = vec![linear(0.0, 0.2), linear(4.0, 0.8)];
        let out = pasted_curve(&existing, &slice, 10.0, 4.0);
        assert!((out[0].position - 10.0).abs() < 1e-9);
        assert!((out[1].position - 14.0).abs() < 1e-9);
    }

    /// Paste clears the landing span first, so a pasted curve never fights the
    /// points it landed on.
    #[test]
    fn paste_replaces_the_span_it_covers() {
        let existing = vec![linear(0.0, 0.0), linear(12.0, 0.5), linear(30.0, 1.0)];
        let slice = vec![linear(0.0, 1.0), linear(4.0, 0.0)];
        let out = pasted_curve(&existing, &slice, 10.0, 4.0);
        assert!(
            !out.iter().any(|p| (p.position - 12.0).abs() < 1e-9),
            "the covered point is gone"
        );
        assert!(
            out.iter().any(|p| (p.position - 30.0).abs() < 1e-9),
            "points past the span survive"
        );
        assert!(out.windows(2).all(|w| w[0].position <= w[1].position));
    }

    /// A channel whose first lane holds two regions and whose second lane is
    /// empty, so a move can be watched both for index safety on the source and
    /// for landing intact on the target.
    fn fixture_two_lanes() -> (UIData, String, String) {
        let mut data = super::super::tests::fixture_with_automation();
        let source = data.channels[0].decks[0].uuid.clone();
        let target = data.channels[1].decks[0].uuid.clone();
        let arrangement = data.arrangement.as_mut().expect("the fixture arranges");
        // The fixture's lane already holds 4..12; a second region is what makes
        // the descending-index rule mean anything.
        arrangement.config.lanes[0]
            .regions
            .push(RegionConfig::new(14.0, 18.0));
        arrangement
            .config
            .lanes
            .push(crate::arrangement::LaneConfig::new(&target));
        (data, source, target)
    }

    fn spanning(deck_uuid: &str, start: f64, end: f64) -> Selection {
        Selection {
            start,
            end,
            decks: vec![deck_uuid.to_string()],
            envelopes: Vec::new(),
        }
    }

    fn kinked() -> Vec<Breakpoint> {
        vec![linear(0.0, 0.0), linear(4.0, 0.5), linear(20.0, 1.0)]
    }

    /// Moving a curve slice takes the shape with it and leaves the source span
    /// flat, in one pass so an overlapping destination cannot clear away what
    /// the move has just laid down.
    #[test]
    fn moving_a_curve_slice_clears_its_source_and_lands_at_the_destination() {
        let moved = moved_curve(&kinked(), 2.0, 6.0, 10.0, false);

        assert!(
            !moved.iter().any(|p| (p.position - 4.0).abs() < 1e-9),
            "the authored point left its old home: {moved:?}"
        );
        let carried = moved
            .iter()
            .find(|p| (p.position - 14.0).abs() < 1e-9)
            .expect("the point rides along, four seconds into the landing");
        assert!((carried.value - 0.5).abs() < 1e-6);
        assert!(moved.iter().any(|p| (p.position - 12.0).abs() < 1e-9));
        assert!(moved.windows(2).all(|w| w[0].position <= w[1].position));
    }

    #[test]
    fn duplicating_a_curve_slice_leaves_the_source_where_it_was() {
        let copied = moved_curve(&kinked(), 2.0, 6.0, 10.0, true);
        assert!(
            copied.iter().any(|p| (p.position - 4.0).abs() < 1e-9),
            "an Alt drag keeps the original: {copied:?}"
        );
        assert!(copied.iter().any(|p| (p.position - 14.0).abs() < 1e-9));
    }

    #[test]
    fn copying_part_of_a_region_copies_only_the_crop() {
        let (data, source, _) = fixture_two_lanes();
        let slice = build_slice(&data, &spanning(&source, 6.0, 10.0));

        assert_eq!(slice.regions, vec![RegionConfig::new(0.0, 4.0)]);
        assert!((slice.duration - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn deleting_part_of_a_region_leaves_the_unselected_fragments() {
        let (data, source, _) = fixture_two_lanes();
        let commands = delete_commands(&data, &spanning(&source, 6.0, 10.0));

        assert!(matches!(
            commands.first(),
            Some(EngineCommand::RemoveRegion {
                deck_uuid,
                index: 0
            }) if *deck_uuid == source
        ));
        let fragments: Vec<RegionConfig> = commands
            .iter()
            .filter_map(|command| match command {
                EngineCommand::AddRegion { deck_uuid, region } if *deck_uuid == source => {
                    Some(*region)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            fragments,
            vec![
                RegionConfig::new(4.0, 6.0).with_fades(1.0, 0.0),
                RegionConfig::new(10.0, 12.0).with_fades(0.0, 2.0),
            ]
        );
    }

    #[test]
    fn moving_part_of_a_region_splits_it_and_moves_only_the_crop() {
        let (data, source, _) = fixture_two_lanes();
        let commands = move_commands(&data, &spanning(&source, 6.0, 10.0), 10.0, &[], false);

        assert!(matches!(
            commands.first(),
            Some(EngineCommand::RemoveRegion {
                deck_uuid,
                index: 0
            }) if *deck_uuid == source
        ));
        let added: Vec<RegionConfig> = commands
            .iter()
            .filter_map(|command| match command {
                EngineCommand::AddRegion { deck_uuid, region } if *deck_uuid == source => {
                    Some(*region)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            added,
            vec![
                RegionConfig::new(4.0, 6.0).with_fades(1.0, 0.0),
                RegionConfig::new(10.0, 12.0).with_fades(0.0, 2.0),
                RegionConfig::new(16.0, 20.0),
            ]
        );
    }

    #[test]
    fn alt_dragging_part_of_a_region_keeps_it_and_copies_only_the_crop() {
        let (data, source, _) = fixture_two_lanes();
        let commands = move_commands(&data, &spanning(&source, 6.0, 10.0), 10.0, &[], true);

        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            EngineCommand::AddRegion {
                deck_uuid,
                region,
            } if *deck_uuid == source && *region == RegionConfig::new(16.0, 20.0)
        ));
    }

    /// A move within one lane keeps each region's index, so nothing is renumbered
    /// and the whole gesture is a handful of in-place updates.
    #[test]
    fn a_same_lane_move_updates_regions_in_place() {
        let (data, source, _) = fixture_two_lanes();
        let commands = move_commands(&data, &spanning(&source, 4.0, 18.0), 5.0, &[], false);

        let updates: Vec<(usize, RegionConfig)> = commands
            .iter()
            .filter_map(|command| match command {
                EngineCommand::UpdateRegion {
                    deck_uuid,
                    index,
                    region,
                } if *deck_uuid == source => Some((*index, *region)),
                _ => None,
            })
            .collect();
        assert_eq!(updates.len(), 2, "{commands:?}");
        assert_eq!(updates[0].0, 1, "the later region is rewritten first");
        assert!((updates[0].1.start - 19.0).abs() < 1e-9);
        assert!((updates[1].1.start - 9.0).abs() < 1e-9);
        assert!((updates[1].1.end - 17.0).abs() < 1e-9);
    }

    /// Crossing to another lane is a removal and an addition, and every removal
    /// has to be emitted first: a lane that is both a source and a target would
    /// otherwise be renumbered underneath its own pending commands.
    #[test]
    fn a_cross_lane_move_removes_before_it_adds() {
        let (data, source, target) = fixture_two_lanes();
        let lane_map = vec![(source.clone(), target.clone())];
        let commands = move_commands(&data, &spanning(&source, 4.0, 18.0), 5.0, &lane_map, false);

        let first_add = commands
            .iter()
            .position(|c| matches!(c, EngineCommand::AddRegion { .. }))
            .expect("the regions land somewhere");
        let last_remove = commands
            .iter()
            .rposition(|c| matches!(c, EngineCommand::RemoveRegion { .. }))
            .expect("and leave where they were");
        assert!(last_remove < first_add, "{commands:?}");

        let removed: Vec<usize> = commands
            .iter()
            .filter_map(|c| match c {
                EngineCommand::RemoveRegion { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(removed, vec![1, 0], "removal runs high index first");
        assert!(commands.iter().all(|c| !matches!(
            c,
            EngineCommand::AddRegion { deck_uuid, .. } if *deck_uuid != target
        )));
    }

    #[test]
    fn an_alt_move_adds_without_removing_anything() {
        let (data, source, _) = fixture_two_lanes();
        let commands = move_commands(&data, &spanning(&source, 4.0, 18.0), 5.0, &[], true);

        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|c| matches!(
            c,
            EngineCommand::AddRegion { deck_uuid, .. } if *deck_uuid == source
        )));
    }

    /// A partial selection moves the crop, not the region before it, so the crop
    /// may land at zero even when the intersecting region began earlier.
    #[test]
    fn the_move_floor_is_the_selection_start() {
        let (data, source, _) = fixture_two_lanes();
        let floor = move_floor(&data, &spanning(&source, 6.0, 10.0));
        assert!((floor - 6.0).abs() < 1e-9, "{floor}");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Arbitrary cuts, including tiny edge slivers and selections extending
        /// far past either side, must partition a region exactly: no gap, no
        /// overlap, and no fade migrating onto a newly cut edge.
        #[test]
        fn chaos_region_crops_partition_the_original_exactly(
            region_start in 0u16..8_000,
            region_span in 1u16..4_000,
            selection_a in 0u16..12_000,
            selection_b in 0u16..12_000,
            fade_in in 0u16..4_000,
            fade_out in 0u16..4_000,
        ) {
            let start = f64::from(region_start) / 16.0;
            let end = start + f64::from(region_span) / 16.0;
            let region = RegionConfig {
                start,
                end,
                fade_in: f64::from(fade_in) / 16.0,
                fade_out: f64::from(fade_out) / 16.0,
            };
            let selection_start = f64::from(selection_a.min(selection_b)) / 16.0;
            let selection_end = f64::from(selection_a.max(selection_b)) / 16.0;
            let sliced = region_slice(&region, selection_start, selection_end);

            if !intersects(&region, selection_start, selection_end) {
                prop_assert!(sliced.is_none());
                return Ok(());
            }
            let sliced = sliced.expect("an intersection always has a slice");
            let mut pieces = sliced.remainders.clone();
            pieces.push(sliced.selected);
            pieces.sort_by(|a, b| a.start.total_cmp(&b.start));

            prop_assert!(pieces.iter().all(|region| region.is_valid()));
            prop_assert!((pieces[0].start - region.start).abs() < 1e-9);
            prop_assert!((pieces[pieces.len() - 1].end - region.end).abs() < 1e-9);
            prop_assert!(
                pieces
                    .windows(2)
                    .all(|pair| (pair[0].end - pair[1].start).abs() < 1e-9)
            );
            let total: f64 = pieces.iter().map(|region| region.span()).sum();
            prop_assert!((total - region.span()).abs() < 1e-9);

            let last = pieces.len() - 1;
            for (index, piece) in pieces.iter().enumerate() {
                if index == 0 {
                    prop_assert_eq!(piece.fade_in, region.fade_in);
                } else {
                    prop_assert_eq!(piece.fade_in, 0.0);
                }
                if index == last {
                    prop_assert_eq!(piece.fade_out, region.fade_out);
                } else {
                    prop_assert_eq!(piece.fade_out, 0.0);
                }
            }
        }

        /// A move is a clear and a paste that must not fight each other, and the
        /// generated distances deliberately include destinations that overlap
        /// the source. Whatever the curve and however far it travels, the result
        /// stays a valid envelope with its landing edges where the move put them,
        /// and a duplicate destroys nothing outside its landing span.
        #[test]
        fn chaos_random_curve_moves_land_without_corrupting_the_shape(
            raw in prop::collection::vec((any::<u16>(), any::<u16>(), any::<u8>()), 0..80),
            a in 0u16..12_000,
            width in 1u16..4_000,
            travel in -4_000i32..4_000,
            duplicate in any::<bool>(),
        ) {
            let points = hostile_curve(&raw);
            let start = f64::from(a) / 16.0;
            let end = start + f64::from(width) / 16.0;
            // Nothing may be dragged before the start of the show.
            let delta = (f64::from(travel) / 16.0).max(-start);
            let moved = moved_curve(&points, start, end, delta, duplicate);

            assert_curve_sane(&moved)?;
            let anchor = start + delta;
            let last = (end - start) + anchor;
            prop_assert!(
                moved.iter().any(|point| (point.position - anchor).abs() < 1e-9),
                "the landing has no leading edge: {moved:?}"
            );
            prop_assert!(
                moved.iter().any(|point| (point.position - last).abs() < 1e-9),
                "the landing has no trailing edge: {moved:?}"
            );
            if duplicate {
                for original in points
                    .iter()
                    .filter(|point| point.position < anchor || point.position > last)
                {
                    prop_assert!(
                        moved.contains(original),
                        "a duplicate destroyed {original:?}"
                    );
                }
            }
        }

        /// The batch is executed in order against the live scene, so index
        /// safety is the invariant that matters: additions come last, removals
        /// on a lane run high index first, and nothing invalid reaches the
        /// engine however hostile the span and the distance.
        #[test]
        fn chaos_move_command_batches_stay_index_safe(
            a in 0u16..2_000,
            b in 0u16..2_000,
            travel in -600i32..600,
            duplicate in any::<bool>(),
            cross_lane in any::<bool>(),
        ) {
            let (data, source, target) = fixture_two_lanes();
            let start = f64::from(a.min(b)) / 16.0;
            let end = f64::from(a.max(b)) / 16.0;
            let selection = spanning(&source, start, end);
            let delta = (f64::from(travel) / 16.0).max(-move_floor(&data, &selection));
            let lane_map = if cross_lane {
                vec![(source.clone(), target.clone())]
            } else {
                Vec::new()
            };
            let commands = move_commands(&data, &selection, delta, &lane_map, duplicate);

            let first_add = commands
                .iter()
                .position(|c| matches!(c, EngineCommand::AddRegion { .. }));
            let last_edit = commands.iter().rposition(|c| matches!(
                c,
                EngineCommand::RemoveRegion { .. } | EngineCommand::UpdateRegion { .. }
            ));
            if let (Some(add), Some(edit)) = (first_add, last_edit) {
                prop_assert!(edit < add, "an addition ran before an edit: {commands:?}");
            }

            let removals: Vec<usize> = commands
                .iter()
                .filter_map(|c| match c {
                    EngineCommand::RemoveRegion { index, .. } => Some(*index),
                    _ => None,
                })
                .collect();
            prop_assert!(
                removals.windows(2).all(|pair| pair[0] > pair[1]),
                "removals must run high index first: {removals:?}"
            );

            if duplicate {
                let nothing_was_taken_away = commands.iter().all(|c| matches!(
                    c,
                    EngineCommand::AddRegion { .. } | EngineCommand::SetEnvelopeBreakpoints { .. }
                ));
                prop_assert!(nothing_was_taken_away);
            }

            for command in &commands {
                match command {
                    EngineCommand::AddRegion { region, .. }
                    | EngineCommand::UpdateRegion { region, .. } => {
                        prop_assert!(region.is_valid() && region.start >= 0.0, "{region:?}");
                    }
                    EngineCommand::SetEnvelopeBreakpoints { breakpoints, .. } => {
                        assert_curve_sane(breakpoints)?;
                    }
                    _ => {}
                }
            }
        }

        /// Offensive curve coverage: arbitrary point order, duplicate positions,
        /// one-point envelopes, every curve kind, and ranges that can start or
        /// end outside the authored curve. The copied slice must still be a
        /// finite, ordered shape whose edge values are exactly what the renderer
        /// evaluates at those instants.
        #[test]
        fn chaos_random_curves_slice_without_corrupting_the_shape(
            raw in prop::collection::vec((any::<u16>(), any::<u16>(), any::<u8>()), 0..80),
            a in 0u16..12_000,
            width in 1u16..4_000,
        ) {
            let points = hostile_curve(&raw);
            let start = f64::from(a) / 16.0;
            let end = start + f64::from(width) / 16.0;
            let sliced = curve_slice(&points, start, end);

            assert_curve_sane(&sliced)?;
            prop_assert_eq!(sliced.first().map(|p| p.position), Some(start));
            prop_assert_eq!(sliced.last().map(|p| p.position), Some(end));

            let mut cursor = usize::MAX;
            let expected_start = evaluate_envelope(&points, start, &mut cursor);
            let expected_end = evaluate_envelope(&points, end, &mut cursor);
            prop_assert!(
                (sliced[0].value - expected_start).abs() < 1e-5,
                "start edge changed: {} vs {expected_start}",
                sliced[0].value
            );
            prop_assert!(
                (sliced[sliced.len() - 1].value - expected_end).abs() < 1e-5,
                "end edge changed: {} vs {expected_end}",
                sliced[sliced.len() - 1].value
            );
            prop_assert_eq!(
                sliced.len(),
                2 + points
                    .iter()
                    .filter(|point| point.position > start && point.position < end)
                    .count()
            );
        }

        /// Deleting random automation spans must never leak an invalid list to
        /// the engine. Boundary values are pinned to the audible pre-delete
        /// values, all points strictly inside are gone, and points strictly
        /// outside retain their authored identity.
        #[test]
        fn chaos_random_curve_deletes_remain_ordered_and_continuous(
            raw in prop::collection::vec((any::<u16>(), any::<u16>(), any::<u8>()), 0..80),
            a in 0u16..12_000,
            width in 1u16..4_000,
        ) {
            let points = hostile_curve(&raw);
            let start = f64::from(a) / 16.0;
            let end = start + f64::from(width) / 16.0;
            let cleared = curve_cleared(&points, start, end);

            assert_curve_sane(&cleared)?;
            prop_assert!(
                cleared
                    .iter()
                    .all(|point| point.position <= start || point.position >= end),
                "an interior point survived: {cleared:?}"
            );

            let mut cursor = usize::MAX;
            let expected_start = evaluate_envelope(&points, start, &mut cursor);
            let expected_end = evaluate_envelope(&points, end, &mut cursor);
            let at_start = cleared
                .iter()
                .find(|point| point.position == start)
                .expect("clear pins its start");
            let at_end = cleared
                .iter()
                .find(|point| point.position == end)
                .expect("clear pins its end");
            prop_assert!((at_start.value - expected_start).abs() < 1e-5);
            prop_assert!((at_end.value - expected_end).abs() < 1e-5);

            for original in points
                .iter()
                .filter(|point| point.position < start || point.position > end)
            {
                prop_assert!(
                    cleared.contains(original),
                    "outside point {original:?} was rewritten"
                );
            }
        }

        /// Paste is attacked with unrelated target points on both sides and
        /// throughout the landing span. Whatever the distribution, the covered
        /// points are removed, outside points survive, the copied edge times
        /// land exactly at anchor and anchor+duration, and ordering remains
        /// suitable for the engine.
        #[test]
        fn chaos_random_curve_pastes_replace_exactly_one_span(
            existing_raw in prop::collection::vec(
                (any::<u16>(), any::<u16>(), any::<u8>()),
                0..100,
            ),
            slice_raw in prop::collection::vec(
                (any::<u16>(), any::<u16>(), any::<u8>()),
                1..60,
            ),
            anchor_units in 0u16..8_000,
            duration_units in 1u16..2_000,
        ) {
            let existing = hostile_curve(&existing_raw);
            let duration = f64::from(duration_units) / 16.0;
            let anchor = f64::from(anchor_units) / 16.0;
            let source = hostile_curve(&slice_raw);
            let source_start = source[0].position;
            let source_end = source[source.len() - 1].position;
            let slice: Vec<Breakpoint> = if source.len() == 1 {
                vec![
                    Breakpoint {
                        position: 0.0,
                        ..source[0]
                    },
                    Breakpoint {
                        position: duration,
                        ..source[0]
                    },
                ]
            } else {
                let source_span = source_end - source_start;
                source
                    .iter()
                    .map(|point| Breakpoint {
                        position: (point.position - source_start) / source_span * duration,
                        ..*point
                    })
                    .collect()
            };
            let pasted = pasted_curve(&existing, &slice, anchor, duration);

            assert_curve_sane(&pasted)?;
            prop_assert!(pasted.iter().any(|point| point.position == anchor));
            prop_assert!(
                pasted
                    .iter()
                    .any(|point| point.position == anchor + duration)
            );
            for point in existing
                .iter()
                .filter(|point| point.position < anchor || point.position > anchor + duration)
            {
                prop_assert!(pasted.contains(point), "outside point {point:?} was lost");
            }
            prop_assert!(
                existing
                    .iter()
                    .filter(|point| point.position >= anchor
                        && point.position <= anchor + duration)
                    .all(|point| !pasted.contains(point) || slice.iter().any(|source| {
                        source.position + anchor == point.position
                            && source.value == point.value
                            && source.curve == point.curve
                    })),
                "an overwritten target point survived"
            );
        }

        /// Region membership is attacked with overlapping, nested, reversed,
        /// zero-width, and out-of-order spans. It must remain exactly equivalent
        /// to the strict intersection rule and never duplicate an index — where
        /// that rule includes the requirement that the span have width at all,
        /// since an empty span overlaps nothing.
        #[test]
        fn chaos_hostile_region_geometry_has_deterministic_membership(
            raw in prop::collection::vec((any::<u16>(), any::<u16>()), 0..200),
            a in 0u16..12_000,
            b in 0u16..12_000,
        ) {
            let regions: Vec<RegionConfig> = raw
                .iter()
                .map(|(start, end)| RegionConfig::new(
                    f64::from(*start) / 16.0,
                    f64::from(*end) / 16.0,
                ))
                .collect();
            let start = f64::from(a.min(b)) / 16.0;
            let end = f64::from(a.max(b)) / 16.0;
            let members = regions_in_span(&regions, start, end);
            let expected: Vec<usize> = regions
                .iter()
                .enumerate()
                .filter(|(_, region)| end > start && region.start < end && region.end > start)
                .map(|(index, _)| index)
                .collect();

            prop_assert_eq!(members.as_slice(), expected.as_slice());
            prop_assert!(members.windows(2).all(|pair| pair[0] < pair[1]));
        }

        /// Selection and clipboard memory are UI-session state, so users can
        /// thrash Copy, Clear, Delete, and Paste in any order. Stale lane IDs,
        /// empty selections, and mismatched targets must remain no-ops rather
        /// than panics or commands aimed at unrelated objects.
        #[test]
        fn chaos_clipboard_operation_storm_stays_coherent(
            ops in prop::collection::vec((0u8..7, 0u16..4_000, 0u16..4_000), 1..300),
        ) {
            let data = super::super::tests::fixture_with_automation();
            let ctx = egui::Context::default();
            let deck_uuid = data.channels[0].decks[0].uuid.clone();
            let envelope_uuid = data
                .modulation_sources
                .iter()
                .find(|entry| matches!(entry.source, ModSourceUI::Envelope { .. }))
                .map(|entry| entry.uuid.clone())
                .expect("automation fixture has an envelope");

            for (op, a, b) in ops {
                let start = f64::from(a.min(b)) / 16.0;
                let end = f64::from(a.max(b)) / 16.0;
                let current = Selection {
                    start,
                    end,
                    decks: if op % 3 == 0 {
                        vec!["stale-deck".to_string()]
                    } else {
                        vec![deck_uuid.clone()]
                    },
                    envelopes: if op % 4 == 0 {
                        vec!["stale-envelope".to_string()]
                    } else {
                        vec![envelope_uuid.clone()]
                    },
                };

                match op {
                    0 => store(&ctx, current),
                    1 => clear(&ctx),
                    2 => copy(&ctx, &data, &current),
                    3 => {
                        let commands = delete_commands(&data, &current);
                        let targets_are_live = commands.iter().all(|command| matches!(
                            command,
                            EngineCommand::RemoveRegion { deck_uuid: target, .. }
                                if target == &deck_uuid
                        ) || matches!(
                            command,
                            EngineCommand::SetEnvelopeBreakpoints { uuid, .. }
                                if uuid == &envelope_uuid
                        ));
                        prop_assert!(targets_are_live);
                    }
                    4 => {
                        let commands = paste_commands(
                            &ctx,
                            &data,
                            start,
                            &PasteTarget::Deck(deck_uuid.clone()),
                        );
                        let regions_are_valid = commands.iter().all(|command| matches!(
                            command,
                            EngineCommand::AddRegion { deck_uuid: target, region }
                                if target == &deck_uuid && region.is_valid()
                        ));
                        prop_assert!(regions_are_valid);
                    }
                    5 => {
                        let commands = paste_commands(
                            &ctx,
                            &data,
                            start,
                            &PasteTarget::Envelope(envelope_uuid.clone()),
                        );
                        let curves_are_valid = commands.iter().all(|command| matches!(
                            command,
                            EngineCommand::SetEnvelopeBreakpoints { uuid, breakpoints }
                                if uuid == &envelope_uuid
                                    && assert_curve_sane(breakpoints).is_ok()
                        ));
                        prop_assert!(curves_are_valid);
                    }
                    _ => {
                        let stale = paste_commands(
                            &ctx,
                            &data,
                            start,
                            &PasteTarget::Envelope("stale-envelope".to_string()),
                        );
                        let stale_target_is_not_redirected = stale.iter().all(|command| matches!(
                            command,
                            EngineCommand::SetEnvelopeBreakpoints { uuid, .. }
                                if uuid == "stale-envelope"
                        ));
                        prop_assert!(stale_target_is_not_redirected);
                    }
                }

                if let Some(armed) = load(&ctx) {
                    prop_assert!(armed.start.is_finite() && armed.end.is_finite());
                    prop_assert!(armed.start <= armed.end);
                }
            }
        }
    }
}
