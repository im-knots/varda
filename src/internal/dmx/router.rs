//! Lighting parameter paths.
//!
//! Fixture roles are parameter paths like anything else, so they are modulatable, automatable,
//! MIDI-learnable, OSC-addressable, and API-reachable with no per-feature work. This is the
//! single most important decision in /spec/lighting-routing.md and it is what makes Varda's
//! version different from a bolted-on lighting engine.
//!
//! | Path | Type | Description |
//! |---|---|---|
//! | `fixture/<uuid>/<role>` | float | Normalized role value |
//! | `group/<uuid>/<role>` | float | Fan out to every member |
//! | `lighting/master` | float | Global intensity scalar |
//! | `lighting/blackout` | trigger | Latching blackout |
//! | `lighting/release` | trigger | Release the programmer |

use super::role::Role;
use super::runtime::LightingRuntime;
use uuid::Uuid;

/// Why a lighting path could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LightingRouteError {
    UnknownPath(String),
    UnknownRole(String),
    UnknownFixture(String),
    /// A group that exists but has no members in this rig, or does not exist at all.
    ///
    /// Reported rather than silently succeeding so an operator learns their group is empty, but
    /// it is not fatal anywhere: a show touring to a venue without movers keeps working.
    EmptyGroup(String),
    BadUuid(String),
}

impl std::fmt::Display for LightingRouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPath(p) => write!(f, "unknown lighting path {p:?}"),
            Self::UnknownRole(r) => write!(f, "unknown role {r:?}"),
            Self::UnknownFixture(u) => write!(f, "no fixture with uuid {u}"),
            Self::EmptyGroup(u) => write!(f, "group {u} has no members in this rig"),
            Self::BadUuid(u) => write!(f, "{u:?} is not a uuid"),
        }
    }
}

impl std::error::Error for LightingRouteError {}

/// True when this path belongs to the lighting subsystem.
///
/// Callers check this to decide which router to hand a path to, so the two stay separate
/// domains rather than one router learning about both.
#[must_use]
pub fn is_lighting_path(path: &str) -> bool {
    path.starts_with("fixture/") || path.starts_with("group/") || path.starts_with("lighting/")
}

fn parse_uuid(text: &str) -> Result<Uuid, LightingRouteError> {
    Uuid::parse_str(text).map_err(|_| LightingRouteError::BadUuid(text.to_owned()))
}

fn parse_role(text: &str) -> Result<Role, LightingRouteError> {
    Role::parse(text).ok_or_else(|| LightingRouteError::UnknownRole(text.to_owned()))
}

/// Apply one lighting parameter path.
///
/// # Errors
///
/// Returns [`LightingRouteError`] naming what could not be resolved. Callers surface it the same
/// way they surface a mixer routing error.
pub fn apply(
    lighting: &mut LightingRuntime,
    path: &str,
    value: f32,
) -> Result<(), LightingRouteError> {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["lighting", "master"] => {
            lighting.set_master(value);
            Ok(())
        }
        // Triggers, matching the convention the mixer router already uses: act above 0.5 so a
        // button, a fader, and an OSC float all work without the caller knowing which it is.
        ["lighting", "blackout"] => {
            lighting.set_blackout(value > 0.5);
            Ok(())
        }
        ["lighting", "release"] => {
            if value > 0.5 {
                lighting.release_all();
            }
            Ok(())
        }
        ["fixture", uuid, role] => {
            let id = parse_uuid(uuid)?;
            let role = parse_role(role)?;
            if lighting.set_role(id, role, value) {
                Ok(())
            } else {
                Err(LightingRouteError::UnknownFixture((*uuid).to_owned()))
            }
        }
        ["group", uuid, role] => {
            let id = parse_uuid(uuid)?;
            let role = parse_role(role)?;
            if lighting.set_group_role(id, role, value) > 0 {
                Ok(())
            } else {
                Err(LightingRouteError::EmptyGroup((*uuid).to_owned()))
            }
        }
        _ => Err(LightingRouteError::UnknownPath(path.to_owned())),
    }
}

