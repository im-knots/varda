//! UI-owned layout and selection state.
//!
//! Presentation concerns the engine never sees. Each UI consumer keeps its own
//! instance; the egui consumer's copy is persisted in `stage.json` via
//! `StagePrefs`. See /spec/ui-engine-boundary.md.

use super::{DomeAction, UIActions};
use crate::camera::CameraId;
use crate::renderer::slicer::{DomeGeometry, DomePreset};
use crate::surface::detect::{DetectedContour, DetectionParams};

/// Zoom bounds for the arrangement timeline, in pixels per second.
///
/// The floor keeps an hour-long show legible on one screen; the ceiling is about
/// one pixel per frame at 60fps, past which there is nothing left to resolve.
pub const MIN_PIXELS_PER_SECOND: f32 = 0.5;
pub const MAX_PIXELS_PER_SECOND: f32 = 400.0;

/// A marked stretch of show, in seconds. The thing a performer is dialling in.
///
/// Ordered on construction, so a range drawn right to left is the same range as
/// one drawn left to right rather than an empty one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusRange {
    pub start: f64,
    pub end: f64,
}

impl FocusRange {
    pub fn new(a: f64, b: f64) -> Self {
        Self {
            start: a.min(b).max(0.0),
            end: a.max(b).max(0.0),
        }
    }

    pub fn span(self) -> f64 {
        self.end - self.start
    }

    /// Whether the range is long enough to be worth keeping. A click that
    /// happened to land on the strip is not a range.
    pub fn is_usable(self) -> bool {
        self.span() > f64::EPSILON
    }
}

/// UI-consumer-owned layout and selection state.
///
/// These fields are presentation concerns that don't belong in the engine.
/// Each UI consumer (egui, CLI, HTTP API) maintains its own instance.
/// Persisted in `stage.json` via the `StagePrefs` struct.
// Independent panel/toggle flags; grouping them into enums would not model reality.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub struct UILayoutState {
    /// Currently selected deck for detail view in bottom bar (`ch_idx`, `deck_idx`)
    pub selected_deck: Option<(usize, usize)>,
    /// Currently selected channel for detail view in bottom bar (`ch_idx`)
    pub selected_channel: Option<usize>,
    /// Whether the master output is selected for detail view in bottom bar
    pub selected_master: bool,
    /// Currently selected sequence for detail view in bottom bar (`seq_idx`)
    pub selected_sequence: Option<usize>,
    /// Currently selected step within the selected sequence (`seq_idx`, `step_idx`)
    pub selected_sequence_step: Option<(usize, usize)>,
    /// Currently selected macro (by UUID) for detail view in bottom bar
    pub selected_macro: Option<String>,
    /// Whether the LIGHTS band is expanded in the central area.
    ///
    /// Collapsed is the default, and with it collapsed the layout is today's layout: same
    /// radiating split, same channel widths, same full central height. Adding lighting costs a
    /// user who does not run lights a thin strip and nothing else.
    /// See /spec/lighting-routing.md § Performance-Mode UI.
    pub lights_band_open: bool,
    /// Whether the VIDEO band is expanded. A lighting-only installation collapses it.
    pub video_band_open: bool,
    /// Fraction of the central area given to the VIDEO band when both are open.
    ///
    /// Defaults to an equal split, which assumes nothing about how a given performer works.
    /// The divider is there to be moved, and the ratio persists in `stage.json`.
    pub band_split: f32,
    /// Currently selected lighting deck (by UUID) for detail view in the bottom bar.
    pub selected_lighting_deck: Option<String>,
    /// Whether the full-screen stage editor is open (replaces deck view)
    pub stage_editor_open: bool,
    /// Stage editor grid size (normalized, e.g. 0.05 = 20 divisions)
    pub stage_editor_grid_size: f32,
    /// Whether snap-to-grid is enabled in the stage editor
    pub stage_editor_snap: bool,
    /// Whether the central area shows the arrangement timeline instead of the
    /// mixer. See /spec/arrangement.md § UI.
    pub arrangement_mode_open: bool,
    /// Timeline horizontal zoom, in pixels per second of show time.
    pub arrangement_pixels_per_second: f32,
    /// Show position at the timeline's left edge.
    pub arrangement_scroll: f64,
    /// Rows scrolled off the top of the timeline, in pixels. A show with more
    /// channels than fit has to stay reachable.
    pub arrangement_scroll_y: f32,
    /// Whether timeline edits round to whole frames at the ruler's rate.
    pub arrangement_snap: bool,
    /// The stretch of show being worked on, if one is marked. Looping is one
    /// thing done to it rather than the thing it is, so it outlives the loop it
    /// sets. See /spec/arrangement.md § The focus area.
    pub arrangement_focus: Option<FocusRange>,
    /// Whether the library panel (left sidebar) is open
    pub library_panel_open: bool,
    /// Whether the right panel (master output sidebar) is open
    pub right_panel_open: bool,
    /// Whether the 3D dome preview is open in the stage editor
    pub dome_preview_open: bool,
    /// Whether the stage editor is in 3D Dome mode (vs 2D Polygon mode)
    pub dome_mode_active: bool,
    /// Active dome preset
    pub dome_preset: DomePreset,
    /// Active dome geometry (radius, truncation, tilt)
    pub dome_geometry: DomeGeometry,
    /// Camera detection mode state
    pub camera_detect_mode: CameraDetectMode,
}

