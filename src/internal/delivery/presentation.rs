//! What each kind of delivery can carry: the presentation a recording, stream,
//! Syphon server or Spout sender resolves a request to, and which modes it can
//! offer at all. Streams and recordings answer through the same ffmpeg plan
//! that runs when they start, so the picker cannot disagree with the outcome.
//! Each sink calls the function for its own kind.

use crate::delivery::StreamingProtocol;
use crate::engine::value::render::{
    AlphaMode, ModeAvailability, PresentationCapabilities, PresentationColorProfile,
    PresentationDepth, PresentationFormat, PresentationMode, PresentationPixelFormat,
    PresentationRequest, PresentationTransfer, RecordingCodec, ResolvedPresentation,
    StreamingCodec,
};

/// Every mode with the reason `resolve` cannot deliver it.
///
/// A mode is deliverable when resolving it gives no fallback reason; otherwise
/// the reason is the resolver's. Derived from the resolver, not a capability
/// table, so the picker matches the outcome.
pub fn modes_for(
    resolve: impl Fn(PresentationRequest) -> ResolvedPresentation,
) -> Vec<ModeAvailability> {
    PresentationMode::ALL
        .into_iter()
        .map(|mode| ModeAvailability {
            mode,
            blocked: resolve(PresentationRequest::default().with_mode(mode)).fallback_reason,
        })
        .collect()
}

fn resolve_eight_bit(
    request: PresentationRequest,
    pixel_format: PresentationPixelFormat,
    color_profile: PresentationColorProfile,
    alpha_mode: AlphaMode,
    fallback_reason: impl Into<String>,
) -> ResolvedPresentation {
    PresentationCapabilities::new(
        vec![PresentationFormat {
            depth: PresentationDepth::Sdr8,
            transfer: PresentationTransfer::Sdr,
            pixel_format,
            color_profile,
            alpha_mode,
        }],
        Some(fallback_reason.into()),
    )
    .resolve(request)
    .expect("every output adapter provides an eight-bit presentation format")
}

/// Eight-bit SDR, and `reason` for anything else. What a window promises
/// before its surface exists.
pub fn resolve_eight_bit_sdr(request: PresentationRequest, reason: &str) -> ResolvedPresentation {
    resolve_eight_bit(
        request,
        PresentationPixelFormat::Rgba8,
        PresentationColorProfile::SrgbFull,
        AlphaMode::Opaque,
        reason,
    )
}

/// What a Syphon server can carry: BGRA8 and nothing else.
pub fn syphon_presentation(request: PresentationRequest) -> ResolvedPresentation {
    resolve_eight_bit(
        request,
        PresentationPixelFormat::Bgra8,
        PresentationColorProfile::SrgbFull,
        AlphaMode::Premultiplied,
        "Syphon interoperability is limited to BGRA8",
    )
}

/// What a Spout sender can carry.
///
/// Spout textures can also be `R10G10B10A2`, so ten-bit SDR is offered. There
/// is no HDR mode: Spout carries no transfer function, primaries or mastering
/// metadata, so receivers could not interpret it.
///
/// Eight-bit is listed first because a blocked HDR request degrades to the
/// first non-HDR entry, and BGRA8 is the interoperable one. A ten-bit request
/// still matches exactly, since `resolve` checks for an exact match first.
///
/// # Panics
///
/// Never: the BGRA8 format is always offered, so every request resolves.
pub fn spout_presentation(request: PresentationRequest) -> ResolvedPresentation {
    let formats = vec![
        PresentationFormat {
            depth: PresentationDepth::Sdr8,
            transfer: PresentationTransfer::Sdr,
            pixel_format: PresentationPixelFormat::Bgra8,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Premultiplied,
        },
        PresentationFormat {
            depth: PresentationDepth::Sdr10,
            transfer: PresentationTransfer::Sdr,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Premultiplied,
        },
    ];
    PresentationCapabilities::new(
        formats,
        Some("Spout carries pixels with no transfer signalling; HDR cannot be expressed".into()),
    )
    .resolve(request)
    .expect("Spout always offers the BGRA8 fallback")
}

