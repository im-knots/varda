//! Render a generator shader headless and write a PNG, for checking a shader
//! while authoring it.
//!
//! The frame comes off the mixer composite, so the PNG has been through the same
//! linear-light compositing and tonemap a live show uses. Time steps on a fixed
//! 60 fps clock, so a frame index is reproducible run to run.
//!
//! ```text
//! cargo run --release --example shader_preview -- shaders/foo.fs /tmp/foo.png \
//!     --size 960x540 --frame 120 --set speed=0.4
//! ```
//!
//! Options:
//!
//! - `--size WxH`, `--frame N`, `--set name=value` (clamped to the declared
//!   `MIN`/`MAX`, with a warning). A color takes four values: `--set
//!   fog_color=0.1,0.2,0.3,1`.
//! - `--warmup N` and `--settle MS`: extra frames at time 0, each followed by a
//!   pause, so background preprocessors can publish before the capture.
//! - `--time N`: after the capture, render N more frames and report ms/frame for
//!   the render loop alone.
//! - `--probe`: per-channel statistics of the linear values the shader wrote,
//!   read before the PNG's sRGB encode.
//! - `--pair`: also capture frame N-1 from the same process, write it next to
//!   the output, and report their difference at 1x, 4x, 8x and 16x downsampling.
//! - `--state JSON` (or `--state @file.json`): restore preprocessor state before
//!   the first frame, as a scene does, for example a fractal explorer camera
//!   pose. The value maps preprocessor type to its state.
//! - `--print-state`: print the preprocessor state after the capture, to save
//!   a pose found by flying.
//! - `--sweep name=lo:hi`, `--sweep2 name=lo:hi`, `--grid CxR`: a contact sheet
//!   of the same shot with one or two parameters walked across it. `--size` is
//!   the whole sheet.
//!
//! Every capture prints the frame's mean Laplacian. Near zero is a flat field
//! however bright it is; tens is a frame carrying detail. Report timings next
//! to it, because an empty frame is cheap.

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::{Context, Result, bail};
use varda::{
    audio::AudioData,
    deck::Deck,
    isf::ISFShader,
    mixer::{FrameInputs, Mixer},
    modulation::{AnalyzerValues, AudioValues},
    params::ShaderParams,
    renderer::context::GpuContext,
};

/// Authoring rate. A frame index names a point on this clock.
const FPS: f32 = 60.0;

/// Linear RGB, row-major.
type Pixels = Vec<[f32; 3]>;

/// One parameter walked across an axis of the contact sheet.
struct Sweep {
    name: String,
    lo: f32,
    hi: f32,
}

fn parse_sweep(spec: &str) -> Result<Sweep> {
    let (name, range) = spec.split_once('=').context("--sweep wants name=lo:hi")?;
    let (lo, hi) = range.split_once(':').context("--sweep wants name=lo:hi")?;
    Ok(Sweep {
        name: name.to_string(),
        lo: lo.parse().context("bad sweep low bound")?,
        hi: hi.parse().context("bad sweep high bound")?,
    })
}

/// Value for cell `index` of `count` along a sweep axis. A single-cell axis sits
/// at the low end.
fn sweep_value(sweep: &Sweep, index: u32, count: u32) -> f32 {
    if count <= 1 {
        return sweep.lo;
    }
    sweep.lo + (sweep.hi - sweep.lo) * (index as f32 / (count - 1) as f32)
}

struct Options {
    shader: String,
    output: String,
    width: u32,
    height: u32,
    warmup: u32,
    settle: u64,
    frame: u32,
    time: u32,
    probe: bool,
    pair: bool,
    state: Option<serde_json::Map<String, serde_json::Value>>,
    print_state: bool,
    overrides: Vec<(String, f32)>,
    colors: Vec<(String, [f32; 4])>,
    across: Option<Sweep>,
    down: Option<Sweep>,
    cols: u32,
    rows: u32,
}

fn next<T: std::str::FromStr>(args: &mut impl Iterator<Item = String>, what: &str) -> Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    args.next()
        .with_context(|| format!("{what} wants a value"))?
        .parse()
        .with_context(|| format!("bad value for {what}"))
}

