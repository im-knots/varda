//! Render/output configuration value types — plain, serializable data (no
//! `wgpu` / `winit` / `egui`) that describes *what* to render and *where* to
//! send it. Formerly `internal::renderer::config`; relocated here per
//! /spec/engine-value-types.md so the engine contract layer names these types
//! directly instead of reaching into `internal::renderer`. The GPU modules
//! (`context`, `tonemap`, `edge_blend`) `pub use` these to keep their existing
//! call paths working, and attach any framework-specific inherent impls there.

// ── SDR presentation precision ──────────────────────────────────────

/// Requested and resolved SDR integer precision at an output boundary.
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
#[serde(rename_all = "snake_case")]
pub enum PresentationDepth {
    /// Eight-bit SDR presentation.
    #[default]
    Sdr8,
    /// Ten-bit SDR presentation.
    Sdr10,
}

impl PresentationDepth {
    /// Values in user-facing order.
    pub const ALL: [Self; 2] = [Self::Sdr8, Self::Sdr10];

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sdr8 => "8-bit SDR",
            Self::Sdr10 => "10-bit SDR",
        }
    }
}

/// ITU-R BT.2408 HDR Reference White, in cd/m².
///
/// Linear 1.0 maps here, so existing SDR content keeps the apparent brightness it
/// has today and the range above 1.0 that the mixer tonemap used to discard becomes
/// the HDR gain. See /spec/hdr-color-management.md Decision 2.
pub const HDR_REFERENCE_WHITE_NITS: f32 = 203.0;

/// Default per-output peak luminance for HDR presentation, in cd/m².
pub const HDR_DEFAULT_PEAK_NITS: u16 = 1000;

/// Lowest peak luminance an output may request, in cd/m².
///
/// Must exceed [`HDR_REFERENCE_WHITE_NITS`], or the contract stops meaning
/// anything: a peak at or below reference white leaves no headroom above display
/// white, so an "HDR" output could not reach the brightness an SDR one already
/// shows. This floor was 100, which put headroom at 0.49 and had the CPU and the
/// shader disagreeing, because `tonemap.wgsl` clamps headroom at 1.0 and
/// `linear_headroom` did not. Found by `tests/presentation_chaos.rs`.
///
/// 400 is about one stop over reference white and matches the lowest commonly
/// cited HDR display tier.
pub const HDR_PEAK_NITS_MIN: u16 = 400;
/// Upper bound of requestable peak luminance, in cd/m² (the PQ signal ceiling).
pub const HDR_PEAK_NITS_MAX: u16 = 10_000;

/// Transfer and dynamic-range contract requested at an output boundary.
///
/// Deliberately a sibling of [`PresentationDepth`] rather than a widening of it:
/// bit depth is integer precision, dynamic range is a different axis, and
/// conflating them is the mistake /spec/sdr-presentation-precision.md was written
/// to avoid.
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
#[serde(rename_all = "snake_case")]
pub enum PresentationTransfer {
    /// Standard dynamic range: sRGB or Rec.709 transfer with Rec.709 primaries.
    #[default]
    Sdr,
    /// HDR10: ST 2084 (PQ) transfer, BT.2020 primaries, static mastering metadata.
    Hdr10Pq,
    /// HLG: ARIB STD-B67 transfer, BT.2020 primaries, no mastering metadata.
    ///
    /// Relative rather than absolute, so it carries no peak and needs no content
    /// light level. That is why it is the default for live paths, which have no
    /// finalize step in which `MaxCLL` could be measured.
    Hlg,
    /// Apple EDR: linear scRGB, Rec.709 primaries, no transfer encode at all.
    ///
    /// The one HDR contract where Varda stops encoding rather than encoding
    /// differently. `1.0` is the display's SDR white, which is what Varda's
    /// linear 1.0 already means, so values are written unchanged and the
    /// compositor clips at whatever headroom it currently allows.
    ///
    /// A monitoring contract, never delivery. See /spec/hdr-edr-display.md.
    EdrLinear,
}

impl PresentationTransfer {
    /// Values in user-facing order.
    pub const ALL: [Self; 4] = [Self::Sdr, Self::Hdr10Pq, Self::Hlg, Self::EdrLinear];

    /// Human-readable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Sdr => "SDR",
            Self::Hdr10Pq => "HDR10 (PQ)",
            Self::Hlg => "HLG",
            Self::EdrLinear => "EDR",
        }
    }

    /// Whether this contract carries high dynamic range.
    #[must_use]
    pub const fn is_hdr(self) -> bool {
        matches!(self, Self::Hdr10Pq | Self::Hlg | Self::EdrLinear)
    }

    /// Whether this contract encodes a transfer function at all.
    ///
    /// EDR does not: it writes linear values to an extended-range surface. Every
    /// other contract, SDR included, encodes something.
    #[must_use]
    pub const fn encodes_a_transfer(self) -> bool {
        !matches!(self, Self::EdrLinear)
    }

    /// Whether this contract can be carried by a file or a network stream.
    ///
    /// EDR cannot. It is a monitoring contract that writes linear values to an
    /// extended-range display surface, and no container, codec, or stream carries
    /// it: there is nothing to signal and nothing that would read the signal.
    ///
    /// Deliberately not the negation of [`is_hdr`](Self::is_hdr), which EDR does
    /// satisfy. Treating "is HDR" as "is deliverable" is what let an EDR request
    /// resolve cleanly on an HEVC recording and then be written out as plain
    /// Rec.709, with the output card still claiming EDR.
    #[must_use]
    pub const fn is_deliverable(self) -> bool {
        !matches!(self, Self::EdrLinear)
    }

    /// Whether this contract carries ST 2086 and CTA-861.3 mastering metadata.
    ///
    /// PQ is absolute and needs it. HLG is relative and needs none, which is what
    /// removes the declared-versus-measured `MaxCLL` problem on live paths.
    #[must_use]
    pub const fn carries_mastering_metadata(self) -> bool {
        matches!(self, Self::Hdr10Pq)
    }

    /// Whether the peak-luminance setting means anything for this contract.
    #[must_use]
    pub const fn uses_peak_nits(self) -> bool {
        // EDR's peak is not a display property; it names the deliverable being
        // monitored, which is what makes the preview comparable to the file.
        matches!(self, Self::Hdr10Pq | Self::EdrLinear)
    }
}

/// Framework-free pixel format selected for an output adapter.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PresentationPixelFormat {
    /// Eight-bit RGBA.
    Rgba8,
    /// Eight-bit BGRA.
    Bgra8,
    /// Packed ten-bit RGB with two unused or alpha bits.
    Rgb10A2,
    /// Sixteen-bit RGBA storage used by alpha-capable adapters.
    Rgba16,
    /// Eight-bit packed 4:2:2 YUV.
    Uyvy,
    /// NDI high-bit 4:2:2 format.
    P216,
    /// Adapter-native encoded format not represented by a GPU texture enum.
    EncoderNative(String),
}

impl std::fmt::Display for PresentationPixelFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rgba8 => write!(f, "RGBA8"),
            Self::Bgra8 => write!(f, "BGRA8"),
            Self::Rgb10A2 => write!(f, "RGB10A2"),
            Self::Rgba16 => write!(f, "RGBA16"),
            Self::Uyvy => write!(f, "UYVY"),
            Self::P216 => write!(f, "P216"),
            Self::EncoderNative(name) => f.write_str(name),
        }
    }
}

/// Transfer, primaries, matrix, and range contract for presentation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PresentationColorProfile {
    /// Full-range sRGB display presentation.
    SrgbFull,
    /// Limited-range Rec.709 video presentation.
    Rec709Limited,
    /// Full-range Rec.709 video presentation.
    Rec709Full,
    /// Limited-range PQ with BT.2020 primaries and non-constant-luminance matrix.
    Pq2020Limited,
    /// Full-range PQ with BT.2020 primaries.
    Pq2020Full,
}

/// Alpha representation at the output boundary.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    /// Output does not preserve alpha.
    Opaque,
    /// RGB is premultiplied by alpha.
    Premultiplied,
    /// RGB and alpha are independent.
    Straight,
}

/// The presentation contract a user picks, as one choice.
///
/// Depth and transfer are separate axes in the domain, but not every pairing is
/// meaningful: HDR10 is a ten-bit contract by definition, and an eight-bit PQ
/// signal is not a picture anyone would ship. Offering the combinations as one
/// list keeps the meaningless ones unreachable from the UI and the API instead
/// of relying on validation to reject them afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationMode {
    /// Eight-bit SDR, the compatibility default.
    Sdr8,
    /// Ten-bit SDR: more code values, same Rec.709 picture.
    Sdr10,
    /// HDR10: PQ transfer, BT.2020 primaries, ten-bit.
    Hdr10,
    /// HLG: relative HDR with no mastering metadata. The live-path default.
    Hlg,
    /// Apple EDR: linear extended range for monitoring on a capable display.
    Edr,
}