/// What a recording with `codec` delivers for `request`, resolved by the same
/// plan that runs when recording starts.
pub fn recording_presentation(
    codec: &RecordingCodec,
    request: PresentationRequest,
) -> ResolvedPresentation {
    crate::delivery::RecordingPlan::resolved_for_codec(codec, request)
}

/// What a stream over `protocol` with `codec` delivers for `request`.
pub(crate) fn streaming_presentation(
    protocol: StreamingProtocol,
    codec: &StreamingCodec,
    request: PresentationRequest,
) -> ResolvedPresentation {
    let plan = crate::delivery::StreamingPlan::for_stream(protocol, codec.clone(), request);
    debug_assert_eq!(
        plan.expected_readback(),
        if plan.resolved.resolved == PresentationDepth::Sdr10 {
            crate::renderer::ReadbackFormat::Rgb10A2
        } else {
            crate::renderer::ReadbackFormat::Rgba8
        }
    );
    plan.resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The modes `resolve` can actually deliver, for tests about the set
    /// rather than the reasons.
    fn deliverable(
        resolve: impl Fn(PresentationRequest) -> ResolvedPresentation,
    ) -> Vec<PresentationMode> {
        modes_for(resolve)
            .into_iter()
            .filter(ModeAvailability::is_available)
            .map(|entry| entry.mode)
            .collect()
    }

    fn blocked_reason(
        resolve: impl Fn(PresentationRequest) -> ResolvedPresentation,
        mode: PresentationMode,
    ) -> Option<String> {
        modes_for(resolve)
            .into_iter()
            .find(|entry| entry.mode == mode)
            .and_then(|entry| entry.blocked)
    }

    /// What a stream can carry follows its codec. SRT and HLS default to H.264,
    /// which is eight-bit.
    ///
    /// Asserts on the blocking reason rather than the set, because whether HEVC
    /// carries ten bits depends on the installed FFmpeg.
    #[test]
    fn a_streams_blocking_reason_follows_its_codec() {
        let h264 = |r| streaming_presentation(StreamingProtocol::Srt, &StreamingCodec::H264, r);
        assert_eq!(
            deliverable(h264),
            vec![PresentationMode::Sdr8],
            "H.264 streaming is eight-bit, so nothing else should be selectable"
        );
        let h264_reason =
            blocked_reason(h264, PresentationMode::Hdr10).expect("HDR10 is blocked on H.264");
        assert!(
            h264_reason.contains("H.264"),
            "the reason should name the codec the operator can change: {h264_reason}"
        );

        // On HEVC the mode is either deliverable or blocked by the installed
        // encoder, never by the H.264 reason.
        let h265 = |r| streaming_presentation(StreamingProtocol::Hls, &StreamingCodec::H265, r);
        if let Some(reason) = blocked_reason(h265, PresentationMode::Hdr10) {
            assert!(
                !reason.contains("H.264"),
                "an HEVC stream still blamed H.264, so the set went stale: {reason}"
            );
            assert!(
                reason.contains("FFmpeg") || reason.contains("encoder"),
                "the only legitimate remaining obstacle is the installed encoder: {reason}"
            );
        }
    }

    /// Spout offers ten-bit; Syphon does not.
    #[test]
    fn spout_offers_ten_bit_where_syphon_cannot() {
        assert_eq!(
            deliverable(spout_presentation),
            vec![PresentationMode::Sdr8, PresentationMode::Sdr10]
        );
        assert_eq!(
            deliverable(syphon_presentation),
            vec![PresentationMode::Sdr8]
        );
    }

    /// Spout carries no transfer signaling, so HDR is blocked with that reason.
    #[test]
    fn spout_blocks_hdr_and_explains_that_it_carries_no_transfer() {
        for mode in [
            PresentationMode::Hdr10,
            PresentationMode::Hlg,
            PresentationMode::Edr,
        ] {
            assert!(
                !deliverable(spout_presentation).contains(&mode),
                "{mode:?} must not be selectable on Spout"
            );
        }
        let resolved =
            spout_presentation(PresentationRequest::default().with_mode(PresentationMode::Hdr10));
        assert_eq!(
            resolved.transfer,
            PresentationTransfer::Sdr,
            "an HDR request must degrade rather than be published as HDR"
        );
        let reason = resolved.fallback_reason.expect("HDR is blocked");
        assert!(
            reason.contains("transfer signalling"),
            "the reason should name what Spout cannot carry: {reason}"
        );
    }

    /// HDR degrades to the interoperable eight-bit format, not ten-bit.
    #[test]
    fn an_hdr_request_on_spout_degrades_to_eight_bit_not_ten() {
        let resolved =
            spout_presentation(PresentationRequest::default().with_mode(PresentationMode::Hdr10));
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
    }

    /// No container, codec or stream carries EDR, so delivery paths never
    /// offer it.
    #[test]
    fn edr_is_never_offered_on_a_delivery_path() {
        let recording = |r| recording_presentation(&RecordingCodec::H265, r);
        let hls = |r| streaming_presentation(StreamingProtocol::Hls, &StreamingCodec::H265, r);
        let dash = |r| streaming_presentation(StreamingProtocol::Dash, &StreamingCodec::H265, r);
        assert!(!deliverable(recording).contains(&PresentationMode::Edr));
        assert!(!deliverable(hls).contains(&PresentationMode::Edr));
        assert!(!deliverable(dash).contains(&PresentationMode::Edr));
        assert!(!deliverable(syphon_presentation).contains(&PresentationMode::Edr));
    }

    /// The request degrades with a reason.
    #[test]
    fn an_edr_request_on_a_recording_degrades_and_names_the_reason() {
        let resolved = recording_presentation(
            &RecordingCodec::H265,
            PresentationRequest::default().with_mode(PresentationMode::Edr),
        );
        assert_ne!(
            resolved.transfer,
            PresentationTransfer::EdrLinear,
            "a recording must not claim to be delivering EDR"
        );
        let reason = resolved
            .fallback_reason
            .expect("an undeliverable request must explain itself");
        assert!(
            reason.contains("EDR"),
            "the reason should name EDR rather than blame the codec: {reason}"
        );
    }

    /// An idle HEVC recording resolves an HDR10 request by its codec, not as
    /// blanket eight-bit.
    #[test]
    fn an_idle_hdr_recording_reports_its_codec_not_a_blanket_eight_bit() {
        let resolved = recording_presentation(
            &RecordingCodec::H265,
            PresentationRequest::default().with_mode(PresentationMode::Hdr10),
        );
        if resolved.transfer.is_hdr() {
            assert_eq!(resolved.resolved, PresentationDepth::Sdr10);
            assert!(resolved.fallback_reason.is_none());
        } else {
            let reason = resolved
                .fallback_reason
                .expect("a degraded HDR request must explain itself");
            assert!(
                !reason.contains("the active FFmpeg path is configured for eight-bit video"),
                "idle resolution still gives the blanket answer: {reason}"
            );
        }
    }

    #[test]
    fn an_idle_recording_on_an_eight_bit_codec_names_that_codec() {
        let resolved = recording_presentation(
            &RecordingCodec::H264,
            PresentationRequest::default().with_mode(PresentationMode::Hdr10),
        );
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        let reason = resolved.fallback_reason.expect("must explain itself");
        assert!(reason.contains("H.264"), "reason was: {reason}");
    }

    #[test]
    fn syphon_eight_bit_request_has_no_fallback_reason() {
        let resolved = syphon_presentation(PresentationRequest::default());
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(resolved.pixel_format, PresentationPixelFormat::Bgra8);
        assert!(resolved.fallback_reason.is_none());
    }

    #[test]
    fn syphon_truthfully_resolves_ten_bit_requests_to_bgra8() {
        let resolved = syphon_presentation(PresentationRequest {
            depth: PresentationDepth::Sdr10,
            dither: true,
            ..PresentationRequest::default()
        });
        assert_eq!(resolved.requested, PresentationDepth::Sdr10);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(resolved.pixel_format, PresentationPixelFormat::Bgra8);
        assert_eq!(resolved.alpha_mode, AlphaMode::Premultiplied);
        assert!(resolved.fallback_reason.unwrap().contains("BGRA8"));
    }
}