fn parse_args() -> Result<Options> {
    let mut args = std::env::args().skip(1);
    let shader = args
        .next()
        .context("usage: shader_preview <shader.fs> <out.png> [options]")?;
    let output = args.next().context("missing output path")?;
    let mut opts = Options {
        shader,
        output,
        width: 960,
        height: 540,
        warmup: 0,
        settle: 8,
        frame: 90,
        time: 0,
        probe: false,
        pair: false,
        state: None,
        print_state: false,
        overrides: Vec::new(),
        colors: Vec::new(),
        across: None,
        down: None,
        cols: 4,
        rows: 3,
    };
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--size" => {
                let spec: String = next(&mut args, "--size")?;
                let (w, h) = spec.split_once('x').context("--size wants WxH")?;
                opts.width = w.parse().context("bad width")?;
                opts.height = h.parse().context("bad height")?;
            }
            "--warmup" => opts.warmup = next(&mut args, "--warmup")?,
            "--settle" => opts.settle = next(&mut args, "--settle")?,
            "--frame" => opts.frame = next(&mut args, "--frame")?,
            "--time" => opts.time = next(&mut args, "--time")?,
            "--probe" => opts.probe = true,
            "--pair" => opts.pair = true,
            "--print-state" => opts.print_state = true,
            "--state" => {
                let spec: String = next(&mut args, "--state")?;
                let text = match spec.strip_prefix('@') {
                    Some(path) => {
                        std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?
                    }
                    None => spec,
                };
                let value: serde_json::Value =
                    serde_json::from_str(&text).context("--state wants a JSON object")?;
                opts.state = Some(
                    value
                        .as_object()
                        .context("--state wants a JSON object")?
                        .clone(),
                );
            }
            "--set" => {
                let spec: String = next(&mut args, "--set")?;
                let (name, value) = spec.split_once('=').context("--set wants name=value")?;
                if value.contains(',') {
                    let channels: Vec<f32> = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<_, _>>()
                        .context("a color wants r,g,b,a")?;
                    let color: [f32; 4] = channels
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("a color wants four values"))?;
                    opts.colors.push((name.to_string(), color));
                } else {
                    opts.overrides.push((name.to_string(), value.parse()?));
                }
            }
            "--sweep" => opts.across = Some(parse_sweep(&next::<String>(&mut args, "--sweep")?)?),
            "--sweep2" => opts.down = Some(parse_sweep(&next::<String>(&mut args, "--sweep2")?)?),
            "--grid" => {
                let spec: String = next(&mut args, "--grid")?;
                let (c, r) = spec.split_once('x').context("--grid wants CxR")?;
                opts.cols = c.parse().context("bad grid columns")?;
                opts.rows = r.parse().context("bad grid rows")?;
            }
            other => bail!("unknown option {other}"),
        }
    }
    let sheet = opts.across.is_some() || opts.down.is_some();
    if sheet && (opts.pair || opts.probe || opts.time > 0) {
        bail!("--pair, --probe and --time apply to a single shot, not a sweep");
    }
    if opts.pair && opts.frame == 0 {
        bail!("--pair needs --frame of at least 1");
    }
    Ok(opts)
}

/// Apply a `--set` override through the setter that matches how the engine
/// packs the parameter, clamped to its declared range.
fn apply_override(params: &mut ShaderParams, name: &str, value: f32) -> Result<()> {
    let definition = params
        .definitions
        .get(name)
        .with_context(|| format!("shader has no parameter `{name}`"))?;
    let lo = definition.min.unwrap_or(f32::NEG_INFINITY);
    let hi = definition.max.unwrap_or(f32::INFINITY);
    let clamped = value.clamp(lo, hi);
    if !(lo..=hi).contains(&value) {
        eprintln!("warning: --set {name}={value} is outside [{lo}, {hi}]; using {clamped}");
    }
    match definition.input_type.as_str() {
        "float" => params.set_float(name, clamped),
        "bool" => params.set_bool(name, clamped > 0.5),
        "long" => params.set_long(name, clamped as i32),
        other => bail!("`{name}` is a {other}, which --set cannot express"),
    }
    Ok(())
}

