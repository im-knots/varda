//! Keeps queue submits out of surface configuration.
//!
//! `Surface::configure` waits for the queue to drain and fails if another
//! thread submits during the wait. Windows are configured on their presenter
//! threads while other threads submit, so every submit and every configure
//! goes through one gate. A configure stops submits until the queue has
//! drained, which takes up to a frame of GPU work.

use std::sync::{Arc, PoisonError, RwLock};

/// Clones share one gate.
#[derive(Clone, Default)]
pub struct SubmitGate {
    lock: Arc<RwLock<()>>,
}

impl SubmitGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Submit `commands` to `queue`, unless a configure is running, in which
    /// case wait for it.
    pub fn submit<I>(&self, queue: &wgpu::Queue, commands: I) -> wgpu::SubmissionIndex
    where
        I: IntoIterator<Item = wgpu::CommandBuffer>,
    {
        let _open = self.lock.read().unwrap_or_else(PoisonError::into_inner);
        queue.submit(commands)
    }

    /// Configure `surface` with no submit running.
    pub fn configure(
        &self,
        surface: &wgpu::Surface<'_>,
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) {
        self.exclusive(|| surface.configure(device, config));
    }

    fn exclusive<R>(&self, work: impl FnOnce() -> R) -> R {
        let _closed = self.lock.write().unwrap_or_else(PoisonError::into_inner);
        work()
    }
}

#[cfg(test)]
mod tests {
    use super::SubmitGate;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// `Surface::configure` fails unless a wait leaves the queue empty, which
    /// is what this checks, since a test has no window to configure.
    #[test]
    fn a_wait_inside_the_gate_leaves_the_queue_empty_under_submits() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let gate = SubmitGate::new();
        let stop = AtomicBool::new(false);
        let interrupted = std::thread::scope(|scope| {
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    let encoder = context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                    gate.submit(&context.queue, std::iter::once(encoder.finish()));
                }
            });
            let interrupted = (0..300)
                .filter(|_| {
                    let status = gate.exclusive(|| {
                        context
                            .device
                            .poll(wgpu::PollType::wait_indefinitely())
                            .expect("the device is alive")
                    });
                    !status.is_queue_empty()
                })
                .count();
            stop.store(true, Ordering::Relaxed);
            interrupted
        });
        assert_eq!(interrupted, 0);
    }
}
