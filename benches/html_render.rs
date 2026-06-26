/// HTML deck render-thread cost benchmark.
///
/// Servo pumping (layout/script/rasterization) runs on a dedicated off-thread
/// engine; the render thread only does `HtmlManager::update()` — a non-blocking
/// `try_lock` + take + `queue.write_texture` per deck. This bench therefore
/// measures the render-thread per-frame cost as a function of active HTML decks
/// (0/1/2), not page complexity. The timed unit is `update()` followed by a
/// queue flush so the upload is real and the staging belt is recalled each
/// iteration; the `0_decks` case isolates the constant flush floor.
///
/// Decks use an animating (requestAnimationFrame) document so the off-thread
/// engine keeps a fresh frame waiting in every slot — the worst case where each
/// `update()` performs a real upload rather than skipping.
///
/// Skips cleanly when no GPU adapter is present or the `html` feature is off.
use std::time::{Duration, Instant};

use base64::Engine as _;
use criterion::{criterion_group, criterion_main, Criterion};
use varda::{html::HtmlManager, renderer::context::GpuContext};

/// 720p — a common output resolution and a balance between realism and the cost
/// of Servo's CPU rasterizer. Bump to 1920×1080 to profile the full deck size.
const W: u32 = 1280;
const H: u32 = 720;

/// An animating document: a requestAnimationFrame loop mutating element style
/// each frame. This keeps Servo `animating()` true so the off-thread engine
/// repaints continuously and a fresh frame is always waiting in the slot — the
/// worst case for render-thread cost (every `update()` does a real upload).
/// `tag` makes each deck's `data:` URL unique so `start_render` does not dedupe.
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

/// One frame as the engine sees it: pump Servo + upload, then flush the queue so
/// the upload actually executes and the wgpu staging belt is recalled.
fn frame(gpu: &GpuContext, mgr: &mut HtmlManager) {
    mgr.update(&gpu.device, &gpu.queue);
    gpu.queue.submit(std::iter::empty::<wgpu::CommandBuffer>());
    poll(gpu);
}

/// Give Servo wall-clock time to load the data URL and reach steady-state
/// animation before timing. Sleeps between pumps (real async load), unlike the
/// timed loop which measures raw per-frame cost.
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

    // Sweep the render-thread cost of update() against the number of active HTML
    // decks. Pumping is off-thread, so this isolates try_lock + take +
    // write_texture (+ queue flush) and is independent of page complexity. The
    // 0_decks case is the constant flush floor; each added deck is one more
    // upload (animating content guarantees a fresh frame every iteration).
    //
    // A SINGLE manager is reused and decks are added incrementally: Servo's
    // global `Opts` can be initialized only once per process, so each deck must
    // be a WebView on the one shared engine — spawning a second `Servo` panics
    // ("Already initialized"). This mirrors the production design exactly.
    let mut mgr = HtmlManager::new();
    for decks in 0usize..=2 {
        if decks > 0 {
            mgr.start_render(&data_url(&animating_doc(decks - 1)), W, H, &gpu.device)
                .expect("start_render returned None with the html feature enabled");
            warmup(&gpu, &mut mgr, Duration::from_secs(3));
        }
        group.bench_function(format!("{decks}_decks"), |b| {
            b.iter(|| frame(&gpu, &mut mgr))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_render_thread);
criterion_main!(benches);