impl Default for UILayoutState {
    fn default() -> Self {
        Self {
            // Open. The channel's lighting section is part of the channel; a performer who does
            // not use it collapses it once.
            lights_band_open: true,
            video_band_open: true,
            band_split: 0.5,
            selected_lighting_deck: None,
            selected_deck: None,
            selected_channel: None,
            selected_master: false,
            selected_sequence: None,
            selected_sequence_step: None,
            selected_macro: None,
            stage_editor_open: false,
            stage_editor_grid_size: 0.05,
            stage_editor_snap: true,
            arrangement_mode_open: false,
            // Ten seconds across a 400px area: wide enough to see a structure,
            // tight enough that a fade handle is grabbable.
            arrangement_pixels_per_second: 40.0,
            arrangement_scroll: 0.0,
            arrangement_scroll_y: 0.0,
            // On by default: a show cut to picture wants frame-aligned edits,
            // and a music-led one can turn it off in one click.
            arrangement_snap: true,
            arrangement_focus: None,
            library_panel_open: true,
            right_panel_open: true,
            dome_preview_open: false,
            dome_mode_active: false,
            dome_preset: DomePreset::Quad,
            dome_geometry: DomeGeometry::default(),
            camera_detect_mode: CameraDetectMode::Off,
        }
    }
}

/// Camera detection mode state machine.
///
/// Off → Live (camera feed) → Preview (frozen frame with contour selection) → Off
#[derive(Debug, Clone, Default)]
pub enum CameraDetectMode {
    #[default]
    Off,
    Live {
        camera_id: CameraId,
        params: DetectionParams,
    },
    Preview {
        camera_id: CameraId,
        contours: Vec<DetectedContour>,
        selected: Vec<bool>,
    },
}

/// Actions emitted by the camera detection UI.
#[derive(Debug, Clone)]
pub enum CameraDetectAction {
    Enter { camera_id: CameraId },
    Exit,
    UpdateParams(DetectionParams),
    Capture,
    ToggleContour(usize),
    SelectAll(bool),
    Accept,
}

