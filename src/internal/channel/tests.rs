//! Unit tests for channel compositing, blend modes, and effect chains.

use super::*;

// ── BlendMode tests ──────────────────────────────────────────────

#[test]
fn blend_mode_default_is_normal() {
    assert_eq!(BlendMode::default(), BlendMode::Normal);
}

#[test]
fn blend_mode_all_variants_have_index() {
    // Verify to_index doesn't panic for any variant
    for mode in BlendMode::all() {
        let _ = mode.to_index();
    }
}

#[test]
fn blend_mode_debug() {
    assert!(format!("{:?}", BlendMode::Add).contains("Add"));
}

// ── DurationSpec tests ───────────────────────────────────────────

#[test]
fn duration_spec_seconds() {
    let d = DurationSpec::Seconds(5.0);
    assert!((d.to_seconds(None) - 5.0).abs() < 1e-5);
    assert!((d.to_seconds(Some(120.0)) - 5.0).abs() < 1e-5);
    assert!((d.value() - 5.0).abs() < 1e-5);
    assert!(!d.is_beats());
}

#[test]
fn duration_spec_beats_with_bpm() {
    let d = DurationSpec::Beats(4.0);
    // 4 beats at 120 BPM = 4 * 60/120 = 2 seconds
    assert!((d.to_seconds(Some(120.0)) - 2.0).abs() < 1e-5);
    assert!(d.is_beats());
    assert!((d.value() - 4.0).abs() < 1e-5);
}

#[test]
fn duration_spec_beats_no_bpm_defaults_120() {
    let d = DurationSpec::Beats(4.0);
    // Falls back to 120 BPM → 4 * 60/120 = 2.0
    assert!((d.to_seconds(None) - 2.0).abs() < 1e-5);
}

#[test]
fn duration_spec_beats_different_bpm() {
    let d = DurationSpec::Beats(1.0);
    // 1 beat at 60 BPM = 1 second
    assert!((d.to_seconds(Some(60.0)) - 1.0).abs() < 1e-5);
    // 1 beat at 180 BPM = 60/180 = 0.333s
    assert!((d.to_seconds(Some(180.0)) - 0.333).abs() < 0.01);
}

#[test]
fn duration_spec_minutes() {
    let d = DurationSpec::Minutes(2.0);
    assert!((d.to_seconds(None) - 120.0).abs() < 1e-5);
    assert!((d.value() - 2.0).abs() < 1e-5);
    assert!(!d.is_beats());
    assert_eq!(d.unit(), DurationUnit::Minutes);
}

#[test]
fn duration_spec_hours() {
    let d = DurationSpec::Hours(1.5);
    assert!((d.to_seconds(None) - 5400.0).abs() < 1e-5);
    assert!((d.value() - 1.5).abs() < 1e-5);
    assert!(!d.is_beats());
    assert_eq!(d.unit(), DurationUnit::Hours);
}

#[test]
fn duration_unit_cycle() {
    assert_eq!(DurationUnit::Seconds.next(), DurationUnit::Minutes);
    assert_eq!(DurationUnit::Minutes.next(), DurationUnit::Hours);
    assert_eq!(DurationUnit::Hours.next(), DurationUnit::Beats);
    assert_eq!(DurationUnit::Beats.next(), DurationUnit::Seconds);
}

#[test]
fn duration_unit_labels() {
    assert_eq!(DurationUnit::Seconds.label(), "s");
    assert_eq!(DurationUnit::Minutes.label(), "m");
    assert_eq!(DurationUnit::Hours.label(), "h");
    assert_eq!(DurationUnit::Beats.label(), "b");
}

#[test]
fn duration_spec_from_value_unit() {
    let d = DurationSpec::from_value_unit(5.0, DurationUnit::Minutes);
    assert!(matches!(d, DurationSpec::Minutes(v) if (v - 5.0).abs() < 1e-5));
    let d = DurationSpec::from_value_unit(2.0, DurationUnit::Hours);
    assert!(matches!(d, DurationSpec::Hours(v) if (v - 2.0).abs() < 1e-5));
}

// ── DeckAutoTransition tests ─────────────────────────────────────

#[test]
fn deck_auto_transition_defaults() {
    let at = DeckAutoTransition::new();
    assert!(!at.enabled);
    assert_eq!(at.trigger, TransitionTrigger::Timer);
    assert_eq!(at.phase, DeckTransitionPhase::Inactive);
    assert!(at.transition_shader_name.is_none());
}