impl PresentationMode {
    /// Values in user-facing order.
    pub const ALL: [Self; 5] = [Self::Sdr8, Self::Sdr10, Self::Hdr10, Self::Hlg, Self::Edr];

    /// Human-readable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Sdr8 => "8-bit SDR",
            Self::Sdr10 => "10-bit SDR",
            Self::Hdr10 => "HDR10",
            Self::Hlg => "HLG",
            Self::Edr => "EDR (monitor)",
        }
    }

    /// One line on what choosing this does, for the picker.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Sdr8 => "Compatibility default. Eight-bit Rec.709.",
            Self::Sdr10 => "More code values after tonemap. Same brightness and gamut.",
            Self::Hdr10 => "PQ transfer and BT.2020 container. HDR range, not wide gamut.",
            Self::Hlg => "Relative HDR for live paths. No mastering metadata needed.",
            Self::Edr => "Linear extended range for monitoring on this Mac. Not a delivery format.",
        }
    }

    /// Whether this contract carries high dynamic range.
    #[must_use]
    pub const fn is_hdr(self) -> bool {
        matches!(self, Self::Hdr10 | Self::Hlg | Self::Edr)
    }
}

/// One entry in an output's format picker: a mode, and why it is unavailable.
///
/// The picker lists every mode and disables the blocked ones with their reason,
/// rather than hiding them. Hiding is honest but silent, and a user who knows
/// their protocol carries HDR is owed the actual obstacle, which is usually a
/// codec set elsewhere on the same card.
///
/// See /spec/presentation-mode-offering.md.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModeAvailability {
    pub mode: PresentationMode,
    /// `None` when this output can deliver the mode. Otherwise the reason it
    /// cannot, taken from the resolver that would have degraded the request.
    pub blocked: Option<String>,
}

impl ModeAvailability {
    /// Whether this mode can be selected.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.blocked.is_none()
    }
}

/// Peak luminance values offered in the picker, in cd/m².
///
/// Deliverable targets and common LED-processor ceilings, not a continuous
/// range: a peak nobody masters to is not a useful choice.
pub const HDR_PEAK_NITS_PRESETS: [u16; 4] = [600, 1000, 1500, 4000];

/// Where an HDR output's content light level metadata came from.
///
/// CTA-861.3 expects `MaxCLL` and `MaxFALL` to describe the *content*, measured
/// across the programme. Declaring them from the configured peak is true by
/// construction only because the encoder clamps the signal to that peak, and it
/// is still not what the standard asks for, so which one is in force is reported
/// rather than assumed. See /spec/hdr-recording-output.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HdrMetadataSource {
    /// Derived from the output's configured peak, with the signal clamped to it.
    DeclaredFromPeak,
    /// Measured across the recorded programme and written at finalize.
    MeasuredFromContent,
}

impl HdrMetadataSource {
    /// Human-readable status for the output card.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::DeclaredFromPeak => "declared from peak",
            Self::MeasuredFromContent => "measured from content",
        }
    }
}

/// Persisted precision and dithering request for one output.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub struct PresentationRequest {
    /// Requested integer code precision.
    #[serde(default, rename = "presentation_depth")]
    pub depth: PresentationDepth,
    /// Whether deterministic destination-aware dithering is enabled.
    #[serde(default = "presentation_dither_default")]
    pub dither: bool,
    /// Requested transfer and dynamic-range contract.
    #[serde(default)]
    pub transfer: PresentationTransfer,
    /// Peak luminance in cd/m², meaningful only when `transfer` is HDR.
    #[serde(default = "presentation_peak_nits_default")]
    pub peak_nits: u16,
}

const fn presentation_dither_default() -> bool {
    true
}

const fn presentation_peak_nits_default() -> u16 {
    HDR_DEFAULT_PEAK_NITS
}

impl Default for PresentationRequest {
    fn default() -> Self {
        Self {
            depth: PresentationDepth::default(),
            dither: presentation_dither_default(),
            transfer: PresentationTransfer::default(),
            peak_nits: presentation_peak_nits_default(),
        }
    }
}

impl PresentationRequest {
    /// Coerce a request into a self-consistent one before resolution.
    ///
    /// HDR10 is a ten-bit contract: PQ quantized to eight bits bands severely, so
    /// an eight-bit HDR request is upgraded rather than resolved into a picture no
    /// one would ship. Peak luminance is clamped to the representable range. This
    /// runs inside [`PresentationCapabilities::resolve`], so a hand-edited
    /// `stage.json` cannot produce an incoherent runtime contract.
    /// The single user-facing contract this request represents.
    #[must_use]
    pub fn mode(self) -> PresentationMode {
        if self.transfer == PresentationTransfer::Hlg {
            PresentationMode::Hlg
        } else if self.transfer == PresentationTransfer::EdrLinear {
            PresentationMode::Edr
        } else if self.transfer.is_hdr() {
            PresentationMode::Hdr10
        } else if self.depth == PresentationDepth::Sdr10 {
            PresentationMode::Sdr10
        } else {
            PresentationMode::Sdr8
        }
    }

    /// Apply a user-facing contract, leaving dithering and peak untouched.
    #[must_use]
    pub fn with_mode(self, mode: PresentationMode) -> Self {
        let (depth, transfer) = match mode {
            PresentationMode::Sdr8 => (PresentationDepth::Sdr8, PresentationTransfer::Sdr),
            PresentationMode::Sdr10 => (PresentationDepth::Sdr10, PresentationTransfer::Sdr),
            PresentationMode::Hdr10 => (PresentationDepth::Sdr10, PresentationTransfer::Hdr10Pq),
            PresentationMode::Hlg => (PresentationDepth::Sdr10, PresentationTransfer::Hlg),
            // The surface is `Rgba16Float`, so integer depth does not apply; the
            // ten-bit request keeps the contract coherent for the resolver.
            PresentationMode::Edr => (PresentationDepth::Sdr10, PresentationTransfer::EdrLinear),
        };
        Self {
            depth,
            transfer,
            ..self
        }
    }

    #[must_use]
    pub fn normalized(mut self) -> Self {
        if self.transfer.is_hdr() {
            self.depth = PresentationDepth::Sdr10;
        }
        self.peak_nits = self.peak_nits.clamp(HDR_PEAK_NITS_MIN, HDR_PEAK_NITS_MAX);
        self
    }
}

/// One concrete format an adapter can present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationFormat {
    /// Precision carried by the format.
    pub depth: PresentationDepth,
    /// Transfer and dynamic-range contract carried by the format.
    pub transfer: PresentationTransfer,
    /// Pixel or encoded storage format.
    pub pixel_format: PresentationPixelFormat,
    /// Transfer and range contract.
    pub color_profile: PresentationColorProfile,
    /// Alpha representation.
    pub alpha_mode: AlphaMode,
}

/// Ordered formats and fallback explanation reported by an adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationCapabilities {
    formats: Vec<PresentationFormat>,
    fallback_reason: Option<String>,
}

impl PresentationCapabilities {
    /// Build capabilities in adapter preference order.
    pub fn new(formats: Vec<PresentationFormat>, fallback_reason: Option<String>) -> Self {
        Self {
            formats,
            fallback_reason,
        }
    }

    /// Best result this path can carry when the exact request is unavailable.
    ///
    /// An HDR request degrades to the best SDR format the adapter advertised, in
    /// adapter preference order, rather than erroring: an HDR10 recording request
    /// on an eight-bit codec is a capability answer, not a configuration error. An
    /// SDR ten-bit request keeps the existing eight-bit ladder.
    fn fallback_for(&self, request: PresentationRequest) -> Option<&PresentationFormat> {
        if request.transfer.is_hdr() {
            return self.formats.iter().find(|format| !format.transfer.is_hdr());
        }
        if request.depth == PresentationDepth::Sdr10 {
            return self.formats.iter().find(|format| {
                format.depth == PresentationDepth::Sdr8 && !format.transfer.is_hdr()
            });
        }
        None
    }

