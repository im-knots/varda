/// Render-thread cost of HTML decks.
///
/// Servo layout, script, and rasterization run off-thread; the render thread
/// only calls `HtmlManager::update()`, a non-blocking `try_lock` + take +
/// `queue.write_texture` per deck. The timed unit is `update()` plus a queue
/// flush, measured against 0/1/2 active decks; `0_decks` is the flush floor.
///
/// Decks animate with requestAnimationFrame so every `update()` does a real
/// upload (worst case).
///
/// Skips without a GPU adapter or the `html` feature.
use std::time::{Duration, Instant};

use base64::Engine as _;
use criterion::{Criterion, criterion_group, criterion_main};
use varda::{html::HtmlManager, renderer::context::GpuContext};

/// 720p keeps Servo's CPU rasterizer affordable. Use 1920×1080 to profile the
/// full deck size.
const W: u32 = 1280;
const H: u32 = 720;

/// An animating document: a requestAnimationFrame loop mutating element style
/// each frame, so Servo repaints continuously and a fresh frame is always
/// waiting. `tag` makes each deck's `data:` URL unique so `start_render` does
/// not dedupe.
fn animating_doc(tag: usize) -> String {
    format!(
        "<!doctype html><!--{tag}--><html><head><style>html,body{{margin:0;\
height:100%;background:#000}}#b{{position:absolute;width:60px;height:60px;\
background:#0f0}}</style></head><body><div id=b></div><script>\
var b=document.getElementById('b'),t=0;function f(){{t+=2;\
b.style.left=(t%400)+'px';b.style.top=((t*0.7)%300)+'px';\
requestAnimationFrame(f);}}requestAnimationFrame(f);</script></body></html>"
    )
}

/// Wrap an HTML document in a base64 `data:` URL (avoids percent-encoding).
fn data_url(html: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(html.as_bytes());
    format!("data:text/html;base64,{b64}")
}

fn make_context() -> Option<GpuContext> {
    GpuContext::new_headless().ok()
}

fn poll(ctx: &GpuContext) {
    ctx.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .ok();
}

/// One engine frame: pump Servo and upload, then flush the queue so the upload
/// executes and the wgpu staging belt is recalled.
fn frame(gpu: &GpuContext, mgr: &mut HtmlManager) {
    mgr.update(&gpu.device, &gpu.queue);
    gpu.queue.submit(std::iter::empty::<wgpu::CommandBuffer>());
    poll(gpu);
}

/// Give Servo wall-clock time to load the data URL and reach steady animation
/// before timing. Sleeps between pumps, unlike the timed loop.
fn warmup(gpu: &GpuContext, mgr: &mut HtmlManager, dur: Duration) {
    let start = Instant::now();
    while start.elapsed() < dur {
        frame(gpu, mgr);
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn bench_render_thread(c: &mut Criterion) {
    let Some(gpu) = make_context() else {
        eprintln!("no GPU adapter — skipping html_render benchmarks");
        return;
    };
    if !HtmlManager::new().is_available() {
        eprintln!("html feature disabled — skipping html_render benchmarks");
        return;
    }

    let mut group = c.benchmark_group("html_render_thread_update");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(2));

    // One manager, decks added incrementally: Servo's global `Opts` initialize
    // once per process, so a second `Servo` panics ("Already initialized").
    // Each deck is a WebView on the shared engine, as in production.
    let mut mgr = HtmlManager::new();
    for decks in 0usize..=2 {
        if decks > 0 {
            mgr.start_render(&data_url(&animating_doc(decks - 1)), W, H, &gpu.device)
                .expect("start_render returned None with the html feature enabled");
            warmup(&gpu, &mut mgr, Duration::from_secs(3));
        }
        group.bench_function(format!("{decks}_decks"), |b| {
            b.iter(|| frame(&gpu, &mut mgr));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_render_thread);
criterion_main!(benches);