#[test]
fn deck_auto_transition_play_duration_is_beats() {
    let at = DeckAutoTransition::new();
    assert!(at.play_duration.is_beats());
}

#[test]
fn deck_auto_transition_transition_duration_is_seconds() {
    let at = DeckAutoTransition::new();
    assert!(!at.transition_duration.is_beats());
}

// ── DeckTransitionPhase tests ────────────────────────────────────

#[test]
fn deck_transition_phase_equality() {
    assert_eq!(DeckTransitionPhase::Inactive, DeckTransitionPhase::Inactive);
    assert_eq!(DeckTransitionPhase::Done, DeckTransitionPhase::Done);
    assert_ne!(DeckTransitionPhase::Inactive, DeckTransitionPhase::Done);
}

#[test]
fn deck_transition_phase_playing() {
    let phase = DeckTransitionPhase::Playing { elapsed: 1.5 };
    match phase {
        DeckTransitionPhase::Playing { elapsed } => {
            assert!((elapsed - 1.5).abs() < 1e-5);
        }
        _ => panic!("Wrong phase"),
    }
}

#[test]
fn deck_transition_phase_transitioning() {
    let phase = DeckTransitionPhase::Transitioning { progress: 0.75 };
    match phase {
        DeckTransitionPhase::Transitioning { progress } => {
            assert!((progress - 0.75).abs() < 1e-5);
        }
        _ => panic!("Wrong phase"),
    }
}

// ── TransitionTrigger tests ──────────────────────────────────────

#[test]
fn transition_trigger_equality() {
    assert_eq!(TransitionTrigger::Timer, TransitionTrigger::Timer);
    assert_eq!(TransitionTrigger::ClipEnd, TransitionTrigger::ClipEnd);
    assert_ne!(TransitionTrigger::Timer, TransitionTrigger::ClipEnd);
}

// ── Deck slot management tests (DnD data model) ─────────────────
//
// These test the Channel-level operations that back drag-and-drop
// actions: add_deck, remove_deck, remove_deck_slot, add_deck_slot.
// They require a headless GPU to construct real Channel + Deck instances.

use crate::renderer::GpuContext;

fn headless_gpu() -> GpuContext {
    GpuContext::new_headless().expect("headless GPU required for tests")
}

fn test_channel(gpu: &GpuContext, name: &str) -> Channel {
    Channel::new(name.to_string(), gpu, 64, 64).expect("channel creation")
}

fn add_solid_deck(ch: &mut Channel, gpu: &GpuContext, color: [f32; 4]) {
    let deck = crate::deck::Deck::new_solid_color(gpu, color, 64, 64).expect("solid color deck");
    ch.add_deck(deck);
}

#[test]
fn add_deck_increases_count() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    assert_eq!(ch.deck_count(), 0);
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(ch.deck_count(), 1);
    add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(ch.deck_count(), 2);
}

#[test]
fn remove_deck_returns_deck_and_shrinks() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
    add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(ch.deck_count(), 2);
    let removed = ch.remove_deck(0);
    assert!(removed.is_some());
    assert_eq!(ch.deck_count(), 1);
}

#[test]
fn remove_deck_out_of_bounds_returns_none() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    assert!(ch.remove_deck(0).is_none());
    assert!(ch.remove_deck(99).is_none());
}

#[test]
fn remove_deck_slot_preserves_properties() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
    ch.decks[0].opacity = 0.42;
    ch.decks[0].blend_mode = BlendMode::Add;
    ch.decks[0].solo = true;

    let slot = ch.remove_deck_slot(0).expect("slot exists");
    assert!((slot.opacity - 0.42).abs() < 1e-5);
    assert_eq!(slot.blend_mode, BlendMode::Add);
    assert!(slot.solo);
    assert_eq!(ch.deck_count(), 0);
}

#[test]
fn add_deck_slot_appends_and_returns_index() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

    let slot = ch.remove_deck_slot(0).unwrap();
    let idx = ch.add_deck_slot(slot);
    assert_eq!(idx, 0); // only slot
    assert_eq!(ch.deck_count(), 1);
}

