//! Offensive tests for the output presentation contract.
//!
//! A presentation request can come from a hand-edited `stage.json`, the HTTP
//! API, or an in-process command, and it is resolved against whatever the
//! installed GPU, FFmpeg build, and NDI runtime happen to offer. None of those
//! combinations may panic, invent a contract the adapter did not advertise, or
//! quietly rewrite what the user asked for.
//!
//! See /spec/sdr-presentation-precision.md § Negotiation Rules and
//! /spec/hdr-per-output-encode.md § Domain contract extension.

use proptest::prelude::*;
use varda::engine::value::render::{
    AlphaMode, HDR_PEAK_NITS_MAX, HDR_PEAK_NITS_MIN, PresentationCapabilities,
    PresentationColorProfile, PresentationDepth, PresentationFormat, PresentationMode,
    PresentationPixelFormat, PresentationRequest, PresentationTransfer, TonemapMode,
};

fn any_depth() -> impl Strategy<Value = PresentationDepth> {
    prop_oneof![
        Just(PresentationDepth::Sdr8),
        Just(PresentationDepth::Sdr10)
    ]
}

fn any_transfer() -> impl Strategy<Value = PresentationTransfer> {
    prop_oneof![
        Just(PresentationTransfer::Sdr),
        Just(PresentationTransfer::Hdr10Pq),
        Just(PresentationTransfer::Hlg),
    ]
}

fn any_pixel_format() -> impl Strategy<Value = PresentationPixelFormat> {
    prop_oneof![
        Just(PresentationPixelFormat::Rgba8),
        Just(PresentationPixelFormat::Bgra8),
        Just(PresentationPixelFormat::Rgb10A2),
        Just(PresentationPixelFormat::Rgba16),
        Just(PresentationPixelFormat::Uyvy),
        Just(PresentationPixelFormat::P216),
        ".{0,12}".prop_map(PresentationPixelFormat::EncoderNative),
    ]
}

fn any_profile() -> impl Strategy<Value = PresentationColorProfile> {
    prop_oneof![
        Just(PresentationColorProfile::SrgbFull),
        Just(PresentationColorProfile::Rec709Limited),
        Just(PresentationColorProfile::Rec709Full),
        Just(PresentationColorProfile::Pq2020Limited),
        Just(PresentationColorProfile::Pq2020Full),
    ]
}

fn any_alpha() -> impl Strategy<Value = AlphaMode> {
    prop_oneof![
        Just(AlphaMode::Opaque),
        Just(AlphaMode::Premultiplied),
        Just(AlphaMode::Straight),
    ]
}

fn any_request() -> impl Strategy<Value = PresentationRequest> {
    (any_depth(), any::<bool>(), any_transfer(), any::<u16>()).prop_map(
        |(depth, dither, transfer, peak_nits)| PresentationRequest {
            depth,
            dither,
            transfer,
            peak_nits,
        },
    )
}

fn any_format() -> impl Strategy<Value = PresentationFormat> {
    (
        any_depth(),
        any_transfer(),
        any_pixel_format(),
        any_profile(),
        any_alpha(),
    )
        .prop_map(
            |(depth, transfer, pixel_format, color_profile, alpha_mode)| PresentationFormat {
                depth,
                transfer,
                pixel_format,
                color_profile,
                alpha_mode,
            },
        )
}

