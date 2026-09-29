//! Apple EDR headroom, read from `NSScreen` (wgpu and the Metal device don't
//! expose it).
//!
//! Gate capability on `potential`, not `current`: macOS allocates headroom
//! only once EDR content is on screen, so `current` reads 1.0 until then
//! (a Liquid Retina XDR reports current 1.0, potential 16.0).

/// What a display can and currently does allow above SDR white.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdrHeadroom {
    /// Maximum the display can reach; use this for capability.
    pub potential: f32,
    /// Headroom allocated now. 1.0 until EDR content is presented; varies with
    /// brightness and ambient light.
    pub current: f32,
}

impl EdrHeadroom {
    /// Whether this display can show anything above SDR white (`potential` > 1.0).
    #[must_use]
    pub fn is_capable(self) -> bool {
        self.potential > 1.001
    }

    /// Whether the current allocation covers a program reaching `headroom`.
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

    // `NSScreen` is main-thread-only; other threads get `None`.
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
        // A Liquid Retina XDR with no EDR content on screen reports this.
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
        // A 1000 cd/m² program needs 4.93x over reference white.
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
