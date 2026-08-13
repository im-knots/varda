//! Receiving SMPTE timecode, and resolving it into one position.
//!
//! Two protocols arrive here (LTC off an audio input, MTC off a MIDI port) and
//! one position leaves, which the transport chases. Everything downstream of
//! the transport (automation, arrangement regions, cue resolution) never learns
//! that timecode exists, so all of it works with no hardware present.
//!
//! This is deliberately not the tempo clock ([`crate::clock`]): timecode carries
//! absolute position and no tempo at all, and neither derives from the other.
//!
//! See /spec/timecode.md.

pub mod ltc;
pub mod mtc;

use std::time::{Duration, Instant};

use crate::midi::DeviceId;
use crate::transport::TimecodeRate;

/// How many frames of silence the reader coasts through before giving up.
///
/// Much shorter than the clock's two second stale timeout: the clock is
/// smoothing a jittery tempo estimate, while timecode is a positional signal
/// that should be trusted immediately and abandoned quickly.
const FREEWHEEL_FRAMES: f64 = 5.0;

/// How far a frame may land from where the reader expected it before the jump
/// is called a locate rather than drift. Two frames, so ordinary jitter and a
/// dropped frame or two stay quiet.
const DISCONTINUITY_FRAMES: f64 = 2.0;

/// Smoothing on the measured speed, which is noisy at one sample per frame.
const SPEED_ALPHA: f64 = 0.2;

/// One decoded time address, and the rate it was sent at.
///
/// Frames are what both protocols deliver, so both decoders produce this and
/// the manager never learns which protocol it is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimecodeFrame {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub rate: TimecodeRate,
}

impl TimecodeFrame {
    pub fn new(hours: u8, minutes: u8, seconds: u8, frames: u8, rate: TimecodeRate) -> Self {
        Self {
            hours,
            minutes,
            seconds,
            frames,
            rate,
        }
    }

    /// The label this position carries at `rate`.
    ///
    /// The inverse of [`Self::position`], and built from the same arithmetic the
    /// UI formats with, so a decoded frame and the readout can never disagree
    /// about drop-frame.
    pub fn at(position: f64, rate: TimecodeRate) -> Self {
        let (hours, minutes, seconds, frames) = rate.label_parts(position);
        Self::new(hours, minutes, seconds, frames, rate)
    }

    /// Absolute position in seconds.
    ///
    /// Drop-frame labels skip numbers to stay with wall time, so the elapsed
    /// frame count is recovered by putting the dropped ones back before dividing
    /// by the exact rate (30000/1001), never by 30.
    pub fn position(self) -> f64 {
        let nominal = self.rate.nominal_fps();
        let total_minutes = u64::from(self.hours) * 60 + u64::from(self.minutes);
        let labelled =
            (u64::from(self.hours) * 3600 + u64::from(self.minutes) * 60 + u64::from(self.seconds))
                * nominal
                + u64::from(self.frames);
        let elapsed = if self.rate.is_drop_frame() {
            // Two labels are skipped at the top of every minute but the tenth.
            labelled.saturating_sub(2 * (total_minutes - total_minutes / 10))
        } else {
            labelled
        };
        elapsed as f64 / self.rate.fps()
    }

    /// `HH:MM:SS:FF`, with `;` before the frames on drop-frame.
    pub fn label(self) -> String {
        self.rate.format(self.position())
    }

    /// The address `n` frames later.
    ///
    /// Counted in frames rather than added in seconds: a frame boundary is
    /// exactly where floating point is least trustworthy, and stepping by
    /// `1.0 / fps` lands twice on the same label often enough to matter.
    #[must_use]
    pub fn plus_frames(self, n: i64) -> Self {
        let index = (self.position() * self.rate.fps()).round() as i64 + n;
        // Aim at the middle of the target frame, so the flooring in
        // `label_parts` cannot land a hair short of it.
        let mid = (index.max(0) as f64 + 0.5) / self.rate.fps();
        Self::at(mid, self.rate)
    }
}

/// Where a timecode signal is coming from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimecodeSource {
    /// Audio input, and the channel of it the signal is on. The standard field
    /// rig sends music to the PA on one channel and timecode to us on the other.
    Ltc {
        source_id: crate::audio::AudioSourceId,
        channel: u16,
    },
    Mtc {
        device_id: DeviceId,
        device_name: String,
    },
}

impl TimecodeSource {
    /// Stable name for this input in the API and the UI.
    pub fn key(&self) -> String {
        match self {
            TimecodeSource::Ltc { .. } => "ltc".to_string(),
            TimecodeSource::Mtc { device_id, .. } => format!("mtc:{device_id}"),
        }
    }

    pub fn label(&self) -> String {
        match self {
            TimecodeSource::Ltc { channel, .. } => format!("LTC (channel {})", channel + 1),
            TimecodeSource::Mtc { device_name, .. } => format!("MTC ({device_name})"),
        }
    }

    fn is_ltc(&self) -> bool {
        matches!(self, TimecodeSource::Ltc { .. })
    }

    /// How many frames apart this protocol delivers addresses.
    ///
    /// LTC carries one per frame. MTC spends eight quarter-frame messages on
    /// one address, which takes two. Judging MTC against LTC's cadence would
    /// call a healthy MIDI master late for half of every cycle.
    fn cadence_frames(&self) -> f64 {
        match self {
            TimecodeSource::Ltc { .. } => 1.0,
            TimecodeSource::Mtc { .. } => 2.0,
        }
    }
}

/// Which audio input carries LTC.
///
/// Held beside the preference rather than inside it: this is a patch decision
/// that should survive switching to `Auto` and back, and burying it in one
/// enum variant would lose it on every switch.
/// See /spec/timecode.md § Preference and Priority.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub struct LtcInput {
    pub source_id: crate::audio::AudioSourceId,
    /// Zero-based channel index within that device.
    pub channel: u16,
    /// Rate to read the signal at, or `None` to infer it from cadence.
    ///
    /// Only 29.97 non-drop actually needs naming: it is a thousandth away from
    /// 30 in the signal and identical in the labels, but 3.6 seconds an hour
    /// apart in position. See [`ltc::LtcDecoder::set_rate_override`].
    #[serde(default)]
    pub rate: Option<TimecodeRate>,
}

