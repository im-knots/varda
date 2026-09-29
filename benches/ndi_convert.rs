/// Render-thread cost of NDI video conversion at 1080p.
///
///   `ndi_send/uyvy_1080p` — one 8-bit frame from a rendered output to the
///                          UYVY bytes the sender publishes: recording and
///                          submitting GPU work, then collecting the frame.
///                          GPU execution time between the two is excluded.
///
/// Skipped without a GPU adapter.
use criterion::{Criterion, criterion_group, criterion_main};
use std::time::{Duration, Instant};
use varda::renderer::context::GpuContext;
use varda::testing::{NdiReceiveBench, NdiSendBench};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

fn bench_ndi_send(c: &mut Criterion) {
    let Ok(gpu) = GpuContext::new_headless() else {
        eprintln!("ndi_convert: no GPU adapter, skipping");
        return;
    };
    let mut send = NdiSendBench::new(gpu, WIDTH, HEIGHT);
    let mut g = c.benchmark_group("ndi_send");
    g.bench_function("uyvy_1080p", |b| {
        b.iter_custom(|iters| {
            let mut spent = Duration::ZERO;
            for _ in 0..iters {
                let t = Instant::now();
                send.encode();
                spent += t.elapsed();
                // A readback takes two collections: one issues the map, the
                // next returns the bytes.
                let mut got = None;
                while got.is_none() {
                    send.wait();
                    let t = Instant::now();
                    got = std::hint::black_box(send.read());
                    spent += t.elapsed();
                }
            }
            spent
        });
    });
    g.finish();
}

fn bench_ndi_receive(c: &mut Criterion) {
    let Ok(gpu) = GpuContext::new_headless() else {
        eprintln!("ndi_convert: no GPU adapter, skipping");
        return;
    };
    let mut recv = NdiReceiveBench::new(gpu, WIDTH, HEIGHT);
    let mut g = c.benchmark_group("ndi_receive");
    g.bench_function("receiver_thread_1080p", |b| b.iter(|| recv.receive()));
    g.bench_function("render_thread_1080p", |b| {
        b.iter_custom(|iters| {
            let mut spent = Duration::ZERO;
            for _ in 0..iters {
                recv.receive();
                let t = Instant::now();
                recv.upload();
                spent += t.elapsed();
                recv.wait();
            }
            spent
        });
    });
    g.finish();
}

criterion_group!(benches, bench_ndi_send, bench_ndi_receive);
criterion_main!(benches);
