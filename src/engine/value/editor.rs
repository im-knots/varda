//! Stage editor preferences the engine persists on the GUI's behalf.
//!
//! The engine stores these in `stage.json` and hands them back on load, but
//! never interprets them. See /spec/ui-engine-boundary.md (WS5).

/// Cosmetic stage editor state. Field names match `stage.json`.
// Independent persisted toggles, one per panel or editor mode.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct EditorPrefs {
    /// Stage editor grid size (normalized).
    pub grid_size: f32,
    /// Snap stage editor edits to the grid.
    pub snap: bool,
    pub library_panel_open: bool,
    pub right_panel_open: bool,
    pub stage_editor_open: bool,
    pub dome_preview_open: bool,
    /// Whether the stage editor shows its 3D dome view.
    pub dome_mode_active: bool,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            grid_size: 0.05,
            snap: true,
            library_panel_open: false,
            right_panel_open: true,
            stage_editor_open: false,
            dome_preview_open: false,
            dome_mode_active: false,
        }
    }
}
