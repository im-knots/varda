//! UI-owned layout and selection state.
//!
//! Presentation state the engine never sees. Each UI consumer keeps its own
//! instance; the egui consumer's is persisted in `stage.json` via `StagePrefs`.

use super::{DomeAction, UIActions};
use crate::camera::CameraId;
use crate::engine::value::editor::EditorPrefs;
use crate::surface::detect::{DetectedContour, DetectionParams};

/// Zoom bounds for the arrangement timeline, in pixels per second.
///
/// The minimum fits an hour-long show on one screen; the maximum is about one
/// pixel per frame at 60fps.
pub const MIN_PIXELS_PER_SECOND: f32 = 0.5;
pub const MAX_PIXELS_PER_SECOND: f32 = 400.0;

/// A marked span of the show, in seconds.
///
/// Ordered on construction, so a range drawn right to left equals one drawn
/// left to right.
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

    /// Whether the range is long enough to keep. A stray click on the strip is not
    /// a range.
    pub fn is_usable(self) -> bool {
        self.span() > f64::EPSILON
    }
}

/// UI-consumer-owned layout and selection state.
///
/// Each UI consumer (egui, CLI, HTTP API) keeps its own instance. Persisted in
/// `stage.json` via `StagePrefs`.
// Independent panel/toggle flags; enums would not model them.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub struct UILayoutState {
    /// Selected deck for the bottom-bar detail view (`ch_idx`, `deck_idx`).
    pub selected_deck: Option<(usize, usize)>,
    /// Selected channel for the bottom-bar detail view (`ch_idx`).
    pub selected_channel: Option<usize>,
    /// Whether master output is selected for the bottom-bar detail view.
    pub selected_master: bool,
    /// Selected sequence for the bottom-bar detail view (`seq_idx`).
    pub selected_sequence: Option<usize>,
    /// Selected step within the selected sequence (`seq_idx`, `step_idx`).
    pub selected_sequence_step: Option<(usize, usize)>,
    /// Selected macro (by UUID) for the bottom-bar detail view.
    pub selected_macro: Option<String>,
    /// Whether the full-screen stage editor is open (replaces the deck view).
    pub stage_editor_open: bool,
    /// Stage editor grid size (normalized; 0.05 = 20 divisions).
    pub stage_editor_grid_size: f32,
    /// Whether snap-to-grid is on in the stage editor.
    pub stage_editor_snap: bool,
    /// Whether the central area shows the arrangement timeline instead of the
    /// mixer.
    pub arrangement_mode_open: bool,
    /// Timeline horizontal zoom, in pixels per second of show time.
    pub arrangement_pixels_per_second: f32,
    /// Show position at the timeline's left edge.
    pub arrangement_scroll: f64,
    /// Pixels of rows scrolled off the top of the timeline.
    pub arrangement_scroll_y: f32,
    /// Whether timeline edits round to whole frames at the ruler's rate.
    pub arrangement_snap: bool,
    /// The part of the show being worked on, if marked. It outlives any loop set
    /// on it.
    pub arrangement_focus: Option<FocusRange>,
    /// Whether the library panel (left sidebar) is open.
    pub library_panel_open: bool,
    /// Whether the right panel (master output sidebar) is open.
    pub right_panel_open: bool,
    /// Whether the 3D dome preview is open in the stage editor.
    pub dome_preview_open: bool,
    /// Whether the stage editor is in 3D Dome mode (vs 2D Polygon mode).
    pub dome_mode_active: bool,
    /// Camera detection mode state.
    pub camera_detect_mode: CameraDetectMode,
}