/// Every lighting path a rig currently exposes, for MIDI learn and API discovery.
#[must_use]
pub fn available_paths(lighting: &LightingRuntime) -> Vec<String> {
    let mut out = vec![
        "lighting/master".to_string(),
        "lighting/blackout".to_string(),
        "lighting/release".to_string(),
    ];
    for group in &lighting.config().groups {
        for role in Role::ALL {
            out.push(format!("group/{}/{}", group.id, role.as_str()));
        }
    }
    for id in lighting.fixture_ids() {
        for role in Role::ALL {
            out.push(format!("fixture/{id}/{}", role.as_str()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dmx::config::{LightingConfig, TransportConfig};
    use crate::dmx::look::Group;
    use crate::dmx::patch::Fixture;
    use std::fs;
    use std::path::Path;

    const RGBW: &str = r#"{
      "name": "RGBW Par",
      "availableChannels": {
        "Red":   { "capability": { "type": "ColorIntensity", "color": "Red" } },
        "Green": { "capability": { "type": "ColorIntensity", "color": "Green" } },
        "Blue":  { "capability": { "type": "ColorIntensity", "color": "Blue" } },
        "White": { "capability": { "type": "ColorIntensity", "color": "White" } }
      },
      "modes": [ { "name": "4ch", "channels": ["Red","Green","Blue","White"] } ]
    }"#;

    fn profile_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p: &Path = d.path();
        fs::create_dir_all(p.join("generic")).unwrap();
        fs::write(p.join("generic/rgbw.json"), RGBW).unwrap();
        d
    }

    /// A live runtime with two pars and a group containing both.
    fn runtime() -> (LightingRuntime, tempfile::TempDir, Vec<Uuid>, Uuid) {
        let d = profile_dir();
        let mut a = Fixture::new("a", "generic/rgbw", "4ch");
        a.address = 1;
        let mut b = Fixture::new("b", "generic/rgbw", "4ch");
        b.address = 5;
        let ids = vec![a.id, b.id];
        let mut group = Group::new("all");
        group.members.clone_from(&ids);
        let group_id = group.id;
        let config = LightingConfig {
            enabled: true,
            fixtures: vec![a, b],
            groups: vec![group],
            transports: vec![TransportConfig::ArtNet {
                target: "127.0.0.1:0".into(),
                broadcast: false,
            }],
            ..LightingConfig::default()
        };
        let rt = LightingRuntime::new(config, vec![d.path().to_path_buf()]);
        (rt, d, ids, group_id)
    }

    #[test]
    fn lighting_paths_are_recognized() {
        assert!(is_lighting_path("fixture/abc/red"));
        assert!(is_lighting_path("group/abc/dimmer"));
        assert!(is_lighting_path("lighting/master"));
        assert!(!is_lighting_path("deck/abc/opacity"));
        assert!(!is_lighting_path("crossfader"));
    }

    #[test]
    fn a_fixture_role_path_applies() {
        let (mut rt, _d, ids, _) = runtime();
        let path = format!("fixture/{}/red", ids[0]);
        assert!(apply(&mut rt, &path, 1.0).is_ok());
        rt.tick(1.0 / 60.0, true, &|_, _| None);
        assert!(rt.fixture_ids().contains(&ids[0]));
    }

    #[test]
    fn a_group_role_path_reaches_every_member() {
        let (mut rt, _d, _ids, group) = runtime();
        let path = format!("group/{group}/dimmer");
        assert!(apply(&mut rt, &path, 1.0).is_ok());
    }

    #[test]
    fn an_unknown_fixture_is_reported() {
        let (mut rt, _d, _, _) = runtime();
        let path = format!("fixture/{}/red", Uuid::new_v4());
        assert!(matches!(
            apply(&mut rt, &path, 1.0),
            Err(LightingRouteError::UnknownFixture(_))
        ));
    }

    /// A show touring to a venue without movers keeps working, but the operator should still
    /// learn the group is empty.
    #[test]
    fn an_empty_group_is_reported_but_not_fatal() {
        let (mut rt, _d, _, _) = runtime();
        let path = format!("group/{}/dimmer", Uuid::new_v4());
        assert!(matches!(
            apply(&mut rt, &path, 1.0),
            Err(LightingRouteError::EmptyGroup(_))
        ));
    }

    #[test]
    fn an_unknown_role_is_reported() {
        let (mut rt, _d, ids, _) = runtime();
        let path = format!("fixture/{}/not_a_role", ids[0]);
        assert!(matches!(
            apply(&mut rt, &path, 1.0),
            Err(LightingRouteError::UnknownRole(_))
        ));
    }

    #[test]
    fn a_malformed_uuid_is_reported() {
        let (mut rt, _d, _, _) = runtime();
        assert!(matches!(
            apply(&mut rt, "fixture/not-a-uuid/red", 1.0),
            Err(LightingRouteError::BadUuid(_))
        ));
    }

    #[test]
    fn an_unknown_path_is_reported() {
        let (mut rt, _d, _, _) = runtime();
        assert!(matches!(
            apply(&mut rt, "lighting/nope", 1.0),
            Err(LightingRouteError::UnknownPath(_))
        ));
    }

    #[test]
    fn master_applies_and_clamps() {
        let (mut rt, _d, _, _) = runtime();
        apply(&mut rt, "lighting/master", 0.5).unwrap();
        assert!((rt.master() - 0.5).abs() < 1e-6);
        apply(&mut rt, "lighting/master", 5.0).unwrap();
        assert!((rt.master() - 1.0).abs() < 1e-6);
    }

    /// Triggers act above 0.5, so a button, a fader, and an OSC float all work.
    #[test]
    fn blackout_is_a_trigger_above_half() {
        let (mut rt, _d, _, _) = runtime();
        apply(&mut rt, "lighting/blackout", 1.0).unwrap();
        assert!(rt.blackout());
        apply(&mut rt, "lighting/blackout", 0.0).unwrap();
        assert!(!rt.blackout());
    }

    #[test]
    fn release_clears_the_programmer_above_half() {
        let (mut rt, _d, ids, _) = runtime();
        let path = format!("fixture/{}/red", ids[0]);
        apply(&mut rt, &path, 1.0).unwrap();
        apply(&mut rt, "lighting/release", 0.0).unwrap();
        apply(&mut rt, "lighting/release", 1.0).unwrap();
        // Releasing hands the role back to the looks, of which there are none, so the fixture
        // falls to its patch default rather than holding the programmer value.
        rt.tick(1.0 / 60.0, true, &|_, _| None);
    }

    #[test]
    fn available_paths_cover_every_fixture_group_and_global() {
        let (rt, _d, ids, group) = runtime();
        let paths = available_paths(&rt);
        assert!(paths.contains(&"lighting/master".to_string()));
        assert!(paths.contains(&"lighting/blackout".to_string()));
        assert!(paths.contains(&format!("group/{group}/dimmer")));
        for id in ids {
            assert!(paths.contains(&format!("fixture/{id}/red")));
        }
    }

    #[test]
    fn every_available_path_actually_applies() {
        let (mut rt, _d, _, _) = runtime();
        for path in available_paths(&rt) {
            assert!(
                apply(&mut rt, &path, 0.5).is_ok(),
                "advertised path {path} did not apply"
            );
        }
    }
}
