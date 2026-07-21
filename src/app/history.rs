//! Undo/redo history manager — snapshot-based, unified scene + stage timeline.
//!
//! Stores combined `HistorySnapshot` values (mixer/scene + stage/venue) before
//! undoable mutations, so a single timeline covers both subsystems.
//! Undo pops the undo stack and pushes current state onto redo.
//! Redo pops the redo stack and pushes current state onto undo.
//! New undoable actions clear the redo stack (fork).

use crate::persistence::StagePrefs;
use crate::scene::SceneConfig;

/// Maximum number of undo snapshots retained.
const MAX_HISTORY_DEPTH: usize = 50;

/// One entry on the unified undo/redo timeline: a snapshot of both authored
/// subsystems captured *before* an undoable action.
///
/// Both halves are always captured together so a single entry fully describes
/// "the world before this action", regardless of which subsystem it touched.
/// `StagePrefs` is plain data (no GPU resources), so the extra half is cheap.
#[derive(Debug, Clone)]
pub struct HistorySnapshot {
    /// Mixer/scene state (channels, decks, effects, modulation, ...).
    pub scene: SceneConfig,
    /// Stage/venue state (surfaces, warp, holes, assignments, dome, editor prefs).
    pub stage: StagePrefs,
}

/// Snapshot-based undo/redo history.
pub struct HistoryManager {
    undo_stack: Vec<HistorySnapshot>,
    redo_stack: Vec<HistorySnapshot>,
}

impl HistoryManager {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Record current state before an undoable mutation.
    /// Clears the redo stack (new action branch).
    pub fn push(&mut self, snapshot: HistorySnapshot) {
        if self.undo_stack.len() >= MAX_HISTORY_DEPTH {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
        self.redo_stack.clear();
    }

    /// Undo: push `current` onto redo, pop and return top of undo stack.
    pub fn undo(&mut self, current: HistorySnapshot) -> Option<HistorySnapshot> {
        let snapshot = self.undo_stack.pop()?;
        self.redo_stack.push(current);
        Some(snapshot)
    }

    /// Redo: push `current` onto undo, pop and return top of redo stack.
    pub fn redo(&mut self, current: HistorySnapshot) -> Option<HistorySnapshot> {
        let snapshot = self.redo_stack.pop()?;
        self.undo_stack.push(current);
        Some(snapshot)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Clear all history (e.g. on workspace load).
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

/// Result of a successful undo/redo restore, returned to the caller.
///
/// A windowed consumer uses `structural_changed` to decide whether to
/// re-register GPU preview textures and reads dome layout flags off the
/// restored `snapshot`'s stage half. The headless/API consumer only needs to
/// know a restore happened (`Some(_)` vs `None`).
pub struct HistoryRestore {
    /// The state that was restored onto live engine state.
    pub snapshot: HistorySnapshot,
    /// True if the scene diff changed deck/channel structure (GPU resources
    /// were rebuilt, so preview textures must be re-registered).
    pub structural_changed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneConfig;

    fn make_scene(crossfader: f32) -> SceneConfig {
        SceneConfig {
            version: 2,
            channels: vec![],
            crossfader,
            active_transition: None,
            master_effects: vec![],
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
        }
    }

    /// Build a snapshot tagged by `crossfader` (scene) and `grid_size` (stage)
    /// so tests can assert both halves round-trip through the timeline.
    fn make_snapshot(crossfader: f32, grid_size: f32) -> HistorySnapshot {
        HistorySnapshot {
            scene: make_scene(crossfader),
            stage: StagePrefs {
                grid_size,
                ..StagePrefs::default()
            },
        }
    }

    #[test]
    fn push_and_undo() {
        let mut h = HistoryManager::new();
        assert!(!h.can_undo());

        h.push(make_snapshot(0.0, 0.01));
        assert!(h.can_undo());

        let restored = h.undo(make_snapshot(0.5, 0.02)).unwrap();
        assert!((restored.scene.crossfader - 0.0).abs() < 1e-5);
        // Stage half round-trips too.
        assert!((restored.stage.grid_size - 0.01).abs() < 1e-5);
        assert!(!h.can_undo());
        assert!(h.can_redo());
    }

    #[test]
    fn undo_then_redo() {
        let mut h = HistoryManager::new();
        h.push(make_snapshot(0.0, 0.01));
        h.push(make_snapshot(0.3, 0.03));

        let s1 = h.undo(make_snapshot(0.7, 0.07)).unwrap();
        assert!((s1.scene.crossfader - 0.3).abs() < 1e-5);
        assert!((s1.stage.grid_size - 0.03).abs() < 1e-5);

        let s2 = h.redo(make_snapshot(0.3, 0.03)).unwrap();
        assert!((s2.scene.crossfader - 0.7).abs() < 1e-5);
        assert!((s2.stage.grid_size - 0.07).abs() < 1e-5);
    }

    #[test]
    fn new_action_clears_redo() {
        let mut h = HistoryManager::new();
        h.push(make_snapshot(0.0, 0.01));
        h.push(make_snapshot(0.5, 0.05));
        let _ = h.undo(make_snapshot(1.0, 0.1));
        assert!(h.can_redo());

        h.push(make_snapshot(0.8, 0.08));
        assert!(!h.can_redo());
    }

    #[test]
    fn max_depth_eviction() {
        let mut h = HistoryManager::new();
        for i in 0..60 {
            h.push(make_snapshot(i as f32, 0.01));
        }
        assert_eq!(h.undo_stack.len(), 50);
        // Oldest should have been evicted; first entry is 10.0
        assert!((h.undo_stack[0].scene.crossfader - 10.0).abs() < 1e-5);
    }

    #[test]
    fn clear_resets_both_stacks() {
        let mut h = HistoryManager::new();
        h.push(make_snapshot(0.0, 0.01));
        let _ = h.undo(make_snapshot(0.5, 0.05));
        assert!(h.can_redo());
        h.clear();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }
}
