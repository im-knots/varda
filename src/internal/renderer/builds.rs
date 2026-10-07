//! A small pool of threads for building shaders and pipelines off the render
//! thread.

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;

/// The pool's queue. Workers start on the first build.
fn queue() -> &'static Mutex<mpsc::Sender<Job>> {
    static QUEUE: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = std::sync::Arc::new(Mutex::new(rx));
        let workers = std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .div_ceil(2);
        for i in 0..workers {
            let rx = std::sync::Arc::clone(&rx);
            let _ = std::thread::Builder::new()
                .name(format!("shader-build-{i}"))
                .spawn(move || {
                    loop {
                        let job = match rx.lock() {
                            Ok(rx) => rx.recv(),
                            Err(_) => return,
                        };
                        match job {
                            Ok(job) => job(),
                            Err(_) => return,
                        }
                    }
                });
        }
        Mutex::new(tx)
    })
}

/// Run `build` on a pool thread and return its result through the receiver.
/// A panic in `build` arrives as an error.
pub fn spawn<T: Send + 'static>(
    build: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> mpsc::Receiver<anyhow::Result<T>> {
    let (tx, rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(build))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the build panicked")));
        // The owner may be gone, e.g. an effect removed while it built.
        let _ = tx.send(result);
    });
    if let Ok(queue) = queue().lock() {
        let _ = queue.send(job);
    }
    rx
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_build_returns_its_result_and_a_panic_becomes_an_error() {
        let ok = super::spawn(|| Ok(7));
        assert_eq!(ok.recv().expect("result").expect("ok"), 7);
        let panicked = super::spawn(|| -> anyhow::Result<()> { panic!("boom") });
        assert!(panicked.recv().expect("result").is_err());
    }
}