proptest! {
    /// An adapter advertising an arbitrary format list must never make the
    /// resolver panic, and whatever comes back must be something the adapter
    /// actually offered.
    #[test]
    fn resolution_only_ever_selects_an_advertised_format(
        request in any_request(),
        formats in prop::collection::vec(any_format(), 0..6),
        reason in prop::option::of(".{0,40}"),
    ) {
        let caps = PresentationCapabilities::new(formats.clone(), reason);
        let Ok(resolved) = caps.resolve(request) else {
            // No usable format is a legitimate answer, not a panic.
            return Ok(());
        };
        prop_assert!(
            formats.iter().any(|f| {
                f.depth == resolved.resolved
                    && f.transfer == resolved.transfer
                    && f.pixel_format == resolved.pixel_format
                    && f.color_profile == resolved.color_profile
                    && f.alpha_mode == resolved.alpha_mode
            }),
            "resolver invented a contract no adapter advertised: {resolved:?} from {formats:?}"
        );
    }

    /// The Phase 49 rule that survives every later phase: a capability failure is
    /// runtime state and must never rewrite what the user stored.
    #[test]
    fn resolution_never_rewrites_the_request(
        request in any_request(),
        formats in prop::collection::vec(any_format(), 0..6),
    ) {
        let caps = PresentationCapabilities::new(formats, None);
        let Ok(resolved) = caps.resolve(request) else {
            return Ok(());
        };
        let normalized = request.normalized();
        prop_assert_eq!(resolved.requested, normalized.depth);
        prop_assert_eq!(resolved.requested_transfer, normalized.transfer);
    }

    /// Degrading is always reported, and never reported when nothing degraded.
    #[test]
    fn a_fallback_reason_appears_exactly_when_something_degraded(
        request in any_request(),
        formats in prop::collection::vec(any_format(), 0..6),
    ) {
        let caps = PresentationCapabilities::new(formats, None);
        let Ok(resolved) = caps.resolve(request) else {
            return Ok(());
        };
        let normalized = request.normalized();
        let degraded =
            resolved.resolved != normalized.depth || resolved.transfer != normalized.transfer;
        prop_assert_eq!(
            degraded,
            resolved.fallback_reason.is_some(),
            "degraded={} but reason={:?}",
            degraded,
            resolved.fallback_reason
        );
    }

    /// An SDR request must never come back HDR, whatever an adapter advertises
    /// or in whatever order. Delivering unrequested HDR would silently change
    /// the picture a show ships.
    #[test]
    fn an_sdr_request_never_resolves_to_hdr(
        depth in any_depth(),
        dither in any::<bool>(),
        formats in prop::collection::vec(any_format(), 0..6),
    ) {
        let request = PresentationRequest {
            depth,
            dither,
            transfer: PresentationTransfer::Sdr,
            ..PresentationRequest::default()
        };
        let caps = PresentationCapabilities::new(formats, None);
        let Ok(resolved) = caps.resolve(request) else {
            return Ok(());
        };
        prop_assert!(
            !resolved.transfer.is_hdr(),
            "an SDR request resolved to {:?}",
            resolved.transfer
        );
    }

    /// Peak luminance is reported only where it means something, and only within
    /// the representable range. A peak on an SDR output would be a number the
    /// operator could act on that describes nothing.
    #[test]
    fn peak_is_present_only_for_a_transfer_that_uses_one(
        request in any_request(),
        formats in prop::collection::vec(any_format(), 0..6),
    ) {
        let caps = PresentationCapabilities::new(formats, None);
        let Ok(resolved) = caps.resolve(request) else {
            return Ok(());
        };
        prop_assert_eq!(
            resolved.peak_nits.is_some(),
            resolved.transfer.uses_peak_nits(),
            "peak {:?} against transfer {:?}",
            resolved.peak_nits,
            resolved.transfer
        );
        if let Some(peak) = resolved.peak_nits {
            prop_assert!(
                (HDR_PEAK_NITS_MIN..=HDR_PEAK_NITS_MAX).contains(&peak),
                "peak {peak} escaped the representable range"
            );
        }
    }

    /// Normalization is idempotent and always lands somewhere coherent, however
    /// incoherent the stored request was.
    #[test]
    fn normalization_reaches_a_fixed_point_in_one_step(request in any_request()) {
        let once = request.normalized();
        let twice = once.normalized();
        prop_assert_eq!(once, twice, "normalization is not idempotent");
        prop_assert!(
            (HDR_PEAK_NITS_MIN..=HDR_PEAK_NITS_MAX).contains(&once.peak_nits),
            "peak {} escaped the range", once.peak_nits
        );
        prop_assert!(
            !once.transfer.is_hdr() || once.depth == PresentationDepth::Sdr10,
            "an HDR contract survived at eight bits"
        );
    }

    /// The user-facing picker and the domain must agree in both directions, so
    /// no reachable request displays as a mode that would set something else.
    #[test]
    fn mode_and_request_agree_in_both_directions(
        request in any_request(),
        mode in prop_oneof![
            Just(PresentationMode::Sdr8),
            Just(PresentationMode::Sdr10),
            Just(PresentationMode::Hdr10),
            Just(PresentationMode::Hlg),
        ],
    ) {
        let applied = request.with_mode(mode);
        prop_assert_eq!(
            applied.mode(),
            mode,
            "mode did not round trip through {:?}",
            applied
        );
        // Switching contract must not silently discard the operator's other
        // settings.
        prop_assert_eq!(applied.dither, request.dither);
        prop_assert_eq!(applied.peak_nits, request.peak_nits);
    }
}

proptest! {
    /// A curve fitted against an SDR target must never reach an HDR output
    /// transform, from any entry point. This is the gate that stops a plausible
    /// wrong picture, so it is asserted over the whole enum rather than the two
    /// cases the UI happens to offer.
    #[test]
    fn no_sdr_fitted_curve_ever_reaches_an_hdr_output(
        index in 0usize..TonemapMode::ALL.len(),
        peak in HDR_PEAK_NITS_MIN..=HDR_PEAK_NITS_MAX,
    ) {
        let mode = TonemapMode::ALL[index];
        let key = varda::mixer::ProgramKey::for_output(mode, Some(peak));
        prop_assert!(
            key.tonemap.has_hdr_form(),
            "{mode:?} reached an HDR output without an HDR form"
        );
        prop_assert!(key.headroom() > 1.0, "an HDR program must target more than display white");
    }

    /// SDR is left alone: every curve is inside its fitted range there, so
    /// substituting would change a picture that was already correct.
    #[test]
    fn sdr_outputs_keep_every_curve_untouched(index in 0usize..TonemapMode::ALL.len()) {
        let mode = TonemapMode::ALL[index];
        let key = varda::mixer::ProgramKey::for_output(mode, None);
        prop_assert_eq!(key.tonemap, mode);
        prop_assert!((key.headroom() - 1.0).abs() < f32::EPSILON);
    }
}
