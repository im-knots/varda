//! Fixture roles: the semantic meaning of a DMX channel.
//!
//! The engine writes roles. A [`super::profile::FixtureProfile`] decides which slot each role
//! lands in and at what resolution, so the same show drives an 8-bit par and a 16-bit
//! moving head identically.
//!
//! See /spec/dmx-output.md § Roles.

use std::fmt;

/// Semantic meaning of one fixture channel.
///
/// Every role value is a normalized `f32` in `0.0..=1.0`. Resolution (8 or 16 bit) is a
/// property of the patch, never of the role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    // Intensity
    Dimmer,
    Shutter,
    Strobe,
    // Color
    Red,
    Green,
    Blue,
    White,
    Amber,
    Uv,
    Lime,
    Cyan,
    Magenta,
    Yellow,
    // Color control
    Cto,
    Ctb,
    ColorWheel,
    // Position
    Pan,
    Tilt,
    PanTiltSpeed,
    // Beam
    Zoom,
    Focus,
    Iris,
    Frost,
    Prism,
    PrismRotation,
    // Gobo
    GoboWheel,
    GoboRotation,
    GoboShake,
    // Control
    Function,
    Reset,
    LampControl,
}

/// Coarse grouping used for merge defaults, guard scoping, and smoothing coefficients.
///
/// The merge rule follows from the group rather than from a per-deck choice, matching how
/// lighting consoles behave: intensity is HTP, everything else is LTP.
/// See /spec/lighting-routing.md § Mixing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoleGroup {
    Intensity,
    Color,
    Position,
    Beam,
    Control,
}

/// How a role responds to the output smoother.
///
/// The distinction that matters is [`SmoothingClass::Discrete`]. Smoothing a wheel, a shutter,
/// or a macro channel ramps it through every intervening DMX value, so a color-wheel change
/// spins the wheel through every gel on the way and a strobe channel sweeps its rate. These
/// channels select rather than vary, and must reach their target on the frame they change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SmoothingClass {
    /// Smoothed, and shares one transient decision with the other color roles of its fixture
    /// so the attack of a hit cannot skew hue.
    Color,
    /// Smoothed with its own transient response.
    Intensity,
    /// Smoothed, never given a transient fast path: a snap on a moving head is both ugly and
    /// mechanically unkind.
    Position,
    /// Smoothed, no transient.
    Beam,
    /// Never smoothed.
    Discrete,
}

