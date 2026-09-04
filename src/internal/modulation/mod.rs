//! Parameter modulation engine for automating shader parameters
//!
//! Supports LFOs, envelopes, and audio-reactive modulation sources.

mod audio;
mod engine;
mod envelope;
mod sources;

pub use audio::{AnalyzerValues, AudioSourceValues, AudioValues};
pub use engine::{ModulationEngine, ResolvedModulation};
pub use envelope::{
    Breakpoint, CurveKind, active_between as envelope_active_between, evaluate as evaluate_envelope,
};
pub use sources::ModulationSource;

use crate::deck::generate_short_uuid;
use crate::timebase::Timebase;

use serde::{Deserialize, Serialize};

/// LFO waveform types
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum LFOWaveform {
    #[default]
    Sine,
    Square,
    Triangle,
    Sawtooth,
    Random,
}

/// Depth given to a modulation assignment created from a parameter's dropdown.
///
/// Full range. The contribution is scaled by the parameter's range before it is
/// applied, so 1.0 means "this source can traverse the whole slider" — which is
/// the entire point of a sweep mode like [`AudioReactMode::Increase`]. Anything
/// less silently caps the sweep partway: at 0.5 an Increase source climbs to the
/// midpoint and resets, never reaching the top of the fader.
///
/// Performers dial depth back on the source (LFO `amplitude`, audio `gain`), not
/// here — see /spec/modulation.md § Range-Scaled Modulation.
pub const DEFAULT_ASSIGNMENT_AMOUNT: f32 = 1.0;

/// How audio energy drives the modulation value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum AudioReactMode {
    /// Direct: output = audio energy (standard envelope follower)
    #[default]
    Direct,
    /// Increase: audio energy sweeps the value upward (accumulates)
    Increase,
    /// Decrease: audio energy sweeps the value downward (accumulates)
    Decrease,
}

/// Audio frequency band presets (convenience for UI quick-select).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum AudioBandPreset {
    #[default]
    Low, // 20–250 Hz
    Mid,  // 250–2000 Hz
    High, // 2000–20000 Hz
    Full, // 20–20000 Hz (overall level)
}

impl AudioBandPreset {
    /// Get the frequency range for this preset.
    pub fn freq_range(self) -> (f32, f32) {
        match self {
            AudioBandPreset::Low => (20.0, 250.0),
            AudioBandPreset::Mid => (250.0, 2000.0),
            AudioBandPreset::High => (2000.0, 20000.0),
            AudioBandPreset::Full => (20.0, 20000.0),
        }
    }
}

/// ADSR envelope stage
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum ADSRStage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Step sequencer interpolation mode
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum StepInterpolation {
    /// Hard steps, no interpolation
    #[default]
    None,
    /// Linear interpolation between steps
    Linear,
    /// Smooth cubic interpolation
    Smooth,
}

/// How an assignment's contribution combines with the parameter's base value.
/// See /spec/automation.md § Absolute vs Additive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssignmentMode {
    /// Contribution is summed onto the base value, range-scaled. Existing behaviour.
    #[default]
    Additive,
    /// Source output replaces the base value before additive sources are summed.
    Absolute,
}

/// Modulation assignment linking a source to a parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamModulation {
    /// UUID of the modulation source
    pub source_id: String,
    /// Modulation depth/amount (-1.0 to 1.0, negative inverts)
    pub amount: f32,
    /// For color params: which component (0=R, 1=G, 2=B, 3=A), None for scalar
    pub component: Option<usize>,
    /// Defaults to `Additive`, which is what every assignment did before
    /// automation existed, so older scenes deserialize unchanged.
    #[serde(default)]
    pub mode: AssignmentMode,
}

/// A modulation source paired with a stable UUID identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulationSourceEntry {
    pub uuid: String,
    pub source: ModulationSource,
    /// Which notion of time this source follows. Defaults to `FreeRun`, which
    /// is the behaviour every scene had before timebases existed, so older
    /// scenes deserialize unchanged. See /spec/timebase.md.
    #[serde(default)]
    pub timebase: Timebase,
}

impl ModulationSourceEntry {
    pub fn new(source: ModulationSource) -> Self {
        Self {
            uuid: generate_short_uuid(),
            source,
            timebase: Timebase::default(),
        }
    }

    pub fn with_uuid(uuid: String, source: ModulationSource) -> Self {
        Self {
            uuid,
            source,
            timebase: Timebase::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_audio() -> AudioValues {
        AudioValues::default()
    }

    fn empty_analyzers() -> AnalyzerValues {
        AnalyzerValues::default()
    }

    // ── Timebase selection (/spec/timebase.md) ───────────────────────

    use crate::timebase::{TimeContext, TimebaseInput, TimebaseResolver, TimebaseSet};

    fn ctx(time: f32, dt: f32, running: bool) -> TimeContext {
        TimeContext {
            time,
            dt,
            running,
            discontinuity: false,
        }
    }

    /// Every timebase deliberately disagrees, so a test can tell which one a
    /// source actually read.
    fn split_timebases(seconds: f32, beats: f32) -> TimebaseSet {
        split_timebases_with(seconds, beats, 0.0)
    }

    fn split_timebases_with(seconds: f32, beats: f32, show: f32) -> TimebaseSet {
        TimebaseSet::new(
            ctx(seconds, 0.016, true),
            ctx(beats, 0.1, true),
            ctx(show, 0.016, true),
        )
    }

    #[test]
    fn source_defaults_to_free_run() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        assert_eq!(engine.timebase(&uuid), Some(Timebase::FreeRun));
    }

    #[test]
    fn beat_locked_lfo_reads_beats_not_seconds() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.set_timebase(&uuid, Timebase::Beat);
        engine.assign("p", &uuid, 1.0, None);

        // A sine LFO at frequency 1.0 peaks a quarter of the way through its
        // cycle. On the beat timebase that is beat 0.25, whatever the wall
        // clock says.
        engine.update(
            &split_timebases(0.0, 0.25),
            &empty_audio(),
            &empty_analyzers(),
        );
        let at_quarter_beat = engine.get_modulation("p");

        engine.update(
            &split_timebases(0.25, 0.0),
            &empty_audio(),
            &empty_analyzers(),
        );
        let at_quarter_second = engine.get_modulation("p");

        assert!(
            (at_quarter_beat - 1.0).abs() < 1e-3,
            "beat-locked LFO should peak at beat 0.25, got {at_quarter_beat}"
        );
        assert!(
            (at_quarter_second - 0.5).abs() < 1e-3,
            "wall-clock time must not move a beat-locked LFO, got {at_quarter_second}"
        );
    }

    #[test]
    fn free_run_lfo_ignores_the_beat_clock() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.assign("p", &uuid, 1.0, None);

