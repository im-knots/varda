//! Keyboard mapping value types: what a key combination is and what it can be
//! bound to. The binding store and learn logic stay in `internal::keymap`.
//!
//! See /spec/ui-engine-boundary.md (WS7).

use serde::{Deserialize, Serialize};

/// A key combination: a key + modifier state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyCombo {
    pub key: String,
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}

/// What a key binding targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyTarget {
    /// A discrete application action.
    Action(ActionId),
    /// A `param_path` (same addressing as MIDI).
    ParamPath(String),
}

/// All discrete actions that can be keyboard-mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionId {
    Undo,
    Redo,
    Save,
    ToggleLibrary,
    ToggleStageEditor,
    ToolSelect,
    ToolRectangle,
    ToolPolygon,
    ToolCircle,
    DuplicateSurface,
    FlipHorizontal,
    FlipVertical,
    DeleteSurface,
    ClearDrawing,
    CombineSurfaces,
    ToggleMidiLearn,
    ToggleKeyboardLearn,
    /// Copy, paste, and duplicate the current selection: the deck, channel, or
    /// effect the bottom bar is already following. See /spec/clipboard.md.
    Copy,
    Paste,
    Duplicate,
}
