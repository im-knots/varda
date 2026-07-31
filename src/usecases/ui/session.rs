//! `UISession` — the half of a frame's UI output that is *not* an engine
//! mutation: selection focus, panel visibility, learn-mode targeting, dialog
//! triggers, and the undo/redo/save flags.
//!
//! See /spec/ui-engine-boundary.md WS4 / Decision #11.

use super::{CameraDetectAction, DomeAction};

/// UI-local session/ephemeral state accumulated during a frame (Population 2).
///
/// This is the half of the frame's UI output that is **not** an engine mutation:
/// selection focus, panel visibility, learn-mode targeting, dialog-open triggers,
/// notification dismissals, gesture continuation, the async shader-load request,
/// and the layout-coupled undo/redo/save triggers. None of it belongs on the
/// command bus (the HTTP/CLI/MIDI consumers neither can nor should express it);
/// it targets UI-local state (`UILayoutState`) or the runner, not the engine.
///
/// See /spec/ui-engine-boundary.md WS4 / Decision #11 — the deliberate split of
/// "what I tell the engine" (`UIActions::commands`) from "my local view state".
// Per-frame request flags that are independently set and cleared; an enum cannot express them.
#[allow(clippy::struct_excessive_bools)]
pub struct UISession {
    /// (`channel_uuid`, `generator_registry_idx`) — add a shader as a new deck to a
    /// channel. Resolved off-frame via `spawn_deck_loads` (not a command), so the
    /// channel is held by UUID: a channel index captured at click time can name a
    /// different channel by the time the shader finishes compiling. See
    /// [`/spec/api-addressing.md`].
    pub shader_to_add: Option<(String, usize)>,
    /// Channel UUID to open an image file dialog for (deferred to outside egui
    /// frame). A UUID rather than an index because the dialog outlives the
    /// frame that requested it — see [`/spec/api-addressing.md`].
    pub open_image_dialog_for_channel: Option<String>,
    /// Channel UUID to open a video file dialog for (deferred to outside egui frame)
    pub open_video_dialog_for_channel: Option<String>,
    pub notifications_to_dismiss: Vec<usize>,
    /// Info notifications to push (e.g. "Copied URL to clipboard")
    pub info_notifications: Vec<String>,
    /// MIDI learn: toggle learn mode on/off
    pub midi_learn_toggle: bool,
    /// MIDI learn: select a parameter as learn target (in learn mode, clicking a param)
    pub midi_learn_select: Option<String>,
    /// Keyboard learn: toggle learn mode on/off
    pub keyboard_learn_toggle: bool,
    /// Keyboard learn: select a target (Action or `ParamPath`)
    pub keyboard_learn_select: Option<crate::keymap::KeyTarget>,
    /// Keyboard learn: bind a key combo to current target
    pub keyboard_learn_bind: Option<crate::keymap::KeyCombo>,
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
    /// Remove a channel from the mixer (by index). Retained as a non-command
    /// field because the runner needs the removed index to fix up UI selection
    /// state (`layout.fixup_channel_removal`).
    pub remove_channel: Option<usize>,
    /// Toggle stage editor open/closed
    pub toggle_stage_editor: bool,
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
            shader_to_add: None,
            open_image_dialog_for_channel: None,
            open_video_dialog_for_channel: None,
            notifications_to_dismiss: Vec::new(),
            info_notifications: Vec::new(),
            midi_learn_toggle: false,
            midi_learn_select: None,
            keyboard_learn_toggle: false,
            keyboard_learn_select: None,
            keyboard_learn_bind: None,
            select_deck: None,
            select_channel: None,
            select_master: false,
            select_sequence: None,
            select_sequence_step: None,
            select_macro: None,
            deselect_macro: false,
            remove_channel: None,
            toggle_stage_editor: false,
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
