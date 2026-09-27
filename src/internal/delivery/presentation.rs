//! What each kind of delivery can carry: the presentation a recording, stream,
//! Syphon server or Spout sender resolves a request to, and which modes it can
//! offer at all. Streams and recordings answer through the same ffmpeg plan
//! that runs when they start, so the picker cannot disagree with the outcome.
//! Each sink calls the function for its own kind. See
//! /spec/presentation-mode-offering.md.

use crate::delivery::StreamingProtocol;
use crate::engine::value::render::{
    AlphaMode, ModeAvailability, PresentationCapabilities, PresentationColorProfile,
    PresentationDepth, PresentationFormat, PresentationMode, PresentationPixelFormat,
    PresentationRequest, PresentationTransfer, RecordingCodec, ResolvedPresentation,
    StreamingCodec,
};

/// Every mode with the reason `resolve` cannot deliver it.
///
/// A mode is deliverable when resolving it produces no fallback reason, and
/// the reason is the one the resolver would have reported. Derived from the
/// resolver itself rather than a capability table, so the picker cannot
/// disagree with the outcome.
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
/// Unlike Syphon, which is BGRA8 and nothing else, Spout's shared textures can be
/// `R10G10B10A2` as well, so ten-bit SDR is a real option here rather than a
/// fallback warning.
///
/// There is no HDR mode and there will not be one. Spout shares a texture and
/// carries no transfer function, primaries, or mastering metadata, so declaring a
/// float format and calling it HDR would be a private convention no receiver
/// could honour. That is the same reasoning that rules out NDI HDR in
/// /spec/hdr-output.md.
///
/// Eight-bit is listed first because order decides where a blocked request
/// lands: an HDR request degrades to the first non-HDR entry, and BGRA8 is the
/// interoperable one. A ten-bit request still matches exactly, since `resolve`
/// looks for an exact depth and transfer before it considers the order.
/// See /spec/spout-output.md § Presentation contract.
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
/// plan that runs when recording starts rather than assumed to be eight-bit.
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

    /// What a stream can carry follows its codec, so the picker follows it too.
    /// Reported from the field as a bug: switching an output from Syphon to SRT
    /// still showed only eight-bit. It is not stale state. SRT and HLS default to
    /// H.264, which is eight-bit.
    ///
    /// Asserted through the blocking *reason* rather than the resulting set,
    /// because whether HEVC can actually carry ten bits is a fact about the
    /// installed FFmpeg rather than about this code.
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

        // On HEVC the codec stops being the obstacle. Either the mode opens up,
        // or the obstacle becomes the installed encoder. What must never happen
        // is the H.264 reason surviving a codec change.
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

    /// Spout carries more than Syphon does, and the picker should say so.
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

    /// Spout shares a texture and carries no transfer signalling, so there is no
    /// HDR mode to offer and the reason has to say that.
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

    /// The degrade lands on the interoperable format, not merely the best one.
    #[test]
    fn an_hdr_request_on_spout_degrades_to_eight_bit_not_ten() {
        let resolved =
            spout_presentation(PresentationRequest::default().with_mode(PresentationMode::Hdr10));
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
    }

    /// EDR is a display monitoring contract. No container, codec, or stream
    /// carries it, so it must never be offered on a delivery path.
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

    /// The request must degrade and say why, not resolve silently.
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

    /// Reported from the field: an HDR10 request on an HEVC recording showed
    /// eight-bit before Start was pressed, because the idle resolver answered
    /// eight-bit for every codec.
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
