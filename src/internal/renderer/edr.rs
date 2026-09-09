//! Apple EDR headroom, read from `NSScreen`.
//!
//! wgpu does not expose EDR headroom, and it does not live on the Metal device,
//! so this is an AppKit query rather than the `as_hal` interop the rest of the
//! macOS code uses.
//!
//! **Capability is `potential`, never `current`.** macOS allocates headroom on
//! demand, once a layer actually asks for extended range, so a display capable
//! of 16x reports a current headroom of 1.0 until EDR content is on screen.
//! Gating availability on the current value would mean EDR never engages.
//! Measured on a Liquid Retina XDR: current 1.0, potential 16.0.
//!
//! See /spec/hdr-edr-display.md § Headroom, measured.

/// What a display can and currently does allow above SDR white.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdrHeadroom {
    /// Ceiling the display could reach, and the honest capability signal.
    pub potential: f32,
    /// Headroom allocated right now. Sits at 1.0 until EDR content is presented,
    /// then climbs, and moves with display brightness and ambient light.
    pub current: f32,
}

impl EdrHeadroom {
    /// Whether this display can show anything above SDR white at all.
    ///
    /// Keyed on `potential`: see the module note. A display reporting a
    /// potential of 1.0 has no extended range to offer.
    #[must_use]
    pub fn is_capable(self) -> bool {
        self.potential > 1.001
    }

    /// Whether the current allocation covers a program reaching `headroom`.
    ///
    /// Answers the question an operator actually has, which is whether the
    /// monitor can show the top of the range being graded right now.
    #[must_use]
    pub fn covers(self, headroom: f32) -> bool {
        self.current >= headroom
    }
}

/// Headroom of the display currently showing content, if one can be read.
///
/// `None` off macOS, and on macOS when no screen is available.
#[cfg(target_os = "macos")]
#[must_use]
pub fn primary_headroom() -> Option<EdrHeadroom> {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;

    // `NSScreen` is main-thread-only. A caller off the main thread gets `None`
    // rather than a crash, which for a status line is the right trade.
    let mtm = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(mtm)?;
    #[allow(clippy::cast_possible_truncation)]
    Some(EdrHeadroom {
        potential: screen.maximumPotentialExtendedDynamicRangeColorComponentValue() as f32,
        current: screen.maximumExtendedDynamicRangeColorComponentValue() as f32,
    })
}

/// Headroom of the display currently showing content, if one can be read.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn primary_headroom() -> Option<EdrHeadroom> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_comes_from_potential_not_current() {
        // The trap this module exists to record. A Liquid Retina XDR with no EDR
        // content on screen reports exactly this, and gating on `current` would
        // refuse EDR on a display that can do 16x.
        let idle_xdr = EdrHeadroom {
            potential: 16.0,
            current: 1.0,
        };
        assert!(
            idle_xdr.is_capable(),
            "capability must not depend on current"
        );
    }

    #[test]
    fn a_display_with_no_extended_range_is_not_capable() {
        let sdr = EdrHeadroom {
            potential: 1.0,
            current: 1.0,
        };
        assert!(!sdr.is_capable());
    }

    #[test]
    fn coverage_is_about_the_current_allocation() {
        // Capability says the display could show the range; coverage says whether
        // it is showing it now. A 1000 cd/m² program needs 4.93x over reference
        // white.
        let engaged = EdrHeadroom {
            potential: 16.0,
            current: 5.2,
        };
        assert!(engaged.covers(4.93));

        let dimmed = EdrHeadroom {
            potential: 16.0,
            current: 2.0,
        };
        assert!(!dimmed.covers(4.93), "a dimmed display clips the top");
        assert!(dimmed.is_capable(), "but it is still an EDR display");
    }
}
