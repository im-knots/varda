/// Per-frame cost of handing a read-back frame to a headless output's target,
/// the step every active headless output takes once per rendered frame.
///
/// `ndi_send` delivers a 1080p frame to an NDI sender on a disabled runtime, so
/// what is measured is the dispatch around the send rather than NDI itself.
/// Skipped when no GPU adapter is available.
use criterion::{Criterion, criterion_group, criterion_main};
use varda::delivery::Delivery;
use varda::ndi::NdiManager;
use varda::renderer::ReadbackFrame;
use varda::renderer::context::{GpuContext, HeadlessOutput, OutputSource, OutputTarget};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

struct Fixture {
    output: HeadlessOutput,
    delivery: Delivery,
    ndi: NdiManager,
    frame: ReadbackFrame,
}

/// One real frame, read back from the output's texture.
fn read_back(gpu: &GpuContext, output: &mut HeadlessOutput) -> ReadbackFrame {
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    output
        .readback
        .begin_readback(&mut encoder, &output.texture);
    gpu.queue.submit(std::iter::once(encoder.finish()));
    loop {
        if let Some(frame) = output.readback.try_read(&gpu.device) {
            return frame;
        }
        let _ = gpu.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}

fn fixture(gpu: &GpuContext) -> Fixture {
    let target = OutputTarget::NdiSend {
        sender_name: "bench".to_string(),
    };
    let mut output = HeadlessOutput::new(
        &gpu.device,
        "bench".to_string(),
        OutputSource::Master,
        target,
        WIDTH,
        HEIGHT,
        varda::delivery::presentation::plan,
    );
    let frame = read_back(gpu, &mut output);
    Fixture {
        output,
        delivery: Delivery::default(),
        ndi: NdiManager::new_disabled(),
        frame,
    }
}

fn deliver_once(f: &mut Fixture) {
    let _ = f
        .delivery
        .deliver(&f.output.target, &f.output.name, &f.frame, &mut f.ndi, 60);
}

fn bench_output_delivery(c: &mut Criterion) {
    let Ok(gpu) = GpuContext::new_headless() else {
        eprintln!("output_delivery: no GPU adapter, skipping");
        return;
    };
    let mut f = fixture(&gpu);
    let mut g = c.benchmark_group("output_delivery");
    g.bench_function("ndi_send", |b| {
        b.iter(|| deliver_once(std::hint::black_box(&mut f)));
    });
    g.finish();
}

criterion_group!(benches, bench_output_delivery);
criterion_main!(benches);
