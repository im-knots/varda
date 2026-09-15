//! Persisted lighting configuration.
//!
//! Venue state: the rig belongs to the room, not to the show. Lives in `stage.json` alongside
//! surfaces, outputs, and warp calibration. See /spec/dmx-output.md § Persistence.

use super::guard::WhiteGuardConfig;
use super::patch::Fixture;
use super::transport::{ArtNetSender, DmxTransport, SacnDestination, SacnSender};
use super::{artnet, sacn};
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

/// Where sACN packets are addressed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SacnDestinationConfig {
    /// Standard E1.31 multicast, one group per universe.
    #[default]
    Multicast,
    /// Unicast to one receiver, for networks where multicast is filtered.
    Unicast { target: String },
}

/// One configured output transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "kebab-case")]
pub enum TransportConfig {
    ArtNet {
        /// `host:port`, or `host` to use the standard Art-Net port.
        target: String,
        #[serde(default)]
        broadcast: bool,
    },
    Sacn {
        #[serde(default)]
        destination: SacnDestinationConfig,
        /// Outgoing interface address; empty lets the OS choose.
        #[serde(default)]
        interface: String,
    },
}

/// Parse `host:port`, defaulting the port when absent.
fn parse_target(text: &str, default_port: u16) -> Result<SocketAddr, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("address is empty".to_string());
    }
    if let Ok(addr) = trimmed.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let ip: Ipv4Addr = trimmed
        .parse()
        .map_err(|_| format!("{trimmed:?} is not an IPv4 address or host:port"))?;
    Ok(SocketAddr::from((ip, default_port)))
}

impl TransportConfig {
    /// Build a live transport.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the address cannot be parsed or the socket cannot
    /// be bound. The caller surfaces it as a patch-level finding rather than failing the app.
    pub fn build(
        &self,
        source_name: &str,
        cid: [u8; 16],
        priority: u8,
    ) -> Result<Box<dyn DmxTransport>, String> {
        match self {
            Self::ArtNet { target, broadcast } => {
                let addr = parse_target(target, artnet::ARTNET_PORT)?;
                let sender =
                    ArtNetSender::new(addr, *broadcast).map_err(|e| format!("art-net: {e}"))?;
                Ok(Box::new(sender))
            }
            Self::Sacn {
                destination,
                interface,
            } => {
                let iface: Ipv4Addr = if interface.trim().is_empty() {
                    Ipv4Addr::UNSPECIFIED
                } else {
                    interface.trim().parse().map_err(|_| {
                        format!("sacn interface {interface:?} is not an IPv4 address")
                    })?
                };
                let dest = match destination {
                    SacnDestinationConfig::Multicast => SacnDestination::Multicast,
                    SacnDestinationConfig::Unicast { target } => {
                        SacnDestination::Unicast(parse_target(target, sacn::SACN_PORT)?)
                    }
                };
                let mut cfg = sacn::SourceConfig::new(cid, source_name);
                cfg.priority = priority;
                let sender = SacnSender::new(cfg, dest, iface).map_err(|e| format!("sacn: {e}"))?;
                Ok(Box::new(sender))
            }
        }
    }
}

fn default_fps() -> u32 {
    super::driver::DEFAULT_FPS
}
fn default_source_name() -> String {
    "Varda".to_string()
}
fn default_priority() -> u8 {
    sacn::DEFAULT_PRIORITY
}

/// The whole lighting rig as persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightingConfig {
    /// Master switch. False leaves the subsystem inert even with fixtures patched, which is what
    /// an operator wants when rehearsing without the rig powered.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default)]
    pub transports: Vec<TransportConfig>,
    /// Advertised sACN source name.
    #[serde(default = "default_source_name")]
    pub source_name: String,
    /// sACN per-universe priority.
    #[serde(default = "default_priority")]
    pub priority: u8,
    /// Stable sACN Component Identifier.
    ///
    /// Must persist across restarts. A source that invents a new CID each launch appears to
    /// receivers as a second, competing source, and merging consoles treat the two as rivals
    /// for the same universe. Generated once and then left alone.
    #[serde(default = "uuid::Uuid::new_v4")]
    pub cid: uuid::Uuid,
    #[serde(default)]
    pub white_guard: WhiteGuardConfig,
    /// Extra directories searched for fixture profiles, ahead of the bundled library, so a
    /// workspace definition always overrides a shipped one.
    #[serde(default)]
    pub profile_dirs: Vec<PathBuf>,
    #[serde(default)]
    pub fixtures: Vec<Fixture>,
    /// Named fixture sets.
    ///
    /// Venue state, because membership is a list of fixture UUIDs and fixtures belong to the
    /// room. A show references a group by UUID and the venue decides what is in it, which is
    /// exactly what lets the same show drive a different rig.
    #[serde(default)]
    pub groups: Vec<super::look::Group>,
    /// Position palettes.
    ///
    /// Venue state: "Downstage Centre" is a different pan and tilt for every mover on the truss
    /// and a different set of angles in every room. Color and beam palettes live in
    /// `scene.json` instead. See /spec/lighting-routing.md § Palettes.
    #[serde(default)]
    pub palettes: Vec<super::palette::Palette>,
}