    /// Every mode, each with the reason this path cannot deliver it.
    ///
    /// Derived by asking [`resolve`](Self::resolve) rather than from a parallel
    /// capability table, so what the picker says cannot disagree with what the
    /// output then does. A mode is deliverable exactly when resolving it produces
    /// no fallback reason, and `fallback_reason` is set at the single point where
    /// degradation is decided, which is also where the wording comes from.
    ///
    /// Every mode is returned rather than only the deliverable ones, because a
    /// mode that is silently absent teaches nothing: the picker shows the rest
    /// disabled, with the reason. See /spec/presentation-mode-offering.md.
    #[must_use]
    pub fn mode_availability(&self) -> Vec<ModeAvailability> {
        PresentationMode::ALL
            .into_iter()
            .map(|mode| {
                let blocked = match self.resolve(PresentationRequest::default().with_mode(mode)) {
                    Ok(resolved) => resolved.fallback_reason,
                    Err(_) => Some("this output has no usable presentation format".to_string()),
                };
                ModeAvailability { mode, blocked }
            })
            .collect()
    }

    /// Resolve a request without mutating the persisted requested precision.
    ///
    /// # Errors
    ///
    /// Returns [`PresentationResolveError::NoSupportedFormats`] when the adapter
    /// reports no requested format and no valid eight-bit fallback.
    pub fn resolve(
        &self,
        request: PresentationRequest,
    ) -> Result<ResolvedPresentation, PresentationResolveError> {
        let request = request.normalized();
        let exact = self
            .formats
            .iter()
            .find(|format| format.depth == request.depth && format.transfer == request.transfer);
        let selected = exact.or_else(|| self.fallback_for(request));
        let Some(selected) = selected else {
            return Err(PresentationResolveError::NoSupportedFormats);
        };
        let degraded = selected.depth != request.depth || selected.transfer != request.transfer;
        let fallback_reason = degraded.then(|| {
            self.fallback_reason.clone().unwrap_or_else(|| {
                if request.transfer.is_hdr() {
                    "HDR10 is unavailable for this output".to_string()
                } else {
                    "10-bit SDR is unavailable for this output".to_string()
                }
            })
        });

        Ok(ResolvedPresentation {
            requested: request.depth,
            resolved: selected.depth,
            requested_transfer: request.transfer,
            transfer: selected.transfer,
            peak_nits: selected
                .transfer
                .uses_peak_nits()
                .then_some(request.peak_nits),
            // Adapters that write mastering metadata set this; the pure resolver
            // has no way to know whether one does.
            hdr_metadata: None,
            pixel_format: selected.pixel_format.clone(),
            color_profile: selected.color_profile,
            alpha_mode: selected.alpha_mode,
            dither: request.dither,
            fallback_reason,
        })
    }
}

/// Runtime precision and concrete format selected for an output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct ResolvedPresentation {
    /// Persisted requested precision.
    pub requested: PresentationDepth,
    /// Precision the active path carries.
    pub resolved: PresentationDepth,
    /// Persisted requested transfer contract.
    pub requested_transfer: PresentationTransfer,
    /// Transfer contract the active path carries.
    pub transfer: PresentationTransfer,
    /// Peak luminance in cd/m², present only when the resolved transfer is HDR.
    pub peak_nits: Option<u16>,
    /// Origin of `MaxCLL` and `MaxFALL`, present only when the resolved transfer is
    /// HDR and the adapter writes mastering metadata.
    pub hdr_metadata: Option<HdrMetadataSource>,
    /// Active pixel or encoded format.
    pub pixel_format: PresentationPixelFormat,
    /// Active color contract.
    pub color_profile: PresentationColorProfile,
    /// Active alpha representation.
    pub alpha_mode: AlphaMode,
    /// Whether presentation dithering is active.
    pub dither: bool,
    /// Explanation when requested and resolved precision differ.
    pub fallback_reason: Option<String>,
}

impl ResolvedPresentation {
    /// The user-facing contract this output is actually delivering.
    ///
    /// The counterpart to [`PresentationRequest::mode`], read from the resolved
    /// fields rather than the requested ones. The picker shows this when a stored
    /// request is not deliverable, so the control never displays a mode the output
    /// is not producing. See /spec/presentation-mode-offering.md.
    #[must_use]
    pub fn mode(&self) -> PresentationMode {
        if self.transfer == PresentationTransfer::Hlg {
            PresentationMode::Hlg
        } else if self.transfer == PresentationTransfer::EdrLinear {
            PresentationMode::Edr
        } else if self.transfer.is_hdr() {
            PresentationMode::Hdr10
        } else if self.resolved == PresentationDepth::Sdr10 {
            PresentationMode::Sdr10
        } else {
            PresentationMode::Sdr8
        }
    }
}

impl Default for ResolvedPresentation {
    fn default() -> Self {
        Self {
            requested: PresentationDepth::Sdr8,
            resolved: PresentationDepth::Sdr8,
            requested_transfer: PresentationTransfer::Sdr,
            transfer: PresentationTransfer::Sdr,
            peak_nits: None,
            hdr_metadata: None,
            pixel_format: PresentationPixelFormat::Rgba8,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Opaque,
            dither: presentation_dither_default(),
            fallback_reason: None,
        }
    }
}

/// Failure to find any usable presentation format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationResolveError {
    /// Adapter reported neither the requested format nor an eight-bit fallback.
    NoSupportedFormats,
}

impl std::fmt::Display for PresentationResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSupportedFormats => f.write_str("output has no supported presentation formats"),
        }
    }
}

impl std::error::Error for PresentationResolveError {}

// ── Output rotation ──────────────────────────────────────────────────

/// Per-output rotation applied at the final blit stage.
/// For 90°/270°, intermediate textures are created at swapped dimensions
/// (portrait content for landscape projectors and vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OutputRotation {
    #[default]
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl OutputRotation {
    /// All rotation variants for UI dropdowns.
    pub const ALL: [OutputRotation; 4] = [
        OutputRotation::Deg0,
        OutputRotation::Deg90,
        OutputRotation::Deg180,
        OutputRotation::Deg270,
    ];

    /// GPU-side index (0–3) for the shader uniform.
    pub fn index(&self) -> u32 {
        match self {
            OutputRotation::Deg0 => 0,
            OutputRotation::Deg90 => 1,
            OutputRotation::Deg180 => 2,
            OutputRotation::Deg270 => 3,
        }
    }

    /// Whether this rotation swaps width and height.
    pub fn swaps_dimensions(&self) -> bool {
        matches!(self, OutputRotation::Deg90 | OutputRotation::Deg270)
    }

    /// Effective texture dimensions after rotation.
    /// For 0°/180° returns (w, h); for 90°/270° returns (h, w).
    pub fn effective_dimensions(&self, w: u32, h: u32) -> (u32, u32) {
        if self.swaps_dimensions() {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Human-readable label for UI display.
    pub fn label(&self) -> &'static str {
        match self {
            OutputRotation::Deg0 => "0°",
            OutputRotation::Deg90 => "90°",
            OutputRotation::Deg180 => "180°",
            OutputRotation::Deg270 => "270°",
        }
    }
}

// ── Output source ────────────────────────────────────────────────────

/// Content source that an output window can display
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum OutputSource {
    /// The master mix (final composited output)
    Master,
    /// A specific channel's composited output (by index)
    Channel(usize),
    /// A subset of channels composited together (sub-mix).
    /// Each channel contributes with its own opacity and blend mode.
    /// Master effects are NOT applied to sub-mixes.
    Channels(Vec<usize>),
    /// A specific deck's raw output (channel index, deck index)
    Deck(usize, usize),
    /// The domemaster fisheye output (equidistant azimuthal projection)
    Domemaster,
}

impl OutputSource {
    /// Returns the channel indices involved in this source, if any.
    pub fn channel_indices(&self) -> Option<Vec<usize>> {
        match self {
            OutputSource::Channel(idx) => Some(vec![*idx]),
            OutputSource::Channels(indices) => Some(indices.clone()),
            _ => None,
        }
    }
}

impl std::fmt::Display for OutputSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputSource::Master => write!(f, "Master"),
            OutputSource::Channel(idx) => write!(f, "Ch {idx}"),
            OutputSource::Channels(indices) => {
                let names: Vec<String> = indices.iter().map(|i| format!("Ch {i}")).collect();
                write!(f, "{}", names.join("+"))
            }
            OutputSource::Deck(ch, dk) => write!(f, "Ch {} Deck {}", ch + 1, dk + 1),
            OutputSource::Domemaster => write!(f, "Domemaster"),
        }
    }
}

// ── Calibration mode ─────────────────────────────────────────────────