/// Which signal the transport should follow.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
pub enum TimecodePreference {
    /// LTC if an input is named and it is arriving, otherwise MTC.
    #[default]
    Auto,
    /// The named LTC input only. Legible as "no input selected" when there is
    /// none, rather than silently following MIDI instead.
    ForceLtc,
    ForceMtc {
        device_id: DeviceId,
    },
    /// Ignore timecode entirely. Unlike the tempo clock, timecode has a
    /// plausible "definitely not during this rehearsal" state.
    Off,
}

/// The timecode patch as it is written to `stage.json`.
///
/// Devices are named here rather than numbered. Ids are handed out at
/// enumeration and shift whenever the rig changes between load-ins, so a saved
/// id would point at whatever box happened to enumerate in that slot next time.
/// `midi.json` keys its mappings by name for the same reason.
///
/// It lives in stage rather than scene because which cable carries timecode is
/// a property of the room, not of the show playing in it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TimecodeConfig {
    #[serde(default)]
    pub preference: PreferenceConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ltc_input: Option<LtcInputConfig>,
}

/// [`TimecodePreference`] with the MIDI port named instead of numbered.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PreferenceConfig {
    #[default]
    Auto,
    ForceLtc,
    ForceMtc {
        device: String,
    },
    Off,
}

/// [`LtcInput`] with the audio device named instead of numbered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LtcInputConfig {
    pub device: String,
    #[serde(default)]
    pub channel: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<TimecodeRate>,
}

/// One input the reader is listening to, resolved or not.
///
/// Reported even while it is not the one driving the transport, because a
/// performer chasing a bad cable needs to see the input that is *not*
/// resolving. See /spec/timecode.md § Dual simultaneous inputs.
#[derive(Debug, Clone)]
pub struct TimecodeInput {
    pub source: TimecodeSource,
    /// Position as of the last read, extrapolated while freewheeling.
    pub position: f64,
    pub rate: TimecodeRate,
    /// Frames are arriving, or the freewheel is still coasting.
    pub running: bool,
    /// Coasting: no frame has arrived on schedule but the window is open.
    pub freewheeling: bool,
    /// Measured against wall time. 1.0 while a master plays forwards.
    pub speed: f64,
    /// Position of the last frame actually received, before extrapolation.
    frame_position: f64,
    /// When that frame arrived.
    at: Instant,
    /// Raised for one read after a jump larger than the drift tolerance.
    discontinuity: bool,
}

impl TimecodeInput {
    /// `HH:MM:SS:FF` for the diagnostics readout.
    pub fn label(&self) -> String {
        self.rate.format(self.position)
    }

    /// How long after the last address the next one was due.
    fn cadence(&self) -> Duration {
        Duration::from_secs_f64(self.source.cadence_frames() / self.rate.fps())
    }

    /// How long the reader coasts once an address is overdue.
    fn freewheel_window(&self) -> Duration {
        Duration::from_secs_f64(FREEWHEEL_FRAMES / self.rate.fps())
    }
}

/// The resolved signal, for the transport and for diagnostics.
#[derive(Debug, Clone)]
pub struct TimecodeState {
    /// Absolute position in seconds. `f64` because shows start at hour 1 or
    /// later, where `f32` would quantise to about 0.4 ms.
    pub position: f64,
    pub rate: TimecodeRate,
    /// Frames are arriving, or the freewheel is still coasting.
    pub running: bool,
    pub freewheeling: bool,
    /// True for one read after a jump larger than the freewheel tolerance, so
    /// the transport can tell a locate from drift.
    pub discontinuity: bool,
    pub speed: f64,
    pub source: Option<TimecodeSource>,
}

impl Default for TimecodeState {
    fn default() -> Self {
        Self {
            position: 0.0,
            rate: TimecodeRate::default(),
            running: false,
            freewheeling: false,
            discontinuity: false,
            speed: 1.0,
            source: None,
        }
    }
}

/// Collects frames from every listening input and resolves one position.
#[derive(Debug, Default)]
pub struct TimecodeManager {
    inputs: Vec<TimecodeInput>,
    preference: TimecodePreference,
    ltc_input: Option<LtcInput>,
    /// Built on the first PCM chunk, when the device's sample rate is known.
    decoder: Option<ltc::LtcDecoder>,
    /// One per device: two masters on two ports are two conversations, and
    /// interleaving their nibbles would assemble a time neither of them sent.
    assemblers: std::collections::HashMap<DeviceId, mtc::QuarterFrameAssembler>,
    state: TimecodeState,
}

