/// HTML deck per-frame render benchmarks (Servo software rasterization).
///
///   plain_html  — static document; baseline cost of spin + paint + readback +
///                 texture upload with no per-frame layout/script work.
///   html_css    — gradient background plus many CSS-keyframe-animated boxes;
///                 forces a full repaint every frame (no JS). Difference vs
///                 plain ≈ per-frame paint/layout cost.
///   html_css_js — requestAnimationFrame loop mutating element style each frame;
///                 forces JS execution + style recalc + layout + paint.
///                 Difference vs html_css ≈ per-frame scripting cost.
///
/// All three reuse a single Servo instance via `navigate()` — engine startup is
/// heavy and Servo multi-instance lifetime is unproven, so one instance is both
/// faster and safer. The timed unit is `HtmlManager::update()` (the exact call
/// the engine makes per frame) followed by a queue flush so the upload is real
/// and the staging belt cannot grow unbounded across iterations.
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

const PLAIN: &str = "<!doctype html><html><body style=\"background:#204060;color:#fff;\
font-family:sans-serif;margin:24px\"><h1>Varda HTML deck</h1>\
<p>Plain static HTML baseline.</p></body></html>";

const JS: &str = "<!doctype html><html><head><style>html,body{margin:0;height:100%;\
background:#000}#b{position:absolute;width:60px;height:60px;background:#0f0}</style></head>\
<body><div id=b></div><script>var b=document.getElementById('b'),t=0;\
function f(){t+=2;b.style.left=(t%400)+'px';b.style.top=((t*0.7)%300)+'px';\
requestAnimationFrame(f);}requestAnimationFrame(f);</script></body></html>";

/// Wrap an HTML document in a base64 `data:` URL (avoids percent-encoding).
fn data_url(html: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(html.as_bytes());
    format!("data:text/html;base64,{b64}")
}

/// Gradient + many CSS-keyframe-animated boxes; continuous repaint, no JS.
fn css_doc() -> String {
    let mut boxes = String::new();
    for _ in 0..24 {
        boxes.push_str("<div class=box></div>");
    }
    format!(
        "<!doctype html><html><head><style>html,body{{margin:0;height:100%}}\
body{{background:linear-gradient(45deg,#102030,#304060)}}\
.box{{width:80px;height:80px;margin:8px;display:inline-block;background:#5af;\
animation:spin 1s linear infinite}}\
@keyframes spin{{from{{transform:rotate(0)}}to{{transform:rotate(360deg)}}}}\
</style></head><body>{boxes}</body></html>"
    )
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

fn bench_html_decks(c: &mut Criterion) {
    let Some(gpu) = make_context() else {
        eprintln!("no GPU adapter — skipping html_render benchmarks");
        return;
    };
    let mut mgr = HtmlManager::new();
    if !mgr.is_available() {
        eprintln!("html feature disabled — skipping html_render benchmarks");
        return;
    }
    let Some(idx) = mgr.start_render(&data_url(PLAIN), W, H, &gpu.device) else {
        eprintln!("start_render returned None — skipping html_render benchmarks");
        return;
    };

    let mut group = c.benchmark_group("html_deck_update");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(2));

    let profiles: [(&str, String); 3] = [
        ("plain_html", data_url(PLAIN)),
        ("html_css", data_url(&css_doc())),
        ("html_css_js", data_url(JS)),
    ];

    for (name, url) in &profiles {
        mgr.navigate(idx, url);
        warmup(&gpu, &mut mgr, Duration::from_secs(2));
        group.bench_function(*name, |b| b.iter(|| frame(&gpu, &mut mgr)));
    }

    group.finish();
}

criterion_group!(benches, bench_html_decks);
criterion_main!(benches);