impl UILayoutState {
    /// Apply selection actions from `UIActions`.
    pub fn apply_selections(&mut self, ui_actions: &UIActions) {
        let session = &ui_actions.session;
        if let Some(sel) = session.select_deck {
            self.selected_deck = Some(sel);
            self.selected_channel = None;
            self.selected_master = false;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
            self.selected_macro = None;
            self.selected_lighting_deck = None;
        }
        if let Some(ch) = session.select_channel {
            self.selected_channel = Some(ch);
            self.selected_deck = None;
            self.selected_master = false;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
            self.selected_macro = None;
            self.selected_lighting_deck = None;
        }
        if session.select_master {
            self.selected_master = true;
            self.selected_deck = None;
            self.selected_channel = None;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
            self.selected_macro = None;
            self.selected_lighting_deck = None;
        }
        if let Some(seq) = session.select_sequence {
            self.selected_sequence = Some(seq);
            self.selected_sequence_step = None;
            self.selected_deck = None;
            self.selected_channel = None;
            self.selected_master = false;
            self.selected_macro = None;
            self.selected_lighting_deck = None;
        }
        if let Some(step) = session.select_sequence_step {
            self.selected_sequence_step = Some(step);
            // Ensure sequence is also selected
            self.selected_sequence = Some(step.0);
        }
        if let Some(uuid) = &session.select_macro {
            self.selected_macro = Some(uuid.clone());
            self.selected_deck = None;
            self.selected_channel = None;
            self.selected_master = false;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
            self.selected_lighting_deck = None;
        }
        // A lighting deck takes the bottom bar the same way a video deck does, so it belongs in
        // this block with the rest rather than in a branch of its own. Living outside it was why
        // selecting a video deck left a lighting deck selected: nothing cleared it, and the
        // bottom bar checks lighting first, so the video detail could never be reached again.
        if let Some(uuid) = &session.select_lighting_deck {
            // An empty string is the close button: clear rather than select nothing.
            self.selected_lighting_deck = (!uuid.is_empty()).then(|| uuid.clone());
            if !uuid.is_empty() {
                self.selected_deck = None;
                self.selected_channel = None;
                self.selected_master = false;
                self.selected_sequence = None;
                self.selected_sequence_step = None;
                self.selected_macro = None;
            }
        }
        if session.deselect_macro {
            self.selected_macro = None;
        }
        if session.toggle_stage_editor {
            self.stage_editor_open = !self.stage_editor_open;
        }
        if session.toggle_arrangement_mode {
            self.arrangement_mode_open = !self.arrangement_mode_open;
        }
        if let Some(pps) = session.set_arrangement_zoom {
            self.arrangement_pixels_per_second =
                pps.clamp(MIN_PIXELS_PER_SECOND, MAX_PIXELS_PER_SECOND);
        }
        if let Some(scroll) = session.set_arrangement_scroll {
            self.arrangement_scroll = scroll.max(0.0);
        }
        if let Some(scroll) = session.set_arrangement_scroll_y {
            // The panel clamps against its own height, which only it knows.
            self.arrangement_scroll_y = scroll.max(0.0);
        }
        if session.toggle_arrangement_snap {
            self.arrangement_snap = !self.arrangement_snap;
        }
        if let Some(range) = session.set_arrangement_focus {
            self.arrangement_focus = Some(range);
        }
        if session.clear_arrangement_focus {
            self.arrangement_focus = None;
        }
        if let Some(size) = session.set_grid_size {
            self.stage_editor_grid_size = size;
        }
        if session.toggle_snap {
            self.stage_editor_snap = !self.stage_editor_snap;
        }
        if session.toggle_library_panel {
            self.library_panel_open = !self.library_panel_open;
        }
        if session.toggle_right_panel {
            self.right_panel_open = !self.right_panel_open;
        }
        if session.toggle_dome_preview {
            self.dome_preview_open = !self.dome_preview_open;
        }
        // Dome mode actions
        for action in &session.dome_actions {
            match action {
                DomeAction::SetMode(active) => {
                    self.dome_mode_active = *active;
                    // When entering dome mode, also open dome preview
                    if *active {
                        self.dome_preview_open = true;
                    }
                }
                DomeAction::SetPreset(preset) => self.dome_preset = *preset,
                DomeAction::SetRadius(r) => self.dome_geometry.radius = *r,
                DomeAction::SetTruncation(deg) => self.dome_geometry.truncation_degrees = *deg,
                DomeAction::SetTilt(deg) => self.dome_geometry.tilt_degrees = *deg,
                DomeAction::SetContentAzimuth(deg) => {
                    self.dome_geometry.content_azimuth_degrees = *deg;
                }
                DomeAction::SetContentElevation(deg) => {
                    self.dome_geometry.content_elevation_degrees = *deg;
                }
                DomeAction::SetContentRoll(deg) => self.dome_geometry.content_roll_degrees = *deg,
                DomeAction::RotateCamera { .. }
                | DomeAction::ZoomCamera { .. }
                | DomeAction::ResetCamera => {
                    // Camera actions are handled by the runner, not layout state
                }
            }
        }
    }