/// Per-output calibration display mode.
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
pub enum CalibrationMode {
    /// Normal content rendering.
    #[default]
    Off,
    /// A single full-frame test card fills the whole output, bypassing surface
    /// geometry and warp — for physical projector alignment.
    Projector,
    /// Each surface shows a colored per-surface test card through its own warp —
    /// for verifying surface mapping and warp.
    Surfaces,
}

// ── Output target ────────────────────────────────────────────────────

/// Where an output sends its content — unified across windowed and headless outputs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum OutputTarget {
    /// Floating window (default)
    Windowed,
    /// Fullscreen/borderless on a specific monitor (identified by name + index)
    Display {
        /// Monitor name (e.g. "Built-in Retina Display", "HDMI-1")
        name: String,
        /// Index into the available monitors list (for lookup)
        monitor_index: usize,
    },
    /// Record frames to a video file via ffmpeg subprocess
    Recording {
        path: String,
        codec: RecordingCodec,
        /// Audio passthrough device NAME (None = silent). See spec/audio-passthrough.md.
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Stream frames via SRT (Secure Reliable Transport) through ffmpeg
    SrtStream {
        url: String,
        codec: SrtCodec,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Stream frames as HLS segments via ffmpeg
    HlsStream {
        name: String,
        codec: StreamingCodec,
        /// One-second segments instead of two.
        ///
        /// Not RFC 8216bis low-latency HLS: FFmpeg's muxer writes no partial
        /// segments, so there are none to deliver. This shortens segments and
        /// nothing more. Named `low_latency` until it was measured; the alias
        /// keeps existing `stage.json` files loading.
        /// See /spec/hls-dash-io.md and /spec/ll-hls-output.md.
        #[serde(alias = "low_latency")]
        short_segments: bool,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Stream frames as DASH segments via ffmpeg
    DashStream {
        name: String,
        codec: StreamingCodec,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Push frames to an RTMP/RTMPS ingest endpoint via ffmpeg
    RtmpStream {
        url: String,
        codec: StreamingCodec,
        /// Receiver codec signaling contract. Legacy is the interoperable default.
        #[serde(default)]
        codec_contract: RtmpCodecContract,
        #[serde(default)]
        audio_device: Option<String>,
    },
    /// Send frames over NDI network protocol
    NdiSend { sender_name: String },
    /// Publish frames via Syphon (macOS inter-app sharing)
    SyphonServer { server_name: String },
    /// Publish frames via Spout (Windows inter-app sharing).
    ///
    /// The Windows counterpart to [`Self::SyphonServer`], and like it, only ever
    /// resolvable on its own platform. See /spec/spout-output.md.
    SpoutSender { sender_name: String },
}

impl OutputTarget {
    /// Whether this target requires an OS window.
    pub fn is_windowed(&self) -> bool {
        matches!(self, OutputTarget::Windowed | OutputTarget::Display { .. })
    }

    /// Whether this target is headless (no OS window).
    pub fn is_headless(&self) -> bool {
        !self.is_windowed()
    }

    /// The selected audio passthrough device name, if this is an ffmpeg target
    /// configured with audio. `None` for video-only or non-ffmpeg targets.
    pub fn audio_device(&self) -> Option<&str> {
        match self {
            OutputTarget::Recording { audio_device, .. }
            | OutputTarget::SrtStream { audio_device, .. }
            | OutputTarget::HlsStream { audio_device, .. }
            | OutputTarget::DashStream { audio_device, .. }
            | OutputTarget::RtmpStream { audio_device, .. } => audio_device.as_deref(),
            _ => None,
        }
    }

    /// Return a clone of this target with the audio passthrough device replaced.
    /// No-op for non-ffmpeg targets. Lets the GUI flip the device without
    /// re-specifying every variant field.
    #[must_use]
    pub fn with_audio_device(&self, device: Option<String>) -> OutputTarget {
        let mut target = self.clone();
        match &mut target {
            OutputTarget::Recording { audio_device, .. }
            | OutputTarget::SrtStream { audio_device, .. }
            | OutputTarget::HlsStream { audio_device, .. }
            | OutputTarget::DashStream { audio_device, .. }
            | OutputTarget::RtmpStream { audio_device, .. } => *audio_device = device,
            _ => {}
        }
        target
    }
}

impl std::fmt::Display for OutputTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputTarget::Windowed => write!(f, "Windowed"),
            OutputTarget::Display { name, .. } => write!(f, "{name}"),
            OutputTarget::Recording { path, codec, .. } => write!(f, "Rec [{codec}]: {path}"),
            OutputTarget::SrtStream { url, codec, .. } => write!(f, "SRT [{codec}]: {url}"),
            OutputTarget::HlsStream {
                name,
                codec,
                short_segments,
                ..
            } => {
                if *short_segments {
                    write!(f, "HLS-short [{codec}]: {name}")
                } else {
                    write!(f, "HLS [{codec}]: {name}")
                }
            }
            OutputTarget::DashStream { name, codec, .. } => write!(f, "DASH [{codec}]: {name}"),
            OutputTarget::RtmpStream { url, codec, .. } => write!(f, "RTMP [{codec}]: {url}"),
            OutputTarget::NdiSend { sender_name } => write!(f, "NDI: {sender_name}"),
            OutputTarget::SyphonServer { server_name } => write!(f, "Syphon: {server_name}"),
            OutputTarget::SpoutSender { sender_name } => write!(f, "Spout: {sender_name}"),
        }
    }
}

// ── ffmpeg codecs ────────────────────────────────────────────────────

/// Recording codec for ffmpeg subprocess.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum RecordingCodec {
    /// H.264 ultrafast preset (-c:v libx264 -preset ultrafast -crf 18)
    H264,
    /// H.265 / HEVC (-c:v libx265 -preset ultrafast -crf 20)
    H265,
    /// AV1 via SVT-AV1 (-c:v libsvtav1 -preset 10 -crf 28)
    AV1,
    /// `ProRes` 422 (-c:v `prores_ks` -profile:v 2)
    ProRes,
    /// `ProRes` 4444 with alpha (-c:v `prores_ks` -profile:v 4 -`pix_fmt` yuva444p10le)
    ProRes4444,
    /// HAP (-c:v hap -format hap)
    Hap,
    /// HAP Alpha (-c:v hap -format `hap_alpha`)
    HapAlpha,
    /// HAP Q (-c:v hap -format `hap_q`)
    HapQ,
}

impl std::fmt::Display for RecordingCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordingCodec::H264 => write!(f, "H.264"),
            RecordingCodec::H265 => write!(f, "H.265 (HEVC)"),
            RecordingCodec::AV1 => write!(f, "AV1"),
            RecordingCodec::ProRes => write!(f, "ProRes 422"),
            RecordingCodec::ProRes4444 => write!(f, "ProRes 4444"),
            RecordingCodec::Hap => write!(f, "HAP"),
            RecordingCodec::HapAlpha => write!(f, "HAP Alpha"),
            RecordingCodec::HapQ => write!(f, "HAP Q"),
        }
    }
}

/// Streaming codec for SRT output.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum SrtCodec {
    /// H.264 ultrafast + zerolatency
    #[default]
    H264,
    /// H.265 / HEVC ultrafast + zerolatency
    H265,
}

impl std::fmt::Display for SrtCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SrtCodec::H264 => write!(f, "H.264"),
            SrtCodec::H265 => write!(f, "H.265 (HEVC)"),
        }
    }
}

/// Streaming codec for HLS/DASH output.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema, Default,
)]
pub enum StreamingCodec {
    /// H.264 ultrafast preset
    #[default]
    H264,
    /// H.265 / HEVC ultrafast preset
    H265,
    /// AV1 via SVT-AV1
    AV1,
}

impl std::fmt::Display for StreamingCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamingCodec::H264 => write!(f, "H.264"),
            StreamingCodec::H265 => write!(f, "H.265 (HEVC)"),
            StreamingCodec::AV1 => write!(f, "AV1"),
        }
    }
}

/// Codec signaling contract declared by an RTMP/RTMPS endpoint.
///
/// URL scheme alone cannot establish Enhanced RTMP support, so this value is
/// explicit and persisted with the target.
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
#[serde(rename_all = "snake_case")]
pub enum RtmpCodecContract {
    /// Legacy FLV signaling. Only interoperable H.264 is permitted.
    #[default]
    Legacy,
    /// Enhanced RTMP signaling declared by the configured endpoint.
    Enhanced,
}

// ── Tonemap ──────────────────────────────────────────────────────────

