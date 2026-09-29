//! Per-frame `queue.submit()` count. Submits are expensive, especially on Metal.
//!
//! The counter is per `GpuContext` so tests running many contexts in parallel
//! don't see each other's submits.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Counts `queue.submit()` calls against one GPU context.
///
/// Clones share one tally, so loader threads count into the same total.
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

    /// Return submits since the last take and reset. Called once per frame.
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
