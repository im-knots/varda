//! A small pool of threads for building shaders and pipelines off the render
//! thread.

use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send>;

/// The process-wide pool. Workers start on the first build.
static POOL: OnceLock<Pool> = OnceLock::new();

/// Run `build` on a pool thread and return its result through the receiver.
/// A panic in `build` arrives as an error, as does a build after [`shutdown`].
pub fn spawn<T: Send + 'static>(
    build: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> mpsc::Receiver<anyhow::Result<T>> {
    POOL.get_or_init(|| {
        Pool::new(
            std::thread::available_parallelism()
                .map_or(1, std::num::NonZero::get)
                .div_ceil(2),
        )
    })
    .spawn(build)
}

/// Stop taking builds, drop the queued ones, and wait up to `timeout` for the
/// running ones to finish. Returns whether they all finished.
///
/// Call before the process exits. `exit` destroys the shader compiler's global
/// state, and a compile still running on a pool thread then crashes.
pub fn shutdown(timeout: Duration) -> bool {
    POOL.get().is_none_or(|pool| pool.shutdown(timeout))
}

/// How many builds are running, and whether the pool still takes new ones.
#[derive(Default)]
struct State {
    running: usize,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    idle: Condvar,
}

struct Pool {
    queue: Mutex<mpsc::Sender<Job>>,
    shared: Arc<Shared>,
}

impl Pool {
    fn new(workers: usize) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let shared = Arc::new(Shared::default());
        for i in 0..workers {
            let rx = Arc::clone(&rx);
            let shared = Arc::clone(&shared);
            let _ = std::thread::Builder::new()
                .name(format!("shader-build-{i}"))
                .spawn(move || work(&rx, &shared));
        }
        Self {
            queue: Mutex::new(tx),
            shared,
        }
    }

    fn spawn<T: Send + 'static>(
        &self,
        build: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    ) -> mpsc::Receiver<anyhow::Result<T>> {
        let (tx, rx) = mpsc::channel();
        let refused = tx.clone();
        let job: Job = Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(build))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the build panicked")));
            // The owner may be gone, e.g. an effect removed while it built.
            let _ = tx.send(result);
        });
        let queued = !self.is_closed() && self.queue.lock().is_ok_and(|q| q.send(job).is_ok());
        if !queued {
            let _ = refused.send(Err(anyhow::anyhow!("the build pool is shut down")));
        }
        rx
    }

    fn shutdown(&self, timeout: Duration) -> bool {
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        state.closed = true;
        self.shared
            .idle
            .wait_timeout_while(state, timeout, |s| s.running > 0)
            .is_ok_and(|(state, _)| state.running == 0)
    }

    fn is_closed(&self) -> bool {
        self.shared.state.lock().map_or(true, |s| s.closed)
    }
}

/// A worker: run queued jobs until the queue is gone. Once the pool is closed,
/// jobs still in the queue are dropped unrun, which disconnects their receivers.
fn work(rx: &Mutex<mpsc::Receiver<Job>>, shared: &Shared) {
    loop {
        let job = match rx.lock() {
            Ok(rx) => rx.recv(),
            Err(_) => return,
        };
        let Ok(job) = job else {
            return;
        };
        {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            if state.closed {
                continue;
            }
            state.running += 1;
        }
        job();
        if let Ok(mut state) = shared.state.lock() {
            state.running -= 1;
        }
        shared.idle.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::Pool;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    const LONG: Duration = Duration::from_secs(10);

    #[test]
    fn a_build_returns_its_result_and_a_panic_becomes_an_error() {
        let ok = super::spawn(|| Ok(7));
        assert_eq!(ok.recv().expect("result").expect("ok"), 7);
        let panicked = super::spawn(|| -> anyhow::Result<()> { panic!("boom") });
        assert!(panicked.recv().expect("result").is_err());
    }

    /// A build that reports when it starts and finishes only when released.
    fn held_build(pool: &Pool) -> (mpsc::Receiver<()>, mpsc::Sender<()>, Arc<AtomicBool>) {
        let (started_tx, started) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&finished);
        let _ = pool.spawn(move || {
            let _ = started_tx.send(());
            let _ = release_rx.recv();
            done.store(true, Ordering::SeqCst);
            Ok(())
        });
        (started, release, finished)
    }

    #[test]
    fn shutdown_waits_for_a_running_build_to_finish() {
        let pool = Arc::new(Pool::new(1));
        let (started, release, finished) = held_build(&pool);
        started.recv_timeout(LONG).expect("the build starts");

        let closer = Arc::clone(&pool);
        let shutdown = std::thread::spawn(move || closer.shutdown(LONG));
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !shutdown.is_finished(),
            "shutdown returned before the build finished"
        );

        release.send(()).expect("release");
        assert!(
            shutdown.join().expect("join"),
            "every build finished in time"
        );
        assert!(finished.load(Ordering::SeqCst));
    }

    #[test]
    fn shutdown_reports_a_build_still_running_at_the_timeout() {
        let pool = Pool::new(1);
        let (started, release, _) = held_build(&pool);
        started.recv_timeout(LONG).expect("the build starts");
        assert!(!pool.shutdown(Duration::from_millis(20)));
        release.send(()).expect("release");
    }

    #[test]
    fn a_build_queued_at_shutdown_never_runs() {
        let pool = Arc::new(Pool::new(1));
        let (started, release, _) = held_build(&pool);
        started.recv_timeout(LONG).expect("the build starts");
        let ran = Arc::new(AtomicBool::new(false));
        let ran_flag = Arc::clone(&ran);
        let queued = pool.spawn(move || {
            ran_flag.store(true, Ordering::SeqCst);
            Ok(())
        });

        let closer = Arc::clone(&pool);
        let shutdown = std::thread::spawn(move || closer.shutdown(LONG));
        while !pool.is_closed() {
            std::thread::yield_now();
        }
        release.send(()).expect("release");
        assert!(shutdown.join().expect("join"));

        assert!(
            queued.recv_timeout(LONG).is_err(),
            "the queued build sends nothing"
        );
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn a_build_after_shutdown_is_an_error() {
        let pool = Pool::new(1);
        assert!(pool.shutdown(LONG));
        let late = pool.spawn(|| Ok(1));
        assert!(late.recv_timeout(LONG).expect("a reply").is_err());
    }
}
