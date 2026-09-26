/// Per-frame cost of handing a read-back frame to a headless output's target,
/// the step every active headless output takes once per rendered frame.
///
/// `no_encoder` delivers a 1080p frame header to an output with no encoder
/// running: the dispatch alone. The frame carries no bytes, so freeing it,
/// which the render thread pays whether or not an encoder runs, is left out.
/// `recording_feed` hands a freshly read-back 1080p frame to a recording's
/// writer queue: `accepting`, where the writer keeps up, and `full`, where the
/// frame is dropped. Making the frame is not timed; dropping it on the render
/// thread is.
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use varda::delivery::Delivery;
use varda::renderer::ReadbackFrame;
use varda::renderer::context::OutputTarget;
use varda::testing::RecordingFeedBench;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

fn bench_output_delivery(c: &mut Criterion) {
    let target = OutputTarget::NdiSend {
        sender_name: "bench".to_string(),
    };
    let mut delivery = Delivery::default();
    let mut g = c.benchmark_group("output_delivery");
    g.bench_function("no_encoder", |b| {
        b.iter_batched(
            || ReadbackFrame::rgba8(WIDTH, HEIGHT, Vec::new()),
            |frame| delivery.deliver(&target, "bench", frame),
            BatchSize::SmallInput,
        );
    });
    g.finish();
}

fn bench_recording_feed(c: &mut Criterion) {
    let feed = RecordingFeedBench::new(WIDTH, HEIGHT);
    let mut g = c.benchmark_group("recording_feed");
    g.bench_function("accepting", |b| {
        b.iter_batched(
            || feed.frame(),
            |frame| feed.feed_accepting(frame),
            BatchSize::PerIteration,
        );
    });
    g.bench_function("full", |b| {
        b.iter_batched(
            || feed.frame(),
            |frame| feed.feed_full(frame),
            BatchSize::PerIteration,
        );
    });
    g.finish();
}

criterion_group!(benches, bench_output_delivery, bench_recording_feed);
criterion_main!(benches);