impl Default for LightingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            fps: default_fps(),
            transports: Vec::new(),
            source_name: default_source_name(),
            priority: default_priority(),
            cid: uuid::Uuid::new_v4(),
            white_guard: WhiteGuardConfig::default(),
            profile_dirs: Vec::new(),
            fixtures: Vec::new(),
            groups: Vec::new(),
            palettes: Vec::new(),
        }
    }
}

impl LightingConfig {
    /// True when this config should actually drive anything.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.enabled && !self.fixtures.is_empty() && !self.transports.is_empty()
    }

    /// The CID as the 16 raw bytes E1.31 carries.
    #[must_use]
    pub fn cid_bytes(&self) -> [u8; 16] {
        *self.cid.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_inert() {
        let c = LightingConfig::default();
        assert!(!c.enabled);
        assert!(!c.is_live(), "an unconfigured rig must drive nothing");
        assert!(c.fixtures.is_empty());
        assert!(!c.white_guard.enabled, "the guard is opt-in");
    }

    #[test]
    fn an_empty_json_object_loads_as_default() {
        // Guards the promise that an existing .varda/ gains an empty rig rather than failing.
        let c: LightingConfig = serde_json::from_str("{}").expect("every field defaults");
        assert!(!c.enabled);
        assert_eq!(c.fps, super::super::driver::DEFAULT_FPS);
        assert_eq!(c.source_name, "Varda");
    }

    #[test]
    fn config_round_trips_through_json() {
        let mut c = LightingConfig {
            enabled: true,
            ..LightingConfig::default()
        };
        c.transports.push(TransportConfig::ArtNet {
            target: "10.0.0.5".into(),
            broadcast: false,
        });
        c.transports.push(TransportConfig::Sacn {
            destination: SacnDestinationConfig::Multicast,
            interface: String::new(),
        });
        let text = serde_json::to_string(&c).unwrap();
        let back: LightingConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn cid_survives_a_round_trip() {
        let c = LightingConfig::default();
        let text = serde_json::to_string(&c).unwrap();
        let back: LightingConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(
            c.cid, back.cid,
            "a changing CID makes receivers see a second competing source"
        );
        assert_eq!(back.cid_bytes().len(), 16);
    }

    #[test]
    fn a_bare_ip_takes_the_protocol_default_port() {
        assert_eq!(
            parse_target("10.0.0.5", artnet::ARTNET_PORT).unwrap(),
            "10.0.0.5:6454".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn an_explicit_port_is_honoured() {
        assert_eq!(
            parse_target("10.0.0.5:9999", artnet::ARTNET_PORT).unwrap(),
            "10.0.0.5:9999".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn a_bad_address_is_reported_not_panicked() {
        assert!(parse_target("not an address", 1).is_err());
        assert!(parse_target("", 1).is_err());
    }

    #[test]
    fn artnet_transport_builds() {
        let t = TransportConfig::ArtNet {
            target: "127.0.0.1:0".into(),
            broadcast: false,
        };
        let built = t.build("Varda", [0; 16], 100).expect("builds");
        assert!(built.label().starts_with("art-net"));
    }

    #[test]
    fn sacn_transport_builds() {
        let t = TransportConfig::Sacn {
            destination: SacnDestinationConfig::Multicast,
            interface: "127.0.0.1".into(),
        };
        let built = t.build("Varda", [0; 16], 100).expect("builds");
        assert_eq!(built.label(), "sacn multicast");
    }

    #[test]
    fn a_malformed_transport_address_reports_rather_than_binding() {
        let t = TransportConfig::ArtNet {
            target: "nope".into(),
            broadcast: false,
        };
        assert!(t.build("Varda", [0; 16], 100).is_err());
    }

    #[test]
    fn is_live_needs_fixtures_and_a_transport() {
        let mut c = LightingConfig {
            enabled: true,
            ..LightingConfig::default()
        };
        assert!(!c.is_live(), "enabled alone is not enough");
        c.transports.push(TransportConfig::ArtNet {
            target: "10.0.0.5".into(),
            broadcast: false,
        });
        assert!(!c.is_live(), "a transport with no fixtures drives nothing");
        c.fixtures.push(Fixture::new("par", "generic/rgbw", "4ch"));
        assert!(c.is_live());
    }
}