/// Read the composite back as linear RGB. Blocking is fine in a one-shot tool.
fn read_composite(context: &GpuContext, mixer: &Mixer, width: u32, height: u32) -> Pixels {
    let bytes_per_pixel = 8u32; // Rgba16Float
    let padded = (width * bytes_per_pixel).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shader_preview readback"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = context
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: mixer.composite_texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    context.queue.submit(std::iter::once(encoder.finish()));

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    rx.recv().expect("map callback").expect("map succeeded");

    let data = slice.get_mapped_range().expect("mapped range");
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let row = (y * padded) as usize;
        for x in 0..width {
            let at = row + (x * bytes_per_pixel) as usize;
            let channel = |i: usize| -> f32 {
                let bits = u16::from_le_bytes([data[at + i * 2], data[at + i * 2 + 1]]);
                f32::from(half::f16::from_bits(bits))
            };
            pixels.push([channel(0), channel(1), channel(2)]);
        }
    }
    drop(data);
    buffer.unmap();
    pixels
}

/// sRGB display encoding: the composite is linear light, a PNG is not.
fn encode_srgb(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let encoded = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

fn to_image(pixels: &Pixels, width: u32, height: u32) -> image::RgbImage {
    image::RgbImage::from_fn(width, height, |x, y| {
        let rgb = pixels[(y * width + x) as usize];
        image::Rgb([
            encode_srgb(rgb[0]),
            encode_srgb(rgb[1]),
            encode_srgb(rgb[2]),
        ])
    })
}

/// Mean absolute Laplacian of the sRGB luminance: whether the frame contains
/// anything.
fn frame_detail(image: &image::RgbImage) -> f64 {
    let (w, h) = (i64::from(image.width()), i64::from(image.height()));
    if w < 3 || h < 3 {
        return 0.0;
    }
    let luma = |x: i64, y: i64| -> f64 {
        let p = image.get_pixel(x as u32, y as u32);
        (f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])) / 3.0
    };
    let mut total = 0.0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            total += (4.0 * luma(x, y)
                - luma(x - 1, y)
                - luma(x + 1, y)
                - luma(x, y - 1)
                - luma(x, y + 1))
            .abs();
        }
    }
    total / ((w - 2) * (h - 2)) as f64
}

/// Per-channel mean and percentiles of the linear values.
fn print_probe(pixels: &Pixels) {
    for (channel, name) in ["R", "G", "B"].iter().enumerate() {
        let mut values: Vec<f32> = pixels.iter().map(|p| p[channel]).collect();
        values.sort_by(f32::total_cmp);
        let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
        let mean = values.iter().map(|v| f64::from(*v)).sum::<f64>() / values.len() as f64;
        println!(
            "probe {name}: mean {mean:.5} p50 {:.5} p90 {:.5} p99 {:.5} max {:.5}",
            at(0.5),
            at(0.9),
            at(0.99),
            at(1.0)
        );
    }
}

/// Difference of two frames after box-downsampling by `factor`, in sRGB
/// 8-bit units: (mean max-channel difference, fraction of cells over 16).
fn frame_difference(first: &image::RgbImage, second: &image::RgbImage, factor: u32) -> (f64, f64) {
    let (cols, rows) = (first.width() / factor, first.height() / factor);
    let cell = |img: &image::RgbImage, cx: u32, cy: u32| -> [f64; 3] {
        let mut sum = [0.0; 3];
        for y in cy * factor..(cy + 1) * factor {
            for x in cx * factor..(cx + 1) * factor {
                let px = img.get_pixel(x, y);
                for c in 0..3 {
                    sum[c] += f64::from(px[c]);
                }
            }
        }
        sum.map(|s| s / f64::from(factor * factor))
    };
    let mut total = 0.0;
    let mut over = 0usize;
    for cy in 0..rows {
        for cx in 0..cols {
            let (pa, pb) = (cell(first, cx, cy), cell(second, cx, cy));
            let diff = (0..3).map(|c| (pa[c] - pb[c]).abs()).fold(0.0, f64::max);
            total += diff;
            if diff > 16.0 {
                over += 1;
            }
        }
    }
    let cells = f64::from(cols * rows).max(1.0);
    (total / cells, over as f64 / cells)
}