impl TimecodeManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Configuration ───────────────────────────────────────────

    pub fn preference(&self) -> TimecodePreference {
        self.preference
    }

    pub fn set_preference(&mut self, preference: TimecodePreference) {
        self.preference = preference;
        if preference == TimecodePreference::Off {
            self.inputs.clear();
            self.assemblers.clear();
            self.decoder = None;
        }
    }

    pub fn ltc_input(&self) -> Option<LtcInput> {
        self.ltc_input
    }

    /// Name the audio input carrying LTC, or stop listening for it.
    ///
    /// Changing it drops what the old input had decoded: those frames describe
    /// a signal nobody is reading any more.
    pub fn set_ltc_input(&mut self, input: Option<LtcInput>) {
        if input == self.ltc_input {
            return;
        }
        self.ltc_input = input;
        self.decoder = None;
        self.inputs.retain(|i| !i.source.is_ltc());
    }

    /// Whether LTC should be listened for at all, which is what decides if an
    /// audio device is opened for it.
    pub fn wants_ltc(&self) -> bool {
        self.ltc_input.is_some()
            && matches!(
                self.preference,
                TimecodePreference::Auto | TimecodePreference::ForceLtc
            )
    }

    /// Whether MTC from `device_id` is worth parsing.
    pub fn wants_mtc(&self, device_id: DeviceId) -> bool {
        match self.preference {
            TimecodePreference::Auto => true,
            TimecodePreference::ForceMtc { device_id: wanted } => wanted == device_id,
            TimecodePreference::ForceLtc | TimecodePreference::Off => false,
        }
    }

    // ── Persistence ─────────────────────────────────────────────

    /// The patch in its saveable form, with devices named.
    ///
    /// A device that has since disappeared keeps no name, so the patch is
    /// dropped rather than saved pointing at nothing.
    pub fn to_config(
        &self,
        audio_name: impl Fn(crate::audio::AudioSourceId) -> Option<String>,
        midi_name: impl Fn(DeviceId) -> Option<String>,
    ) -> TimecodeConfig {
        TimecodeConfig {
            preference: match self.preference {
                TimecodePreference::Auto => PreferenceConfig::Auto,
                TimecodePreference::ForceLtc => PreferenceConfig::ForceLtc,
                TimecodePreference::ForceMtc { device_id } => midi_name(device_id)
                    .map_or(PreferenceConfig::Auto, |device| {
                        PreferenceConfig::ForceMtc { device }
                    }),
                TimecodePreference::Off => PreferenceConfig::Off,
            },
            ltc_input: self.ltc_input.and_then(|input| {
                Some(LtcInputConfig {
                    device: audio_name(input.source_id)?,
                    channel: input.channel,
                    rate: input.rate,
                })
            }),
        }
    }

    /// Restore a saved patch against the devices present now.
    ///
    /// Returns what could not be restored, for the notification bar: a rig
    /// that is one interface short should say so on load rather than sit
    /// silently unsynced until someone notices the show is not moving.
    pub fn apply_config(
        &mut self,
        config: &TimecodeConfig,
        audio_id: impl Fn(&str) -> Option<crate::audio::AudioSourceId>,
        midi_id: impl Fn(&str) -> Option<DeviceId>,
    ) -> Vec<String> {
        let mut warnings = Vec::new();

        let ltc_input = config.ltc_input.as_ref().and_then(|saved| {
            let source_id = audio_id(&saved.device);
            if source_id.is_none() {
                warnings.push(format!(
                    "Audio input \"{}\" is not connected, so LTC is not being listened for",
                    saved.device
                ));
            }
            Some(LtcInput {
                source_id: source_id?,
                channel: saved.channel,
                rate: saved.rate,
            })
        });
        self.set_ltc_input(ltc_input);

        let preference = match &config.preference {
            PreferenceConfig::Auto => TimecodePreference::Auto,
            PreferenceConfig::ForceLtc => TimecodePreference::ForceLtc,
            PreferenceConfig::ForceMtc { device } => {
                if let Some(device_id) = midi_id(device) {
                    TimecodePreference::ForceMtc { device_id }
                } else {
                    warnings.push(format!(
                        "MIDI port \"{device}\" is not connected, so timecode is following whatever arrives"
                    ));
                    TimecodePreference::Auto
                }
            }
            PreferenceConfig::Off => TimecodePreference::Off,
        };
        self.set_preference(preference);

        warnings
    }

    // ── Reads ───────────────────────────────────────────────────

    pub fn state(&self) -> &TimecodeState {
        &self.state
    }

    pub fn inputs(&self) -> &[TimecodeInput] {
        &self.inputs
    }

    /// Which input is driving, by [`TimecodeSource::key`].
    pub fn resolved_key(&self) -> Option<String> {
        self.state.source.as_ref().map(TimecodeSource::key)
    }

    // ── Per-frame ───────────────────────────────────────────────

    /// Record a frame that just arrived.
    pub fn ingest(&mut self, source: TimecodeSource, frame: TimecodeFrame, at: Instant) {
        if self.preference == TimecodePreference::Off {
            return;
        }
        let position = frame.position();
        // Matched by key rather than by value: a MIDI device that is renamed or
        // re-enumerated is the same input, not a second one.
        let key = source.key();
        let Some(input) = self.inputs.iter_mut().find(|i| i.source.key() == key) else {
            self.inputs.push(TimecodeInput {
                source,
                position,
                rate: frame.rate,
                running: true,
                freewheeling: false,
                speed: 1.0,
                frame_position: position,
                at,
                // The first frame after silence is where the show is, not a
                // jump from wherever it was before the cable was plugged in.
                discontinuity: true,
            });
            return;
        };

        let elapsed = at.saturating_duration_since(input.at).as_secs_f64();
        let travelled = position - input.frame_position;
        let expected = if input.running {
            elapsed * input.speed
        } else {
            0.0
        };
        let tolerance = DISCONTINUITY_FRAMES / frame.rate.fps();

        if !input.running || (travelled - expected).abs() > tolerance.max(elapsed) {
            // A locate, or the master coming back after a dropout. Speed
            // measured across a jump is meaningless, so it resets.
            input.discontinuity = true;
            input.speed = 1.0;
        } else if elapsed > 0.0 {
            let measured = (travelled / elapsed).clamp(-4.0, 4.0);
            input.speed = SPEED_ALPHA * measured + (1.0 - SPEED_ALPHA) * input.speed;
        }

        input.rate = frame.rate;
        input.frame_position = position;
        input.position = position;
        input.at = at;
        input.running = true;
        input.freewheeling = false;
    }

    /// Offer a MIDI message to the MTC receiver.
    ///
    /// Everything that is not timecode is ignored here, so the caller can hand
    /// over the whole stream rather than filtering it first.
    pub fn ingest_midi(&mut self, message: &crate::midi::MidiMessage, at: Instant) {
        use crate::midi::MidiMessage;
        let device_id = message.device_id();
        if !self.wants_mtc(device_id) {
            return;
        }
        let frame = match message {
            MidiMessage::MtcQuarterFrame { data, .. } => {
                self.assemblers.entry(device_id).or_default().feed(*data)
            }
            MidiMessage::MtcFullFrame { payload, .. } => {
                // A locate abandons whatever the nibbles were building: the
                // master is telling us directly where it went.
                self.assemblers.remove(&device_id);
                mtc::full_frame(*payload)
            }
            _ => return,
        };
        let Some(frame) = frame else { return };
        self.ingest(
            TimecodeSource::Mtc {
                device_id,
                device_name: String::new(),
            },
            frame,
            at,
        );
    }

    /// Offer a block of interleaved PCM to the LTC decoder.
    ///
    /// `channels` is the device's channel count, because the standard field rig
    /// puts music on one channel and timecode on the other, and the tee hands
    /// over both.
    pub fn ingest_pcm(
        &mut self,
        source_id: crate::audio::AudioSourceId,
        samples: &[f32],
        channels: u16,
        sample_rate: u32,
        at: Instant,
    ) {
        let Some(input) = self.ltc_input else { return };
        if !self.wants_ltc() || input.source_id != source_id || channels == 0 {
            return;
        }
        let channel = usize::from(input.channel.min(channels - 1));
        let decoder = self.decoder.get_or_insert_with(|| {
            let mut decoder = ltc::LtcDecoder::new(f64::from(sample_rate));
            decoder.set_rate_override(input.rate);
            decoder
        });

        let mono: Vec<f32> = samples
            .iter()
            .skip(channel)
            .step_by(usize::from(channels))
            .copied()
            .collect();
        let mut frames = Vec::new();
        decoder.feed(&mono, |frame| frames.push(frame));

        for frame in frames {
            self.ingest(
                TimecodeSource::Ltc {
                    source_id,
                    channel: input.channel,
                },
                frame,
                at,
            );
        }
    }

    /// Name the device a resolved MTC input is on, for the readout.
    ///
    /// Held apart from ingestion because the name is a display concern that
    /// changes when a device is renamed or re-enumerated, and threading it
    /// through every quarter frame would allocate one per nibble.
    pub fn name_device(&mut self, device_id: DeviceId, name: &str) {
        for input in &mut self.inputs {
            if let TimecodeSource::Mtc {
                device_id: id,
                device_name,
            } = &mut input.source
            {
                if *id == device_id && device_name != name {
                    device_name.clear();
                    device_name.push_str(name);
                }
            }
        }
    }

    /// Age every input and resolve one of them. Call once per frame, before
    /// anything reads [`Self::state`].
    pub fn update(&mut self, now: Instant) {
        for input in &mut self.inputs {
            let idle = now.saturating_duration_since(input.at);
            // Lateness is measured from when the next address was *due*, not
            // from the last one, so a protocol that speaks every two frames is
            // not judged against one that speaks every frame.
            let late = idle.saturating_sub(input.cadence());
            if late > input.freewheel_window() {
                // The window expired: hold the last position rather than
                // extrapolating into a signal that is not there.
                input.running = false;
                input.freewheeling = false;
                input.position = input.frame_position;
            } else if input.running {
                // Between addresses the reader coasts, so a 25 fps signal
                // drives a 60 fps render smoothly instead of in stair steps.
                // Coasting inside the cadence is normal; past it is a dropout.
                input.freewheeling = late > Duration::from_secs_f64(0.5 / input.rate.fps());
                input.position = input.frame_position + idle.as_secs_f64() * input.speed;
            }
        }

        let resolved = self.resolve();
        self.state = match resolved.and_then(|key| {
            self.inputs
                .iter_mut()
                .find(|i| i.source.key() == key)
                .map(|input| {
                    let state = TimecodeState {
                        position: input.position,
                        rate: input.rate,
                        running: input.running,
                        freewheeling: input.freewheeling,
                        discontinuity: input.discontinuity,
                        speed: input.speed,
                        source: Some(input.source.clone()),
                    };
                    // Published once, like the transport's own jump flag.
                    input.discontinuity = false;
                    state
                })
        }) {
            Some(state) => state,
            None => TimecodeState {
                // Position holds through a source switch rather than snapping to
                // zero, which would be a black frame on a dropped cable.
                position: self.state.position,
                rate: self.state.rate,
                ..TimecodeState::default()
            },
        };
    }

    /// Which input's key should drive, by preference then by priority.
    ///
    /// LTC outranks MTC because it is clocked by the audio device it arrives
    /// on, whereas MTC follows the sending machine's clock and shares a bus
    /// with every other MIDI message.
    fn resolve(&self) -> Option<String> {
        let running = |input: &&TimecodeInput| input.running;
        match self.preference {
            TimecodePreference::Off => None,
            TimecodePreference::ForceLtc => self
                .inputs
                .iter()
                .find(|i| i.source.is_ltc())
                .map(|i| i.source.key()),
            TimecodePreference::ForceMtc { device_id } => self
                .inputs
                .iter()
                .find(|i| matches!(&i.source, TimecodeSource::Mtc { device_id: d, .. } if *d == device_id))
                .map(|i| i.source.key()),
            TimecodePreference::Auto => self
                .inputs
                .iter()
                .filter(running)
                .find(|i| i.source.is_ltc())
                .or_else(|| self.inputs.iter().find(|i| running(i)))
                // Nothing is arriving: name an input anyway, so the UI can say
                // which one it is waiting on rather than showing no source.
                .or_else(|| self.inputs.first())
                .map(|i| i.source.key()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ltc() -> TimecodeSource {
        TimecodeSource::Ltc {
            source_id: 0,
            channel: 1,
        }
    }

    fn mtc(device_id: DeviceId) -> TimecodeSource {
        TimecodeSource::Mtc {
            device_id,
            device_name: format!("Device {device_id}"),
        }
    }

    fn frame(position: f64, rate: TimecodeRate) -> TimecodeFrame {
        TimecodeFrame::at(position, rate)
    }

    /// The label and the position are the same fact in two forms, and a decoder
    /// that disagreed with the readout would be a bug nobody could see.
    #[test]
    fn a_label_and_a_position_convert_both_ways() {
        for rate in TimecodeRate::ALL {
            for seconds in [0.0, 1.0, 59.9, 61.5, 3599.0, 3661.25, 36_000.0] {
                let frame = TimecodeFrame::at(seconds, rate);
                let round_tripped = frame.position();
                assert!(
                    (round_tripped - seconds).abs() <= 1.0 / rate.fps(),
                    "{rate:?} at {seconds}s came back as {round_tripped}"
                );
                assert_eq!(
                    frame.label(),
                    rate.format(seconds),
                    "{rate:?} at {seconds}s"
                );
            }
        }
    }

    /// Drop-frame skips labels to stay with wall time, so an hour of it is an
    /// hour. Getting this backwards is the classic timecode bug: it looks right
    /// for the first minute and is 3.6 seconds out by the end of the hour.
    #[test]
    fn an_hour_of_drop_frame_is_an_hour_of_wall_time() {
        let frame = TimecodeFrame::new(1, 0, 0, 0, TimecodeRate::Fps2997Drop);
        assert!(
            (frame.position() - 3600.0).abs() < 0.05,
            "01:00:00;00 should be an hour, got {}",
            frame.position()
        );

        // Non-drop counts every label, so the same address is 3.6 seconds later.
        let nondrop = TimecodeFrame::new(1, 0, 0, 0, TimecodeRate::Fps2997);
        assert!(
            (nondrop.position() - 3603.6).abs() < 0.05,
            "01:00:00:00 non-drop drifts, got {}",
            nondrop.position()
        );
    }

    /// The rule drop-frame is named for. Two numbers are skipped at the top of
    /// every minute except the tenth, and a reader that skips at the tenth too,
    /// or forgets to skip at all, drifts against the master by seconds an hour.
    #[test]
    fn drop_frame_skips_two_labels_every_minute_but_the_tenth() {
        let rate = TimecodeRate::Fps2997Drop;

        let end_of_minute = TimecodeFrame::new(0, 0, 59, 29, rate);
        let next = end_of_minute.plus_frames(1);
        assert_eq!(
            (next.minutes, next.seconds, next.frames),
            (1, 0, 2),
            "the top of a minute starts at frame 2, got {}",
            next.label()
        );

        let end_of_tenth = TimecodeFrame::new(0, 9, 59, 29, rate);
        let next = end_of_tenth.plus_frames(1);
        assert_eq!(
            (next.minutes, next.seconds, next.frames),
            (10, 0, 0),
            "the tenth minute is the one that keeps its numbers, got {}",
            next.label()
        );

        // Every label survives the trip through seconds and back, including the
        // ones either side of a skip.
        for label in [
            TimecodeFrame::new(0, 0, 59, 29, rate),
            TimecodeFrame::new(0, 1, 0, 2, rate),
            TimecodeFrame::new(0, 9, 59, 29, rate),
            TimecodeFrame::new(0, 10, 0, 0, rate),
            TimecodeFrame::new(1, 0, 0, 0, rate),
        ] {
            assert_eq!(
                TimecodeFrame::at(label.position(), rate),
                label,
                "{} did not survive the round trip",
                label.label()
            );
        }
    }

    /// The reader must coast through a scuffed cable rather than stopping the
    /// show, and must give up quickly enough that a real stop is not held for
    /// seconds.
    #[test]
    fn a_dropout_is_coasted_through_and_then_given_up_on() {
        let mut manager = TimecodeManager::new();
        let start = Instant::now();
        manager.ingest(ltc(), frame(10.0, TimecodeRate::Fps25), start);
        manager.update(start);
        assert!(manager.state().running);

        // Two frames of silence: still running, and still moving.
        let coasting = start + Duration::from_millis(80);
        manager.update(coasting);
        assert!(manager.state().running, "a dropped frame is not a stop");
        assert!(
            manager.state().position > 10.0,
            "the reader coasts at the signal's speed"
        );
        assert!(manager.state().freewheeling);

        // Past the window: stopped, holding the last position it was sure of.
        manager.update(start + Duration::from_millis(400));
        assert!(!manager.state().running);
        assert!(
            (manager.state().position - 10.0).abs() < 1e-9,
            "it holds where it lost the signal"
        );
    }

    /// MTC spends two frames delivering one address, so a healthy MIDI master
    /// is silent between them by design. Judging it against LTC's one-per-frame
    /// cadence flagged it as freewheeling for half of every cycle, which read
    /// on stage as a signal dropping in and out.
    #[test]
    fn a_healthy_mtc_master_is_not_called_a_dropout() {
        let mut manager = TimecodeManager::new();
        let mut now = Instant::now();
        let step = Duration::from_secs_f64(2.0 / 25.0);

        manager.ingest(mtc(1), frame(10.0, TimecodeRate::Fps25), now);
        for i in 1..10 {
            // Sampled mid-cycle, where the old threshold called it late.
            manager.update(now + step.mul_f64(0.9));
            assert!(
                !manager.state().freewheeling,
                "two frames between addresses is how MTC works, cycle {i}"
            );
            assert!(manager.state().running);

            now += step;
            manager.ingest(
                mtc(1),
                frame(10.0 + f64::from(i) * 0.08, TimecodeRate::Fps25),
                now,
            );
        }

        // A master that actually stops still gets given up on.
        manager.update(now + Duration::from_millis(500));
        assert!(!manager.state().running);
    }

    /// A locate has to be distinguishable from drift, because the video chase
    /// servo seeks for one and trims speed for the other.
    #[test]
    fn a_jump_is_reported_but_drift_is_not() {
        let mut manager = TimecodeManager::new();
        let start = Instant::now();
        manager.ingest(ltc(), frame(10.0, TimecodeRate::Fps25), start);
        manager.update(start);
        assert!(manager.state().discontinuity, "arriving is itself a jump");

        let next = start + Duration::from_millis(40);
        manager.ingest(ltc(), frame(10.04, TimecodeRate::Fps25), next);
        manager.update(next);
        assert!(
            !manager.state().discontinuity,
            "one frame later at one frame on is just playing"
        );

        let jumped = next + Duration::from_millis(40);
        manager.ingest(ltc(), frame(300.0, TimecodeRate::Fps25), jumped);
        manager.update(jumped);
        assert!(manager.state().discontinuity, "a locate is a jump");

        manager.update(jumped + Duration::from_millis(1));
        assert!(
            !manager.state().discontinuity,
            "and it is published once, not held"
        );
    }

    /// LTC is clocked by the audio device it rides on; MTC follows the sending
    /// machine and shares a bus. When both are live, follow the better one.
    #[test]
    fn ltc_outranks_mtc_when_both_are_arriving() {
        let mut manager = TimecodeManager::new();
        let now = Instant::now();
        manager.ingest(mtc(3), frame(5.0, TimecodeRate::Fps25), now);
        manager.ingest(ltc(), frame(90.0, TimecodeRate::Fps25), now);
        manager.update(now);

        assert_eq!(manager.resolved_key().as_deref(), Some("ltc"));
        assert!((manager.state().position - 90.0).abs() < 0.01);
    }

    /// With no LTC arriving, MIDI is what there is.
    #[test]
    fn mtc_drives_when_no_ltc_is_arriving() {
        let mut manager = TimecodeManager::new();
        let now = Instant::now();
        manager.ingest(mtc(3), frame(5.0, TimecodeRate::Fps25), now);
        manager.update(now);
        assert_eq!(manager.resolved_key().as_deref(), Some("mtc:3"));
    }

    /// Forcing an input that is not there has to read as "waiting on that
    /// input", not as "quietly following the other one".
    #[test]
    fn forcing_an_input_never_silently_follows_another() {
        let mut manager = TimecodeManager::new();
        manager.set_preference(TimecodePreference::ForceLtc);
        let now = Instant::now();
        manager.ingest(mtc(3), frame(5.0, TimecodeRate::Fps25), now);
        manager.update(now);

        assert_eq!(manager.resolved_key(), None);
        assert!(!manager.state().running);
        assert_eq!(
            manager.inputs().len(),
            1,
            "the MIDI input is still reported, so a wrong forcing is diagnosable"
        );
    }

    /// Changing which input to follow must not throw the show to the top of the
    /// arrangement. Losing a source is a reason to stop, never a reason to cut
    /// to whatever is rendered at zero.
    #[test]
    fn the_show_holds_where_it_was_when_the_source_goes_away() {
        let mut manager = TimecodeManager::new();
        let now = Instant::now();
        manager.ingest(mtc(3), frame(3600.0, TimecodeRate::Fps25), now);
        manager.update(now);
        assert!((manager.state().position - 3600.0).abs() < 0.05);

        // Told to follow LTC, which nothing is patched to.
        manager.set_preference(TimecodePreference::ForceLtc);
        manager.update(now);

        assert_eq!(manager.resolved_key(), None);
        assert!(!manager.state().running);
        assert!(
            (manager.state().position - 3600.0).abs() < 0.05,
            "it held at {} instead of an hour in",
            manager.state().position
        );
    }

    /// Naming the port to follow is a promise that nothing else drives the
    /// show, including the protocol that would otherwise outrank it.
    #[test]
    fn forcing_a_port_ignores_a_live_ltc_master() {
        let mut manager = TimecodeManager::new();
        manager.set_preference(TimecodePreference::ForceMtc { device_id: 7 });
        let now = Instant::now();
        manager.ingest(ltc(), frame(90.0, TimecodeRate::Fps25), now);
        manager.ingest(mtc(7), frame(5.0, TimecodeRate::Fps25), now);
        manager.update(now);

        assert_eq!(manager.resolved_key().as_deref(), Some("mtc:7"));
        assert!((manager.state().position - 5.0).abs() < 0.05);
    }

    /// A master coming back from a dropout is not playing on from where the
    /// reader guessed it would be. The chase servo has to seek rather than trim
    /// speed, so the return has to read as a jump.
    #[test]
    fn a_master_returning_after_a_dropout_is_published_as_a_jump() {
        let mut manager = TimecodeManager::new();
        let start = Instant::now();
        manager.ingest(ltc(), frame(10.0, TimecodeRate::Fps25), start);
        manager.update(start);

        let given_up = start + Duration::from_millis(400);
        manager.update(given_up);
        assert!(!manager.state().running, "the window has expired");

        // Back, at exactly where uninterrupted playback would have put it.
        manager.ingest(ltc(), frame(10.4, TimecodeRate::Fps25), given_up);
        manager.update(given_up);
        assert!(
            manager.state().discontinuity,
            "returning after silence is a jump even when the numbers line up"
        );
    }

    /// Naming a port also means ignoring the others: a second machine idling on
    /// the same bus must not be able to take the show.
    #[test]
    fn mtc_from_a_port_nobody_named_is_never_parsed() {
        use crate::midi::MidiMessage;

        let mut manager = TimecodeManager::new();
        manager.set_preference(TimecodePreference::ForceMtc { device_id: 1 });
        let now = Instant::now();
        for piece in 0..8_u8 {
            manager.ingest_midi(
                &MidiMessage::MtcQuarterFrame {
                    device_id: 2,
                    data: piece << 4,
                },
                now,
            );
        }
        manager.update(now);

        assert!(manager.inputs().is_empty(), "it is not our master");
    }

    /// A patch is a pair of a device and a channel, and a mono input has only
    /// one channel to offer. Reading past its end must not take the show down.
    #[test]
    fn a_channel_the_device_does_not_have_is_read_off_its_last() {
        let mut manager = TimecodeManager::new();
        manager.set_ltc_input(Some(LtcInput {
            source_id: 3,
            // Patched for a stereo pair, plugged into a mono input.
            channel: 1,
            rate: None,
        }));

        let first = TimecodeFrame::new(0, 5, 0, 0, TimecodeRate::Fps25);
        let mono = ltc::encode::run(first, 8, 48_000.0, 0.5);

        let now = Instant::now();
        manager.ingest_pcm(3, &mono, 1, 48_000, now);
        manager.update(now);

        assert_eq!(manager.resolved_key().as_deref(), Some("ltc"));
        assert!(
            (manager.state().position - 300.0).abs() < 0.5,
            "five minutes in, got {}",
            manager.state().position
        );
    }

    /// Off means off: a rehearsal with a timecode cable still patched should not
    /// have the transport twitching.
    #[test]
    fn off_stops_listening_and_forgets_what_it_heard() {
        let mut manager = TimecodeManager::new();
        let now = Instant::now();
        manager.ingest(ltc(), frame(10.0, TimecodeRate::Fps25), now);
        manager.set_preference(TimecodePreference::Off);
        manager.ingest(ltc(), frame(20.0, TimecodeRate::Fps25), now);
        manager.update(now);

        assert!(manager.inputs().is_empty());
        assert!(!manager.state().running);
        assert_eq!(manager.resolved_key(), None);
    }

    /// LTC costs an audio device open, so it is listened for only when an input
    /// has been named. MTC is already flowing and costs nothing.
    #[test]
    fn ltc_is_only_listened_for_on_a_named_input() {
        let mut manager = TimecodeManager::new();
        assert!(!manager.wants_ltc(), "nothing named yet");
        assert!(manager.wants_mtc(0), "MIDI is already arriving");

        manager.set_ltc_input(Some(LtcInput {
            source_id: 0,
            channel: 1,
            rate: None,
        }));
        assert!(manager.wants_ltc());

        manager.set_preference(TimecodePreference::ForceMtc { device_id: 2 });
        assert!(!manager.wants_ltc());
        assert!(manager.wants_mtc(2));
        assert!(
            !manager.wants_mtc(3),
            "one device was named, not all of them"
        );

        manager.set_preference(TimecodePreference::Off);
        assert!(!manager.wants_ltc());
        assert!(!manager.wants_mtc(2));
    }

    /// Re-patching LTC to another channel must not leave the old channel's
    /// frames driving the show.
    #[test]
    fn re_patching_ltc_drops_what_the_old_input_decoded() {
        let mut manager = TimecodeManager::new();
        let now = Instant::now();
        manager.ingest(ltc(), frame(10.0, TimecodeRate::Fps25), now);
        manager.set_ltc_input(Some(LtcInput {
            source_id: 0,
            channel: 0,
            rate: None,
        }));

        assert!(manager.inputs().is_empty());
    }

    /// The path the Tascam takes: nibbles off the wire, a position out.
    #[test]
    fn a_midi_stream_of_quarter_frames_moves_the_show() {
        use crate::midi::MidiMessage;

        let mut manager = TimecodeManager::new();
        let sent = TimecodeFrame::new(1, 0, 30, 0, TimecodeRate::Fps25);
        let hours = sent.hours | (mtc::rate_bits(sent.rate) << 5);
        let nibbles = [
            sent.frames & 0x0F,
            sent.frames >> 4,
            sent.seconds & 0x0F,
            sent.seconds >> 4,
            sent.minutes & 0x0F,
            sent.minutes >> 4,
            hours & 0x0F,
            hours >> 4,
        ];

        let now = Instant::now();
        for (piece, value) in nibbles.iter().enumerate() {
            manager.ingest_midi(
                &MidiMessage::MtcQuarterFrame {
                    device_id: 7,
                    data: ((piece as u8) << 4) | value,
                },
                now,
            );
        }
        manager.update(now);

        assert_eq!(manager.resolved_key().as_deref(), Some("mtc:7"));
        assert!(
            (manager.state().position - sent.plus_frames(2).position()).abs() < 0.001,
            "the show is where the master said, plus the two frames it took to say it"
        );

        // A locate arrives whole, and lands immediately. `0x20` is rate bits 01
        // (25 fps) with hour zero; the rest is plain binary, not BCD.
        manager.ingest_midi(
            &MidiMessage::MtcFullFrame {
                device_id: 7,
                payload: [0x20, 0x0A, 0x00, 0x00],
            },
            now,
        );
        manager.update(now);
        assert!((manager.state().position - 600.0).abs() < 0.001);
    }

    /// The standard field rig: music down one channel to the PA, timecode down
    /// the other to us. Reading the wrong one is silence, so the channel
    /// selection is the feature.
    #[test]
    fn ltc_is_read_off_the_channel_it_was_patched_to() {
        let mut manager = TimecodeManager::new();
        manager.set_ltc_input(Some(LtcInput {
            source_id: 3,
            channel: 1,
            rate: None,
        }));

        let first = TimecodeFrame::new(0, 5, 0, 0, TimecodeRate::Fps25);
        let timecode = ltc::encode::run(first, 8, 48_000.0, 0.5);
        // Something musical on the left, which must not be mistaken for a
        // signal or interfere with the one on the right.
        let interleaved: Vec<f32> = timecode
            .iter()
            .enumerate()
            .flat_map(|(i, tc)| {
                let music = (i as f32 * 0.01).sin() * 0.8;
                [music, *tc]
            })
            .collect();

        let now = Instant::now();
        manager.ingest_pcm(3, &interleaved, 2, 48_000, now);
        manager.update(now);

        assert_eq!(manager.resolved_key().as_deref(), Some("ltc"));
        assert!(
            (manager.state().position - 300.0).abs() < 0.5,
            "five minutes in, got {}",
            manager.state().position
        );
        assert_eq!(manager.state().rate, TimecodeRate::Fps25);

        // The same audio arriving on a device nobody patched is not timecode.
        let mut other = TimecodeManager::new();
        other.set_ltc_input(Some(LtcInput {
            source_id: 3,
            channel: 1,
            rate: None,
        }));
        other.ingest_pcm(9, &interleaved, 2, 48_000, now);
        other.update(now);
        assert!(other.inputs().is_empty());
    }

    /// A master running at half speed is worth knowing about: it is what the
    /// video chase servo trims against, and it is a diagnostic in its own right.
    #[test]
    fn the_measured_speed_follows_the_master() {
        let mut manager = TimecodeManager::new();
        let mut now = Instant::now();
        manager.ingest(ltc(), frame(10.0, TimecodeRate::Fps25), now);
        // Half speed: one frame of timecode every two frames of wall time.
        for i in 1..40 {
            now += Duration::from_millis(80);
            manager.ingest(
                ltc(),
                frame(10.0 + f64::from(i) * 0.04, TimecodeRate::Fps25),
                now,
            );
        }
        manager.update(now);

        let speed = manager.state().speed;
        assert!(
            (speed - 0.5).abs() < 0.05,
            "half-speed playback should read as about 0.5, got {speed}"
        );
    }

    // ── Persistence ─────────────────────────────────────────────

    /// The patch is saved by device name and comes back pointing at whatever
    /// id those devices hold today, which is the whole reason names are stored.
    #[test]
    fn a_saved_patch_survives_devices_being_renumbered() {
        let mut manager = TimecodeManager::new();
        manager.set_ltc_input(Some(LtcInput {
            source_id: 3,
            channel: 1,
            rate: Some(TimecodeRate::Fps2997),
        }));
        manager.set_preference(TimecodePreference::ForceMtc { device_id: 7 });

        let config = manager.to_config(
            |id| (id == 3).then(|| "Scarlett 2i2".to_string()),
            |id| (id == 7).then(|| "Tascam DA-6400".to_string()),
        );
        assert_eq!(
            config.ltc_input.as_ref().map(|i| i.device.as_str()),
            Some("Scarlett 2i2")
        );

        // Next load-in the same boxes enumerate in a different order.
        let mut restored = TimecodeManager::new();
        let warnings = restored.apply_config(
            &config,
            |name| (name == "Scarlett 2i2").then_some(11),
            |name| (name == "Tascam DA-6400").then_some(2),
        );

        assert!(warnings.is_empty());
        assert_eq!(
            restored.ltc_input(),
            Some(LtcInput {
                source_id: 11,
                channel: 1,
                rate: Some(TimecodeRate::Fps2997),
            })
        );
        assert_eq!(
            restored.preference(),
            TimecodePreference::ForceMtc { device_id: 2 }
        );
    }

    /// A rig one interface short must say so. Silently following nothing looks
    /// identical to a show that simply has not started.
    #[test]
    fn a_missing_device_is_reported_rather_than_guessed_at() {
        let config = TimecodeConfig {
            preference: PreferenceConfig::ForceMtc {
                device: "Tascam DA-6400".to_string(),
            },
            ltc_input: Some(LtcInputConfig {
                device: "Scarlett 2i2".to_string(),
                channel: 1,
                rate: None,
            }),
        };

        let mut manager = TimecodeManager::new();
        let warnings = manager.apply_config(&config, |_| None, |_| None);

        assert_eq!(
            warnings.len(),
            2,
            "both absences are worth saying: {warnings:?}"
        );
        assert!(warnings.iter().any(|w| w.contains("Scarlett 2i2")));
        assert!(warnings.iter().any(|w| w.contains("Tascam DA-6400")));
        assert_eq!(manager.ltc_input(), None);
        assert_eq!(
            manager.preference(),
            TimecodePreference::Auto,
            "a missing named port falls back to following whatever arrives"
        );
    }

    /// A patch is only meaningful next to the box it names. Writing one for a
    /// device that has already gone would restore a patch onto whatever
    /// happens to answer to that name next time, which is worse than none.
    #[test]
    fn a_patch_pointing_at_a_vanished_device_is_not_written_down() {
        let mut manager = TimecodeManager::new();
        manager.set_ltc_input(Some(LtcInput {
            source_id: 3,
            channel: 1,
            rate: None,
        }));
        manager.set_preference(TimecodePreference::ForceMtc { device_id: 7 });

        let config = manager.to_config(|_| None, |_| None);

        assert_eq!(config.ltc_input, None);
        assert_eq!(
            config.preference,
            PreferenceConfig::Auto,
            "a port that is gone by save time cannot be the one to follow"
        );
    }

    /// The readout names the machine, not the port number, because that is what
    /// is written on the box a performer is about to go and check.
    #[test]
    fn the_readout_names_the_port_the_master_is_on() {
        let mut manager = TimecodeManager::new();
        let now = Instant::now();
        manager.ingest_midi(
            &crate::midi::MidiMessage::MtcFullFrame {
                device_id: 7,
                payload: [0x20, 0x0A, 0x00, 0x00],
            },
            now,
        );
        manager.name_device(7, "Tascam Model 12");
        manager.update(now);

        assert_eq!(
            manager.inputs().first().map(|input| input.source.label()),
            Some("MTC (Tascam Model 12)".to_string())
        );
    }

    /// `Off` is a decision, not an absence, so it must come back as one.
    #[test]
    fn choosing_to_ignore_timecode_is_remembered() {
        let mut manager = TimecodeManager::new();
        manager.set_preference(TimecodePreference::Off);
        let config = manager.to_config(|_| None, |_| None);

        let mut restored = TimecodeManager::new();
        restored.apply_config(&config, |_| None, |_| None);
        assert_eq!(restored.preference(), TimecodePreference::Off);
    }

    /// Stage files written before timecode existed must still load.
    #[test]
    fn a_stage_saved_before_timecode_existed_still_loads() {
        let config: TimecodeConfig = serde_json::from_str("{}").expect("empty object");
        assert_eq!(config, TimecodeConfig::default());
        assert_eq!(config.preference, PreferenceConfig::Auto);
        assert_eq!(config.ltc_input, None);
    }
}