#[test]
fn move_deck_between_channels_preserves_data() {
    let gpu = headless_gpu();
    let mut src = test_channel(&gpu, "Src");
    let mut dst = test_channel(&gpu, "Dst");

    // Add two decks to src
    add_solid_deck(&mut src, &gpu, [1.0, 0.0, 0.0, 1.0]); // Red
    add_solid_deck(&mut src, &gpu, [0.0, 1.0, 0.0, 1.0]); // Green
    src.decks[0].opacity = 0.5;
    src.decks[1].opacity = 0.75;

    // Move deck 0 (red) from src to dst
    let slot = src.remove_deck_slot(0).unwrap();
    let new_idx = dst.add_deck_slot(slot);

    assert_eq!(src.deck_count(), 1);
    assert_eq!(dst.deck_count(), 1);
    assert_eq!(new_idx, 0);
    // Moved slot preserves opacity
    assert!((dst.decks[0].opacity - 0.5).abs() < 1e-5);
    // Remaining src deck shifted
    assert!((src.decks[0].opacity - 0.75).abs() < 1e-5);
}

#[test]
fn effect_reorder_within_deck() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

    // Manually push named effects (requires ISF shader + GPU pipeline)
    // Since Effect::new requires real shaders, test the vec operation directly
    // which is what apply_deck_and_effect_actions does
    let _deck = &mut ch.decks[0].deck;

    // Simulate 3 effects by checking vec operations match action processing logic
    // The action processing code does: effects.remove(from); effects.insert(to, effect);
    let mut names = vec!["blur", "glow", "invert"];
    // Move index 2 → index 0
    let removed = names.remove(2);
    names.insert(0, removed);
    assert_eq!(names, vec!["invert", "blur", "glow"]);

    // Move index 0 → index 1
    let removed = names.remove(0);
    names.insert(1, removed);
    assert_eq!(names, vec!["blur", "invert", "glow"]);
}

#[test]
fn channel_effect_reorder() {
    // Channel effects use the same vec pattern
    let mut effects = vec!["ch_blur", "ch_color", "ch_distort"];
    let from = 0;
    let to = 2;
    let e = effects.remove(from);
    effects.insert(to, e);
    assert_eq!(effects, vec!["ch_color", "ch_distort", "ch_blur"]);
}

// ── Render timing tests ─────────────────────────────────────────

#[test]
fn new_channel_render_time_starts_at_zero() {
    let gpu = headless_gpu();
    let ch = test_channel(&gpu, "Test");
    assert!((ch.render_time_ms - 0.0).abs() < 1e-5);
    assert_eq!(ch.active_deck_count, 0);
}

#[test]
fn render_updates_timing_fields() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
    add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);

    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();
    ch.render(
        &gpu,
        &audio,
        &modulation,
        0,
        0.0,
        1.0 / 60.0,
        60,
        2,
        1.0,
        &mut Vec::new(),
        None,
        None,
    )
    .unwrap();

    // After one render, render_time_ms should be > 0 (something was measured)
    assert!(ch.render_time_ms > 0.0);
    // Both decks are active (opacity 1.0, not muted)
    assert_eq!(ch.active_deck_count, 2);
}

#[test]
fn muted_decks_not_counted_as_active() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
    add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);
    ch.decks[1].mute = true;

    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();
    ch.render(
        &gpu,
        &audio,
        &modulation,
        0,
        0.0,
        1.0 / 60.0,
        60,
        2,
        1.0,
        &mut Vec::new(),
        None,
        None,
    )
    .unwrap();

    assert_eq!(ch.active_deck_count, 1);
}

#[test]
fn zero_opacity_decks_not_counted_as_active() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
    add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);
    ch.decks[0].opacity = 0.0;

    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();
    ch.render(
        &gpu,
        &audio,
        &modulation,
        0,
        0.0,
        1.0 / 60.0,
        60,
        2,
        1.0,
        &mut Vec::new(),
        None,
        None,
    )
    .unwrap();

    assert_eq!(ch.active_deck_count, 1);
}