impl Default for UILayoutState {
    fn default() -> Self {
        Self {
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
            // Ten seconds across 400px: wide enough to see structure, close enough to grab
            // a fade handle.
            arrangement_pixels_per_second: 40.0,
            arrangement_scroll: 0.0,
            arrangement_scroll_y: 0.0,
            // On by default: shows cut to picture need frame-aligned edits.
            arrangement_snap: true,
            arrangement_focus: None,
            library_panel_open: true,
            right_panel_open: true,
            dome_preview_open: false,
            dome_mode_active: false,
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
        }
        if let Some(ch) = session.select_channel {
            self.selected_channel = Some(ch);
            self.selected_deck = None;
            self.selected_master = false;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
            self.selected_macro = None;
        }
        if session.select_master {
            self.selected_master = true;
            self.selected_deck = None;
            self.selected_channel = None;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
            self.selected_macro = None;
        }
        if let Some(seq) = session.select_sequence {
            self.selected_sequence = Some(seq);
            self.selected_sequence_step = None;
            self.selected_deck = None;
            self.selected_channel = None;
            self.selected_master = false;
            self.selected_macro = None;
        }
        if let Some(step) = session.select_sequence_step {
            self.selected_sequence_step = Some(step);
            // Also select the step's sequence.
            self.selected_sequence = Some(step.0);
        }
        if let Some(uuid) = &session.select_macro {
            self.selected_macro = Some(uuid.clone());
            self.selected_deck = None;
            self.selected_channel = None;
            self.selected_master = false;
            self.selected_sequence = None;
            self.selected_sequence_step = None;
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
            // The panel clamps against its own height.
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
        for action in &session.dome_actions {
            match action {
                DomeAction::SetMode(active) => {
                    self.dome_mode_active = *active;
                    // Entering dome mode also opens the dome preview.
                    if *active {
                        self.dome_preview_open = true;
                    }
                }
                DomeAction::RotateCamera { .. }
                | DomeAction::ZoomCamera { .. }
                | DomeAction::ResetCamera => {
                    // Camera actions are handled by the runner.
                }
            }
        }
    }

    /// The stage editor prefs the engine persists for this layout.
    pub fn editor_prefs(&self) -> EditorPrefs {
        EditorPrefs {
            grid_size: self.stage_editor_grid_size,
            snap: self.stage_editor_snap,
            library_panel_open: self.library_panel_open,
            right_panel_open: self.right_panel_open,
            stage_editor_open: self.stage_editor_open,
            dome_preview_open: self.dome_preview_open,
            dome_mode_active: self.dome_mode_active,
        }
    }

    /// Adopt editor prefs loaded from the workspace.
    pub fn apply_editor_prefs(&mut self, prefs: EditorPrefs) {
        self.stage_editor_grid_size = prefs.grid_size;
        self.stage_editor_snap = prefs.snap;
        self.library_panel_open = prefs.library_panel_open;
        self.right_panel_open = prefs.right_panel_open;
        self.stage_editor_open = prefs.stage_editor_open;
        self.dome_preview_open = prefs.dome_preview_open;
        self.dome_mode_active = prefs.dome_mode_active;
    }

    /// Channels to force-render for preview, from the current selection.
    ///
    /// Selecting a deck or channel cues that channel so its off-air preview updates
    /// live. Master or no selection cues nothing. Returns a list (0 or 1 entries)
    /// to allow multi-cue.
    pub fn preview_channels(&self) -> Vec<usize> {
        if let Some((ch, _)) = self.selected_deck {
            vec![ch]
        } else if let Some(ch) = self.selected_channel {
            vec![ch]
        } else {
            Vec::new()
        }
    }

    /// Fix up selection indices after a channel is removed.
    pub fn fixup_channel_removal(&mut self, removed_ch: usize) {
        if let Some((sel_ch, _)) = self.selected_deck {
            if sel_ch == removed_ch {
                self.selected_deck = None;
            } else if sel_ch > removed_ch {
                // sel_ch > removed_ch, so selected_deck is Some (matched above).
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
        assert_eq!(layout.preview_channels().len(), 0);
    }

    #[test]
    fn no_selection_cues_nothing() {
        let layout = UILayoutState::default();
        assert_eq!(layout.preview_channels().len(), 0);
    }

    #[test]
    fn deck_takes_precedence_over_channel() {
        // apply_selections keeps these mutually exclusive, but the result must be
        // deterministic if both are set.
        let layout = UILayoutState {
            selected_deck: Some((0, 0)),
            selected_channel: Some(1),
            ..Default::default()
        };
        assert_eq!(layout.preview_channels(), vec![0]);
    }
}
