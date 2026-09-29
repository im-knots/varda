//! `UIActions`: everything a frame of UI emits, plus the drag payload types.

use super::{ParamUIInfo, UISession};
use crate::ShaderParams;

/// All UI output collected during a frame.
///
/// - [`commands`](Self::commands): outbound `EngineCommand`s, the same mutation
///   vocabulary the HTTP/CLI consumers use. The app layer runs each through the
///   command bus dispatch.
/// - [`session`](Self::session): UI-local ephemeral state (selection, panel
///   visibility, learn mode, dialog triggers, undo/redo/save). See [`UISession`].
pub struct UIActions {
    /// Outbound engine mutations.
    pub commands: Vec<crate::engine::EngineCommand>,
    /// UI-local ephemeral state.
    pub session: UISession,
}

impl Default for UIActions {
    fn default() -> Self {
        Self::new()
    }
}

impl UIActions {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            session: UISession::new(),
        }
    }
}

/// Library drag-and-drop payload.
#[derive(Debug, Clone)]
pub enum LibraryDrag {
    /// A library entry of any deck source type.
    Source(crate::source::SourceConfig),
    /// Effect shader from the library (registry index).
    Effect(usize),
    /// Deck preset (index into `preset_library.deck_presets`).
    DeckPreset(usize),
    /// Channel preset (index into `preset_library.channel_presets`).
    ChannelPreset(usize),
}

/// Drag payload for moving a deck to another channel or within its own. Uses
/// the UUID because the deck's index can change between drag start and release.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckDrag {
    pub deck_uuid: String,
}

/// Drag payload for reordering effects within a chain. Uses the chain's UUID
/// because the drop applies after release, when an index could name another
/// entity.
#[derive(Debug, Clone, PartialEq)]
pub enum EffectDrag {
    /// Deck effect: (`deck_uuid`, `effect_idx`)
    Deck(String, usize),
    /// Channel effect: (`channel_uuid`, `effect_idx`)
    Channel(String, usize),
    /// Master effect: (`effect_idx`)
    Master(usize),
}

/// Drag payload for reordering steps within a sequence (bottom bar only).
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceStepDrag {
    pub sequence_uuid: String,
    pub step_idx: usize,
}

/// Extract params from `ShaderParams` for UI display.
pub fn collect_params(params: &ShaderParams) -> Vec<ParamUIInfo> {
    params
        .param_order
        .iter()
        .filter_map(|name| {
            let value = params.values.get(name)?;
            let def = params.definitions.get(name);
            Some(ParamUIInfo {
                name: name.clone(),
                label: def.and_then(|d| d.label.clone()),
                value: *value,
                min: def.and_then(|d| d.min),
                max: def.and_then(|d| d.max),
                group: def.and_then(|d| d.group.clone()),
                choices: def
                    .map(crate::isf::ISFInput::choices)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(value, label)| crate::usecases::ui::data::ParamChoiceUI { value, label })
                    .collect(),
            })
        })
        .collect()
}