        engine.update(
            &split_timebases(0.25, 0.0),
            &empty_audio(),
            &empty_analyzers(),
        );
        assert!((engine.get_modulation("p") - 1.0).abs() < 1e-3);
    }

    /// The point of measuring in beats: the same LFO settings track the tempo,
    /// so a performer never re-dials frequency after a BPM change.
    #[test]
    fn beat_locked_lfo_retunes_with_bpm() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.set_timebase(&uuid, Timebase::Beat);
        engine.assign("p", &uuid, 1.0, None);

        // Half a second of wall time is one beat at 120 BPM and half a beat at
        // 60 BPM. The LFO must be at a different point in its cycle for each,
        // with no change to its frequency.
        let mut sample = |beats: f32| {
            engine.update(
                &split_timebases(0.5, beats),
                &empty_audio(),
                &empty_analyzers(),
            );
            engine.get_modulation("p")
        };

        let at_120 = sample(1.0);
        let at_60 = sample(0.5);

        assert!(
            (at_120 - 0.5).abs() < 1e-3,
            "one full cycle should land back at the start, got {at_120}"
        );
        assert!(
            (at_60 - 0.5).abs() < 1e-3,
            "half a cycle sits at the same value but travelling the other way"
        );

        // A quarter beat apart is unambiguous: the tempo genuinely changes where
        // the LFO is.
        assert!((sample(0.25) - 1.0).abs() < 1e-3);
        assert!((sample(0.75) - 0.0).abs() < 1e-3);
    }

    #[test]
    fn beat_locked_source_freezes_when_the_clock_is_gone() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.set_timebase(&uuid, Timebase::Beat);
        engine.assign("p", &uuid, 1.0, None);

        let mut resolver = TimebaseResolver::new();
        let mut frame = |secs: f32, beats: Option<f64>| {
            let set = resolver.resolve(TimebaseInput {
                free_run_time: secs,
                beat_time: beats,
                transport: None,
            });
            engine.update(&set, &empty_audio(), &empty_analyzers());
            engine.get_modulation("p")
        };

        frame(0.0, Some(0.0));
        let held = frame(0.1, Some(0.25));

        // Clock drops out. Wall time keeps advancing; the source must not.
        assert!((frame(0.2, None) - held).abs() < 1e-6);
        assert!((frame(5.0, None) - held).abs() < 1e-6);
        assert!(
            (held - 1.0).abs() < 1e-3,
            "should have frozen at the peak it had reached"
        );
    }

    /// The point of a transport-locked LFO: the same show position gives the
    /// same value, whenever it is played and however it was reached.
    #[test]
    fn transport_locked_source_is_deterministic_from_position() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.set_timebase(&uuid, Timebase::Transport);
        engine.assign("p", &uuid, 1.0, None);

        let sample_at = |engine: &mut ModulationEngine, wall: f32, show: f32| {
            engine.update(
                &split_timebases_with(wall, 0.0, show),
                &empty_audio(),
                &empty_analyzers(),
            );
            engine.get_modulation("p")
        };

        let first = sample_at(&mut engine, 0.0, 12.25);
        // Wall clock has moved on; the show position has not.
        let replay = sample_at(&mut engine, 90.0, 12.25);
        assert!((first - replay).abs() < 1e-6);
    }

    #[test]
    fn transport_locked_source_freezes_before_the_show_runs() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.set_timebase(&uuid, Timebase::Transport);
        engine.assign("p", &uuid, 1.0, None);

        let mut resolver = TimebaseResolver::new();
        let mut frame = |secs: f32| {
            let set = resolver.resolve(TimebaseInput {
                free_run_time: secs,
                beat_time: None,
                transport: None,
            });
            engine.update(&set, &empty_audio(), &empty_analyzers());
            engine.get_modulation("p")
        };

        let held = frame(0.0);
        assert!((frame(3.0) - held).abs() < 1e-6);
        assert!((frame(30.0) - held).abs() < 1e-6);
    }

    // ── Automation envelopes (/spec/automation.md) ───────────────────

    /// Build an engine holding one transport-locked envelope assigned to `param`
    /// in the given mode, and a closure that samples the resolved contribution
    /// at a show position.
    fn envelope_engine(
        breakpoints: Vec<Breakpoint>,
        mode: AssignmentMode,
    ) -> (ModulationEngine, String) {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::envelope(breakpoints));
        engine.set_timebase(&uuid, Timebase::Transport);
        engine.assign_with_mode("p", &uuid, 1.0, None, mode);
        (engine, uuid)
    }

    fn sample_at(engine: &mut ModulationEngine, show: f32) -> super::engine::ResolvedModulation {
        engine.update(
            &split_timebases_with(0.0, 0.0, show),
            &empty_audio(),
            &empty_analyzers(),
        );
        engine.resolve("p", None)
    }

    #[test]
    fn an_absolute_envelope_replaces_the_base_and_an_additive_one_does_not() {
        let curve = vec![Breakpoint::new(0.0, 0.25), Breakpoint::new(10.0, 0.75)];

        let (mut absolute, _) = envelope_engine(curve.clone(), AssignmentMode::Absolute);
        let resolved = sample_at(&mut absolute, 5.0);
        assert!((resolved.absolute.unwrap() - 0.5).abs() < 1e-5);
        assert!((resolved.additive - 0.0).abs() < 1e-6);

        let (mut additive, _) = envelope_engine(curve, AssignmentMode::Additive);
        let resolved = sample_at(&mut additive, 5.0);
        assert!(resolved.absolute.is_none());
        assert!((resolved.additive - 0.5).abs() < 1e-5);
    }

    /// The combination the two modes exist to produce: a scheduled shape that
    /// still breathes. Absolute sets the base, additive rides on top of it.
    #[test]
    fn absolute_and_additive_sources_compose_on_one_parameter() {
        let (mut engine, _) = envelope_engine(
            vec![Breakpoint::new(0.0, 0.4), Breakpoint::new(10.0, 0.4)],
            AssignmentMode::Absolute,
        );
        let lfo = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.assign("p", &lfo, 1.0, None);

        let resolved = sample_at(&mut engine, 5.0);
        assert!((resolved.absolute.unwrap() - 0.4).abs() < 1e-5);
        assert!(
            resolved.additive.abs() > 1e-6,
            "the LFO must still contribute alongside the curve"
        );
    }

    /// A lane exists before any point is drawn on it, and overriding the base
    /// with zero in the meantime would black the parameter out.
    #[test]
    fn an_empty_envelope_contributes_nothing_in_absolute_mode() {
        let (mut engine, _) = envelope_engine(vec![], AssignmentMode::Absolute);
        let resolved = sample_at(&mut engine, 5.0);
        assert!(resolved.absolute.is_none());
        assert!((resolved.additive - 0.0).abs() < 1e-6);
    }

    #[test]
    fn stacked_absolute_sources_resolve_to_the_last_assigned() {
        let (mut engine, _) = envelope_engine(
            vec![Breakpoint::new(0.0, 0.2), Breakpoint::new(10.0, 0.2)],
            AssignmentMode::Absolute,
        );
        let second = engine.add_source(ModulationSource::envelope(vec![
            Breakpoint::new(0.0, 0.8),
            Breakpoint::new(10.0, 0.8),
        ]));
        engine.set_timebase(&second, Timebase::Transport);
        engine.assign_with_mode("p", &second, 1.0, None, AssignmentMode::Absolute);

        let resolved = sample_at(&mut engine, 5.0);
        assert!((resolved.absolute.unwrap() - 0.8).abs() < 1e-5);
    }

    /// The reason automation is a modulation source rather than a parallel
    /// system: it inherits jump-safety from being a pure function of position.
    #[test]
    fn locating_to_a_position_matches_playing_to_it() {
        let curve = vec![
            Breakpoint::new(0.0, 0.0),
            Breakpoint::new(4.0, 1.0).with_curve(CurveKind::Smooth),
            Breakpoint::new(9.0, 0.3),
        ];

        let (mut played, _) = envelope_engine(curve.clone(), AssignmentMode::Absolute);
        let mut t = 0.0;
        while t < 6.0 {
            sample_at(&mut played, t);
            t += 0.016;
        }
        let after_playing = sample_at(&mut played, 6.0).absolute.unwrap();

        let (mut located, _) = envelope_engine(curve, AssignmentMode::Absolute);
        let after_locate = sample_at(&mut located, 6.0).absolute.unwrap();

        assert!((after_playing - after_locate).abs() < 1e-5);
    }

    #[test]
    fn a_transport_locked_envelope_freezes_before_the_show_runs() {
        let (mut engine, _) = envelope_engine(
            vec![Breakpoint::new(0.0, 0.0), Breakpoint::new(10.0, 1.0)],
            AssignmentMode::Absolute,
        );

        let mut resolver = TimebaseResolver::new();
        let mut frame = |secs: f32| {
            let set = resolver.resolve(TimebaseInput {
                free_run_time: secs,
                beat_time: None,
                transport: None,
            });
            engine.update(&set, &empty_audio(), &empty_analyzers());
            engine.resolve("p", None).absolute.unwrap()
        };

        let held = frame(0.0);
        assert!((frame(5.0) - held).abs() < 1e-6);
        assert!((frame(60.0) - held).abs() < 1e-6);
    }

    // ── Live override and re-arm (/spec/arrangement.md § Live override) ──

    /// A flat curve, so any change in the resolved value comes from the
    /// override rather than from the envelope moving underneath the test.
    fn flat_envelope_engine(value: f32) -> ModulationEngine {
        let (engine, _) = envelope_engine(
            vec![Breakpoint::new(0.0, value), Breakpoint::new(1000.0, value)],
            AssignmentMode::Absolute,
        );
        engine
    }

    /// Frames needed to outlast a ramp, given `sample_at` advances 16 ms.
    fn frames_for(seconds: f64) -> usize {
        (seconds / 0.016).ceil() as usize + 2
    }

    #[test]
    fn a_live_touch_suspends_the_envelope_it_lands_on() {
        let mut engine = flat_envelope_engine(0.8);
        assert!((sample_at(&mut engine, 5.0).absolute.unwrap() - 0.8).abs() < 1e-5);

        engine.override_param("p", 0.2);

        assert!(
            sample_at(&mut engine, 5.0).absolute.is_none(),
            "the performer's hand wins, immediately and without confirmation"
        );
        assert!(engine.is_overridden("p"));
        assert_eq!(engine.overridden_params().collect::<Vec<_>>(), vec!["p"]);
    }

    /// An override suspends *arrangement* control, not the parameter. An LFO
    /// the performer never took is still theirs to run.
    #[test]
    fn an_override_leaves_live_modulation_running() {
        let mut engine = flat_envelope_engine(0.8);
        let lfo = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.assign("p", &lfo, 1.0, None);

        engine.override_param("p", 0.2);
        let resolved = sample_at(&mut engine, 5.0);

        assert!(resolved.absolute.is_none(), "the curve is suspended");
        assert!(
            resolved.additive.abs() > 1e-6,
            "the LFO must keep contributing"
        );
    }

    /// Authority is per lane. Grabbing one fader must not stop the rest of the
    /// show, which is the whole reason overrides are scoped rather than global.
    #[test]
    fn an_override_is_scoped_to_the_parameter_that_was_touched() {
        let mut engine = flat_envelope_engine(0.8);
        let other = engine.add_source(ModulationSource::envelope(vec![
            Breakpoint::new(0.0, 0.3),
            Breakpoint::new(1000.0, 0.3),
        ]));
        engine.set_timebase(&other, Timebase::Transport);
        engine.assign_with_mode("q", &other, 1.0, None, AssignmentMode::Absolute);

        engine.override_param("p", 0.1);
        sample_at(&mut engine, 5.0);

        assert!(engine.resolve("p", None).absolute.is_none());
        assert!(
            (engine.resolve("q", None).absolute.unwrap() - 0.3).abs() < 1e-5,
            "an untouched lane keeps following the show"
        );
    }

    /// A jump to the automated value is the correct state and the wrong look,
    /// and this happens live in front of an audience.
    #[test]
    fn re_arm_ramps_back_rather_than_snapping() {
        let mut engine = flat_envelope_engine(0.8);
        engine.override_param("p", 0.0);
        sample_at(&mut engine, 5.0);

        engine.rearm_param("p", 0.5);

        let first = sample_at(&mut engine, 5.0).absolute.unwrap();
        assert!(
            first < 0.1,
            "the first frame after re-arm jumped to {first} instead of easing"
        );

        let mut previous = first;
        for _ in 0..8 {
            let next = sample_at(&mut engine, 5.0).absolute.unwrap();
            assert!(next >= previous, "the ramp went backwards");
            previous = next;
        }
        assert!(previous < 0.8, "the ramp finished far too early");
    }

    #[test]
    fn a_completed_re_arm_retires_the_override() {
        let mut engine = flat_envelope_engine(0.8);
        engine.override_param("p", 0.0);
        engine.rearm_param("p", 0.2);

        for _ in 0..frames_for(0.2) {
            sample_at(&mut engine, 5.0);
        }

        assert_eq!(engine.override_count(), 0, "the record must not leak");
        assert!((sample_at(&mut engine, 5.0).absolute.unwrap() - 0.8).abs() < 1e-5);
    }

    #[test]
    fn re_arming_with_no_duration_hands_over_immediately() {
        let mut engine = flat_envelope_engine(0.8);
        engine.override_param("p", 0.0);
        engine.rearm_param("p", 0.0);

        assert_eq!(engine.override_count(), 0);
        assert!((sample_at(&mut engine, 5.0).absolute.unwrap() - 0.8).abs() < 1e-5);
    }

    /// The performer has spoken more recently than the re-arm did.
    #[test]
    fn re_taking_a_parameter_mid_ramp_cancels_the_ramp() {
        let mut engine = flat_envelope_engine(0.8);
        engine.override_param("p", 0.0);
        engine.rearm_param("p", 10.0);
        for _ in 0..4 {
            sample_at(&mut engine, 5.0);
        }
        assert!(!engine.is_overridden("p"), "mid-ramp is not held");

        engine.override_param("p", 0.6);

        assert!(engine.is_overridden("p"));
        assert!(
            sample_at(&mut engine, 5.0).absolute.is_none(),
            "the curve must be suspended again"
        );
    }

    #[test]
    fn re_arm_all_releases_every_held_parameter() {
        let mut engine = flat_envelope_engine(0.8);
        let other = engine.add_source(ModulationSource::envelope(vec![
            Breakpoint::new(0.0, 0.3),
            Breakpoint::new(1000.0, 0.3),
        ]));
        engine.set_timebase(&other, Timebase::Transport);
        engine.assign_with_mode("q", &other, 1.0, None, AssignmentMode::Absolute);

        engine.override_param("p", 0.0);
        engine.override_param("q", 0.0);
        assert_eq!(engine.override_count(), 2);

        engine.rearm_all(0.2);
        assert_eq!(engine.override_count(), 0, "none are held any more");

        for _ in 0..frames_for(0.2) {
            sample_at(&mut engine, 5.0);
        }
        assert!((engine.resolve("p", None).absolute.unwrap() - 0.8).abs() < 1e-5);
        assert!((engine.resolve("q", None).absolute.unwrap() - 0.3).abs() < 1e-5);
    }

    /// Overrides are session state. A saved override would be an invisible trap
    /// that silently breaks the show the next time the file is opened.
    #[test]
    fn clearing_overrides_restores_full_authority() {
        let mut engine = flat_envelope_engine(0.8);
        engine.override_param("p", 0.1);
        sample_at(&mut engine, 5.0);

        engine.clear_overrides();

        assert_eq!(engine.override_count(), 0);
        assert!((sample_at(&mut engine, 5.0).absolute.unwrap() - 0.8).abs() < 1e-5);
    }

    #[test]
    fn overrides_do_not_survive_serialization() {
        let mut engine = flat_envelope_engine(0.8);
        engine.override_param("p", 0.1);

        let json = serde_json::to_string(&engine).expect("serialize");
        let restored: ModulationEngine = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.override_count(), 0);
        assert!(!json.contains("override"));
    }

    /// An additive automation lane has no held value to ease out of, so it
    /// fades its contribution in instead of appearing at full strength.
    #[test]
    fn an_additive_envelope_fades_in_on_re_arm() {
        let (mut engine, _) = envelope_engine(
            vec![Breakpoint::new(0.0, 0.9), Breakpoint::new(1000.0, 0.9)],
            AssignmentMode::Additive,
        );
        let full = sample_at(&mut engine, 5.0).additive;

        engine.override_param("p", 0.0);
        assert!((sample_at(&mut engine, 5.0).additive).abs() < 1e-6);

        engine.rearm_param("p", 0.5);
        let first = sample_at(&mut engine, 5.0).additive;
        assert!(
            first > 0.0 && first < full,
            "expected a partial contribution"
        );
    }

    #[test]
    fn envelopes_are_counted_as_timebase_followers() {
        let (engine, _) =
            envelope_engine(vec![Breakpoint::new(0.0, 0.5)], AssignmentMode::Absolute);
        assert_eq!(engine.followers_of(Timebase::Transport), 1);
    }

    /// The readouts dim when nothing reads them, so the count must reflect what
    /// would actually stop moving. Signal-driven sources carry a timebase field
    /// but ignore it, so counting them would keep a readout lit for no reason.
    #[test]
    fn follower_count_ignores_sources_that_do_not_read_their_timebase() {
        let mut engine = ModulationEngine::new();

        let lfo = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.set_timebase(&lfo, Timebase::Beat);
        let steps = engine.add_source(ModulationSource::step_sequencer(8, 1.0));
        engine.set_timebase(&steps, Timebase::Beat);

        let adsr = engine.add_source(ModulationSource::adsr(0.1, 0.1, 0.5, 0.1));
        engine.set_timebase(&adsr, Timebase::Beat);

        assert_eq!(
            engine.followers_of(Timebase::Beat),
            2,
            "the ADSR is signal-driven and does not follow the beat"
        );
        assert_eq!(engine.followers_of(Timebase::Transport), 0);

        engine.set_timebase(&lfo, Timebase::Transport);
        assert_eq!(engine.followers_of(Timebase::Beat), 1);
        assert_eq!(engine.followers_of(Timebase::Transport), 1);
    }

    /// Signal-driven sources have no meaningful notion of show position, so
    /// they read wall time no matter what they are set to.
    #[test]
    fn adsr_ignores_its_timebase() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::ADSR {
            attack: 1.0,
            decay: 0.1,
            sustain: 1.0,
            release: 0.1,
            stage: ADSRStage::Idle,
            stage_time: 0.0,
            gate: false,
            current_level: 0.0,
        });
        engine.set_timebase(&uuid, Timebase::Beat);
        engine.assign("p", &uuid, 1.0, None);
        engine.trigger_adsr(&uuid);

        // Beat clock frozen, wall clock advancing: the envelope must still open.
        let stalled_beat = TimebaseSet::new(
            ctx(0.0, 0.5, true),
            ctx(0.0, 0.0, false),
            ctx(0.0, 0.0, false),
        );
        engine.update(&stalled_beat, &empty_audio(), &empty_analyzers());
        assert!(
            engine.get_modulation("p") > 0.0,
            "ADSR must advance on wall time even when set to Beat"
        );
    }

    /// The split between time-driven and signal-driven sources decides which
    /// cards get a selector, so it is asserted per variant rather than inferred.
    #[test]
    fn only_time_driven_sources_follow_a_timebase() {
        let step = ModulationSource::StepSequencer {
            steps: vec![0.0, 1.0],
            rate: 1.0,
            interpolation: StepInterpolation::None,
            bipolar: false,
        };
        assert!(ModulationSource::sine_lfo(1.0).follows_timebase());
        assert!(step.follows_timebase());

        let adsr = ModulationSource::ADSR {
            attack: 0.1,
            decay: 0.1,
            sustain: 0.5,
            release: 0.1,
            stage: ADSRStage::Idle,
            stage_time: 0.0,
            gate: false,
            current_level: 0.0,
        };
        let band = ModulationSource::AudioBand {
            source_id: None,
            freq_low: 20.0,
            freq_high: 250.0,
            gain: 1.0,
            smoothing: 0.5,
            mode: AudioReactMode::Direct,
            noise_gate: 0.1,
        };
        let analyzer = ModulationSource::Analyzer {
            deck_id: "d".into(),
            analyzer_type: "motion".into(),
            output_name: "amount".into(),
            smoothing: 0.5,
        };
        assert!(!adsr.follows_timebase());
        assert!(!band.follows_timebase());
        assert!(!analyzer.follows_timebase());
    }

    #[test]
    fn set_timebase_reports_unknown_uuid() {
        let mut engine = ModulationEngine::new();
        assert!(!engine.set_timebase("nope", Timebase::Beat));
        assert_eq!(engine.timebase("nope"), None);
    }

    /// Scenes written before timebases existed must load as free-running.
    /// See /spec/timebase.md § Backwards Compatibility.
    #[test]
    fn entry_without_timebase_deserializes_as_free_run() {
        let json = r#"{"uuid":"abc123","source":{"LFO":{"waveform":"Sine","frequency":1.0,"phase":0.0,"amplitude":1.0,"bipolar":false}}}"#;
        let entry: ModulationSourceEntry = serde_json::from_str(json).expect("legacy entry loads");
        assert_eq!(entry.timebase, Timebase::FreeRun);
        assert_eq!(entry.uuid, "abc123");
    }

    #[test]
    fn timebase_round_trips_through_serde() {
        let mut entry = ModulationSourceEntry::new(ModulationSource::sine_lfo(1.0));
        entry.timebase = Timebase::Beat;
        let json = serde_json::to_string(&entry).expect("serialize");
        let back: ModulationSourceEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.timebase, Timebase::Beat);
    }

    // ── LFO waveform tests ───────────────────────────────────────────

    #[test]
    fn lfo_sine_unipolar_range() {
        let mut lfo = ModulationSource::sine_lfo(1.0);
        let audio = empty_audio();
        for i in 0..100 {
            let t = i as f32 / 100.0;
            let val = lfo.calculate(t, 0.01, &audio, &empty_analyzers(), 0.0);
            assert!(
                (0.0..=1.0).contains(&val),
                "Sine unipolar out of range: {val} at t={t}"
            );
        }
    }

    #[test]
    fn lfo_sine_bipolar_range() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        for i in 0..100 {
            let t = i as f32 / 100.0;
            let val = lfo.calculate(t, 0.01, &audio, &empty_analyzers(), 0.0);
            assert!(
                (-1.0..=1.0).contains(&val),
                "Sine bipolar out of range: {val}"
            );
        }
    }

    #[test]
    fn lfo_square_values() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Square,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val_first = lfo.calculate(0.1, 0.01, &audio, &empty_analyzers(), 0.0);
        let val_second = lfo.calculate(0.6, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val_first - 1.0).abs() < 1e-5);
        assert!((val_second - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn lfo_triangle_symmetry() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Triangle,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val_start = lfo.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let val_mid = lfo.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(
            (val_start - (-1.0)).abs() < 1e-5,
            "Triangle at 0: {val_start}"
        );
        assert!((val_mid - 1.0).abs() < 1e-5, "Triangle at 0.5: {val_mid}");
    }

    #[test]
    fn lfo_sawtooth_ramp() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sawtooth,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val_0 = lfo.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let val_half = lfo.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val_0 - (-1.0)).abs() < 1e-5);
        assert!((val_half - 0.0).abs() < 1e-5);
    }

    #[test]
    fn lfo_amplitude_scales() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 0.5,
            bipolar: true,
        };
        let audio = empty_audio();
        for i in 0..100 {
            let t = i as f32 / 100.0;
            let val = lfo.calculate(t, 0.01, &audio, &empty_analyzers(), 0.0);
            assert!((-0.5..=0.5).contains(&val), "Amplitude scaling off: {val}");
        }
    }

    #[test]
    fn lfo_frequency_affects_period() {
        let mut lfo_slow = ModulationSource::sine_lfo(1.0);
        let mut lfo_fast = ModulationSource::sine_lfo(2.0);
        let audio = empty_audio();
        let slow = lfo_slow.calculate(0.25, 0.01, &audio, &empty_analyzers(), 0.0);
        let fast = lfo_fast.calculate(0.25, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((slow - fast).abs() > 0.1);
    }

    #[test]
    fn lfo_random_deterministic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Random,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val1 = lfo.calculate(0.3, 0.01, &audio, &empty_analyzers(), 0.0);
        let val2 = lfo.calculate(0.3, 0.01, &audio, &empty_analyzers(), 0.0);
        assert_eq!(
            val1, val2,
            "Random LFO should be deterministic for same time"
        );
    }

    // ── ADSR tests ───────────────────────────────────────────────────

    #[test]
    fn adsr_idle_is_zero() {
        let mut adsr = ModulationSource::adsr(0.1, 0.1, 0.5, 0.1);
        let audio = empty_audio();
        let val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), 0.0);
        assert_eq!(val, 0.0);
    }

    #[test]
    fn adsr_attack_reaches_peak() {
        let mut adsr = ModulationSource::adsr(0.1, 0.1, 0.5, 0.1);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..20 {
            val = adsr.calculate(0.0, 0.01, &audio, &empty_analyzers(), val);
        }
        assert!(
            val > 0.4,
            "ADSR should reach significant level during attack: {val}"
        );
    }

    #[test]
    fn adsr_sustain_holds() {
        let mut adsr = ModulationSource::adsr(0.01, 0.01, 0.7, 0.01);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..100 {
            val = adsr.calculate(0.0, 0.01, &audio, &empty_analyzers(), val);
        }
        assert!(
            (val - 0.7).abs() < 0.05,
            "ADSR should hold at sustain level: {val}"
        );
    }

    #[test]
    fn adsr_release_to_zero() {
        let mut adsr = ModulationSource::adsr(0.01, 0.01, 0.7, 0.05);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.01, &audio, &empty_analyzers(), val);
        }
        adsr.gate_off();
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.01, &audio, &empty_analyzers(), val);
        }
        assert!(val < 0.05, "ADSR should release to near zero: {val}");
    }

    #[test]
    fn adsr_gate_off_noop_when_idle() {
        let mut adsr = ModulationSource::adsr(0.1, 0.1, 0.5, 0.1);
        adsr.gate_off();
        let audio = empty_audio();
        let val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), 0.0);
        assert_eq!(val, 0.0);
    }

    // ── StepSequencer tests ──────────────────────────────────────────

    #[test]
    fn step_sequencer_basic() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 0.5, 1.0, 0.5],
            rate: 4.0,
            interpolation: StepInterpolation::None,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - 0.0).abs() < 1e-5);
        let val = seq.calculate(0.25, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - 0.5).abs() < 1e-5);
    }

    #[test]
    fn step_sequencer_linear_interpolation() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 1.0],
            rate: 1.0,
            interpolation: StepInterpolation::Linear,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - 0.5).abs() < 0.01, "Linear interp mid: {val}");
    }

    #[test]
    fn step_sequencer_bipolar() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 1.0],
            rate: 1.0,
            interpolation: StepInterpolation::None,
            bipolar: true,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - (-1.0)).abs() < 1e-5);
        let val = seq.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - 1.0).abs() < 1e-5);
    }

    #[test]
    fn step_sequencer_empty_returns_zero() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![],
            rate: 1.0,
            interpolation: StepInterpolation::None,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert_eq!(val, 0.0);
    }

    #[test]
    fn step_sequencer_smooth_interpolation() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 1.0],
            rate: 1.0,
            interpolation: StepInterpolation::Smooth,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(val > 0.0 && val < 1.0, "Smooth interp: {val}");
        assert!(
            (val - 0.5).abs() < 0.01,
            "Smoothstep at 0.5 should be 0.5: {val}"
        );
    }

    // ── AudioSourceValues tests ──────────────────────────────────────

    #[test]
    fn audio_energy_empty_fft() {
        let source = AudioSourceValues {
            fft: vec![],
            level: 0.0,
            sample_rate: 48000.0,
        };
        assert_eq!(source.energy_in_range(20.0, 250.0), 0.0);
    }

    #[test]
    fn audio_energy_zero_sample_rate() {
        let source = AudioSourceValues {
            fft: vec![0.5; 256],
            level: 0.5,
            sample_rate: 0.0,
        };
        assert_eq!(source.energy_in_range(20.0, 250.0), 0.0);
    }

    #[test]
    fn audio_energy_silent() {
        let source = AudioSourceValues {
            fft: vec![0.0; 256],
            level: 0.0,
            sample_rate: 48000.0,
        };
        assert_eq!(source.energy_in_range(20.0, 250.0), 0.0);
    }

    #[test]
    fn audio_energy_loud_signal() {
        let source = AudioSourceValues {
            fft: vec![1.0; 256],
            level: 1.0,
            sample_rate: 48000.0,
        };
        let energy = source.energy_in_range(20.0, 20000.0);
        assert!((energy - 1.0).abs() < 0.01, "Full signal energy: {energy}");
    }

    #[test]
    fn audio_values_primary_returns_lowest_id() {
        let mut av = AudioValues::default();
        av.sources.insert(
            5,
            AudioSourceValues {
                fft: vec![],
                level: 0.5,
                sample_rate: 48000.0,
            },
        );
        av.sources.insert(
            2,
            AudioSourceValues {
                fft: vec![],
                level: 0.8,
                sample_rate: 48000.0,
            },
        );
        let primary = av.primary().unwrap();
        assert!((primary.level - 0.8).abs() < 1e-5);
    }

    #[test]
    fn audio_values_primary_none_when_empty() {
        let av = AudioValues::default();
        assert!(av.primary().is_none());
    }

    // ── ModulationEngine tests ───────────────────────────────────────

    #[test]
    fn engine_add_source_returns_uuid() {
        let mut engine = ModulationEngine::new();
        let uuid0 = engine.add_source(ModulationSource::sine_lfo(1.0));
        let uuid1 = engine.add_source(ModulationSource::sine_lfo(2.0));
        assert_ne!(uuid0, uuid1);
        assert_eq!(engine.source_count(), 2);
    }

    #[test]
    fn engine_audio_band_source_ids_lists_only_audio_bands() {
        // audio_band_source_ids drives the capture reconcile (issue #76): it must
        // report every AudioBand's device selection and ignore other source kinds.
        let mut engine = ModulationEngine::new();
        engine.add_source(ModulationSource::sine_lfo(1.0));
        let band = |source_id| ModulationSource::AudioBand {
            source_id,
            freq_low: 20.0,
            freq_high: 200.0,
            gain: 1.0,
            smoothing: 0.6,
            mode: AudioReactMode::Direct,
            noise_gate: 0.1,
        };
        engine.add_source(band(None));
        engine.add_source(band(Some(2)));
        let mut ids = engine.audio_band_source_ids();
        ids.sort();
        assert_eq!(ids, vec![None, Some(2)]);
    }

    #[test]
    fn engine_remove_source_cleans_assignments() {
        let mut engine = ModulationEngine::new();
        let uuid0 = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.add_source(ModulationSource::sine_lfo(2.0));
        let uuid2 = engine.add_source(ModulationSource::sine_lfo(3.0));
        engine.assign("param_a", &uuid0, 1.0, None);
        engine.assign("param_b", &uuid2, 0.5, None);
        engine.remove_source(&uuid0);
        assert!(!engine.has_modulation("param_a"));
        assert!(engine.has_modulation("param_b"));
        assert_eq!(engine.source_count(), 2);
    }

    #[test]
    fn engine_assign_and_get_modulation() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.update_free_running(0.25, &empty_audio(), &empty_analyzers());
        engine.assign("brightness", &uuid, 1.0, None);
        let _mod_val = engine.get_modulation("brightness");
    }

    #[test]
    fn engine_clear_assignments() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.assign("brightness", &uuid, 1.0, None);
        assert!(engine.has_modulation("brightness"));
        engine.clear_assignments("brightness");
        assert!(!engine.has_modulation("brightness"));
    }

    #[test]
    fn engine_update_computes_values() {
        let mut engine = ModulationEngine::new();
        engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.update_free_running(0.0, &empty_audio(), &empty_analyzers());
        let values = engine.current_values();
        assert_eq!(values.len(), 1);
    }

    #[test]
    fn engine_mod_on_mod() {
        let mut engine = ModulationEngine::new();
        let lfo0 = engine.add_source(ModulationSource::sine_lfo(1.0));
        let lfo1 = engine.add_source(ModulationSource::sine_lfo(2.0));
        engine.assign_mod_on_mod(&lfo0, "frequency", &lfo1, 0.5);
        engine.update_free_running(1.0, &empty_audio(), &empty_analyzers());
        assert_eq!(engine.current_values().len(), 2);
    }

    #[test]
    fn engine_clear_mod_on_mod() {
        let mut engine = ModulationEngine::new();
        let lfo0 = engine.add_source(ModulationSource::sine_lfo(1.0));
        let lfo1 = engine.add_source(ModulationSource::sine_lfo(2.0));
        engine.assign_mod_on_mod(&lfo0, "frequency", &lfo1, 0.5);
        assert!(engine.has_modulation(&format!("mod:{lfo0}:frequency")));
        engine.clear_mod_on_mod(&lfo0, "frequency");
        assert!(!engine.has_modulation(&format!("mod:{lfo0}:frequency")));
    }

    #[test]
    fn engine_trigger_adsr() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::adsr(0.01, 0.01, 0.5, 0.01));
        engine.trigger_adsr(&uuid);
        for i in 0..20 {
            engine.update_free_running(i as f32 * 0.01, &empty_audio(), &empty_analyzers());
        }
        let val = engine.current_value_for(&uuid);
        assert!(val > 0.0, "ADSR should produce non-zero after trigger");
    }

    #[test]
    fn engine_release_adsr() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::adsr(0.01, 0.01, 0.5, 0.01));
        engine.trigger_adsr(&uuid);
        for i in 0..30 {
            engine.update_free_running(i as f32 * 0.01, &empty_audio(), &empty_analyzers());
        }
        engine.release_adsr(&uuid);
        for i in 30..80 {
            engine.update_free_running(i as f32 * 0.01, &empty_audio(), &empty_analyzers());
        }
        let val = engine.current_value_for(&uuid);
        assert!(val < 0.1, "ADSR should be near zero after release: {val}");
    }

    #[test]
    fn engine_get_modulation_nonexistent_param() {
        let engine = ModulationEngine::new();
        assert_eq!(engine.get_modulation("nonexistent"), 0.0);
    }

    #[test]
    fn engine_evaluation_order_no_deps() {
        let mut engine = ModulationEngine::new();
        engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.add_source(ModulationSource::sine_lfo(2.0));
        let order = engine.evaluation_order();
        assert_eq!(order.len(), 2);
    }

    #[test]
    fn engine_component_modulation() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.update_free_running(0.25, &empty_audio(), &empty_analyzers());
        engine.assign("color", &uuid, 1.0, Some(0));
        engine.assign("color", &uuid, 0.5, Some(1));
        let r_mod = engine.get_modulation_for_component("color", Some(0));
        let g_mod = engine.get_modulation_for_component("color", Some(1));
        let no_mod = engine.get_modulation_for_component("color", Some(2));
        // Unassigned component contributes nothing.
        assert_eq!(no_mod, 0.0);
        // Both assigned components are driven by the same source; the r
        // component (amount 1.0) must be twice the g component (amount 0.5).
        assert!(r_mod > 0.0, "r component should be modulated: {r_mod}");
        assert!((r_mod - 2.0 * g_mod).abs() < 1e-6, "r={r_mod}, g={g_mod}");
    }

    // ── AudioBandPreset tests ────────────────────────────────────────

    #[test]
    fn audio_band_preset_ranges() {
        assert_eq!(AudioBandPreset::Low.freq_range(), (20.0, 250.0));
        assert_eq!(AudioBandPreset::Mid.freq_range(), (250.0, 2000.0));
        assert_eq!(AudioBandPreset::High.freq_range(), (2000.0, 20000.0));
        assert_eq!(AudioBandPreset::Full.freq_range(), (20.0, 20000.0));
    }

    #[test]
    fn audio_band_from_preset_creates_valid_source() {
        let source = ModulationSource::audio_from_preset(AudioBandPreset::Low);
        match source {
            ModulationSource::AudioBand {
                freq_low,
                freq_high,
                gain,
                ..
            } => {
                assert_eq!(freq_low, 20.0);
                assert_eq!(freq_high, 250.0);
                assert_eq!(gain, 1.0);
            }
            _ => panic!("Expected AudioBand"),
        }
    }

    // ── Constructor tests ────────────────────────────────────────────

    #[test]
    fn step_sequencer_min_steps() {
        let seq = ModulationSource::step_sequencer(1, 1.0);
        match seq {
            ModulationSource::StepSequencer { steps, .. } => {
                assert_eq!(steps.len(), 2);
            }
            _ => panic!("Expected StepSequencer"),
        }
    }

    #[test]
    fn parse_mod_target_valid() {
        assert_eq!(
            ModulationEngine::parse_mod_target("mod:abc123:frequency"),
            Some("abc123")
        );
        assert_eq!(
            ModulationEngine::parse_mod_target("mod:def456:phase"),
            Some("def456")
        );
    }

    #[test]
    fn parse_mod_target_invalid() {
        assert_eq!(ModulationEngine::parse_mod_target("brightness"), None);
        assert_eq!(ModulationEngine::parse_mod_target("deck0:param"), None);
    }

    // ── Audio band with noise gate ───────────────────────────────────

    #[test]
    fn audio_band_noise_gate() {
        let mut source = ModulationSource::AudioBand {
            source_id: Some(0),
            freq_low: 20.0,
            freq_high: 250.0,
            gain: 1.0,
            smoothing: 0.0,
            mode: AudioReactMode::Direct,
            noise_gate: 0.5,
        };
        let mut audio = AudioValues::default();
        audio.sources.insert(
            0,
            AudioSourceValues {
                fft: vec![0.001; 256],
                level: 0.001,
                sample_rate: 48000.0,
            },
        );
        let val = source.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert_eq!(val, 0.0, "Below noise gate should be silent");
    }

    // ── config_eq tests ──────────────────────────────────────────────

    #[test]
    fn config_eq_lfo_same() {
        let a = ModulationSource::sine_lfo(2.0);
        let b = ModulationSource::sine_lfo(2.0);
        assert!(a.config_eq(&b));
    }

    #[test]
    fn config_eq_lfo_different_freq() {
        let a = ModulationSource::sine_lfo(2.0);
        let b = ModulationSource::sine_lfo(3.0);
        assert!(!a.config_eq(&b));
    }

    #[test]
    fn config_eq_adsr_ignores_runtime() {
        let a = ModulationSource::ADSR {
            attack: 0.1,
            decay: 0.2,
            sustain: 0.7,
            release: 0.3,
            stage: ADSRStage::Idle,
            stage_time: 0.0,
            gate: false,
            current_level: 0.0,
        };
        let b = ModulationSource::ADSR {
            attack: 0.1,
            decay: 0.2,
            sustain: 0.7,
            release: 0.3,
            stage: ADSRStage::Attack,
            stage_time: 1.5,
            gate: true,
            current_level: 0.8,
        };
        assert!(a.config_eq(&b));
    }

    #[test]
    fn config_eq_different_variants() {
        let a = ModulationSource::sine_lfo(2.0);
        let b = ModulationSource::adsr(0.1, 0.2, 0.7, 0.3);
        assert!(!a.config_eq(&b));
    }

    // ── find_source_by_uuid tests ───────────────────────────────────

    #[test]
    fn find_source_by_uuid_found() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(2.0));
        assert!(engine.find_source_by_uuid(&uuid).is_some());
    }

    #[test]
    fn find_source_by_uuid_not_found() {
        let engine = ModulationEngine::new();
        assert!(engine.find_source_by_uuid("nonexistent").is_none());
    }

    #[test]
    fn add_source_with_uuid_preserves_uuid() {
        let mut engine = ModulationEngine::new();
        let uuid =
            engine.add_source_with_uuid("custom01".to_string(), ModulationSource::sine_lfo(2.0));
        assert_eq!(uuid, "custom01");
        assert!(engine.has_source("custom01"));
    }

    // ── Gap coverage: chains, removal, edge cases ───────────────────

    #[test]
    fn circular_mod_on_mod_no_hang() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let b = engine.add_source(ModulationSource::sine_lfo(2.0));
        let c = engine.add_source(ModulationSource::sine_lfo(3.0));
        // A modulates B, B modulates C, C modulates A (cycle)
        engine.assign_mod_on_mod(&b, "frequency", &a, 0.5);
        engine.assign_mod_on_mod(&c, "frequency", &b, 0.5);
        engine.assign_mod_on_mod(&a, "frequency", &c, 0.5);
        // Must complete without hanging, values must be finite
        let audio = AudioValues::default();
        engine.update_free_running(1.0, &audio, &empty_analyzers());
        for v in engine.current_values() {
            assert!(v.is_finite(), "circular chain produced non-finite value");
        }
    }

    #[test]
    fn deep_chain_fallback() {
        let mut engine = ModulationEngine::new();
        let mut uuids = Vec::new();
        for i in 0..5 {
            uuids.push(engine.add_source(ModulationSource::sine_lfo((i + 1) as f32)));
        }
        // Chain: 0→1→2→3→4
        for i in 0..4 {
            engine.assign_mod_on_mod(&uuids[i + 1], "frequency", &uuids[i], 0.1);
        }
        let audio = AudioValues::default();
        engine.update_free_running(1.0, &audio, &empty_analyzers());
        // All 5 sources should have been evaluated
        assert_eq!(engine.current_values().len(), 5);
        for v in engine.current_values() {
            assert!(v.is_finite());
        }
    }

    #[test]
    fn evaluation_order_respects_deps() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let b = engine.add_source(ModulationSource::sine_lfo(2.0));
        // A modulates B → A must be evaluated before B
        engine.assign_mod_on_mod(&b, "frequency", &a, 0.5);
        let order = engine.evaluation_order();
        let a_pos = order
            .iter()
            .position(|&i| i == engine.sources.iter().position(|e| e.uuid == a).unwrap())
            .unwrap();
        let b_pos = order
            .iter()
            .position(|&i| i == engine.sources.iter().position(|e| e.uuid == b).unwrap())
            .unwrap();
        assert!(
            a_pos < b_pos,
            "dependency A should be evaluated before target B"
        );
    }

    #[test]
    fn remove_source_mid_chain() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let b = engine.add_source(ModulationSource::sine_lfo(2.0));
        let c = engine.add_source(ModulationSource::sine_lfo(3.0));
        engine.assign_mod_on_mod(&b, "frequency", &a, 0.5);
        engine.assign_mod_on_mod(&c, "frequency", &b, 0.5);
        // Remove the middle source
        engine.remove_source(&b);
        assert_eq!(engine.source_count(), 2);
        // Should still update without panic
        let audio = AudioValues::default();
        engine.update_free_running(1.0, &audio, &empty_analyzers());
        assert!(engine.has_source(&a));
        assert!(engine.has_source(&c));
    }

    #[test]
    fn index_consistency_after_removal() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let b = engine.add_source(ModulationSource::sine_lfo(2.0));
        let c = engine.add_source(ModulationSource::sine_lfo(3.0));
        engine.remove_source(&b);
        // UUIDs a and c should still resolve correctly
        assert!(engine.find_source_by_uuid(&a).is_some());
        assert!(engine.find_source_by_uuid(&c).is_some());
        assert_eq!(engine.source_count(), 2);
    }

    #[test]
    fn empty_source_list_update() {
        let mut engine = ModulationEngine::new();
        let audio = AudioValues::default();
        // Update with 0 sources → no crash
        engine.update_free_running(0.0, &audio, &empty_analyzers());
        assert_eq!(engine.source_count(), 0);
        assert!(engine.current_values().is_empty());
    }

    #[test]
    fn mod_on_mod_removed_target() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let b = engine.add_source(ModulationSource::sine_lfo(2.0));
        engine.assign_mod_on_mod(&a, "frequency", &b, 0.5);
        // Remove the target — assignments should be cleaned up
        engine.remove_source(&a);
        assert!(!engine.has_source(&a));
        // The mod-on-mod key "mod:{a}:frequency" should have been purged
        for key in engine.assignments_iter().map(|(k, _)| k) {
            assert!(
                !key.contains(&a),
                "stale mod-on-mod key found after target removal"
            );
        }
    }

    #[test]
    fn assign_nonexistent_source_ignored() {
        let mut engine = ModulationEngine::new();
        engine.assign("some_param", "bogus_uuid", 1.0, None);
        // No assignment should have been created
        assert!(!engine.has_modulation("some_param"));
    }

    // ── Chaos Tests Round 2: LFO edge values ────────────────────────────

    #[test]
    fn chaos_lfo_zero_frequency_does_not_nan() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: 0.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        for i in 0..100 {
            let val = lfo.calculate(i as f32 * 0.01, 0.01, &audio, &empty_analyzers(), 0.0);
            assert!(val.is_finite(), "LFO freq=0 produced non-finite: {val}");
        }
    }

    #[test]
    fn chaos_lfo_infinity_frequency_does_not_panic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: f32::INFINITY,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val = lfo.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        // (Inf * 1.0 + 0.0) % 1.0 = NaN — document this
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_lfo_nan_frequency_does_not_panic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: f32::NAN,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val = lfo.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_lfo_nan_amplitude_does_not_panic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Triangle,
            frequency: 1.0,
            phase: 0.0,
            amplitude: f32::NAN,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = lfo.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_lfo_negative_frequency_does_not_panic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sawtooth,
            frequency: -10.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val = lfo.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(
            val.is_finite(),
            "negative freq should produce finite: {val}"
        );
    }

    #[test]
    fn chaos_lfo_all_waveforms_at_extreme_time() {
        let audio = empty_audio();
        for waveform in [
            LFOWaveform::Sine,
            LFOWaveform::Square,
            LFOWaveform::Triangle,
            LFOWaveform::Sawtooth,
            LFOWaveform::Random,
        ] {
            let mut lfo = ModulationSource::LFO {
                waveform,
                frequency: 1e6,
                phase: 0.0,
                amplitude: 1.0,
                bipolar: true,
            };
            let val = lfo.calculate(1e10, 0.01, &audio, &empty_analyzers(), 0.0);
            let _ = val; // must not panic
        }
    }

    // ── Chaos Tests Round 2: Step Sequencer edge cases ───────────────────

    #[test]
    fn chaos_step_sequencer_single_step() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.75],
            rate: 1.0,
            interpolation: StepInterpolation::Linear,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(val.is_finite(), "single step produced non-finite: {val}");
    }

    #[test]
    fn chaos_step_sequencer_nan_rate_does_not_panic() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 0.5, 1.0],
            rate: f32::NAN,
            interpolation: StepInterpolation::None,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_step_sequencer_infinity_rate_does_not_panic() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 1.0],
            rate: f32::INFINITY,
            interpolation: StepInterpolation::Smooth,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_step_sequencer_zero_rate() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.2, 0.8],
            rate: 0.0,
            interpolation: StepInterpolation::Linear,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(val.is_finite(), "zero rate produced non-finite: {val}");
    }

    #[test]
    fn chaos_step_sequencer_nan_step_values() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5],
            rate: 1.0,
            interpolation: StepInterpolation::Linear,
            bipolar: false,
        };
        let audio = empty_audio();
        for i in 0..20 {
            let val = seq.calculate(i as f32 * 0.25, 0.01, &audio, &empty_analyzers(), 0.0);
            let _ = val; // must not panic
        }
    }

    // ── Chaos Tests Round 2: ADSR edge cases ────────────────────────────

    #[test]
    fn chaos_adsr_zero_all_times() {
        let mut adsr = ModulationSource::adsr(0.0, 0.0, 0.5, 0.0);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
            assert!(val.is_finite(), "zero-time ADSR produced non-finite: {val}");
        }
        adsr.gate_off();
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
            assert!(val.is_finite(), "zero-time ADSR release non-finite: {val}");
        }
    }

    #[test]
    fn chaos_adsr_nan_attack_does_not_panic() {
        let mut adsr = ModulationSource::ADSR {
            attack: f32::NAN,
            decay: 0.1,
            sustain: 0.5,
            release: 0.1,
            stage: ADSRStage::Idle,
            stage_time: 0.0,
            gate: false,
            current_level: 0.0,
        };
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..20 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
        }
        // must not panic
    }

    #[test]
    fn chaos_adsr_negative_sustain() {
        let mut adsr = ModulationSource::adsr(0.01, 0.01, -1.0, 0.01);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..100 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
        }
        // Sustain = -1.0 may produce negative values — document, must not panic
    }

    #[test]
    fn chaos_adsr_infinity_release() {
        let mut adsr = ModulationSource::adsr(0.01, 0.01, 0.5, f32::INFINITY);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
        }
        adsr.gate_off();
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
            // progress = stage_time / INFINITY = 0 — never completes release
        }
        // must not panic
    }

    #[test]
    fn chaos_adsr_rapid_gate_toggle() {
        let mut adsr = ModulationSource::adsr(0.1, 0.1, 0.5, 0.1);
        let audio = empty_audio();
        let mut val = 0.0;
        for i in 0..100 {
            if i % 3 == 0 {
                adsr.gate_on();
            }
            if i % 5 == 0 {
                adsr.gate_off();
            }
            val = adsr.calculate(0.0, 0.001, &audio, &empty_analyzers(), val);
            assert!(
                val.is_finite(),
                "rapid gate toggle produced non-finite at step {i}: {val}"
            );
        }
    }

    // ── Analyzer source tests ────────────────────────────────────────

    #[test]
    fn analyzer_source_reads_from_values() {
        let mut src = ModulationSource::Analyzer {
            deck_id: "deck-1".into(),
            analyzer_type: "brightness".into(),
            output_name: "brightness".into(),
            smoothing: 0.0, // no smoothing
        };
        let audio = empty_audio();
        let mut av = AnalyzerValues::default();
        av.insert(
            "deck-1".into(),
            "brightness".into(),
            "brightness".into(),
            0.75,
        );
        let val = src.calculate(0.0, 0.016, &audio, &av, 0.0);
        assert!((val - 0.75).abs() < 1e-5, "Expected 0.75, got {val}");
    }

    #[test]
    fn analyzer_source_smoothing() {
        let mut src = ModulationSource::Analyzer {
            deck_id: "d".into(),
            analyzer_type: "brightness".into(),
            output_name: "brightness".into(),
            smoothing: 0.5,
        };
        let audio = empty_audio();
        let mut av = AnalyzerValues::default();
        av.insert("d".into(), "brightness".into(), "brightness".into(), 1.0);

        // First frame: alpha=0.5, prev=0.0 → 0.5*1.0 + 0.5*0.0 = 0.5
        let v1 = src.calculate(0.0, 0.016, &audio, &av, 0.0);
        assert!((v1 - 0.5).abs() < 1e-5, "Expected 0.5, got {v1}");

        // Second frame: 0.5*1.0 + 0.5*0.5 = 0.75
        let v2 = src.calculate(0.016, 0.016, &audio, &av, v1);
        assert!((v2 - 0.75).abs() < 1e-5, "Expected 0.75, got {v2}");
    }

    #[test]
    fn analyzer_source_missing_returns_zero() {
        let mut src = ModulationSource::Analyzer {
            deck_id: "nonexistent".into(),
            analyzer_type: "brightness".into(),
            output_name: "brightness".into(),
            smoothing: 0.0,
        };
        let val = src.calculate(0.0, 0.016, &empty_audio(), &empty_analyzers(), 0.5);
        assert!(
            val.abs() < 1e-5,
            "Missing analyzer should return 0.0, got {val}"
        );
    }
}
