//! Real-hardware screen capture smoke tests.
//!
//! `#[ignore]` by design: these need a display server and, on macOS, an
//! interactively granted Screen Recording permission. Run locally per platform:
//!
//! ```sh
//! LIBRARY_PATH="/opt/homebrew/lib:${LIBRARY_PATH:-}" \
//!   cargo test --test screen_capture_smoke -- --ignored --nocapture
//! ```
//!
//! See spec/screen-capture.md § Testing Strategy.

use std::time::{Duration, Instant};

use varda::screen_capture::backend::{CaptureConfig, DEFAULT_CAPTURE_RATE};
use varda::screen_capture::platform;

/// Enumerate real targets and open the first display, asserting a frame lands
/// quickly. This is the "does the backend work at all" gate.
#[test]
#[ignore = "requires a display server and capture permission"]
fn platform_backend_delivers_a_frame() {
    let targets = match platform::enumerate() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Skipping: enumeration failed ({e})");
            return;
        }
    };
    assert!(
        !targets.is_empty(),
        "a machine with a display must enumerate at least one target"
    );
    let target = &targets[0];

    let config = CaptureConfig {
        scale_to: Some((1280, 720)),
        ..Default::default()
    };
    let mut session = platform::open(target, &config).expect("open first display");

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(frame) = session.next_frame() {
            assert!(frame.width > 0 && frame.height > 0);
            assert_eq!(
                frame.data.len(),
                (frame.width as usize) * (frame.height as usize) * 4
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no frame within 2s from '{}'", target.label);
}

/// Measure the delivered frame cadence two ways: polling far faster than the
/// capture rate (which reveals what the backend actually produces) and polling
/// at exactly the capture rate (what `capture_loop` used to do). A push-based
/// backend paces itself, so the second sampler aliases against it and produces
/// dropped and doubled intervals — the judder that reads as flicker in a
/// self-capture feedback loop.
#[test]
#[ignore = "requires a display server and capture permission"]
fn capture_cadence_is_regular_when_polled_faster_than_the_rate() {
    let Ok(targets) = platform::enumerate() else {
        eprintln!("Skipping: enumeration failed");
        return;
    };
    let Some(target) = targets.first() else {
        eprintln!("Skipping: no capture targets");
        return;
    };

    let config = CaptureConfig {
        rate: DEFAULT_CAPTURE_RATE,
        scale_to: Some((1280, 720)),
        ..Default::default()
    };

    for (label, poll) in [
        ("fast poll (2ms)", Duration::from_millis(2)),
        (
            "rate-paced poll",
            Duration::from_secs_f32(1.0 / DEFAULT_CAPTURE_RATE),
        ),
    ] {
        let mut session = platform::open(target, &config).expect("open target");
        // Discard the startup transient before measuring.
        let warmup = Instant::now() + Duration::from_millis(500);
        while Instant::now() < warmup {
            let _ = session.next_frame();
            std::thread::sleep(poll);
        }

        let mut arrivals: Vec<Duration> = Vec::new();
        let mut last = Instant::now();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if session.next_frame().is_some() {
                let now = Instant::now();
                arrivals.push(now - last);
                last = now;
            }
            std::thread::sleep(poll);
        }

        let count = arrivals.len();
        if count < 2 {
            eprintln!("{label}: only {count} frame(s) in 3s — backend not producing");
            continue;
        }
        let mean = arrivals.iter().sum::<Duration>().as_secs_f64() / count as f64;
        let max = arrivals.iter().max().copied().unwrap_or_default();
        let min = arrivals.iter().min().copied().unwrap_or_default();
        eprintln!(
            "{label}: {count} frames, mean {:.1}ms (={:.1} fps), min {:.1}ms, max {:.1}ms",
            mean * 1000.0,
            1.0 / mean,
            min.as_secs_f64() * 1000.0,
            max.as_secs_f64() * 1000.0,
        );
    }
}
