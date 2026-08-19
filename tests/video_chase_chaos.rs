//! Offensive tests for the video-deck transport chase servo.
//!
//! Chase configuration can come from a hand-edited scene, the HTTP API, or an
//! in-process command, while the transport is fed by external timecode. None of
//! those values may panic the decode thread, produce a non-finite clip position,
//! or escape the clip's playable range.
//!
//! See /spec/timecode.md § Consumer 2.

use proptest::prelude::*;
use varda::video::chase::{step_chase, ChaseInput};
use varda::video::DeckTransportSync;

fn assert_sane_step(input: ChaseInput) -> Result<(), TestCaseError> {
    let out = step_chase(input);
    prop_assert!(
        out.position.is_finite(),
        "chase produced non-finite position {} from {input:?}",
        out.position
    );
    prop_assert!(
        out.position >= 0.0,
        "chase produced negative position {} from {input:?}",
        out.position
    );
    prop_assert!(
        out.frames_to_decode <= 1,
        "one servo tick requested {} frames from {input:?}",
        out.frames_to_decode
    );
    Ok(())
}

proptest! {
    /// Every input is deliberately unconstrained. This includes NaN,
    /// infinities, subnormals, inverted ranges, extreme delays, and values
    /// large enough to overflow intermediate multiplication.
    #[test]
    fn arbitrary_sync_state_cannot_poison_the_decoder(
        position in any::<f64>(),
        in_point in any::<f64>(),
        out_point in any::<f64>(),
        frame_rate in any::<f64>(),
        base_speed in any::<f64>(),
        transport_position in any::<f64>(),
        transport_dt in any::<f64>(),
        transport_fps in any::<f64>(),
        offset in any::<f64>(),
        delay_frames in any::<i32>(),
        discontinuity in any::<bool>(),
    ) {
        assert_sane_step(ChaseInput {
            position,
            in_point,
            out_point,
            frame_rate,
            base_speed,
            transport_position,
            transport_dt,
            transport_fps,
            discontinuity,
            sync: DeckTransportSync {
                offset,
                delay_frames,
                ..DeckTransportSync::default()
            },
        })?;
    }
}

#[test]
fn named_non_finite_cases_hold_a_renderable_frame() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for input in [
            ChaseInput {
                transport_position: bad,
                ..sane_input()
            },
            ChaseInput {
                transport_dt: bad,
                ..sane_input()
            },
            ChaseInput {
                base_speed: bad,
                ..sane_input()
            },
            ChaseInput {
                frame_rate: bad,
                ..sane_input()
            },
            ChaseInput {
                transport_fps: bad,
                ..sane_input()
            },
            ChaseInput {
                sync: DeckTransportSync {
                    offset: bad,
                    ..DeckTransportSync::default()
                },
                ..sane_input()
            },
        ] {
            assert_sane_step(input).expect("named hostile input stayed sane");
        }
    }
}

fn sane_input() -> ChaseInput {
    ChaseInput {
        position: 2.0,
        in_point: 1.0,
        out_point: 8.0,
        frame_rate: 30.0,
        base_speed: 1.0,
        transport_position: 2.0,
        transport_dt: 1.0 / 30.0,
        transport_fps: 30.0,
        discontinuity: false,
        sync: DeckTransportSync::default(),
    }
}
