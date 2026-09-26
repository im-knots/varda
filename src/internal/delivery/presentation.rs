//! What a headless target can carry: the presentation a recording, stream,
//! NDI, Syphon, or Spout target resolves a request to, and which modes it can
//! offer at all. Streams and recordings answer through the same ffmpeg plan that
//! runs when they start, so the picker cannot disagree with the outcome.
//! See /spec/presentation-mode-offering.md.

use crate::engine::value::render::{
    AlphaMode, ModeAvailability, OutputTarget, PresentationCapabilities, PresentationColorProfile,
    PresentationDepth, PresentationFormat, PresentationMode, PresentationPixelFormat,
    PresentationRequest, PresentationTransfer, ResolvedPresentation,
};

/// The presentation `request` resolves to on `target`, and every mode with the
/// reason `target` cannot deliver it: what a headless output needs whenever its
/// target or request changes.
pub fn plan(
    target: &OutputTarget,
    request: PresentationRequest,
) -> (ResolvedPresentation, Vec<ModeAvailability>) {
    (
        resolve_headless_presentation(request, target),
        mode_availability_for_target(target),
    )
}

fn resolve_eight_bit_presentation(
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

pub fn resolve_headless_presentation(
    request: PresentationRequest,
    target: &OutputTarget,
) -> ResolvedPresentation {
    if let Some(plan) = crate::delivery::StreamingPlan::for_target(target, request) {
        debug_assert_eq!(
            plan.expected_readback(),
            if plan.resolved.resolved == PresentationDepth::Sdr10 {
                crate::renderer::ReadbackFormat::Rgb10A2
            } else {
                crate::renderer::ReadbackFormat::Rgba8
            }
        );
        return plan.resolved;
    }
    // A recording's contract depends on its configured codec, so it is resolved
    // by the same plan that will run when recording starts rather than assumed
    // to be eight-bit.
    if let Some(resolved) = crate::delivery::RecordingPlan::resolved_for_target(target, request) {
        return resolved;
    }
    if matches!(target, OutputTarget::SpoutSender { .. }) {
        return spout_presentation(request);
    }
    let (pixel_format, color_profile, alpha_mode, fallback_reason) = match target {
        OutputTarget::NdiSend { .. } => (
            PresentationPixelFormat::Uyvy,
            PresentationColorProfile::Rec709Limited,
            AlphaMode::Opaque,
            "the active NDI sender supports UYVY only",
        ),
        OutputTarget::SyphonServer { .. } => (
            PresentationPixelFormat::Bgra8,
            PresentationColorProfile::SrgbFull,
            AlphaMode::Premultiplied,
            "Syphon interoperability is limited to BGRA8",
        ),
        OutputTarget::Recording { .. }
        | OutputTarget::SrtStream { .. }
        | OutputTarget::HlsStream { .. }
        | OutputTarget::DashStream { .. }
        | OutputTarget::RtmpStream { .. } => {
            unreachable!("ffmpeg targets resolve through their own plan above")
        }
        OutputTarget::SpoutSender { .. } => {
            unreachable!("Spout resolves through spout_presentation above")
        }
        OutputTarget::Windowed | OutputTarget::Display { .. } => (
            PresentationPixelFormat::Rgba8,
            PresentationColorProfile::SrgbFull,
            AlphaMode::Opaque,
            "the output surface is configured for eight-bit SDR",
        ),
    };

    resolve_eight_bit_presentation(
        request,
        pixel_format,
        color_profile,
        alpha_mode,
        fallback_reason,
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
fn spout_presentation(request: PresentationRequest) -> ResolvedPresentation {
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

/// Every presentation mode, with the reason a headless target cannot deliver it.
///
/// Same rule as the surface path: a mode is deliverable when resolving it
/// produces no fallback reason, and the reason is the one the resolver would have
/// reported. Derived from [`resolve_headless_presentation`] itself rather than a
/// capability table, so the picker cannot disagree with the outcome.
/// See /spec/presentation-mode-offering.md.
pub fn mode_availability_for_target(target: &OutputTarget) -> Vec<ModeAvailability> {
    PresentationMode::ALL
        .into_iter()
        .map(|mode| ModeAvailability {
            mode,
            blocked: resolve_headless_presentation(
                PresentationRequest::default().with_mode(mode),
                target,
            )
            .fallback_reason,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{mode_availability_for_target, resolve_headless_presentation};
    use crate::engine::value::render::{
        AlphaMode, OutputTarget, PresentationDepth, PresentationPixelFormat, PresentationRequest,
    };

    /// The modes a target can actually deliver, for tests about the set rather
    /// than the reasons.
    fn deliverable(
        target: &crate::engine::value::render::OutputTarget,
    ) -> Vec<crate::engine::value::render::PresentationMode> {
        mode_availability_for_target(target)
            .into_iter()
            .filter(crate::engine::value::render::ModeAvailability::is_available)
            .map(|entry| entry.mode)
            .collect()
    }

    /// What a stream can carry follows its codec, so the picker follows it too.
    /// Reported from the field as a bug: switching an output from Syphon to SRT
    /// still showed only eight-bit. It is not stale state. SRT and HLS default to
    /// H.264, which is eight-bit.
    ///
    /// Asserted through the blocking *reason* rather than the resulting set,
    /// because whether HEVC can actually carry ten bits is a fact about the
    /// installed FFmpeg rather than about this code. An earlier version of this
    /// test asserted the set and failed on any machine whose libx265 lacks
    /// `yuv420p10le`, which made an environment difference look like a defect.
    #[test]
    fn a_streams_blocking_reason_follows_its_codec() {
        use crate::engine::value::render::{
            ModeAvailability, OutputTarget, PresentationMode, SrtCodec, StreamingCodec,
        };

        fn blocked_reason(target: &OutputTarget, mode: PresentationMode) -> Option<String> {
            mode_availability_for_target(target)
                .into_iter()
                .find(|entry: &ModeAvailability| entry.mode == mode)
                .and_then(|entry| entry.blocked)
        }

        // H.264 is eight-bit whatever FFmpeg is installed, so this half is pure.
        let h264 = OutputTarget::SrtStream {
            url: "srt://example:9000".into(),
            codec: SrtCodec::H264,
            audio_device: None,
        };
        assert_eq!(
            deliverable(&h264),
            vec![PresentationMode::Sdr8],
            "H.264 streaming is eight-bit, so nothing else should be selectable"
        );
        let h264_reason =
            blocked_reason(&h264, PresentationMode::Hdr10).expect("HDR10 is blocked on H.264");
        assert!(
            h264_reason.contains("H.264"),
            "the reason should name the codec the operator can change: {h264_reason}"
        );

        // On HEVC the codec stops being the obstacle. Either the mode opens up, or
        // the obstacle becomes the installed encoder, which is a different answer
        // and a different thing for the operator to fix. What must never happen is
        // the H.264 reason surviving a codec change: that is the staleness this
        // test exists to catch.
        let h265 = OutputTarget::HlsStream {
            name: "show".into(),
            codec: StreamingCodec::H265,
            short_segments: false,
            audio_device: None,
        };
        if let Some(reason) = blocked_reason(&h265, PresentationMode::Hdr10) {
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
    ///
    /// Syphon is BGRA8 and nothing else, so its ten-bit request is a fallback
    /// warning. Spout's shared textures can be `R10G10B10A2`, so ten-bit is a
    /// real choice there. Copying the Syphon contract would have undersold it.
    #[test]
    fn spout_offers_ten_bit_where_syphon_cannot() {
        use crate::engine::value::render::{OutputTarget, PresentationMode};
        let spout = OutputTarget::SpoutSender {
            sender_name: "Varda".into(),
        };
        let syphon = OutputTarget::SyphonServer {
            server_name: "Varda".into(),
        };
        assert_eq!(
            deliverable(&spout),
            vec![PresentationMode::Sdr8, PresentationMode::Sdr10]
        );
        assert_eq!(deliverable(&syphon), vec![PresentationMode::Sdr8]);
    }

    /// Spout shares a texture and carries no transfer signalling, so there is no
    /// HDR mode to offer and the reason has to say that rather than blame a
    /// codec the user could change.
    #[test]
    fn spout_blocks_hdr_and_explains_that_it_carries_no_transfer() {
        use crate::engine::value::render::{
            OutputTarget, PresentationMode, PresentationRequest, PresentationTransfer,
        };
        let target = OutputTarget::SpoutSender {
            sender_name: "Varda".into(),
        };
        for mode in [
            PresentationMode::Hdr10,
            PresentationMode::Hlg,
            PresentationMode::Edr,
        ] {
            assert!(
                !deliverable(&target).contains(&mode),
                "{mode:?} must not be selectable on Spout"
            );
        }
        let resolved = super::resolve_headless_presentation(
            PresentationRequest::default().with_mode(PresentationMode::Hdr10),
            &target,
        );
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
    ///
    /// Every Spout receiver understands BGRA8; a ten-bit surface handed to one
    /// that assumes BGRA8 is misread rather than refused, so an HDR request must
    /// not silently become ten-bit.
    #[test]
    fn an_hdr_request_on_spout_degrades_to_eight_bit_not_ten() {
        use crate::engine::value::render::{
            OutputTarget, PresentationDepth, PresentationMode, PresentationRequest,
        };
        let resolved = super::resolve_headless_presentation(
            PresentationRequest::default().with_mode(PresentationMode::Hdr10),
            &OutputTarget::SpoutSender {
                sender_name: "Varda".into(),
            },
        );
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
    }

    /// EDR is a display monitoring contract. No container, codec, or stream
    /// carries it, so it must never be offered on a delivery path.
    ///
    /// This was a live defect rather than only a menu problem: `EdrLinear`
    /// satisfies `is_hdr`, so an EDR request on an HEVC recording resolved with no
    /// fallback reason and was then written out as plain Rec.709, with the output
    /// card still reporting EDR.
    #[test]
    fn edr_is_never_offered_on_a_delivery_path() {
        use crate::engine::value::render::{
            OutputTarget, PresentationMode, RecordingCodec, StreamingCodec,
        };
        let targets = [
            OutputTarget::Recording {
                path: "/tmp/take.mov".into(),
                codec: RecordingCodec::H265,
                audio_device: None,
            },
            OutputTarget::HlsStream {
                name: "show".into(),
                codec: StreamingCodec::H265,
                short_segments: false,
                audio_device: None,
            },
            OutputTarget::DashStream {
                name: "show".into(),
                codec: StreamingCodec::H265,
                audio_device: None,
            },
            OutputTarget::SyphonServer {
                server_name: "Varda".into(),
            },
        ];
        for target in targets {
            assert!(
                !deliverable(&target).contains(&PresentationMode::Edr),
                "EDR was offered on {target:?}"
            );
        }
    }

    /// The half of the same defect that a menu alone would not have fixed: the
    /// request must degrade and say why, not resolve silently.
    #[test]
    fn an_edr_request_on_a_recording_degrades_and_names_the_reason() {
        use crate::engine::value::render::{
            OutputTarget, PresentationMode, PresentationRequest, PresentationTransfer,
            RecordingCodec,
        };
        let resolved = super::resolve_headless_presentation(
            PresentationRequest::default().with_mode(PresentationMode::Edr),
            &OutputTarget::Recording {
                path: "/tmp/take.mov".into(),
                codec: RecordingCodec::H265,
                audio_device: None,
            },
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

    #[test]
    fn an_idle_hdr_recording_reports_its_codec_not_a_blanket_eight_bit() {
        use crate::engine::value::render::{OutputTarget, PresentationMode, RecordingCodec};
        // Reported from the field: an HDR10 request on an HEVC recording showed
        // "Delivering 8-bit SDR" with "the active FFmpeg path is configured for
        // eight-bit video" before Start was pressed. The idle resolver answered
        // eight-bit for every codec, so a correctly configured output reported a
        // fallback it would not actually take.
        let request = PresentationRequest::default().with_mode(PresentationMode::Hdr10);
        let target = OutputTarget::Recording {
            path: "out.mp4".to_string(),
            codec: RecordingCodec::H265,
            audio_device: None,
        };
        let resolved = resolve_headless_presentation(request, &target);

        if resolved.transfer.is_hdr() {
            assert_eq!(resolved.resolved, PresentationDepth::Sdr10);
            assert!(resolved.fallback_reason.is_none());
        } else {
            // No ten-bit libx265 installed is a legitimate answer, but the reason
            // must name the encoder rather than claim the path is eight-bit.
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
        use crate::engine::value::render::{OutputTarget, PresentationMode, RecordingCodec};
        let request = PresentationRequest::default().with_mode(PresentationMode::Hdr10);
        let target = OutputTarget::Recording {
            path: "out.mp4".to_string(),
            codec: RecordingCodec::H264,
            audio_device: None,
        };
        let resolved = resolve_headless_presentation(request, &target);

        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        let reason = resolved.fallback_reason.expect("must explain itself");
        assert!(reason.contains("H.264"), "reason was: {reason}");
    }

    #[test]
    fn syphon_eight_bit_request_has_no_fallback_reason() {
        let resolved = resolve_headless_presentation(
            PresentationRequest::default(),
            &OutputTarget::SyphonServer {
                server_name: "Precision Test".into(),
            },
        );
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(resolved.pixel_format, PresentationPixelFormat::Bgra8);
        assert!(resolved.fallback_reason.is_none());
    }

    #[test]
    fn syphon_truthfully_resolves_ten_bit_requests_to_bgra8() {
        let resolved = resolve_headless_presentation(
            PresentationRequest {
                depth: PresentationDepth::Sdr10,
                dither: true,
                ..PresentationRequest::default()
            },
            &OutputTarget::SyphonServer {
                server_name: "Precision Test".into(),
            },
        );

        assert_eq!(resolved.requested, PresentationDepth::Sdr10);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(resolved.pixel_format, PresentationPixelFormat::Bgra8);
        assert_eq!(resolved.alpha_mode, AlphaMode::Premultiplied);
        assert!(resolved.fallback_reason.unwrap().contains("BGRA8"));
    }
}
