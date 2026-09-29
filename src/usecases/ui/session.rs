//! `UISession`: the part of a frame's UI output that is not an engine
//! mutation: selection focus, panel visibility, dialog triggers, and the
//! undo/redo/save requests the runner turns into commands.

use super::{CameraDetectAction, DomeAction};

/// UI-local ephemeral state accumulated during a frame.
///
/// Targets UI-local state (`UILayoutState`) or the runner: selection focus,
/// panel visibility, dialog triggers, gesture continuation, and undo/redo/save
/// requests. Engine mutations go on `UIActions::commands`.
#[allow(clippy::struct_excessive_bools)]
pub struct UISession {
    /// A file picker a source type requested, opened outside the egui frame. Holds
    /// a channel UUID rather than an index because the dialog outlives the frame
    /// that requested it.
    pub open_file_dialog: Option<crate::app::render::FileDialogRequest>,
    /// Select a deck for the bottom-bar detail view (`ch_idx`, `deck_idx`).
    pub select_deck: Option<(usize, usize)>,
    /// Select a channel for the bottom-bar detail view (`ch_idx`).
    pub select_channel: Option<usize>,
    /// Select master output for the bottom-bar detail view.
    pub select_master: bool,
    /// Select a sequence for the bottom-bar detail view (`seq_idx`).
    pub select_sequence: Option<usize>,
    /// Select a sequence step for editing in the bottom bar (`seq_idx`, `step_idx`).
    pub select_sequence_step: Option<(usize, usize)>,
    /// Select a macro by UUID for the bottom-bar detail view.
    pub select_macro: Option<String>,
    /// Clear the macro selection (macro deleted, or Close pressed).
    pub deselect_macro: bool,
    /// Toggle the stage editor.
    pub toggle_stage_editor: bool,
    /// Swap the central area between Performance and Arrangement. View only: the
    /// arrangement drives decks whenever the transport runs, in either mode.
    pub toggle_arrangement_mode: bool,
    /// Timeline horizontal zoom, in pixels per second of show time.
    pub set_arrangement_zoom: Option<f32>,
    /// Timeline horizontal scroll, as the show position at the left edge.
    pub set_arrangement_scroll: Option<f64>,
    /// Timeline vertical scroll, in pixels of rows above the top edge.
    pub set_arrangement_scroll_y: Option<f32>,
    /// Round timeline edits to whole frames. Applies to the gesture, never to the
    /// stored position.
    pub toggle_arrangement_snap: bool,
    /// Mark the part of the show being worked on.
    pub set_arrangement_focus: Option<super::state::FocusRange>,
    /// Clear the focus area.
    pub clear_arrangement_focus: bool,
    /// Toggle the 3D dome preview in the stage editor.
    pub toggle_dome_preview: bool,
    /// Dome mode actions (camera, config, mode toggle). UI-local layout state
    /// (`DomeLayoutFields`), committed to the engine only via `GenerateDomeSlices`.
    pub dome_actions: Vec<DomeAction>,
    /// Camera detection actions. Only `Accept` becomes an engine command.
    pub camera_detect_actions: Vec<CameraDetectAction>,
    /// Stage editor grid size (normalized).
    pub set_grid_size: Option<f32>,
    /// Toggle snap-to-grid.
    pub toggle_snap: bool,
    /// Toggle the library panel.
    pub toggle_library_panel: bool,
    /// Toggle the right panel.
    pub toggle_right_panel: bool,
    /// Save workspace (Ctrl+S / Cmd+S). Handled by the runner.
    pub save_requested: bool,
    /// Undo the last undoable action. Handled by the runner.
    pub undo_requested: bool,
    /// Redo the last undone action. Handled by the runner.
    pub redo_requested: bool,
    /// A mutating pointer drag is in progress this frame: a vertex, warp point,
    /// bezier handle, gizmo, or scene param/opacity slider. The runner snapshots
    /// on gesture start and suppresses snapshots while held, so the drag is one
    /// undo step.
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
            open_file_dialog: None,
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
