//! Snapshot-based undo/redo over one timeline covering scene and stage state.
//! A new undoable action clears the redo stack.

use crate::persistence::StagePrefs;
use crate::scene::SceneConfig;

/// Undo snapshots retained.
const MAX_HISTORY_DEPTH: usize = 50;

/// Scene and stage state captured before an undoable action. Both halves are
/// always captured; `StagePrefs` is plain data, so this is cheap.
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

    /// Record the state before an undoable mutation. Clears the redo stack.
    pub fn push(&mut self, snapshot: HistorySnapshot) {
        if self.undo_stack.len() >= MAX_HISTORY_DEPTH {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, current: HistorySnapshot) -> Option<HistorySnapshot> {
        let snapshot = self.undo_stack.pop()?;
        self.redo_stack.push(current);
        Some(snapshot)
    }

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
            modulation: crate::modulation::ModulationEngine::default(),
            macros: crate::macros::MacroBank::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut: None,
            arrangement: None,
            transport: crate::scene::TransportConfig::default(),
        }
    }

    /// Snapshot tagged by `crossfader` (scene) and `grid_size` (stage).
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
        // Oldest evicted.
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