    /// Channels to force-render for preview, derived from the current selection.
    ///
    /// Selecting a deck or a channel cues that channel so its off-air preview
    /// updates live (see /spec/channel-preview.md). Master or no selection cues
    /// nothing. Returned as a set (0 or 1 today) to leave room for multi-cue.
    pub fn preview_channels(&self) -> Vec<usize> {
        if let Some((ch, _)) = self.selected_deck {
            vec![ch]
        } else if let Some(ch) = self.selected_channel {
            vec![ch]
        } else {
            Vec::new()
        }
    }

    /// Drop a lighting-deck selection whose deck no longer exists.
    ///
    /// Without this, removing the selected lighting deck left the bottom bar pinned to "That
    /// lighting deck is gone" — and because the bottom bar checks lighting before video, no
    /// other deck's detail could be opened again for the rest of the session.
    /// Takes a liveness predicate rather than the mixer: this needs one bit of information, and
    /// asking for the whole mixer would drag a GPU context into a test about selection state.
    pub fn prune_lighting_selection(&mut self, is_alive: impl Fn(uuid::Uuid) -> bool) {
        let Some(uuid) = self.selected_lighting_deck.as_deref() else {
            return;
        };
        let alive = uuid::Uuid::parse_str(uuid).is_ok_and(is_alive);
        if !alive {
            self.selected_lighting_deck = None;
        }
    }

    /// Fix up selection indices after a channel is removed.
    pub fn fixup_channel_removal(&mut self, removed_ch: usize) {
        if let Some((sel_ch, _)) = self.selected_deck {
            if sel_ch == removed_ch {
                self.selected_deck = None;
            } else if sel_ch > removed_ch {
                // sel_ch > removed_ch, so selected_deck must be Some (we matched it above)
                if let Some((_, deck_idx)) = self.selected_deck {
                    self.selected_deck = Some((sel_ch - 1, deck_idx));
                }
            }
        }
        if let Some(sel_ch) = self.selected_channel {
            if sel_ch == removed_ch {
                self.selected_channel = None;
            } else if sel_ch > removed_ch {
                self.selected_channel = Some(sel_ch - 1);
            }
        }
    }
}

#[cfg(test)]
mod preview_channel_tests {
    use super::*;

    #[test]
    fn selected_deck_cues_its_channel() {
        let layout = UILayoutState {
            selected_deck: Some((1, 3)),
            ..Default::default()
        };
        assert_eq!(layout.preview_channels(), vec![1]);
    }

    #[test]
    fn selected_channel_cues_itself() {
        let layout = UILayoutState {
            selected_channel: Some(2),
            ..Default::default()
        };
        assert_eq!(layout.preview_channels(), vec![2]);
    }

    #[test]
    fn selected_master_cues_nothing() {
        let layout = UILayoutState {
            selected_master: true,
            ..Default::default()
        };
        assert!(layout.preview_channels().is_empty());
    }

    #[test]
    fn no_selection_cues_nothing() {
        let layout = UILayoutState::default();
        assert!(layout.preview_channels().is_empty());
    }

    #[test]
    fn deck_takes_precedence_over_channel() {
        // apply_selections keeps these mutually exclusive, but the derivation
        // must be deterministic even if both happen to be set.
        let layout = UILayoutState {
            selected_deck: Some((0, 0)),
            selected_channel: Some(1),
            ..Default::default()
        };
        assert_eq!(layout.preview_channels(), vec![0]);
    }