/// Tonemapping mode selection.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
pub enum TonemapMode {
    /// Clamp to [0, 1] — equivalent to pre-tonemap behavior.
    Bypass = 0,
    /// ACES filmic curve — smooth highlight rolloff, preserves saturation.
    #[default]
    Aces = 1,
    /// Simple Reinhard: x/(x+1) per channel.
    Reinhard = 2,
    /// Reinhard with white point control, uses full SDR range.
    ReinhardExtended = 3,
    /// Hable/Uncharted 2 filmic curve — nice toe and shoulder.
    HableFilmic = 4,
    /// Gran Turismo style (Uchimura) — tunable shoulder and toe.
    Uchimura = 5,
    /// AMD Lottes — fast, invertible, high contrast.
    Lottes = 6,
    /// `AgX` — neutral, minimal hue shift, modern ACES alternative.
    AgX = 7,
    /// Khronos PBR Neutral — color-accurate, minimal look.
    KhronosPbrNeutral = 8,
}

impl TonemapMode {
    /// Whether this curve has a defined form for an HDR output transform.
    ///
    /// An HDR output transform targets `[0, peak/203]` instead of `[0, 1]`.
    /// `Bypass` extends by clamping to the wider range, and Reinhard Extended
    /// already carries a white point that becomes the headroom. The rest
    /// (`Aces`, `Reinhard`, `HableFilmic`, `Uchimura`, `Lottes`, `AgX`,
    /// `KhronosPbrNeutral`) have shoulder constants fitted against an SDR target
    /// and do **not** extend by rescaling their output: rescaling a curve outside
    /// the range it was fitted for produces a plausible wrong picture, which is
    /// the failure this phase family exists to prevent.
    ///
    /// ACES has a published HDR output-transform family. Adopting it is a
    /// separate piece of work, not a rescale of the SDR curve already here.
    /// See /spec/hdr-per-output-encode.md § Output transform range.
    /// Every curve, in the order the tonemap panel lists them.
    pub const ALL: [Self; 9] = [
        Self::Bypass,
        Self::Aces,
        Self::Reinhard,
        Self::ReinhardExtended,
        Self::HableFilmic,
        Self::Uchimura,
        Self::Lottes,
        Self::AgX,
        Self::KhronosPbrNeutral,
    ];

    /// Human-readable name, matching the preset list in the tonemap panel.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Bypass => "Bypass (clamp)",
            Self::Aces => "ACES Filmic",
            Self::Reinhard => "Reinhard",
            Self::ReinhardExtended => "Reinhard Extended",
            Self::HableFilmic => "Hable Filmic",
            Self::Uchimura => "Uchimura (GT)",
            Self::Lottes => "Lottes (AMD)",
            Self::AgX => "AgX",
            Self::KhronosPbrNeutral => "PBR Neutral",
        }
    }

    #[must_use]
    pub const fn has_hdr_form(self) -> bool {
        matches!(self, Self::Bypass | Self::ReinhardExtended)
    }

    /// The curve an HDR output actually runs, substituting a safe one when the
    /// selected curve has no HDR form.
    ///
    /// Substituting rather than refusing keeps the transfer contract and the
    /// creative curve independent: a look choice must not silently decide whether
    /// a delivery is HDR. The substitution is reported, never silent.
    #[must_use]
    pub const fn for_hdr(self) -> Self {
        if self.has_hdr_form() {
            self
        } else {
            Self::Bypass
        }
    }
}

// ── Edge blend ───────────────────────────────────────────────────────

/// Controls whether edge blend config is user-set or auto-computed from surface topology.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
    Default,
)]
pub enum EdgeBlendMode {
    /// User sets each edge manually (default — preserves existing behavior).
    #[default]
    Manual,
    /// Blend config is auto-derived from overlapping surfaces across outputs.
    Auto,
}

/// Per-edge blend configuration.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, utoipa::ToSchema)]
pub struct EdgeBlendEdge {
    pub enabled: bool,
    /// Blend zone width as fraction of output dimension (0.0–0.5).
    pub width: f32,
    /// Gamma curve exponent for the blend ramp (typically 1.0–3.0).
    pub gamma: f32,
}

impl Default for EdgeBlendEdge {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 0.1,
            gamma: 2.2,
        }
    }
}

/// Edge blending configuration for an output — four independent edges.
#[derive(
    Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, utoipa::ToSchema, Default,
)]
pub struct EdgeBlendConfig {
    pub left: EdgeBlendEdge,
    pub right: EdgeBlendEdge,
    pub top: EdgeBlendEdge,
    pub bottom: EdgeBlendEdge,
}