/// What one shot renders and captures.
struct Shot<'a> {
    shader_path: &'a str,
    overrides: &'a [(String, f32)],
    colors: &'a [(String, [f32; 4])],
    width: u32,
    height: u32,
    frame: u32,
    warmup: u32,
    settle: u64,
    capture_previous: bool,
    state: Option<&'a serde_json::Map<String, serde_json::Value>>,
    timed_frames: u32,
}

/// Frames the shot captured: the requested one, and the one before it when
/// asked for.
struct Captured {
    frame: Pixels,
    previous: Option<Pixels>,
    /// Preprocessor state after the capture.
    state: Option<serde_json::Value>,
}

fn frame_inputs<'a>(
    audio: &'a AudioData,
    audio_values: &'a AudioValues,
    analyzer_values: &'a AnalyzerValues,
    time: f32,
) -> FrameInputs<'a> {
    FrameInputs {
        audio_data: audio,
        audio_values,
        analyzer_values,
        beat_time: None,
        transport: None,
        free_run_time: Some(time),
        write_param: varda::param_router::write_macro_target,
    }
}

fn render_frame(context: &GpuContext, mixer: &mut Mixer, inputs: &FrameInputs<'_>) -> Result<()> {
    mixer.render(context, inputs, 60, &[])?;
    let _ = context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    Ok(())
}