impl Role {
    /// Every role, in declaration order. Used by palette editors and the patch UI.
    pub const ALL: &'static [Role] = &[
        Role::Dimmer,
        Role::Shutter,
        Role::Strobe,
        Role::Red,
        Role::Green,
        Role::Blue,
        Role::White,
        Role::Amber,
        Role::Uv,
        Role::Lime,
        Role::Cyan,
        Role::Magenta,
        Role::Yellow,
        Role::Cto,
        Role::Ctb,
        Role::ColorWheel,
        Role::Pan,
        Role::Tilt,
        Role::PanTiltSpeed,
        Role::Zoom,
        Role::Focus,
        Role::Iris,
        Role::Frost,
        Role::Prism,
        Role::PrismRotation,
        Role::GoboWheel,
        Role::GoboRotation,
        Role::GoboShake,
        Role::Function,
        Role::Reset,
        Role::LampControl,
    ];

    /// Number of distinct roles. Sized for a dense per-role array.
    pub const COUNT: usize = 31;

    /// Dense index into a per-role array, matching position in [`Role::ALL`].
    ///
    /// # Panics
    ///
    /// Panics if a variant is missing from [`Role::ALL`], which
    /// [`tests::all_contains_every_variant_exactly_once`] makes impossible to ship.
    #[must_use]
    pub fn index(self) -> usize {
        Role::ALL
            .iter()
            .position(|r| *r == self)
            .expect("every Role variant is listed in Role::ALL")
    }

    /// How the output smoother treats this role.
    #[must_use]
    pub fn smoothing(self) -> SmoothingClass {
        match self {
            // Selection and control channels: a smoothed transition would walk through every
            // slot between the old and new selection.
            Role::Shutter
            | Role::Strobe
            | Role::ColorWheel
            | Role::GoboWheel
            | Role::GoboShake
            | Role::Prism
            | Role::PanTiltSpeed
            | Role::Function
            | Role::Reset
            | Role::LampControl => SmoothingClass::Discrete,
            Role::Dimmer => SmoothingClass::Intensity,
            Role::Pan | Role::Tilt => SmoothingClass::Position,
            Role::Zoom
            | Role::Focus
            | Role::Iris
            | Role::Frost
            | Role::PrismRotation
            | Role::GoboRotation => SmoothingClass::Beam,
            _ => SmoothingClass::Color,
        }
    }

    /// The group this role merges and smooths as.
    ///
    /// `ColorWheel` is deliberately [`RoleGroup::Color`] even though it is a discrete
    /// selection channel: it participates in color merge decisions, and the white ceiling
    /// guard skips it explicitly rather than by group.
    #[must_use]
    pub fn group(self) -> RoleGroup {
        match self {
            Role::Dimmer | Role::Shutter | Role::Strobe => RoleGroup::Intensity,
            Role::Red
            | Role::Green
            | Role::Blue
            | Role::White
            | Role::Amber
            | Role::Uv
            | Role::Lime
            | Role::Cyan
            | Role::Magenta
            | Role::Yellow
            | Role::Cto
            | Role::Ctb
            | Role::ColorWheel => RoleGroup::Color,
            Role::Pan | Role::Tilt | Role::PanTiltSpeed => RoleGroup::Position,
            Role::Zoom
            | Role::Focus
            | Role::Iris
            | Role::Frost
            | Role::Prism
            | Role::PrismRotation
            | Role::GoboWheel
            | Role::GoboRotation
            | Role::GoboShake => RoleGroup::Beam,
            Role::Function | Role::Reset | Role::LampControl => RoleGroup::Control,
        }
    }

    /// True for the additive light-emitting color roles the white ceiling guard operates on.
    ///
    /// Excludes `ColorWheel`, `Cto`, and `Ctb`, which are selection and correction channels:
    /// re-saturating them is meaningless and would scramble a gel or wheel slot.
    /// See /spec/lighting-routing.md § Guards.
    #[must_use]
    pub fn is_additive_color(self) -> bool {
        matches!(
            self,
            Role::Red
                | Role::Green
                | Role::Blue
                | Role::White
                | Role::Amber
                | Role::Uv
                | Role::Lime
                | Role::Cyan
                | Role::Magenta
                | Role::Yellow
        )
    }

    /// Stable lowercase identifier used in parameter paths (`fixture/<uuid>/<role>`),
    /// persistence, and the palette format.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Dimmer => "dimmer",
            Role::Shutter => "shutter",
            Role::Strobe => "strobe",
            Role::Red => "red",
            Role::Green => "green",
            Role::Blue => "blue",
            Role::White => "white",
            Role::Amber => "amber",
            Role::Uv => "uv",
            Role::Lime => "lime",
            Role::Cyan => "cyan",
            Role::Magenta => "magenta",
            Role::Yellow => "yellow",
            Role::Cto => "cto",
            Role::Ctb => "ctb",
            Role::ColorWheel => "color_wheel",
            Role::Pan => "pan",
            Role::Tilt => "tilt",
            Role::PanTiltSpeed => "pan_tilt_speed",
            Role::Zoom => "zoom",
            Role::Focus => "focus",
            Role::Iris => "iris",
            Role::Frost => "frost",
            Role::Prism => "prism",
            Role::PrismRotation => "prism_rotation",
            Role::GoboWheel => "gobo_wheel",
            Role::GoboRotation => "gobo_rotation",
            Role::GoboShake => "gobo_shake",
            Role::Function => "function",
            Role::Reset => "reset",
            Role::LampControl => "lamp_control",
        }
    }

    /// Parse a role from its stable identifier. The inverse of [`Role::as_str`].
    #[must_use]
    pub fn parse(s: &str) -> Option<Role> {
        Role::ALL.iter().copied().find(|r| r.as_str() == s)
    }
}

