//! Keyboard mapping value types. The binding store and learn logic live in
//! `internal::keymap`.

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
    /// Copy, paste, and duplicate act on the deck, channel, or effect the
    /// bottom bar is following.
    Copy,
    Paste,
    Duplicate,
}
