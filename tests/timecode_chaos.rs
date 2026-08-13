//! Offensive tests for the timecode receiver: hostile input, not happy paths.
//!
//! Everything here arrives from outside the building. A MIDI port is a shared
//! bus carrying other people's traffic, an audio input carries whatever is
//! plugged into it, and a master is someone else's machine which may be
//! rewinding, stopping, lying about its rate or half unplugged. None of that
//! may panic, invent a position, or leave the show somewhere it cannot render.
//!
//! The invariants are deliberately about the *show*, not about the decoders:
//! a position that is finite, never negative, never beyond the clock, and a
//! resolution that names an input that actually exists.
//!
//! See /spec/timecode.md.

use std::time::{Duration, Instant};

use proptest::prelude::*;
use varda::midi::MidiMessage;
use varda::timecode::{
    LtcInput, TimecodeFrame, TimecodeManager, TimecodePreference, TimecodeSource,
};
use varda::transport::{Chase, TimecodeRate, Transport, TransportSource};

/// The end of the timecode day, which no address can be past.
const CLOCK: f64 = 24.0 * 3600.0;

/// What must be true of the receiver after anything at all has happened to it.
fn assert_sane(manager: &TimecodeManager, after: &str) -> Result<(), TestCaseError> {
    let state = manager.state();
    prop_assert!(
        state.position.is_finite(),
        "{after}: position went to {}",
        state.position
    );
    prop_assert!(
        state.position >= 0.0,
        "{after}: position went to {}",
        state.position
    );
    prop_assert!(
        state.position < CLOCK + 1.0,
        "{after}: position went past the clock, to {}",
        state.position
    );
    prop_assert!(
        state.speed.is_finite() && state.speed.abs() <= 4.0,
        "{after}: speed went to {}",
        state.speed
    );
    if let Some(key) = manager.resolved_key() {
        prop_assert!(
            manager.inputs().iter().any(|i| i.source.key() == key),
            "{after}: resolved \"{key}\", which is not an input it has"
        );
    }
    prop_assert!(
        !(state.running && manager.inputs().is_empty()),
        "{after}: running on no inputs at all"
    );
    Ok(())
}

proptest! {
    /// A MIDI port is a shared bus. Clock, notes from a keyboard someone leant
    /// on, a synth dumping its patches: all of it arrives here, and a reader
    /// that could be talked into a position by any of it would be a show that
    /// jumps when a musician touches a key.
    #[test]
    fn a_bus_full_of_other_peoples_traffic_cannot_break_the_reader(
        packets in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..12), 1..120),
    ) {
        let mut manager = TimecodeManager::new();
        let start = Instant::now();

        for (i, packet) in packets.iter().enumerate() {
            let at = start + Duration::from_millis(i as u64 * 4);
            if let Some(message) = MidiMessage::from_bytes(packet, 3) {
                manager.ingest_midi(&message, at);
            }
            manager.update(at);
            assert_sane(&manager, "after random MIDI")?;
        }
    }

    /// Load-in is hostile scheduling: cables going in and out, someone cycling
    /// the Follow menu to see what happens, a master that rewinds and stops.
    /// The receiver is asked to survive the order those arrive in, not a
    /// rehearsed one.
    #[test]
    fn cables_and_menus_being_thrashed_leave_the_reader_coherent(
        ops in prop::collection::vec(
            (0u8..7, 0u64..400, 0.0f64..80_000.0),
            1..200,
        ),
    ) {
        let mut manager = TimecodeManager::new();
        let start = Instant::now();
        let mut at = start;

        for (op, millis, position) in ops {
            // Time never runs backwards, but it does stand still: two frames
            // can share an instant, and a stalled render can skip minutes.
            at += Duration::from_millis(millis);
            let frame = TimecodeFrame::at(position, TimecodeRate::Fps25);

            match op {
                0 => manager.ingest(
                    TimecodeSource::Ltc { source_id: 1, channel: 0 },
                    frame,
                    at,
                ),
                1 => manager.ingest(
                    TimecodeSource::Mtc { device_id: 1, device_name: String::new() },
                    frame,
                    at,
                ),
                2 => manager.ingest(
                    TimecodeSource::Mtc { device_id: 2, device_name: String::new() },
                    frame,
                    at,
                ),
                3 => manager.set_preference(TimecodePreference::Auto),
                4 => manager.set_preference(TimecodePreference::ForceLtc),
                5 => manager.set_preference(TimecodePreference::Off),
                _ => manager.set_ltc_input(Some(LtcInput {
                    source_id: 1,
                    channel: u16::try_from(millis % 4).unwrap_or(0),
                    rate: None,
                })),
            }

            manager.update(at);
            assert_sane(&manager, "after a thrashed patch")?;
        }
    }

    /// Timecode is audio, and an audio input carries whatever is plugged into
    /// it at whatever the device claims its shape is. A mono input described as
    /// eight channels, or a sample rate of nothing, is a driver bug rather than
    /// a reason to take the show down.
    #[test]
    fn a_lying_audio_device_cannot_take_the_reader_down(
        samples in prop::collection::vec(-1.5f32..1.5, 0..2000),
        channels in 0u16..9,
        sample_rate in prop::sample::select(vec![0u32, 1, 8_000, 44_100, 48_000, u32::MAX]),
        channel in 0u16..8,
    ) {
        let mut manager = TimecodeManager::new();
        manager.set_ltc_input(Some(LtcInput { source_id: 1, channel, rate: None }));

        let at = Instant::now();
        manager.ingest_pcm(1, &samples, channels, sample_rate, at);
        manager.update(at);
        assert_sane(&manager, "after a lying device")?;
    }
}

