//! Render a generator shader headless and write PNG stills, for eyeballing a
//! shader while authoring it.
//!
//! The frame comes off the mixer composite rather than the deck texture, so what
//! lands in the PNG has been through the same linear-light compositing and
//! tonemap a live show sees. Time is stepped on a fixed 60 fps clock, which makes
//! a given frame index reproducible run to run.
//!
//! ```text
//! cargo run --release --example shader_preview -- shaders/foo.fs /tmp/foo.png \
//!     --size 960x540 --frame 120 --set fly_speed=0.4 --set bloom=1.2
//! ```
//!
//! With `--sweep` it renders a contact sheet instead of a single frame: a grid of
//! the same shot with one or two parameters walked across it. Fractal work is
//! mostly "change the numbers and fly around looking for something good", and
//! doing that one still at a time is the slow way. `--size` is the size of the
//! whole sheet, so each cell is that divided by the grid.
//!
//! ```text
//! cargo run --release --example shader_preview -- shaders/foo.fs /tmp/sheet.png \
//!     --size 1600x900 --frame 200 --grid 5x3 \
//!     --sweep fold_scale=1.3:2.6 --sweep2 yaw=0.0:1.5
//! ```

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::{bail, Context, Result};
use varda::{
    audio::AudioData,
    deck::Deck,
    isf::ISFShader,
    mixer::{FrameInputs, Mixer},
    modulation::{AnalyzerValues, AudioValues},
    params::ShaderParams,
    renderer::context::GpuContext,
};

/// Authoring rate. `free_run_time` is derived from it so a frame index names a
/// point on the clock rather than however fast this machine rendered.
const FPS: f32 = 60.0;

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
/// at the low end rather than dividing by zero.
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
    frame: u32,
    overrides: Vec<(String, f32)>,
    across: Option<Sweep>,
    down: Option<Sweep>,
    cols: u32,
    rows: u32,
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
        frame: 90,
        overrides: Vec::new(),
        across: None,
        down: None,
        cols: 4,
        rows: 3,
    };
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--size" => {
                let spec = args.next().context("--size wants WxH")?;
                let (w, h) = spec.split_once('x').context("--size wants WxH")?;
                opts.width = w.parse().context("bad width")?;
                opts.height = h.parse().context("bad height")?;
            }
            "--frame" => {
                opts.frame = args.next().context("--frame wants a number")?.parse()?;
            }
            "--set" => {
                let spec = args.next().context("--set wants name=value")?;
                let (name, value) = spec.split_once('=').context("--set wants name=value")?;
                opts.overrides.push((name.to_string(), value.parse()?));
            }
            "--sweep" => {
                let spec = args.next().context("--sweep wants name=lo:hi")?;
                opts.across = Some(parse_sweep(&spec)?);
            }
            "--sweep2" => {
                let spec = args.next().context("--sweep2 wants name=lo:hi")?;
                opts.down = Some(parse_sweep(&spec)?);
            }
            "--grid" => {
                let spec = args.next().context("--grid wants CxR")?;
                let (c, r) = spec.split_once('x').context("--grid wants CxR")?;
                opts.cols = c.parse().context("bad grid columns")?;
                opts.rows = r.parse().context("bad grid rows")?;
            }
            other => bail!("unknown option {other}"),
        }
    }
    Ok(opts)
}

/// Apply a `--set` override, routing through the setter that matches how the
/// engine packs that parameter's bytes.
fn apply_override(params: &mut ShaderParams, name: &str, value: f32) -> Result<()> {
    let definition = params
        .definitions
        .get(name)
        .with_context(|| format!("shader has no parameter `{name}`"))?;
    match definition.input_type.as_str() {
        "float" => params.set_float(name, value),
        "bool" => params.set_bool(name, value > 0.5),
        "long" => params.set_long(name, value as i32),
        other => bail!("`{name}` is a {other}, which --set cannot express"),
    }
    Ok(())
}

/// Read the composite back as linear-light RGB. Blocking is fine here: this is a
/// one-shot tool, not the render thread.
fn read_composite(context: &GpuContext, mixer: &Mixer, width: u32, height: u32) -> Vec<[f32; 3]> {
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

/// sRGB display encoding — the composite is linear light, a PNG is not.
fn encode_srgb(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let encoded = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

/// Render one shot to linear-light RGB. A fresh deck and mixer per call, because
/// phase accumulators integrate: a fly-through only arrives at frame N by having
/// flown there, so a contact sheet cell cannot resume from its neighbour.
fn render_shot(
    context: &GpuContext,
    shader_path: &str,
    overrides: &[(String, f32)],
    width: u32,
    height: u32,
    frame: u32,
) -> Result<Vec<[f32; 3]>> {
    let shader = ISFShader::from_file(shader_path)?;
    let mut deck = Deck::new(context, shader, width, height)?;
    for (name, value) in overrides {
        apply_override(&mut deck.generator_params, name, *value)?;
    }

    let mut mixer = Mixer::new(context, width, height)?;
    mixer
        .channel_mut(0)
        .context("mixer has no channel 0")?
        .add_deck(deck);

    let audio = AudioData::default();
    let audio_values = AudioValues {
        sources: HashMap::default(),
    };
    let analyzer_values = AnalyzerValues::default();

    for step in 0..=frame {
        let inputs = FrameInputs {
            audio_data: &audio,
            audio_values: &audio_values,
            analyzer_values: &analyzer_values,
            beat_time: None,
            transport: None,
            free_run_time: Some(step as f32 / FPS),
        };
        mixer.render(context, &inputs, 60, &[])?;
        let _ = context.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }

    Ok(read_composite(context, &mixer, width, height))
}

fn main() -> Result<()> {
    let opts = parse_args()?;
    let context = GpuContext::new_headless().context("no headless GPU adapter")?;

    let sheet = opts.across.is_some() || opts.down.is_some();
    let (cols, rows) = if sheet {
        (
            if opts.across.is_some() { opts.cols } else { 1 },
            if opts.down.is_some() { opts.rows } else { 1 },
        )
    } else {
        (1, 1)
    };
    let cell_w = opts.width / cols;
    let cell_h = opts.height / rows;

    let mut image = image::RgbImage::new(cell_w * cols, cell_h * rows);
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

            let pixels = render_shot(
                &context,
                &opts.shader,
                &overrides,
                cell_w,
                cell_h,
                opts.frame,
            )?;
            for y in 0..cell_h {
                for x in 0..cell_w {
                    let rgb = pixels[(y * cell_w + x) as usize];
                    image.put_pixel(
                        col * cell_w + x,
                        row * cell_h + y,
                        image::Rgb([
                            encode_srgb(rgb[0]),
                            encode_srgb(rgb[1]),
                            encode_srgb(rgb[2]),
                        ]),
                    );
                }
            }
            if sheet {
                // No text is drawn into the sheet, so the legend is the only way
                // to get from a cell you liked back to its numbers.
                println!("  r{row} c{col}  {label}");
            }
        }
    }

    image.save(&opts.output)?;
    println!(
        "wrote {} ({}x{})",
        opts.output,
        cell_w * cols,
        cell_h * rows
    );
    Ok(())
}
