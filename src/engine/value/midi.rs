//! MIDI wire values: which device a message came from, and the messages Varda
//! reads. Parsing stays in the MIDI module; clock and timecode read these
//! without depending on it. See /spec/domain-dependencies.md.

/// Stable identifier for a MIDI device within a session.
pub type DeviceId = u32;

/// Parsed MIDI message types we care about
#[derive(Debug, Clone)]
pub enum MidiMessage {
    /// Control Change: channel, cc number, value (0–127)
    ControlChange {
        device_id: DeviceId,
        channel: u8,
        cc: u8,
        value: u8,
    },
    /// Note On: channel, note, velocity
    NoteOn {
        device_id: DeviceId,
        channel: u8,
        note: u8,
        velocity: u8,
    },
    /// Note Off: channel, note, velocity
    NoteOff {
        device_id: DeviceId,
        channel: u8,
        note: u8,
        velocity: u8,
    },
    /// MIDI Clock Tick (0xF8) — 24 per quarter note
    ClockTick { device_id: DeviceId },
    /// MIDI Start (0xFA) — reset to beginning
    ClockStart { device_id: DeviceId },
    /// MIDI Continue (0xFB) — resume from current position
    ClockContinue { device_id: DeviceId },
    /// MIDI Stop (0xFC) — stop clock
    ClockStop { device_id: DeviceId },
    /// MTC quarter frame (0xF1) — one nibble of a position. Interpreted by
    /// the timecode module; this layer only carries it.
    MtcQuarterFrame { device_id: DeviceId, data: u8 },
    /// MTC full-frame locate, as `hh mm ss ff` lifted out of its
    /// system-exclusive wrapper.
    MtcFullFrame {
        device_id: DeviceId,
        payload: [u8; 4],
    },
}