/// A master is someone else's machine, and the transport is what the whole
/// renderer reads. Position has to stay somewhere a show can be rendered even
/// when what arrived is not a number: holding the last look is the worst that
/// may happen, and a black or frozen show is not.
#[test]
fn a_master_talking_nonsense_cannot_poison_the_show() {
    let sane = Chase {
        position: 10.0,
        running: true,
        discontinuity: false,
        freewheeling: false,
        speed: 1.0,
    };

    // Not a place at all: the show holds where it last was.
    for nonsense in [
        Chase {
            position: f64::NAN,
            ..sane
        },
        Chase {
            position: f64::INFINITY,
            ..sane
        },
        Chase {
            position: f64::NEG_INFINITY,
            ..sane
        },
    ] {
        let mut transport = Transport::new();
        transport.set_source(TransportSource::Timecode);
        transport.chase(sane);
        transport.chase(nonsense);

        assert!(
            (transport.position() - sane.position).abs() < 1e-9,
            "{nonsense:?} moved the show to {}",
            transport.position()
        );
    }

    // A place, but an absurd one, and a speed that is not a number: the show
    // still has to be somewhere renderable, moving at some rate.
    for nonsense in [
        Chase {
            position: -1.0e9,
            ..sane
        },
        Chase {
            position: 1.0e18,
            ..sane
        },
        Chase {
            speed: f64::NAN,
            ..sane
        },
        Chase {
            speed: f64::INFINITY,
            ..sane
        },
        Chase {
            speed: -1.0e9,
            ..sane
        },
    ] {
        let mut transport = Transport::new();
        transport.set_source(TransportSource::Timecode);
        transport.chase(sane);
        transport.chase(nonsense);
        transport.tick(1.0 / 60.0);

        let position = transport.position();
        assert!(
            position.is_finite() && position >= 0.0,
            "{nonsense:?} put the show at {position}"
        );
        assert!(
            transport.rate().is_finite(),
            "{nonsense:?} left the speed at {}",
            transport.rate()
        );
    }
}

/// Eight quarter frames make one address, and a bus that drops, repeats or
/// reorders them must not be assembled into a position anyway. Half an address
/// is not a place to put the show.
#[test]
fn a_mangled_quarter_frame_sequence_is_not_assembled_into_a_position() {
    let sent = TimecodeFrame::new(1, 2, 3, 4, TimecodeRate::Fps25);
    let hours = sent.hours | (varda::timecode::mtc::rate_bits(sent.rate) << 5);
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
    let message = |piece: usize| MidiMessage::MtcQuarterFrame {
        device_id: 5,
        data: ((piece as u8) << 4) | nibbles[piece],
    };

    // Every way of losing exactly one of the eight.
    for dropped in 0..8 {
        let mut manager = TimecodeManager::new();
        let at = Instant::now();
        for piece in (0..8).filter(|piece| *piece != dropped) {
            manager.ingest_midi(&message(piece), at);
        }
        manager.update(at);
        assert!(
            manager.inputs().is_empty(),
            "losing piece {dropped} still assembled an address"
        );
    }

    // The same eight, in the order a bus under load might deliver them.
    let mut manager = TimecodeManager::new();
    let at = Instant::now();
    for piece in [7, 0, 3, 1, 6, 2, 5, 4] {
        manager.ingest_midi(&message(piece), at);
    }
    manager.update(at);
    assert!(
        manager.inputs().is_empty(),
        "a shuffled sequence is not an address"
    );
}
