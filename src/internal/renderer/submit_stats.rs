//! Per-frame `queue.submit()` accounting.
//!
//! Each submit is a command buffer commit, and commits are not free — on Metal
//! in particular they cost enough that the compositor already batches draws
//! through a params ring buffer specifically to avoid them (see `blit.rs`).
//! Nothing measured how many a frame actually issues, so batching work was
//! flying blind. This counts them.
//!
//! The counter is per-`GpuContext` rather than process-global so that tests,
//! which build many headless contexts across threads, cannot see each other's
//! submits.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Counts `queue.submit()` calls against one GPU context.
///
/// Cloneable and shared, matching `GpuContext`: clones handed to background
/// loader threads must accumulate into the same tally.
#[derive(Clone, Default)]
pub struct SubmitCounter {
    count: Arc<AtomicU32>,
}

impl SubmitCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one submit. Called by `GpuContext::submit`.
    pub fn record(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// Read the tally and reset it, returning submits since the last take.
    ///
    /// The frame loop calls this once per frame, so the value covers a whole
    /// frame including output, preview, and present work.
    pub fn take(&self) -> u32 {
        self.count.swap(0, Ordering::Relaxed)
    }

    /// Read the tally without resetting.
    pub fn peek(&self) -> u32 {
        self.count.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_takes() {
        let c = SubmitCounter::new();
        assert_eq!(c.peek(), 0);
        c.record();
        c.record();
        assert_eq!(c.peek(), 2);
        assert_eq!(c.take(), 2);
        assert_eq!(c.peek(), 0);
    }

    #[test]
    fn clones_share_one_tally() {
        let a = SubmitCounter::new();
        let b = a.clone();
        a.record();
        b.record();
        assert_eq!(a.peek(), 2);
        assert_eq!(b.take(), 2);
        assert_eq!(a.peek(), 0);
    }
}
