//! Undo/redo history manager — snapshot-based.
//!
//! Stores `SceneConfig` snapshots before undoable mutations.
//! Undo pops the undo stack and pushes current state onto redo.
//! Redo pops the redo stack and pushes current state onto undo.
//! New undoable actions clear the redo stack (fork).

use crate::scene::SceneConfig;

/// Maximum number of undo snapshots retained.
const MAX_HISTORY_DEPTH: usize = 50;

/// Snapshot-based undo/redo history.
pub struct HistoryManager {
    undo_stack: Vec<SceneConfig>,
    redo_stack: Vec<SceneConfig>,
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
    pub fn push(&mut self, snapshot: SceneConfig) {
        if self.undo_stack.len() >= MAX_HISTORY_DEPTH {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
        self.redo_stack.clear();
    }

    /// Undo: push `current` onto redo, pop and return top of undo stack.
    pub fn undo(&mut self, current: SceneConfig) -> Option<SceneConfig> {
        let snapshot = self.undo_stack.pop()?;
        self.redo_stack.push(current);
        Some(snapshot)
    }

    /// Redo: push `current` onto undo, pop and return top of redo stack.
    pub fn redo(&mut self, current: SceneConfig) -> Option<SceneConfig> {
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
            modulation: Default::default(),
            transition_sequences: vec![],
            render_width: None,
            render_height: None,
        }
    }

    #[test]
    fn push_and_undo() {
        let mut h = HistoryManager::new();
        assert!(!h.can_undo());

        h.push(make_scene(0.0));
        assert!(h.can_undo());

        let restored = h.undo(make_scene(0.5)).unwrap();
        assert!((restored.crossfader - 0.0).abs() < 1e-5);
        assert!(!h.can_undo());
        assert!(h.can_redo());
    }

    #[test]
    fn undo_then_redo() {
        let mut h = HistoryManager::new();
        h.push(make_scene(0.0));
        h.push(make_scene(0.3));

        let s1 = h.undo(make_scene(0.7)).unwrap();
        assert!((s1.crossfader - 0.3).abs() < 1e-5);

        let s2 = h.redo(make_scene(0.3)).unwrap();
        assert!((s2.crossfader - 0.7).abs() < 1e-5);
    }

    #[test]
    fn new_action_clears_redo() {
        let mut h = HistoryManager::new();
        h.push(make_scene(0.0));
        h.push(make_scene(0.5));
        let _ = h.undo(make_scene(1.0));
        assert!(h.can_redo());

        h.push(make_scene(0.8));
        assert!(!h.can_redo());
    }

    #[test]
    fn max_depth_eviction() {
        let mut h = HistoryManager::new();
        for i in 0..60 {
            h.push(make_scene(i as f32));
        }
        assert_eq!(h.undo_stack.len(), 50);
        // Oldest should have been evicted; first entry is 10.0
        assert!((h.undo_stack[0].crossfader - 10.0).abs() < 1e-5);
    }

    #[test]
    fn clear_resets_both_stacks() {
        let mut h = HistoryManager::new();
        h.push(make_scene(0.0));
        let _ = h.undo(make_scene(0.5));
        assert!(h.can_redo());
        h.clear();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }
}