    /// Selecting a video deck must release the bottom bar from a lighting deck.
    ///
    /// The bottom bar checks lighting before video, so a lighting selection that nothing cleared
    /// masked every video deck's detail for the rest of the session.
    #[test]
    fn selecting_a_video_deck_clears_a_lighting_deck_selection() {
        let mut layout = UILayoutState {
            selected_lighting_deck: Some("deck-1".to_string()),
            ..Default::default()
        };
        let mut actions = UIActions::new();
        actions.session.select_deck = Some((0, 0));
        layout.apply_selections(&actions);
        assert_eq!(layout.selected_lighting_deck, None);
        assert_eq!(layout.selected_deck, Some((0, 0)));
    }

    /// And the reverse: a lighting deck takes the bar from a video deck.
    #[test]
    fn selecting_a_lighting_deck_clears_a_video_deck_selection() {
        let mut layout = UILayoutState {
            selected_deck: Some((0, 0)),
            ..Default::default()
        };
        let mut actions = UIActions::new();
        actions.session.select_lighting_deck = Some("deck-1".to_string());
        layout.apply_selections(&actions);
        assert_eq!(layout.selected_deck, None);
        assert_eq!(layout.selected_lighting_deck, Some("deck-1".to_string()));
    }

    /// Every other selection releases a lighting deck too, so none of them can be the one that
    /// leaves the bottom bar stuck.
    #[test]
    fn every_selection_releases_a_lighting_deck() {
        type Select = Box<dyn Fn(&mut UIActions)>;
        let cases: Vec<(&str, Select)> = vec![
            (
                "deck",
                Box::new(|a: &mut UIActions| a.session.select_deck = Some((0, 0))),
            ),
            (
                "channel",
                Box::new(|a: &mut UIActions| a.session.select_channel = Some(0)),
            ),
            (
                "master",
                Box::new(|a: &mut UIActions| a.session.select_master = true),
            ),
            (
                "sequence",
                Box::new(|a: &mut UIActions| a.session.select_sequence = Some(0)),
            ),
            (
                "macro",
                Box::new(|a: &mut UIActions| a.session.select_macro = Some("m".to_string())),
            ),
        ];
        for (name, apply) in cases {
            let mut layout = UILayoutState {
                selected_lighting_deck: Some("deck-1".to_string()),
                ..Default::default()
            };
            let mut actions = UIActions::new();
            apply(&mut actions);
            layout.apply_selections(&actions);
            assert_eq!(
                layout.selected_lighting_deck, None,
                "selecting a {name} left a lighting deck holding the bottom bar"
            );
        }
    }

    /// The close button.
    #[test]
    fn an_empty_uuid_closes_the_lighting_detail() {
        let mut layout = UILayoutState {
            selected_lighting_deck: Some("deck-1".to_string()),
            ..Default::default()
        };
        let mut actions = UIActions::new();
        actions.session.select_lighting_deck = Some(String::new());
        layout.apply_selections(&actions);
        assert_eq!(layout.selected_lighting_deck, None);
    }

    /// Removing the selected lighting deck must release the bottom bar, not pin it to
    /// "That lighting deck is gone" forever.
    #[test]
    fn removing_the_selected_lighting_deck_releases_the_bottom_bar() {
        let deck = crate::dmx::LightingDeck::new(crate::dmx::Look::new("Wash"));
        let live = deck.id;
        let uuid = deck.id.to_string();

        let mut layout = UILayoutState {
            selected_lighting_deck: Some(uuid.clone()),
            ..Default::default()
        };
        layout.prune_lighting_selection(|id| id == live);
        assert_eq!(
            layout.selected_lighting_deck,
            Some(uuid),
            "a live deck must stay selected"
        );

        layout.prune_lighting_selection(|_| false);
        assert_eq!(
            layout.selected_lighting_deck, None,
            "a removed deck must release the bottom bar"
        );
    }

    /// A short channel uuid is not a v4 uuid, so the parse must not be what decides this.
    #[test]
    fn an_unparseable_lighting_selection_is_pruned() {
        let mut layout = UILayoutState {
            selected_lighting_deck: Some("not-a-uuid".to_string()),
            ..Default::default()
        };
        layout.prune_lighting_selection(|_| true);
        assert_eq!(layout.selected_lighting_deck, None);
    }
}