// Serialized as the stable string identifier, never as the variant name. The identifier is the
// same one parameter paths and the palette format use, so renaming a Rust variant cannot
// silently invalidate a saved show.
impl serde::Serialize for Role {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for Role {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Role::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("unknown role {text:?}")))
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_round_trips_for_every_role() {
        for &role in Role::ALL {
            assert_eq!(
                Role::parse(role.as_str()),
                Some(role),
                "{role} failed to round-trip"
            );
        }
    }

    #[test]
    fn role_identifiers_are_unique() {
        let mut seen: Vec<&str> = Role::ALL.iter().map(|r| r.as_str()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "duplicate role identifier");
    }

    #[test]
    fn all_contains_every_variant_exactly_once() {
        // Guards against a role being added to the enum but not to ALL, which would make it
        // invisible to the palette editor and unparseable from persistence.
        let mut ids: Vec<&str> = Role::ALL.iter().map(|r| r.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Role::ALL.len());
        assert_eq!(Role::ALL.len(), 31, "update this count when adding a role");
    }

    #[test]
    fn index_is_dense_and_unique() {
        let mut seen = vec![false; Role::COUNT];
        for &r in Role::ALL {
            let i = r.index();
            assert!(i < Role::COUNT, "{r} index {i} out of range");
            assert!(!seen[i], "{r} duplicate index {i}");
            seen[i] = true;
        }
        assert!(seen.into_iter().all(|b| b), "index space must be dense");
    }

    #[test]
    fn count_matches_all() {
        assert_eq!(Role::COUNT, Role::ALL.len());
    }

    /// Smoothing a selection channel walks it through every slot between old and new, so a
    /// color-wheel change would spin the wheel through every gel on the way.
    #[test]
    fn selection_and_rate_channels_are_never_smoothed() {
        for r in [
            Role::ColorWheel,
            Role::GoboWheel,
            Role::GoboShake,
            Role::Shutter,
            Role::Strobe,
            Role::Prism,
            Role::Function,
            Role::Reset,
            Role::LampControl,
            Role::PanTiltSpeed,
        ] {
            assert_eq!(r.smoothing(), SmoothingClass::Discrete, "{r}");
        }
    }

    #[test]
    fn position_roles_smooth_without_a_transient_class() {
        assert_eq!(Role::Pan.smoothing(), SmoothingClass::Position);
        assert_eq!(Role::Tilt.smoothing(), SmoothingClass::Position);
    }

    #[test]
    fn additive_color_roles_share_the_color_smoothing_class() {
        for &r in Role::ALL {
            if r.is_additive_color() {
                assert_eq!(r.smoothing(), SmoothingClass::Color, "{r}");
            }
        }
    }

    #[test]
    fn continuous_beam_roles_smooth() {
        for r in [Role::Zoom, Role::Focus, Role::Iris, Role::Frost] {
            assert_eq!(r.smoothing(), SmoothingClass::Beam, "{r}");
        }
    }

    #[test]
    fn role_serializes_as_its_stable_identifier() {
        let json = serde_json::to_string(&Role::ColorWheel).unwrap();
        assert_eq!(
            json, "\"color_wheel\"",
            "renaming a Rust variant must not invalidate a saved show"
        );
        let back: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Role::ColorWheel);
    }

    #[test]
    fn every_role_survives_a_json_round_trip() {
        for &role in Role::ALL {
            let json = serde_json::to_string(&role).unwrap();
            let back: Role = serde_json::from_str(&json).unwrap();
            assert_eq!(back, role, "{role} failed");
        }
    }

    #[test]
    fn an_unknown_role_string_is_a_deserialize_error() {
        assert!(serde_json::from_str::<Role>("\"not_a_role\"").is_err());
    }

    #[test]
    fn parse_rejects_unknown() {
        assert_eq!(Role::parse("not_a_role"), None);
        assert_eq!(Role::parse(""), None);
    }

    #[test]
    fn intensity_roles_group_as_intensity() {
        for r in [Role::Dimmer, Role::Shutter, Role::Strobe] {
            assert_eq!(r.group(), RoleGroup::Intensity);
        }
    }

    #[test]
    fn position_roles_group_as_position() {
        for r in [Role::Pan, Role::Tilt, Role::PanTiltSpeed] {
            assert_eq!(r.group(), RoleGroup::Position);
        }
    }

    #[test]
    fn selection_and_correction_channels_are_not_additive_color() {
        // The white ceiling guard must not touch these: re-saturating a color wheel slot or a
        // CTO correction value is meaningless and produces a wrong gel.
        assert!(!Role::ColorWheel.is_additive_color());
        assert!(!Role::Cto.is_additive_color());
        assert!(!Role::Ctb.is_additive_color());
        assert!(Role::Red.is_additive_color());
        assert!(Role::White.is_additive_color());
    }

    #[test]
    fn no_position_or_beam_role_is_additive_color() {
        for &r in Role::ALL {
            if matches!(r.group(), RoleGroup::Position | RoleGroup::Beam) {
                assert!(!r.is_additive_color(), "{r} must not be guarded as color");
            }
        }
    }
}
