//! `UIActions` — everything a frame of UI emits, plus the drag payload types.
//!
//! Split into the outbound `EngineCommand` stream and UI-local [`UISession`]
//! state (/spec/ui-engine-boundary.md WS4).

use super::{DomeAction, ParamUIInfo, UISession};
use crate::ShaderParams;

/// All UI output collected during a frame, split into two buckets (WS4):
///
/// - [`commands`](Self::commands): the outbound engine-mutation stream. Panels
///   push `EngineCommand`s directly (the single mutation vocabulary shared with
///   the HTTP/CLI consumers); the app-layer drain runs each through the same
///   dispatch as the command bus.
/// - [`session`](Self::session): UI-local ephemeral state (selection, panel
///   visibility, learn mode, dialog triggers, undo/redo/save). See [`UISession`].
pub struct UIActions {
    /// Outbound engine mutations (see /spec/ui-engine-boundary.md WS2).
    pub commands: Vec<crate::engine::EngineCommand>,
    /// UI-local session/ephemeral state (see [`UISession`], WS4).
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

    /// Whether this frame's actions include any undoable mutation carried by a
    /// non-command field. Source-deck adds, deck remove/move/reorder, channel
    /// add, effects, presets, and mixer edits now flow through `commands` and
    /// are gated by `batch_has_undoable`. Only two irreducible residuals remain
    /// here: the async shader load (`shader_to_add`, resolved off-frame and thus
    /// never on `commands`) and channel removal (`remove_channel`, kept a field
    /// so the runner can fix up UI selection with the removed index).
    pub fn has_undoable_action(&self) -> bool {
        self.session.shader_to_add.is_some() || self.session.remove_channel.is_some()
    }

    /// Whether this frame's actions include any undoable *stage* mutation
    /// (surface geometry/warp/holes/combine/reorder, surface→output
    /// assignments, or authored dome changes).
    ///
    /// Deliberately excludes non-authored actions on the same collections:
    /// output-window lifecycle (create/remove/reposition) and dome preview
    /// camera navigation (rotate/zoom/reset) are live/venue controls, not
    /// stage-editor edits, so they must not create history entries.
    ///
    /// Does NOT distinguish continuous vs discrete edits — gesture collapsing is
    /// handled by the `gesture_active` edge in the runner.
    pub fn has_undoable_stage_action(&self) -> bool {
        self.session.dome_actions.iter().any(|a| {
            !matches!(
                a,
                DomeAction::RotateCamera { .. }
                    | DomeAction::ZoomCamera { .. }
                    | DomeAction::ResetCamera
            )
        })
    }
}

/// Drag payload types for library drag-and-drop
#[derive(Debug, Clone)]
pub enum LibraryDrag {
    /// Generator shader from library (registry index)
    Generator(usize),
    /// Effect/filter shader from library (registry index)
    Effect(usize),
    /// Camera device from library (`CameraId`)
    Camera(crate::camera::CameraId),
    /// Depth sensor from library (`DepthSensorId`)
    DepthSensor(crate::depth::DepthSensorId),
    /// Screen or window capture target, addressed by name rather than by
    /// platform handle so the payload stays valid across a rescan.
    ScreenCapture(crate::scene::CaptureTargetConfig),
    /// Varda's own program or a channel composite. See spec/program-tap.md.
    Tap(crate::scene::TapSourceConfig),
    /// NDI network source (source name)
    Ndi(String),
    /// Syphon server (server name)
    Syphon(String),
    /// SRT network source (url, mode)
    Srt(String, crate::stream::SrtMode),
    /// HLS stream source (url)
    Hls(String),
    /// DASH stream source (url)
    Dash(String),
    /// RTMP stream source (url, mode)
    Rtmp(String, crate::stream::RtmpMode),
    /// HTML content source (url)
    Html(String),
    /// Deck preset from library (index into `preset_library.deck_presets`)
    DeckPreset(usize),
    /// Channel preset from library (index into `preset_library.channel_presets`)
    ChannelPreset(usize),
}

/// Drag payload for moving a deck to another channel or reordering it within
/// its own. Named by UUID because the payload is set on drag start and read on
/// release: the deck's index can shift in between.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckDrag {
    pub deck_uuid: String,
}

/// Drag payload for effect reordering within a chain. The chain is named by
/// UUID because the payload outlives the frame it was created in — the drop is
/// applied after release, by which point an index could name another entity.
#[derive(Debug, Clone, PartialEq)]
pub enum EffectDrag {
    /// Deck effect: (`deck_uuid`, `effect_idx`)
    Deck(String, usize),
    /// Channel effect: (`channel_uuid`, `effect_idx`)
    Channel(String, usize),
    /// Master effect: (`effect_idx`)
    Master(usize),
}

/// Drag payload for reordering steps within a sequence (bottom bar only)
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceStepDrag {
    pub sequence_uuid: String,
    pub step_idx: usize,
}

/// Helper to extract params from `ShaderParams` for UI display
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
            })
        })
        .collect()
}
