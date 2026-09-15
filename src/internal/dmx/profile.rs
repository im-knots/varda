//! The internal fixture profile model.
//!
//! Every on-disk format normalizes into these types and nothing downstream sees the source
//! format. Open Fixture Library JSON is the bundled loader ([`super::ofl`]); GDTF is a later
//! second loader against this same model, not a redesign.
//!
//! See /spec/dmx-output.md § Profile Format.

use super::role::Role;
use super::universe::Resolution;
use std::fmt;

/// A fixture definition: one physical product, with one or more DMX personalities.
#[derive(Debug, Clone, PartialEq)]
pub struct FixtureProfile {
    pub vendor: String,
    pub model: String,
    pub modes: Vec<ProfileMode>,
    /// Things the loader had to decide that an operator should know about.
    ///
    /// Surfaced as patch warnings naming the fixture, so a resolution is reported rather than
    /// silent. See [`super::ofl`] on switching channels.
    pub notes: Vec<String>,
}

/// One DMX personality of a fixture. Channel index is the offset from the start address.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileMode {
    pub name: String,
    pub channels: Vec<ProfileChannel>,
}

/// One channel of a mode.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileChannel {
    pub name: String,
    /// `None` for a channel with no engine-writable meaning, which covers both unrecognized
    /// functions (an effect macro, a maintenance channel) and the fine byte of a 16-bit pair,
    /// whose value is written by its coarse partner.
    pub role: Option<Role>,
    pub resolution: Resolution,
    pub default: u8,
    /// Labelled value ranges, for the patch UI. Not used by the output path.
    pub ranges: Vec<ChannelRange>,
}

/// A labelled span of DMX values within a channel, used to show an operator what a slot means.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelRange {
    pub lo: u8,
    pub hi: u8,
    pub label: String,
}

impl FixtureProfile {
    #[must_use]
    pub fn mode(&self, name: &str) -> Option<&ProfileMode> {
        self.modes.iter().find(|m| m.name == name)
    }

    /// Display identity, used in patch errors and the fixture picker.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} {}", self.vendor, self.model)
    }
}

impl ProfileMode {
    /// Slots this mode occupies. A 16-bit pair counts as the two slots it actually uses,
    /// because both appear as entries in the mode's channel list.
    #[must_use]
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Every role this mode can be driven through, in channel order, with duplicates removed.
    #[must_use]
    pub fn roles(&self) -> Vec<Role> {
        let mut out = Vec::new();
        for ch in &self.channels {
            if let Some(r) = ch.role
                && !out.contains(&r)
            {
                out.push(r);
            }
        }
        out
    }
}

/// Why a profile could not be loaded.
///
/// [`ProfileError::Unsupported`] is deliberately distinct from a parse failure: per
/// /spec/dmx-output.md a definition using a feature this release does not implement must be
/// rejected **by name** rather than loaded at reduced fidelity, because a silently mis-parsed
/// profile is a fixture that behaves wrongly on stage with no error anywhere.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileError {
    Io(String),
    Parse {
        path: String,
        msg: String,
    },
    Empty(String),
    NoSuchMode {
        profile: String,
        mode: String,
    },
    Unsupported {
        path: String,
        feature: String,
    },
    /// The definition is a stub pointing at another fixture.
    ///
    /// Open Fixture Library uses these for renames and for the same product sold under a
    /// different brand. The library follows them; a bare parse cannot, because it has no
    /// search path.
    Redirect {
        path: String,
        to: String,
    },
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Parse { path, msg } => write!(f, "{path}: {msg}"),
            Self::Empty(p) => write!(f, "{p}: profile defines no channels"),
            Self::NoSuchMode { profile, mode } => {
                write!(f, "{profile}: no mode named {mode:?}")
            }
            Self::Unsupported { path, feature } => write!(
                f,
                "{path}: uses {feature}, which this release does not implement"
            ),
            Self::Redirect { path, to } => write!(f, "{path}: redirects to {to:?}"),
        }
    }
}

impl std::error::Error for ProfileError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(name: &str, role: Option<Role>) -> ProfileChannel {
        ProfileChannel {
            name: name.into(),
            role,
            resolution: Resolution::Eight,
            default: 0,
            ranges: Vec::new(),
        }
    }

    fn profile() -> FixtureProfile {
        FixtureProfile {
            vendor: "Generic".into(),
            notes: Vec::new(),
            model: "RGBW Par".into(),
            modes: vec![ProfileMode {
                name: "4-channel".into(),
                channels: vec![
                    ch("Red", Some(Role::Red)),
                    ch("Green", Some(Role::Green)),
                    ch("Blue", Some(Role::Blue)),
                    ch("White", Some(Role::White)),
                ],
            }],
        }
    }

    #[test]
    fn mode_lookup_by_name() {
        let p = profile();
        assert!(p.mode("4-channel").is_some());
        assert!(p.mode("nope").is_none());
    }

    #[test]
    fn channel_count_is_slot_count() {
        assert_eq!(profile().mode("4-channel").unwrap().channel_count(), 4);
    }

    #[test]
    fn roles_are_listed_in_order_without_duplicates() {
        let m = ProfileMode {
            name: "m".into(),
            channels: vec![
                ch("Dim", Some(Role::Dimmer)),
                ch("Red", Some(Role::Red)),
                ch("Macro", None),
                ch("Red2", Some(Role::Red)),
            ],
        };
        assert_eq!(m.roles(), vec![Role::Dimmer, Role::Red]);
    }

    #[test]
    fn label_identifies_the_product() {
        assert_eq!(profile().label(), "Generic RGBW Par");
    }

    #[test]
    fn unsupported_error_names_the_feature() {
        let e = ProfileError::Unsupported {
            path: "acme/mover.json".into(),
            feature: "switchChannels".into(),
        };
        let s = e.to_string();
        assert!(s.contains("switchChannels"), "must name the feature: {s}");
        assert!(s.contains("acme/mover.json"), "must name the file: {s}");
    }

    #[test]
    fn no_such_mode_error_names_both() {
        let e = ProfileError::NoSuchMode {
            profile: "Generic RGBW Par".into(),
            mode: "8-channel".into(),
        };
        let s = e.to_string();
        assert!(s.contains("Generic RGBW Par") && s.contains("8-channel"));
    }
}