impl EdgeBlendConfig {
    /// Returns true if any edge has blending enabled.
    pub fn any_enabled(&self) -> bool {
        self.left.enabled || self.right.enabled || self.top.enabled || self.bottom.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── OutputRotation ───────────────────────────────────────────────

    #[test]
    fn rotation_default_is_deg0() {
        assert_eq!(OutputRotation::default(), OutputRotation::Deg0);
    }

    #[test]
    fn rotation_index_maps_each_variant() {
        assert_eq!(OutputRotation::Deg0.index(), 0);
        assert_eq!(OutputRotation::Deg90.index(), 1);
        assert_eq!(OutputRotation::Deg180.index(), 2);
        assert_eq!(OutputRotation::Deg270.index(), 3);
    }

    #[test]
    fn rotation_swaps_dimensions_only_for_quarter_turns() {
        assert!(!OutputRotation::Deg0.swaps_dimensions());
        assert!(OutputRotation::Deg90.swaps_dimensions());
        assert!(!OutputRotation::Deg180.swaps_dimensions());
        assert!(OutputRotation::Deg270.swaps_dimensions());
    }

    #[test]
    fn rotation_effective_dimensions_swaps_for_quarter_turns() {
        assert_eq!(
            OutputRotation::Deg0.effective_dimensions(1920, 1080),
            (1920, 1080)
        );
        assert_eq!(
            OutputRotation::Deg180.effective_dimensions(1920, 1080),
            (1920, 1080)
        );
        assert_eq!(
            OutputRotation::Deg90.effective_dimensions(1920, 1080),
            (1080, 1920)
        );
        assert_eq!(
            OutputRotation::Deg270.effective_dimensions(1920, 1080),
            (1080, 1920)
        );
    }

    #[test]
    fn rotation_label_per_variant() {
        assert_eq!(OutputRotation::Deg0.label(), "0°");
        assert_eq!(OutputRotation::Deg90.label(), "90°");
        assert_eq!(OutputRotation::Deg180.label(), "180°");
        assert_eq!(OutputRotation::Deg270.label(), "270°");
    }

    #[test]
    fn rotation_all_lists_every_variant_in_order() {
        assert_eq!(
            OutputRotation::ALL,
            [
                OutputRotation::Deg0,
                OutputRotation::Deg90,
                OutputRotation::Deg180,
                OutputRotation::Deg270,
            ]
        );
    }

    // ── OutputSource ─────────────────────────────────────────────────

    #[test]
    fn source_channel_indices_extracts_only_channel_variants() {
        assert_eq!(OutputSource::Master.channel_indices(), None);
        assert_eq!(OutputSource::Domemaster.channel_indices(), None);
        assert_eq!(OutputSource::Deck(0, 1).channel_indices(), None);
        assert_eq!(OutputSource::Channel(2).channel_indices(), Some(vec![2]));
        assert_eq!(
            OutputSource::Channels(vec![0, 3, 5]).channel_indices(),
            Some(vec![0, 3, 5])
        );
    }

    #[test]
    fn source_display_formats() {
        assert_eq!(OutputSource::Master.to_string(), "Master");
        assert_eq!(OutputSource::Channel(0).to_string(), "Ch 0");
        assert_eq!(OutputSource::Channels(vec![0, 1]).to_string(), "Ch 0+Ch 1");
        // Deck display is 1-indexed for humans.
        assert_eq!(OutputSource::Deck(0, 0).to_string(), "Ch 1 Deck 1");
        assert_eq!(OutputSource::Domemaster.to_string(), "Domemaster");
    }

    // ── OutputTarget ─────────────────────────────────────────────────

    #[test]
    fn target_windowed_predicates() {
        assert!(OutputTarget::Windowed.is_windowed());
        assert!(!OutputTarget::Windowed.is_headless());
        let display = OutputTarget::Display {
            name: "HDMI-1".into(),
            monitor_index: 0,
        };
        assert!(display.is_windowed());
        assert!(!display.is_headless());
    }

    #[test]
    fn target_headless_predicates() {
        let ndi = OutputTarget::NdiSend {
            sender_name: "Out".into(),
        };
        assert!(ndi.is_headless());
        assert!(!ndi.is_windowed());
        let rec = OutputTarget::Recording {
            path: "/tmp/out.mov".into(),
            codec: RecordingCodec::H264,
            audio_device: None,
        };
        assert!(rec.is_headless());
    }

    #[test]
    fn target_audio_device_only_for_configured_ffmpeg_targets() {
        // No audio configured → None even on an ffmpeg target.
        assert_eq!(
            OutputTarget::Recording {
                path: "/tmp/a.mov".into(),
                codec: RecordingCodec::H264,
                audio_device: None,
            }
            .audio_device(),
            None
        );
        // Configured audio → Some.
        assert_eq!(
            OutputTarget::SrtStream {
                url: "srt://host:9000".into(),
                codec: SrtCodec::H264,
                audio_device: Some("BlackHole 2ch".into()),
            }
            .audio_device(),
            Some("BlackHole 2ch")
        );
        // Non-ffmpeg targets never carry audio.
        assert_eq!(OutputTarget::Windowed.audio_device(), None);
        assert_eq!(
            OutputTarget::NdiSend {
                sender_name: "Out".into(),
            }
            .audio_device(),
            None
        );
    }

    #[test]
    fn target_with_audio_device_updates_ffmpeg_targets() {
        let rec = OutputTarget::Recording {
            path: "/tmp/a.mov".into(),
            codec: RecordingCodec::H264,
            audio_device: None,
        };
        let updated = rec.with_audio_device(Some("Mic".into()));
        assert_eq!(updated.audio_device(), Some("Mic"));
        // Clearing it back to None works too.
        assert_eq!(updated.with_audio_device(None).audio_device(), None);
    }

    #[test]
    fn target_with_audio_device_is_noop_for_non_ffmpeg() {
        let windowed = OutputTarget::Windowed;
        let updated = windowed.with_audio_device(Some("Mic".into()));
        // Still Windowed, still no audio device.
        assert_eq!(updated, OutputTarget::Windowed);
        assert_eq!(updated.audio_device(), None);
    }

    #[test]
    fn target_display_formats() {
        assert_eq!(OutputTarget::Windowed.to_string(), "Windowed");
        assert_eq!(
            OutputTarget::Display {
                name: "Built-in".into(),
                monitor_index: 1,
            }
            .to_string(),
            "Built-in"
        );
        assert_eq!(
            OutputTarget::HlsStream {
                name: "live".into(),
                codec: StreamingCodec::H264,
                short_segments: true,
                audio_device: None,
            }
            .to_string(),
            "HLS-short [H.264]: live"
        );
        assert_eq!(
            OutputTarget::HlsStream {
                name: "live".into(),
                codec: StreamingCodec::H264,
                short_segments: false,
                audio_device: None,
            }
            .to_string(),
            "HLS [H.264]: live"
        );
    }

    #[test]
    fn rtmp_codec_contract_defaults_to_legacy_and_roundtrips_enhanced() {
        let legacy: RtmpCodecContract = serde_json::from_str(r#""legacy""#).unwrap();
        assert_eq!(legacy, RtmpCodecContract::Legacy);
        assert_eq!(
            serde_json::to_string(&RtmpCodecContract::Enhanced).unwrap(),
            r#""enhanced""#
        );

        let target: OutputTarget = serde_json::from_str(
            r#"{"RtmpStream":{"url":"rtmps://example/live","codec":"H264","audio_device":null}}"#,
        )
        .unwrap();
        assert!(matches!(
            target,
            OutputTarget::RtmpStream {
                codec_contract: RtmpCodecContract::Legacy,
                ..
            }
        ));
    }

    // ── EdgeBlend ────────────────────────────────────────────────────

    #[test]
    fn edge_blend_edge_default_is_disabled_with_ramp_params() {
        let e = EdgeBlendEdge::default();
        assert!(!e.enabled);
        assert!((e.width - 0.1).abs() < 1e-6);
        assert!((e.gamma - 2.2).abs() < 1e-6);
    }

    #[test]
    fn edge_blend_config_default_has_nothing_enabled() {
        assert!(!EdgeBlendConfig::default().any_enabled());
    }

    #[test]
    fn edge_blend_config_any_enabled_detects_each_edge() {
        for pick in 0..4 {
            let mut cfg = EdgeBlendConfig::default();
            match pick {
                0 => cfg.left.enabled = true,
                1 => cfg.right.enabled = true,
                2 => cfg.top.enabled = true,
                _ => cfg.bottom.enabled = true,
            }
            assert!(cfg.any_enabled(), "edge {pick} should register as enabled");
        }
    }

    // ── SDR presentation precision ──────────────────────────────────

    fn sdr8_format() -> PresentationFormat {
        PresentationFormat {
            depth: PresentationDepth::Sdr8,
            transfer: PresentationTransfer::Sdr,
            pixel_format: PresentationPixelFormat::Rgba8,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Opaque,
        }
    }

    fn sdr10_format() -> PresentationFormat {
        PresentationFormat {
            depth: PresentationDepth::Sdr10,
            transfer: PresentationTransfer::Sdr,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Opaque,
        }
    }

    #[test]
    fn presentation_depth_labels_match_the_user_facing_order() {
        assert_eq!(
            PresentationDepth::ALL,
            [PresentationDepth::Sdr8, PresentationDepth::Sdr10]
        );
        assert_eq!(PresentationDepth::Sdr8.label(), "8-bit SDR");
        assert_eq!(PresentationDepth::Sdr10.label(), "10-bit SDR");
    }

    #[test]
    fn pixel_format_display_covers_every_variant() {
        assert_eq!(PresentationPixelFormat::Rgba8.to_string(), "RGBA8");
        assert_eq!(PresentationPixelFormat::Bgra8.to_string(), "BGRA8");
        assert_eq!(PresentationPixelFormat::Rgb10A2.to_string(), "RGB10A2");
        assert_eq!(PresentationPixelFormat::Rgba16.to_string(), "RGBA16");
        assert_eq!(PresentationPixelFormat::Uyvy.to_string(), "UYVY");
        assert_eq!(PresentationPixelFormat::P216.to_string(), "P216");
        assert_eq!(
            PresentationPixelFormat::EncoderNative("yuv420p10le".into()).to_string(),
            "yuv420p10le"
        );
    }

    #[test]
    fn presentation_request_defaults_to_sdr8_with_dither() {
        let request = PresentationRequest::default();
        assert_eq!(request.depth, PresentationDepth::Sdr8);
        assert!(request.dither);
        assert_eq!(request.transfer, PresentationTransfer::Sdr);
        assert_eq!(request.peak_nits, HDR_DEFAULT_PEAK_NITS);
    }

    #[test]
    fn presentation_request_uses_flat_stage_json_names() {
        let request: PresentationRequest =
            serde_json::from_str(r#"{"presentation_depth":"sdr10"}"#).unwrap();
        assert_eq!(request.depth, PresentationDepth::Sdr10);
        assert!(request.dither);

        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["presentation_depth"], "sdr10");
        assert_eq!(json["dither"], true);

        let without_dither: PresentationRequest =
            serde_json::from_str(r#"{"presentation_depth":"sdr8"}"#).unwrap();
        assert!(without_dither.dither);
        let explicit: PresentationRequest =
            serde_json::from_str(r#"{"presentation_depth":"sdr10","dither":false}"#).unwrap();
        assert!(!explicit.dither);
    }

    #[test]
    fn presentation_resolver_uses_requested_sdr10_format() {
        let capabilities = PresentationCapabilities::new(vec![sdr10_format(), sdr8_format()], None);
        let resolved = capabilities
            .resolve(PresentationRequest {
                depth: PresentationDepth::Sdr10,
                dither: false,
                ..PresentationRequest::default()
            })
            .unwrap();

        assert_eq!(resolved.resolved, PresentationDepth::Sdr10);
        assert_eq!(resolved.pixel_format, PresentationPixelFormat::Rgb10A2);
        assert!(!resolved.dither);
        assert_eq!(resolved.fallback_reason, None);
    }

    #[test]
    fn presentation_resolver_falls_back_without_rewriting_request() {
        let capabilities = PresentationCapabilities::new(
            vec![sdr8_format()],
            Some("Syphon interoperability is limited to BGRA8".into()),
        );
        let resolved = capabilities
            .resolve(PresentationRequest {
                depth: PresentationDepth::Sdr10,
                dither: true,
                ..PresentationRequest::default()
            })
            .unwrap();

        assert_eq!(resolved.requested, PresentationDepth::Sdr10);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(
            resolved.fallback_reason.as_deref(),
            Some("Syphon interoperability is limited to BGRA8")
        );
    }

    #[test]
    fn presentation_resolver_rejects_empty_capabilities() {
        let capabilities = PresentationCapabilities::new(Vec::new(), None);
        assert_eq!(
            capabilities.resolve(PresentationRequest::default()),
            Err(PresentationResolveError::NoSupportedFormats)
        );
    }

    #[test]
    fn presentation_resolver_picks_matching_eight_bit_even_when_ten_bit_is_listed_first() {
        let capabilities = PresentationCapabilities::new(vec![sdr10_format(), sdr8_format()], None);
        let resolved = capabilities
            .resolve(PresentationRequest::default())
            .unwrap();

        assert_eq!(resolved.requested, PresentationDepth::Sdr8);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(resolved.pixel_format, PresentationPixelFormat::Rgba8);
        assert!(resolved.fallback_reason.is_none());
    }

    #[test]
    fn presentation_resolver_rejects_eight_bit_request_when_only_ten_bit_exists() {
        let capabilities = PresentationCapabilities::new(vec![sdr10_format()], None);
        assert_eq!(
            capabilities.resolve(PresentationRequest::default()),
            Err(PresentationResolveError::NoSupportedFormats)
        );
    }

    #[test]
    fn presentation_resolver_uses_generic_reason_and_keeps_dither_on_fallback() {
        let capabilities = PresentationCapabilities::new(vec![sdr8_format()], None);
        let resolved = capabilities
            .resolve(PresentationRequest {
                depth: PresentationDepth::Sdr10,
                dither: false,
                ..PresentationRequest::default()
            })
            .unwrap();

        assert_eq!(resolved.requested, PresentationDepth::Sdr10);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert!(!resolved.dither);
        assert_eq!(
            resolved.fallback_reason.as_deref(),
            Some("10-bit SDR is unavailable for this output")
        );
    }

    fn hdr10_format() -> PresentationFormat {
        PresentationFormat {
            depth: PresentationDepth::Sdr10,
            transfer: PresentationTransfer::Hdr10Pq,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::Pq2020Limited,
            alpha_mode: AlphaMode::Opaque,
        }
    }

    #[test]
    fn legacy_stage_json_loads_as_sdr_at_the_default_peak() {
        // A `.varda/` written before Phase 50 carries neither field. It must land on
        // the SDR contract it was written for, not on an HDR one.
        let request: PresentationRequest =
            serde_json::from_str(r#"{"presentation_depth":"sdr10","dither":false}"#).unwrap();
        assert_eq!(request.depth, PresentationDepth::Sdr10);
        assert!(!request.dither);
        assert_eq!(request.transfer, PresentationTransfer::Sdr);
        assert_eq!(request.peak_nits, HDR_DEFAULT_PEAK_NITS);
    }

    #[test]
    fn presentation_request_round_trips_hdr_fields() {
        let request = PresentationRequest {
            transfer: PresentationTransfer::Hdr10Pq,
            peak_nits: 4000,
            ..PresentationRequest::default()
        };
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["transfer"], "hdr10_pq");
        assert_eq!(json["peak_nits"], 4000);
        assert_eq!(
            serde_json::from_value::<PresentationRequest>(json).unwrap(),
            request
        );
    }

    #[test]
    fn normalization_upgrades_eight_bit_hdr_to_ten_bit() {
        // PQ quantized to eight bits bands severely. An incoherent hand-edited
        // request is coerced rather than resolved into a picture no one would ship.
        let normalized = PresentationRequest {
            depth: PresentationDepth::Sdr8,
            transfer: PresentationTransfer::Hdr10Pq,
            ..PresentationRequest::default()
        }
        .normalized();
        assert_eq!(normalized.depth, PresentationDepth::Sdr10);
    }

    #[test]
    fn normalization_clamps_peak_nits_and_leaves_sdr_depth_alone() {
        let low = PresentationRequest {
            peak_nits: 1,
            ..PresentationRequest::default()
        }
        .normalized();
        assert_eq!(low.peak_nits, HDR_PEAK_NITS_MIN);

        let high = PresentationRequest {
            peak_nits: u16::MAX,
            ..PresentationRequest::default()
        }
        .normalized();
        assert_eq!(high.peak_nits, HDR_PEAK_NITS_MAX);

        let sdr = PresentationRequest {
            depth: PresentationDepth::Sdr8,
            ..PresentationRequest::default()
        }
        .normalized();
        assert_eq!(sdr.depth, PresentationDepth::Sdr8);
    }

    #[test]
    fn resolver_selects_hdr10_when_the_adapter_advertises_it() {
        let capabilities = PresentationCapabilities::new(
            vec![hdr10_format(), sdr10_format(), sdr8_format()],
            None,
        );
        let resolved = capabilities
            .resolve(PresentationRequest {
                transfer: PresentationTransfer::Hdr10Pq,
                peak_nits: 1000,
                ..PresentationRequest::default()
            })
            .unwrap();

        assert_eq!(resolved.transfer, PresentationTransfer::Hdr10Pq);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr10);
        assert_eq!(
            resolved.color_profile,
            PresentationColorProfile::Pq2020Limited
        );
        assert_eq!(resolved.peak_nits, Some(1000));
        assert_eq!(resolved.fallback_reason, None);
    }

    #[test]
    fn resolver_degrades_hdr_to_the_best_sdr_the_path_can_carry() {
        // An HDR10 request on an SDR-only path is a capability answer, not an error.
        // Adapter preference order decides which SDR result is "best".
        let capabilities = PresentationCapabilities::new(vec![sdr10_format(), sdr8_format()], None);
        let resolved = capabilities
            .resolve(PresentationRequest {
                transfer: PresentationTransfer::Hdr10Pq,
                ..PresentationRequest::default()
            })
            .unwrap();

        assert_eq!(resolved.requested_transfer, PresentationTransfer::Hdr10Pq);
        assert_eq!(resolved.transfer, PresentationTransfer::Sdr);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr10);
        assert_eq!(resolved.peak_nits, None);
        assert_eq!(
            resolved.fallback_reason.as_deref(),
            Some("HDR10 is unavailable for this output")
        );
    }

    #[test]
    fn resolver_degrades_hdr_all_the_way_to_eight_bit_when_that_is_all_there_is() {
        let capabilities = PresentationCapabilities::new(
            vec![sdr8_format()],
            Some("H.264 is an eight-bit codec".into()),
        );
        let resolved = capabilities
            .resolve(PresentationRequest {
                transfer: PresentationTransfer::Hdr10Pq,
                ..PresentationRequest::default()
            })
            .unwrap();

        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(resolved.transfer, PresentationTransfer::Sdr);
        assert_eq!(
            resolved.fallback_reason.as_deref(),
            Some("H.264 is an eight-bit codec")
        );
    }

    #[test]
    fn resolver_never_answers_an_sdr_request_with_an_hdr_format() {
        // An adapter listing HDR first must not hand HDR to a show that asked for SDR.
        let capabilities = PresentationCapabilities::new(vec![hdr10_format(), sdr8_format()], None);
        let resolved = capabilities
            .resolve(PresentationRequest::default())
            .unwrap();

        assert_eq!(resolved.transfer, PresentationTransfer::Sdr);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert!(resolved.fallback_reason.is_none());
    }

    #[test]
    fn resolver_degrading_sdr_ten_bit_skips_hdr_formats() {
        let capabilities = PresentationCapabilities::new(vec![hdr10_format(), sdr8_format()], None);
        let resolved = capabilities
            .resolve(PresentationRequest {
                depth: PresentationDepth::Sdr10,
                ..PresentationRequest::default()
            })
            .unwrap();

        assert_eq!(resolved.transfer, PresentationTransfer::Sdr);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
    }

    #[test]
    fn resolved_presentation_defaults_to_sdr() {
        let resolved = ResolvedPresentation::default();
        assert_eq!(resolved.transfer, PresentationTransfer::Sdr);
        assert_eq!(resolved.requested_transfer, PresentationTransfer::Sdr);
        assert_eq!(resolved.peak_nits, None);
    }

    #[test]
    fn every_mode_round_trips_through_a_request() {
        for mode in PresentationMode::ALL {
            let request = PresentationRequest::default().with_mode(mode);
            assert_eq!(request.mode(), mode, "{mode:?} did not round trip");
        }
    }

    #[test]
    fn hdr10_mode_implies_a_ten_bit_request() {
        let request = PresentationRequest::default().with_mode(PresentationMode::Hdr10);
        assert_eq!(request.depth, PresentationDepth::Sdr10);
        assert_eq!(request.transfer, PresentationTransfer::Hdr10Pq);
    }

    #[test]
    fn switching_modes_preserves_dither_and_peak() {
        // Changing the contract must not silently discard the operator's other
        // settings, so a round trip through HDR and back is lossless.
        let original = PresentationRequest {
            dither: false,
            peak_nits: 4000,
            ..PresentationRequest::default()
        };
        let hdr = original.with_mode(PresentationMode::Hdr10);
        assert!(!hdr.dither);
        assert_eq!(hdr.peak_nits, 4000);

        let back = hdr.with_mode(PresentationMode::Sdr8);
        assert_eq!(back, original);
    }

    #[test]
    fn mode_labels_are_distinct_and_never_call_ten_bit_sdr_hdr() {
        let labels: Vec<_> = PresentationMode::ALL.iter().map(|m| m.label()).collect();
        assert_eq!(
            labels,
            ["8-bit SDR", "10-bit SDR", "HDR10", "HLG", "EDR (monitor)"]
        );
        // The naming rule from /spec/sdr-presentation-precision.md § Naming and
        // Boundary: 10-bit SDR is not HDR and must never be labelled as such.
        assert!(!PresentationMode::Sdr10.label().contains("HDR"));
        assert!(!PresentationMode::Sdr10.description().contains("HDR"));
    }

    #[test]
    fn edr_is_hdr_but_encodes_nothing_and_keeps_a_peak() {
        // EDR is the one HDR contract that writes linear values, and the one
        // whose peak names a deliverable rather than the display.
        assert!(PresentationTransfer::EdrLinear.is_hdr());
        assert!(!PresentationTransfer::EdrLinear.encodes_a_transfer());
        assert!(!PresentationTransfer::EdrLinear.carries_mastering_metadata());
        assert!(PresentationTransfer::EdrLinear.uses_peak_nits());
    }

    #[test]
    fn hlg_is_hdr_but_carries_no_mastering_metadata() {
        // The property that makes HLG the right live default: nothing to declare,
        // so nothing to report as declared rather than measured.
        assert!(PresentationTransfer::Hlg.is_hdr());
        assert!(!PresentationTransfer::Hlg.carries_mastering_metadata());
        assert!(!PresentationTransfer::Hlg.uses_peak_nits());
        assert!(PresentationTransfer::Hdr10Pq.carries_mastering_metadata());
        assert!(PresentationTransfer::Hdr10Pq.uses_peak_nits());
    }

    #[test]
    fn hlg_mode_round_trips_and_implies_ten_bit() {
        let request = PresentationRequest::default().with_mode(PresentationMode::Hlg);
        assert_eq!(request.mode(), PresentationMode::Hlg);
        assert_eq!(request.transfer, PresentationTransfer::Hlg);
        assert_eq!(request.depth, PresentationDepth::Sdr10);
    }

    #[test]
    fn an_hlg_resolution_reports_no_peak() {
        let hlg = PresentationFormat {
            depth: PresentationDepth::Sdr10,
            transfer: PresentationTransfer::Hlg,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::Pq2020Limited,
            alpha_mode: AlphaMode::Opaque,
        };
        let resolved = PresentationCapabilities::new(vec![hlg, sdr8_format()], None)
            .resolve(PresentationRequest::default().with_mode(PresentationMode::Hlg))
            .unwrap();
        assert_eq!(resolved.transfer, PresentationTransfer::Hlg);
        assert_eq!(
            resolved.peak_nits, None,
            "HLG is relative; a peak is meaningless"
        );
    }

    // ── offered_modes (/spec/presentation-mode-offering.md) ──────────

    /// Every capability set used by the tests below, so the property test and the
    /// specific cases cannot drift onto different fixtures.
    fn caps(formats: Vec<PresentationFormat>) -> PresentationCapabilities {
        PresentationCapabilities::new(formats, Some("test capability set".into()))
    }

    /// The modes a capability set can deliver, for tests that care about the set
    /// rather than the reasons.
    fn available(capabilities: &PresentationCapabilities) -> Vec<PresentationMode> {
        capabilities
            .mode_availability()
            .into_iter()
            .filter(ModeAvailability::is_available)
            .map(|entry| entry.mode)
            .collect()
    }

    fn fmt(depth: PresentationDepth, transfer: PresentationTransfer) -> PresentationFormat {
        PresentationFormat {
            depth,
            transfer,
            pixel_format: PresentationPixelFormat::Rgba8,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Opaque,
        }
    }

    fn sdr8() -> PresentationFormat {
        fmt(PresentationDepth::Sdr8, PresentationTransfer::Sdr)
    }

    /// The contract that makes a derived menu safe: whatever `offered_modes`
    /// lists, `resolve` must deliver unchanged, and whatever it omits must
    /// degrade. Stated as a property over capability sets rather than a table,
    /// because a table would be the very thing this design rejects.
    #[test]
    fn the_offered_set_and_the_resolver_always_agree() {
        let sets = [
            vec![sdr8()],
            vec![
                fmt(PresentationDepth::Sdr10, PresentationTransfer::Sdr),
                sdr8(),
            ],
            vec![
                fmt(PresentationDepth::Sdr10, PresentationTransfer::Hdr10Pq),
                fmt(PresentationDepth::Sdr10, PresentationTransfer::Sdr),
                sdr8(),
            ],
            vec![
                fmt(PresentationDepth::Sdr10, PresentationTransfer::EdrLinear),
                fmt(PresentationDepth::Sdr10, PresentationTransfer::Hlg),
                fmt(PresentationDepth::Sdr10, PresentationTransfer::Hdr10Pq),
                fmt(PresentationDepth::Sdr10, PresentationTransfer::Sdr),
                sdr8(),
            ],
        ];
        for formats in sets {
            let capabilities = caps(formats);
            let offered = available(&capabilities);
            for mode in PresentationMode::ALL {
                let resolved = capabilities
                    .resolve(PresentationRequest::default().with_mode(mode))
                    .expect("every set here carries eight-bit SDR");
                if offered.contains(&mode) {
                    assert!(
                        resolved.fallback_reason.is_none(),
                        "{mode:?} was offered but degraded"
                    );
                } else {
                    assert!(
                        resolved.fallback_reason.is_some(),
                        "{mode:?} was withheld but would have resolved cleanly"
                    );
                }
            }
        }
    }

    /// An empty picker would be a worse failure than an over-full one, and any
    /// path that can present at all can present eight-bit SDR.
    #[test]
    fn eight_bit_is_always_offered() {
        for formats in [
            vec![sdr8()],
            vec![
                fmt(PresentationDepth::Sdr10, PresentationTransfer::Hdr10Pq),
                sdr8(),
            ],
        ] {
            let offered = available(&caps(formats));
            assert!(offered.contains(&PresentationMode::Sdr8));
            assert!(!offered.is_empty());
        }
    }

    /// The case that motivated this phase: an adapter with one fixed format
    /// offers exactly one mode, rather than five with four warnings behind them.
    #[test]
    fn a_single_format_adapter_offers_exactly_one_mode() {
        assert_eq!(available(&caps(vec![sdr8()])), vec![PresentationMode::Sdr8]);
    }

    #[test]
    fn a_ten_bit_sdr_adapter_offers_no_hdr_mode() {
        let offered = available(&caps(vec![
            fmt(PresentationDepth::Sdr10, PresentationTransfer::Sdr),
            sdr8(),
        ]));
        assert_eq!(
            offered,
            vec![PresentationMode::Sdr8, PresentationMode::Sdr10]
        );
    }

    /// Hiding a mode teaches nothing, so every blocked entry has to say why. The
    /// picker renders this text, and an empty reason would be a blank tooltip.
    #[test]
    fn every_blocked_mode_carries_a_reason() {
        for entry in caps(vec![sdr8()]).mode_availability() {
            if entry.is_available() {
                continue;
            }
            let reason = entry.blocked.expect("checked above");
            assert!(
                !reason.trim().is_empty(),
                "{:?} was blocked with no explanation",
                entry.mode
            );
        }
    }

    /// A display surface exposing PQ but no HLG format, which is every display
    /// surface: `select_surface_presentation` has no HLG branch to build one from.
    #[test]
    fn a_pq_surface_offers_hdr10_but_not_hlg() {
        let offered = available(&caps(vec![
            fmt(PresentationDepth::Sdr10, PresentationTransfer::Hdr10Pq),
            fmt(PresentationDepth::Sdr10, PresentationTransfer::Sdr),
            sdr8(),
        ]));
        assert!(offered.contains(&PresentationMode::Hdr10));
        assert!(
            !offered.contains(&PresentationMode::Hlg),
            "HLG has never been deliverable on a display surface"
        );
    }
}
