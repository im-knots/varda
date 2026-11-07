//! Shipped simulations must fill their buffers and seed their state correctly at
//! real output sizes. These run the real shaders on a headless GPU, because a
//! wrongly sized buffer or a missed seed still compiles and still draws something.
//! Skipped without a GPU adapter.

use varda::{isf::ISFShader, renderer::context::GpuContext, testing::render_shader};

const BLACK_HOLE: &str = "shaders/black_hole_sim.comp";
const BIG_BANG: &str = "shaders/big_bang.fs";

fn luma(frame: &[[f32; 4]]) -> Vec<f32> {
    frame
        .iter()
        .map(|[r, g, b, _]| 0.2126 * r + 0.7152 * g + 0.0722 * b)
        .collect()
}

/// Mean light in rows `from..to` of a `width` wide frame.
fn row_mean(luma: &[f32], width: usize, from: usize, to: usize) -> f32 {
    let rows = &luma[from * width..to * width];
    rows.iter().sum::<f32>() / rows.len() as f32
}

fn black_hole_at_1080p(ctx: &GpuContext) -> Vec<f32> {
    let params = [("lensing_steps", 16.0)];
    luma(&render_shader(ctx, BLACK_HOLE, (1920, 1080), &params, 12))
}

/// The binning grid must hold a whole cell for every cell of a 4K frame. Too small
/// a buffer loses the bottom of the frame without any error.
#[test]
fn black_hole_grid_buffer_holds_a_4k_frame() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/black_hole_sim.comp");
    let shader = ISFShader::from_file(path).expect("shader");
    let grid = shader
        .metadata
        .buffers
        .iter()
        .find(|buffer| buffer.name == "grid")
        .expect("grid buffer");
    let const_u32 = |name: &str| -> u32 {
        let line = shader
            .fragment_source
            .lines()
            .find(|line| line.starts_with(&format!("const uint {name}")))
            .unwrap_or_else(|| panic!("no const {name}"));
        let value = line.split('=').nth(1).expect("value");
        value
            .split('u')
            .next()
            .expect("number")
            .trim()
            .parse()
            .expect("u32")
    };
    assert_eq!(grid.stride, const_u32("CELL_STRIDE") * 4);
    assert_eq!(grid.count, const_u32("GRID_CELLS"));
    assert!(grid.count >= 480 * 270, "4K needs 480 by 270 cells");
}

/// A healthy frame reads above 0.005 on the CPU drivers and 0.007 on Metal. A grid
/// too short for the frame reads about 0.0001, and a frame whose dots lost their
/// slots to arcs reads 0.002 on a CPU driver, where workgroups run nearly in order.
#[test]
fn black_hole_particles_reach_the_bottom_of_a_1080p_frame() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let frame = black_hole_at_1080p(&ctx);
    let bottom = row_mean(&frame, 1920, 900, 1080);
    assert!(bottom > 0.003, "the bottom sixth is dark: {bottom}");
}

/// Zeroed biases match the zero-filled sentinel, so nothing asks for a reset. The
/// particles must still seed themselves.
#[test]
fn black_hole_seeds_when_its_inputs_match_the_empty_buffer() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let params = [
        ("equator_bias", 0.0),
        ("streak_length_bias", 0.0),
        ("lensing_steps", 16.0),
        ("streak_density", 0.0),
        ("hawking_glow", 0.0),
        ("shell_density", 1.0),
    ];
    let frame = luma(&render_shader(&ctx, BLACK_HOLE, (640, 360), &params, 12));
    let bottom = row_mean(&frame, 640, 300, 360);
    assert!(bottom > 0.005, "no particles were seeded: {bottom}");
}

/// The gas cloud fills most of the frame from the start, well outside the central
/// burst.
#[test]
fn big_bang_seeds_its_gas_cloud_on_the_first_frame() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let frame = luma(&render_shader(&ctx, BIG_BANG, (960, 540), &[], 30));
    let top_band = row_mean(&frame, 960, 0, 90);
    assert!(top_band > 0.015, "no gas outside the burst: {top_band}");
}
