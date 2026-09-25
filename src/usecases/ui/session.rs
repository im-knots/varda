//! `UISession`: the half of a frame's UI output that is *not* an engine
//! mutation: selection focus, panel visibility, dialog triggers, and the
//! undo/redo/save requests the runner turns into commands.
//!
//! See /spec/ui-engine-boundary.md WS4 / Decision #11.

use super::{CameraDetectAction, DomeAction};

/// UI-local session/ephemeral state accumulated during a frame (Population 2).
///
/// This is the half of the frame's UI output that is **not** an engine mutation:
/// selection focus, panel visibility, dialog-open triggers, gesture
/// continuation, and the undo/redo/save
/// requests. It targets UI-local state (`UILayoutState`) or the runner; engine
/// mutations go on `UIActions::commands`.
///
/// See /spec/ui-engine-boundary.md WS4 / Decision #11 — the deliberate split of
/// "what I tell the engine" (`UIActions::commands`) from "my local view state".
// Per-frame request flags that are independently set and cleared; an enum cannot express them.
#[allow(clippy::struct_excessive_bools)]
pub struct UISession {
    /// Channel UUID to open an image file dialog for (deferred to outside egui
    /// frame). A UUID rather than an index because the dialog outlives the
    /// frame that requested it — see [`/spec/api-addressing.md`].
    pub open_image_dialog_for_channel: Option<String>,
    /// Channel UUID to open a video file dialog for (deferred to outside egui frame)
    pub open_video_dialog_for_channel: Option<String>,
    /// Select a deck for detail view in bottom bar (`ch_idx`, `deck_idx`)
    pub select_deck: Option<(usize, usize)>,
    /// Select a channel for detail view in bottom bar (`ch_idx`)
    pub select_channel: Option<usize>,
    /// Select master output for detail view in bottom bar
    pub select_master: bool,
    /// Select a sequence for detail view in bottom bar (`seq_idx`)
    pub select_sequence: Option<usize>,
    /// Select a step within a sequence for editing in bottom bar (`seq_idx`, `step_idx`)
    pub select_sequence_step: Option<(usize, usize)>,
    /// Select a macro (by UUID) for detail view in bottom bar
    pub select_macro: Option<String>,
    /// Clear the macro selection (e.g. its macro was deleted, or Close pressed)
    pub deselect_macro: bool,
    /// Toggle stage editor open/closed
    pub toggle_stage_editor: bool,
    /// Swap the central area between Performance and Arrangement. Purely a view
    /// change: the arrangement drives decks whenever the transport runs, in
    /// either mode. See /spec/arrangement.md § UI.
    pub toggle_arrangement_mode: bool,
    /// Timeline horizontal zoom, in pixels per second of show time.
    pub set_arrangement_zoom: Option<f32>,
    /// Timeline horizontal scroll, as the show position at the left edge.
    pub set_arrangement_scroll: Option<f64>,
    /// Timeline vertical scroll, in pixels of rows above the top edge.
    pub set_arrangement_scroll_y: Option<f32>,
    /// Round timeline edits to whole frames. A property of the gesture, never of
    /// the stored position. See /spec/arrangement.md § Does the ruler own a
    /// frame rate?
    pub toggle_arrangement_snap: bool,
    /// Mark the stretch of show being worked on. See /spec/arrangement.md § The
    /// focus area.
    pub set_arrangement_focus: Option<super::state::FocusRange>,
    /// Unmark it. Separate from setting so a clear is not a range nobody can
    /// express.
    pub clear_arrangement_focus: bool,
    /// Toggle 3D dome preview in stage editor
    pub toggle_dome_preview: bool,
    /// Dome mode actions (camera, config, mode toggle). UI-local layout state
    /// (`DomeLayoutFields`); only committed to the engine via `GenerateDomeSlices`.
    pub dome_actions: Vec<DomeAction>,
    /// Camera detection actions (preview state machine; only `Accept` becomes a command)
    pub camera_detect_actions: Vec<CameraDetectAction>,
    /// Set stage editor grid size (normalized)
    pub set_grid_size: Option<f32>,
    /// Toggle snap-to-grid
    pub toggle_snap: bool,
    /// Toggle library panel open/closed
    pub toggle_library_panel: bool,
    /// Toggle right panel open/closed
    pub toggle_right_panel: bool,
    /// Save workspace requested (Ctrl+S / Cmd+S). Layout-coupled runner trigger
    /// (accepted deviation, /spec/ui-engine-boundary.md Decision #10).
    pub save_requested: bool,
    /// Undo last undoable action. Layout-coupled runner trigger (accepted deviation).
    pub undo_requested: bool,
    /// Redo last undone action. Layout-coupled runner trigger (accepted deviation).
    pub redo_requested: bool,
    /// A mutating stage/warp pointer drag is in progress this frame. Set by the
    /// stage editor and warp editor while dragging a vertex, warp point, bezier
    /// handle, or gizmo, and by any scene param/opacity slider drag. Used to
    /// collapse a continuous drag into a single undo step (snapshot on gesture
    /// start, suppressed while held).
    pub gesture_active: bool,
}

impl Default for UISession {
    fn default() -> Self {
        Self::new()
    }
}

impl UISession {
    pub fn new() -> Self {
        Self {
            open_image_dialog_for_channel: None,
            open_video_dialog_for_channel: None,
            select_deck: None,
            select_channel: None,
            select_master: false,
            select_sequence: None,
            select_sequence_step: None,
            select_macro: None,
            deselect_macro: false,
            toggle_stage_editor: false,
            toggle_arrangement_mode: false,
            set_arrangement_zoom: None,
            set_arrangement_scroll: None,
            set_arrangement_scroll_y: None,
            toggle_arrangement_snap: false,
            set_arrangement_focus: None,
            clear_arrangement_focus: false,
            toggle_dome_preview: false,
            dome_actions: Vec::new(),
            camera_detect_actions: Vec::new(),
            set_grid_size: None,
            toggle_snap: false,
            toggle_library_panel: false,
            toggle_right_panel: false,
            save_requested: false,
            undo_requested: false,
            redo_requested: false,
            gesture_active: false,
        }
    }
}
