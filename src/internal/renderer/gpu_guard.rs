//! GPU error containment — keep a bad shader from taking down the show.
//!
//! wgpu's default uncaptured-error handler panics, and a panic on the render
//! thread ends the performance. A validation error is not a lost device though:
//! the offending command is dropped and everything else keeps working. So the
//! right response to "this one deck encodes an illegal frame" is to stop
//! rendering *that deck* and tell the performer — not to exit.
//!
//! This installs a handler that records faults instead of panicking, and lets
//! the renderer attribute a fault to whatever it was drawing at the time. See
//! spec/error-handling.md § Shader Errors.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Most recent faults kept for reporting. Bounded so a shader failing every
/// frame cannot grow this without limit.
const MAX_RETAINED: usize = 64;

/// A GPU error captured instead of panicking.
#[derive(Clone, Debug)]
pub struct GpuFault {
    /// What the renderer was drawing when the error surfaced, if known.
    pub context: Option<String>,
    pub message: String,
}

#[derive(Default)]
struct Inner {
    /// Bumped on every fault. Cheap enough to read on the hot path.
    count: AtomicUsize,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    context: Option<String>,
    faults: VecDeque<GpuFault>,
}

/// Installs a non-fatal uncaptured-error handler and records what it catches.
///
/// Cloneable and shared: `GpuContext` is cloned onto background loader threads,
/// and all clones must observe the same faults.
#[derive(Clone, Default)]
pub struct GpuErrorGuard {
    inner: Arc<Inner>,
}

impl GpuErrorGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace wgpu's panicking handler with one that records.
    ///
    /// Errors raised inside an explicit `push_error_scope` still go to that
    /// scope; this only catches what would otherwise abort the process.
    pub fn install(&self, device: &wgpu::Device) {
        let inner = Arc::clone(&self.inner);
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            let message = error.to_string();
            inner.count.fetch_add(1, Ordering::Relaxed);
            let context = match inner.state.lock() {
                Ok(mut state) => {
                    let context = state.context.clone();
                    if state.faults.len() == MAX_RETAINED {
                        state.faults.pop_front();
                    }
                    state.faults.push_back(GpuFault {
                        context: context.clone(),
                        message: message.clone(),
                    });
                    context
                }
                // A poisoned lock must not escalate into the panic this whole
                // module exists to avoid.
                Err(_) => None,
            };
            match &context {
                Some(what) => log::error!("GPU error while rendering {what}: {message}"),
                None => log::error!("GPU error: {message}"),
            }
        }));
    }

    /// Total faults seen since startup.
    pub fn fault_count(&self) -> usize {
        self.inner.count.load(Ordering::Relaxed)
    }

    /// Mark subsequent GPU work as belonging to `context` until the returned
    /// guard is dropped, so a fault can be blamed on the right deck.
    pub fn scope(&self, context: &str) -> GpuErrorScope<'_> {
        let start = self.fault_count();
        let previous = match self.inner.state.lock() {
            Ok(mut state) => state.context.replace(context.to_owned()),
            Err(_) => None,
        };
        GpuErrorScope {
            guard: self,
            start,
            previous,
        }
    }

    /// Drain the retained faults.
    pub fn take_faults(&self) -> Vec<GpuFault> {
        match self.inner.state.lock() {
            Ok(mut state) => state.faults.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn restore_context(&self, previous: Option<String>) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.context = previous;
        }
    }

    fn last_message_since(&self, start: usize) -> Option<String> {
        if self.fault_count() <= start {
            return None;
        }
        self.inner
            .state
            .lock()
            .ok()
            .and_then(|state| state.faults.back().map(|f| f.message.clone()))
    }
}

/// Attribution scope returned by [`GpuErrorGuard::scope`].
pub struct GpuErrorScope<'a> {
    guard: &'a GpuErrorGuard,
    start: usize,
    previous: Option<String>,
}

impl GpuErrorScope<'_> {
    /// The message of the most recent fault raised since this scope opened,
    /// or `None` if the work completed cleanly.
    pub fn faulted(&self) -> Option<String> {
        self.guard.last_message_since(self.start)
    }
}

impl Drop for GpuErrorScope<'_> {
    fn drop(&mut self) {
        self.guard.restore_context(self.previous.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the recording path directly. The handler wgpu installs is a
    /// closure over the same state, so exercising the state machine here covers
    /// everything except wgpu's own dispatch.
    fn record(guard: &GpuErrorGuard, message: &str) {
        guard.inner.count.fetch_add(1, Ordering::Relaxed);
        let mut state = guard.inner.state.lock().expect("lock");
        let context = state.context.clone();
        if state.faults.len() == MAX_RETAINED {
            state.faults.pop_front();
        }
        state.faults.push_back(GpuFault {
            context,
            message: message.to_owned(),
        });
    }

    #[test]
    fn clean_work_reports_no_fault() {
        let guard = GpuErrorGuard::new();
        let scope = guard.scope("deck abcd1234");
        assert_eq!(scope.faulted(), None);
        assert_eq!(guard.fault_count(), 0);
    }

    #[test]
    fn a_fault_is_attributed_to_the_open_scope() {
        let guard = GpuErrorGuard::new();
        {
            let scope = guard.scope("deck abcd1234");
            record(&guard, "conflicting usages");
            assert_eq!(scope.faulted().as_deref(), Some("conflicting usages"));
        }
        let faults = guard.take_faults();
        assert_eq!(faults.len(), 1);
        assert_eq!(faults[0].context.as_deref(), Some("deck abcd1234"));
    }

    #[test]
    fn a_scope_does_not_see_faults_raised_before_it_opened() {
        let guard = GpuErrorGuard::new();
        record(&guard, "earlier failure");
        let scope = guard.scope("deck beef");
        assert_eq!(
            scope.faulted(),
            None,
            "a deck must not be blamed for another deck's error"
        );
    }

    #[test]
    fn context_is_restored_when_a_scope_closes() {
        let guard = GpuErrorGuard::new();
        {
            let _outer = guard.scope("channel 0");
            {
                let _inner = guard.scope("deck abcd");
                record(&guard, "inner");
            }
            record(&guard, "outer");
        }
        record(&guard, "no scope");

        let faults = guard.take_faults();
        let contexts: Vec<_> = faults.iter().map(|f| f.context.as_deref()).collect();
        assert_eq!(
            contexts,
            vec![Some("deck abcd"), Some("channel 0"), None],
            "nesting must restore the enclosing context, not clear it"
        );
    }

    #[test]
    fn retained_faults_are_bounded() {
        let guard = GpuErrorGuard::new();
        for i in 0..(MAX_RETAINED * 3) {
            record(&guard, &format!("fault {i}"));
        }
        let faults = guard.take_faults();
        assert_eq!(
            faults.len(),
            MAX_RETAINED,
            "a shader failing every frame must not grow this without bound"
        );
        // The newest are the ones kept.
        assert_eq!(
            faults.last().map(|f| f.message.as_str()),
            Some(format!("fault {}", MAX_RETAINED * 3 - 1).as_str())
        );
        assert_eq!(
            guard.fault_count(),
            MAX_RETAINED * 3,
            "the counter is total"
        );
        assert!(guard.take_faults().is_empty(), "draining consumes");
    }
}