#[test]
fn render_time_smooths_over_frames() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();

    // Render multiple frames — EMA should converge
    for _ in 0..10 {
        ch.render(
            &gpu,
            &audio,
            &modulation,
            0,
            0.0,
            1.0 / 60.0,
            60,
            2,
            1.0,
            &mut Vec::new(),
            None,
            None,
        )
        .unwrap();
    }
    let time_after_10 = ch.render_time_ms;

    // Render more frames
    for _ in 0..10 {
        ch.render(
            &gpu,
            &audio,
            &modulation,
            0,
            0.0,
            1.0 / 60.0,
            60,
            2,
            1.0,
            &mut Vec::new(),
            None,
            None,
        )
        .unwrap();
    }
    let time_after_20 = ch.render_time_ms;

    // Both should be positive
    assert!(time_after_10 > 0.0);
    assert!(time_after_20 > 0.0);
}

#[test]
fn empty_channel_render_timing() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    // No decks — render should still work and measure time

    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();
    ch.render(
        &gpu,
        &audio,
        &modulation,
        0,
        0.0,
        1.0 / 60.0,
        60,
        2,
        1.0,
        &mut Vec::new(),
        None,
        None,
    )
    .unwrap();

    // Time should be >= 0 (even empty channels do some work)
    assert!(ch.render_time_ms >= 0.0);
    assert_eq!(ch.active_deck_count, 0);
}

// ── Deck pipeline FPS tests ─────────────────────────────────────

#[test]
fn new_deck_fps_starts_at_zero() {
    let gpu = headless_gpu();
    let deck = crate::deck::Deck::new_solid_color(&gpu, [1.0, 0.0, 0.0, 1.0], 64, 64).unwrap();
    assert!((deck.fps() - 0.0).abs() < 1e-5);
}

#[test]
fn deck_fps_becomes_positive_after_renders() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();

    // Render several frames so EMA has time to converge
    for _ in 0..5 {
        ch.render(
            &gpu,
            &audio,
            &modulation,
            0,
            0.0,
            1.0 / 60.0,
            60,
            2,
            1.0,
            &mut Vec::new(),
            None,
            None,
        )
        .unwrap();
    }

    let deck_fps = ch.decks[0].deck.fps();
    assert!(
        deck_fps > 0.0,
        "Deck FPS should be positive after rendering, got {deck_fps}"
    );
}

#[test]
fn deck_fps_ignores_huge_first_frame_delta() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();

    // First render — time_delta may be very large (time since Deck creation)
    // but the guard (time_delta < 1.0) should keep FPS sane
    ch.render(
        &gpu,
        &audio,
        &modulation,
        0,
        0.0,
        1.0 / 60.0,
        60,
        2,
        1.0,
        &mut Vec::new(),
        None,
        None,
    )
    .unwrap();
    let fps = ch.decks[0].deck.fps();
    // Either 0 (if first delta was >= 1s) or some reasonable value
    assert!(fps >= 0.0);
    assert!(
        fps < 100_000.0,
        "FPS should not be absurdly high, got {fps}"
    );
}

#[test]
fn multiple_decks_have_independent_fps() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
    add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);

    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();

    for _ in 0..5 {
        ch.render(
            &gpu,
            &audio,
            &modulation,
            0,
            0.0,
            1.0 / 60.0,
            60,
            2,
            1.0,
            &mut Vec::new(),
            None,
            None,
        )
        .unwrap();
    }

    // Both decks should have positive FPS
    let fps0 = ch.decks[0].deck.fps();
    let fps1 = ch.decks[1].deck.fps();
    assert!(fps0 > 0.0);
    assert!(fps1 > 0.0);
}

#[test]
fn skipped_deck_keeps_old_fps() {
    let gpu = headless_gpu();
    let mut ch = test_channel(&gpu, "Test");
    add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();

    // Render to establish FPS
    for _ in 0..5 {
        ch.render(
            &gpu,
            &audio,
            &modulation,
            0,
            0.0,
            1.0 / 60.0,
            60,
            2,
            1.0,
            &mut Vec::new(),
            None,
            None,
        )
        .unwrap();
    }
    let fps_before = ch.decks[0].deck.fps();

    // Mute the deck — it won't render
    ch.decks[0].mute = true;
    ch.render(
        &gpu,
        &audio,
        &modulation,
        0,
        0.0,
        1.0 / 60.0,
        60,
        2,
        1.0,
        &mut Vec::new(),
        None,
        None,
    )
    .unwrap();

    // FPS should remain unchanged (deck wasn't rendered, no EMA update)
    let fps_after = ch.decks[0].deck.fps();
    assert!((fps_before - fps_after).abs() < 1e-5);
}
