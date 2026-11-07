//! `shaders/dark_matter.comp` builds a universe and collapses it into a web, at any
//! output size. These run the real shader for a short formation on a headless GPU,
//! so a broken solver shows as a flat frame. Skipped without a GPU adapter.
//!
//! Not built on Windows. Rendering does not depend on the platform, and on the CI
//! runner's software rasterizer (WARP) this file takes up to 47 minutes. Linux and
//! macOS run it, and `shader_pipeline_guard` still renders this shader on Windows.
#![cfg(not(target_os = "windows"))]

use varda::{renderer::context::GpuContext, testing::ShaderRun};

const DECK: &str = "shaders/dark_matter.comp";
/// Quick formation with a still camera, so a finished universe is a still frame.
const STILL: [(&str, f64); 3] = [("speed", 12.0), ("drift", 0.0), ("orbit", 0.0)];
const SMALL: (u32, u32) = (160, 90);
const FINISHED: u32 = 600;

fn formed_luma(ctx: &GpuContext, width: u32, height: u32, frames: u32) -> Vec<f32> {
    luma(&varda::testing::render_shader(
        ctx,
        DECK,
        (width, height),
        &STILL,
        frames,
    ))
}

fn luma(frame: &[[f32; 4]]) -> Vec<f32> {
    frame
        .iter()
        .map(|[r, g, b, _]| 0.2126 * r + 0.7152 * g + 0.0722 * b)
        .collect()
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}

fn rms_gap(a: &[f32], b: &[f32]) -> f32 {
    let sum: f32 = a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum();
    (sum / a.len() as f32).sqrt()
}

/// Mean brightness of each of the first `frames` frames.
fn brightness_curve(ctx: &GpuContext, params: &[(&str, f64)], frames: u32) -> Vec<f32> {
    let mut run = ShaderRun::new(ctx, DECK, SMALL, params);
    (0..frames)
        .map(|_| {
            run.advance(ctx, 1);
            mean(&luma(&run.frame(ctx)))
        })
        .collect()
}

/// A fade up out of black must rise a little every frame. Waiting for the simulation
/// shows as a flat run, then a jump.
fn assert_rises_evenly(curve: &[f32]) {
    let first = curve.iter().position(|v| *v > 0.0).expect("never lit");
    assert!(first < 90, "black for {first} frames");
    let peak = curve.iter().copied().fold(0.0, f32::max);
    let rising: Vec<f32> = curve[first..]
        .iter()
        .copied()
        .take_while(|v| *v < 0.6 * peak)
        .collect();
    // The fade itself starts with a few tiny steps.
    let steps: Vec<f32> = rising.windows(2).skip(3).map(|w| w[1] - w[0]).collect();
    let largest = steps.iter().copied().fold(0.0, f32::max);
    let flat = steps.iter().filter(|s| **s < 0.02 * largest).count();
    assert!(
        flat * 20 < steps.len(),
        "{flat} of {} frames stood still",
        steps.len()
    );
}

/// The share of light in the brightest hundredth of pixels.
fn top_share(luma: &[f32]) -> f32 {
    let mut sorted = luma.to_vec();
    sorted.sort_by(f32::total_cmp);
    let total: f32 = sorted.iter().sum();
    let top: f32 = sorted[sorted.len() * 99 / 100..].iter().sum();
    top / total.max(1e-6)
}

#[test]
fn gravity_gathers_light_into_a_web() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let early = top_share(&formed_luma(&ctx, 320, 180, 30));
    let formed = top_share(&formed_luma(&ctx, 320, 180, 900));
    assert!(
        formed > early,
        "a formed universe concentrates light: {early} then {formed}"
    );
    assert!(formed > 0.05, "a web has bright nodes: {formed}");
}

#[test]
fn a_small_output_still_simulates_every_particle() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let luma = formed_luma(&ctx, 64, 36, 900);
    assert!(luma.iter().any(|v| *v > 0.0), "the frame must not be black");
    assert!(
        top_share(&luma) > 0.02,
        "the web forms at low resolution too"
    );
}

#[test]
fn the_opening_fades_up_without_stalling() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    assert_rises_evenly(&brightness_curve(&ctx, &[], 420));
}

#[test]
fn the_top_speed_opening_fades_up_without_stalling() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    assert_rises_evenly(&brightness_curve(&ctx, &[("speed", 12.0)], 420));
}

/// Formation Speed sets the length of the film.
#[test]
fn top_speed_forms_a_universe_in_about_eight_seconds() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let mut run = ShaderRun::new(&ctx, DECK, SMALL, &STILL);
    run.advance(&ctx, 540);
    let at_nine_seconds = luma(&run.frame(&ctx));
    run.advance(&ctx, 300);
    let gap = rms_gap(&at_nine_seconds, &luma(&run.frame(&ctx)));
    assert!(gap < 1e-3, "still forming after nine seconds: {gap}");
}

/// The same seed makes the same universe, whichever universe came before it.
#[test]
fn changing_universe_builds_the_same_universe_as_starting_there() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let mut fresh = STILL.to_vec();
    fresh.push(("seed", 2.0));
    let want = luma(&varda::testing::render_shader(
        &ctx, DECK, SMALL, &fresh, FINISHED,
    ));

    let mut run = ShaderRun::new(&ctx, DECK, SMALL, &STILL);
    run.advance(&ctx, 240);
    run.set("seed", 2.0);
    run.advance(&ctx, FINISHED);
    let got = luma(&run.frame(&ctx));
    let gap = rms_gap(&want, &got);
    assert!(gap < 1e-3, "a changed universe differs: {gap}");
}

/// Simulation work is split across frames and follows speed, so a speed change part
/// way through a step must not skip or repeat any of it.
#[test]
fn changing_speed_mid_formation_keeps_the_universe() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let want = luma(&varda::testing::render_shader(
        &ctx, DECK, SMALL, &STILL, FINISHED,
    ));

    let mut run = ShaderRun::new(&ctx, DECK, SMALL, &STILL);
    for frame in 0..FINISHED {
        run.set("speed", 6.0 + 6.0 * f64::from(frame % 2));
        run.advance(&ctx, 1);
    }
    run.set("speed", 12.0);
    run.advance(&ctx, 300);
    let gap = rms_gap(&want, &luma(&run.frame(&ctx)));
    assert!(gap < 1e-3, "a speed change altered the universe: {gap}");
}

/// Point Detail trades drawing time for a cleaner picture, so it must show.
#[test]
fn point_detail_changes_the_picture() {
    let Some(ctx) = varda::testing::headless_gpu() else {
        return;
    };
    let at = |detail: f64| {
        let mut params = STILL.to_vec();
        params.push(("point_detail", detail));
        luma(&varda::testing::render_shader(
            &ctx,
            DECK,
            (640, 360),
            &params,
            FINISHED,
        ))
    };
    let gap = rms_gap(&at(0.0), &at(1.0));
    assert!(gap > 0.003, "Point Detail did nothing: {gap}");
}