/// Render one shot. A fresh deck and mixer per call, because phase
/// accumulators integrate: a flight only reaches frame N by flying there.
fn render_shot(context: &GpuContext, shot: &Shot<'_>) -> Result<Captured> {
    let shader = ISFShader::from_file(shot.shader_path)?;
    let mut deck = Deck::from_shader(context, shader, shot.width, shot.height)?;
    for (name, value) in shot.overrides {
        apply_override(&mut deck.generator_params, name, *value)?;
    }
    for (name, color) in shot.colors {
        if !deck.generator_params.definitions.contains_key(name) {
            bail!("shader has no parameter `{name}`");
        }
        deck.generator_params.set_color(name, *color);
    }
    // Without this, preprocessor textures stay unbound and read as zero, and the
    // frame looks plausible and is wrong.
    deck.start_declared_preprocessors();
    if let Some(state) = shot.state {
        deck.restore_preprocessor_state(state);
    }

    let mut mixer = Mixer::new(context, shot.width, shot.height)?;
    mixer
        .channel_mut(0)
        .context("mixer has no channel 0")?
        .add_deck(deck);
    for channel in mixer.channels_mut() {
        for slot in &mut channel.decks {
            // Render every frame; the adaptive scheduler would skip slow ones.
            slot.render_fps = varda::channel::DeckRenderFps::Fixed(0);
        }
    }

    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();

    for _ in 0..shot.warmup {
        render_frame(
            context,
            &mut mixer,
            &frame_inputs(&audio, &audio_values, &analyzer_values, 0.0),
        )?;
        std::thread::sleep(std::time::Duration::from_millis(shot.settle));
    }

    let mut previous = None;
    for step in 0..=shot.frame {
        render_frame(
            context,
            &mut mixer,
            &frame_inputs(&audio, &audio_values, &analyzer_values, step as f32 / FPS),
        )?;
        if shot.capture_previous && step + 1 == shot.frame {
            previous = Some(read_composite(context, &mixer, shot.width, shot.height));
        }
    }
    let frame = read_composite(context, &mixer, shot.width, shot.height);

    if shot.timed_frames > 0 {
        let started = std::time::Instant::now();
        for step in 1..=shot.timed_frames {
            let time = (shot.frame + step) as f32 / FPS;
            render_frame(
                context,
                &mut mixer,
                &frame_inputs(&audio, &audio_values, &analyzer_values, time),
            )?;
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        println!(
            "time {} frames: {:.2} ms/frame",
            shot.timed_frames,
            elapsed / f64::from(shot.timed_frames)
        );
    }

    let state = mixer
        .channels()
        .first()
        .and_then(|channel| channel.decks.first())
        .and_then(|slot| slot.deck.source_config().get("preprocessor_state").cloned());
    Ok(Captured {
        frame,
        previous,
        state,
    })
}

/// `out.png` becomes `out_prev.png`.
fn previous_path(output: &str) -> String {
    let path = std::path::Path::new(output);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("frame");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("png");
    path.with_file_name(format!("{stem}_prev.{ext}"))
        .to_string_lossy()
        .into_owned()
}

fn single_shot(context: &GpuContext, opts: &Options) -> Result<()> {
    let captured = render_shot(
        context,
        &Shot {
            shader_path: &opts.shader,
            overrides: &opts.overrides,
            colors: &opts.colors,
            width: opts.width,
            height: opts.height,
            frame: opts.frame,
            warmup: opts.warmup,
            settle: opts.settle,
            capture_previous: opts.pair,
            state: opts.state.as_ref(),
            timed_frames: opts.time,
        },
    )?;
    let image = to_image(&captured.frame, opts.width, opts.height);
    image.save(&opts.output)?;
    println!("frame detail (mean Laplacian) {:.2}", frame_detail(&image));
    if opts.probe {
        print_probe(&captured.frame);
    }
    if opts.print_state {
        println!(
            "state {}",
            captured
                .state
                .map_or_else(|| "{}".to_owned(), |s| s.to_string())
        );
    }
    if let Some(previous) = &captured.previous {
        let previous = to_image(previous, opts.width, opts.height);
        let path = previous_path(&opts.output);
        previous.save(&path)?;
        println!("wrote {path} (frame {})", opts.frame - 1);
        for factor in [1, 4, 8, 16] {
            let (mean, over) = frame_difference(&previous, &image, factor);
            println!(
                "pair {factor:>2}x: mean diff {mean:.2}/255, {:.1}% of cells over 16/255",
                over * 100.0
            );
        }
    }
    println!("wrote {} ({}x{})", opts.output, opts.width, opts.height);
    Ok(())
}

fn contact_sheet(context: &GpuContext, opts: &Options) -> Result<()> {
    let cols = if opts.across.is_some() { opts.cols } else { 1 };
    let rows = if opts.down.is_some() { opts.rows } else { 1 };
    let cell_w = opts.width / cols;
    let cell_h = opts.height / rows;
    let mut sheet = image::RgbImage::new(cell_w * cols, cell_h * rows);
    for row in 0..rows {
        for col in 0..cols {
            let mut overrides = opts.overrides.clone();
            let mut label = String::new();
            if let Some(sweep) = &opts.across {
                let value = sweep_value(sweep, col, cols);
                overrides.push((sweep.name.clone(), value));
                let _ = write!(label, "{}={value:.3} ", sweep.name);
            }
            if let Some(sweep) = &opts.down {
                let value = sweep_value(sweep, row, rows);
                overrides.push((sweep.name.clone(), value));
                let _ = write!(label, "{}={value:.3}", sweep.name);
            }
            let captured = render_shot(
                context,
                &Shot {
                    shader_path: &opts.shader,
                    overrides: &overrides,
                    colors: &opts.colors,
                    width: cell_w,
                    height: cell_h,
                    frame: opts.frame,
                    warmup: opts.warmup,
                    settle: opts.settle,
                    capture_previous: false,
                    state: opts.state.as_ref(),
                    timed_frames: 0,
                },
            )?;
            let cell = to_image(&captured.frame, cell_w, cell_h);
            image::imageops::replace(
                &mut sheet,
                &cell,
                i64::from(col * cell_w),
                i64::from(row * cell_h),
            );
            // No text is drawn into the sheet; this legend maps cells to values.
            println!("  r{row} c{col}  {label}");
        }
    }
    sheet.save(&opts.output)?;
    println!("frame detail (mean Laplacian) {:.2}", frame_detail(&sheet));
    println!(
        "wrote {} ({}x{})",
        opts.output,
        sheet.width(),
        sheet.height()
    );
    Ok(())
}

fn main() -> Result<()> {
    // Preprocessors report failures through the log.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let opts = parse_args()?;
    let context = GpuContext::new_headless().context("no headless GPU adapter")?;
    if opts.across.is_some() || opts.down.is_some() {
        contact_sheet(&context, &opts)
    } else {
        single_shot(&context, &opts)
    }
}
