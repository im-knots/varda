//! OSC (Open Sound Control) support for Varda.
//!
//! All incoming messages use the `/varda/` namespace prefix.
//! Addresses map 1:1 to the shared parameter path system in [`crate::param_router`].
//! Clock messages (`/varda/clock/bpm`, `/varda/clock/beat`) bypass the param router
//! and route directly to [`crate::clock::ClockManager`].

use anyhow::{Context, Result};
use rosc::{OscMessage, OscPacket, OscType};
use serde::{Deserialize, Serialize};
use std::net::{SocketAddrV4, UdpSocket};
use std::path::Path;
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;

// ── OscConfig ────────────────────────────────────────────────────────

/// Persisted OSC configuration (`.varda/osc.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OscConfig {
    /// Whether OSC input is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// UDP port to listen on for incoming OSC messages.
    #[serde(default = "default_in_port")]
    pub in_port: u16,
    /// Feedback targets (ip:port) to echo parameter changes to.
    #[serde(default)]
    pub feedback_targets: Vec<String>,
}

fn default_true() -> bool { true }
fn default_in_port() -> u16 { 9000 }

impl Default for OscConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            in_port: 9000,
            feedback_targets: Vec::new(),
        }
    }
}

impl OscConfig {
    /// Load from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read OSC config: {}", path.as_ref().display()))?;
        let config: OscConfig = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse OSC config: {}", path.as_ref().display()))?;
        Ok(config)
    }

    /// Save to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize OSC config")?;
        std::fs::write(path.as_ref(), content)
            .with_context(|| format!("Failed to write OSC config: {}", path.as_ref().display()))?;
        Ok(())
    }
}

/// The namespace prefix for all Varda OSC addresses.
pub const OSC_PREFIX: &str = "/varda/";

/// A parsed OSC input message ready for routing.
#[derive(Debug, Clone, PartialEq)]
pub enum OscInput {
    /// Parameter path + normalized value → [`crate::param_router::apply_param_by_path`]
    Param { path: String, value: f32 },
    /// Clock BPM (raw, not normalized, e.g. 120.0)
    ClockBpm(f32),
    /// Clock beat phase (0.0–1.0)
    ClockBeat(f32),
    /// Address not recognized (logged for debugging)
    Unknown(String),
}

/// Extract a float value from the first OSC argument.
/// Accepts Float, Int (cast to float), and Bool (1.0 / 0.0).
fn extract_float(args: &[OscType]) -> Option<f32> {
    match args.first()? {
        OscType::Float(f) => Some(*f),
        OscType::Int(i) => Some(*i as f32),
        OscType::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Parse an OSC address + args into an [`OscInput`].
///
/// Strips the `/varda/` prefix, then routes clock messages specially
/// and everything else as a generic `Param` message.
pub fn parse_osc_message(addr: &str, args: &[OscType]) -> OscInput {
    let path = if let Some(stripped) = addr.strip_prefix(OSC_PREFIX) {
        stripped
    } else if let Some(stripped) = addr.strip_prefix("/varda") {
        // Handle `/varda` without trailing slash (e.g. `/vardacrossfader` should be Unknown)
        if stripped.is_empty() { return OscInput::Unknown(addr.to_string()); }
        // `/varda` followed by non-`/` → not our namespace
        return OscInput::Unknown(addr.to_string());
    } else {
        return OscInput::Unknown(addr.to_string());
    };

    // Strip any trailing slash
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return OscInput::Unknown(addr.to_string());
    }

    match path {
        "clock/bpm" => {
            if let Some(v) = extract_float(args) {
                OscInput::ClockBpm(v)
            } else {
                OscInput::Unknown(addr.to_string())
            }
        }
        "clock/beat" => {
            if let Some(v) = extract_float(args) {
                OscInput::ClockBeat(v.clamp(0.0, 1.0))
            } else {
                OscInput::Unknown(addr.to_string())
            }
        }
        _ => {
            if let Some(v) = extract_float(args) {
                OscInput::Param { path: path.to_string(), value: v }
            } else {
                OscInput::Unknown(addr.to_string())
            }
        }
    }
}

// ── OscReceiver ────────────────────────────────────────────────────

/// OSC receiver — background thread listening on a UDP port.
pub struct OscReceiver {
    receiver: Receiver<OscInput>,
    _thread: thread::JoinHandle<()>,
}

impl OscReceiver {
    /// Create a new OSC receiver listening on the given port.
    pub fn new(port: u16) -> Result<Self> {
        let addr = SocketAddrV4::from_str(&format!("0.0.0.0:{}", port))
            .context("Invalid address")?;
        let socket = UdpSocket::bind(addr)
            .context(format!("Failed to bind to port {}", port))?;
        socket.set_read_timeout(Some(std::time::Duration::from_millis(100)))?;

        let (sender, receiver) = channel();
        log::info!("OSC receiver listening on port {}", port);

        let _thread = thread::spawn(move || {
            Self::receive_loop(socket, sender);
        });
        Ok(Self { receiver, _thread })
    }

    fn receive_loop(socket: UdpSocket, sender: Sender<OscInput>) {
        let mut buf = [0u8; rosc::decoder::MTU];
        loop {
            match socket.recv_from(&mut buf) {
                Ok((size, _addr)) => {
                    if let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..size]) {
                        Self::handle_packet(packet, &sender);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    log::error!("OSC receive error: {}", e);
                    break;
                }
            }
        }
    }

    fn handle_packet(packet: OscPacket, sender: &Sender<OscInput>) {
        match packet {
            OscPacket::Message(msg) => {
                let input = parse_osc_message(&msg.addr, &msg.args);
                let _ = sender.send(input);
            }
            OscPacket::Bundle(bundle) => {
                for p in bundle.content {
                    Self::handle_packet(p, sender);
                }
            }
        }
    }

    /// Get the next parsed OSC input (non-blocking).
    pub fn try_recv(&self) -> Option<OscInput> {
        match self.receiver.try_recv() {
            Ok(input) => Some(input),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }
}

// ── OscFeedbackSender ──────────────────────────────────────────────

/// Sends OSC feedback to one or more UDP targets (TouchOSC, Lemur, etc.).
/// Echoes parameter changes back so external controllers stay in sync.
pub struct OscFeedbackSender {
    socket: UdpSocket,
    targets: Vec<SocketAddrV4>,
}

impl OscFeedbackSender {
    /// Create a feedback sender. Targets are added via [`add_target`].
    pub fn new() -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .context("Failed to create UDP socket for OSC feedback")?;
        Ok(Self { socket, targets: Vec::new() })
    }

    /// Add a feedback target (ip:port).
    pub fn add_target(&mut self, addr: &str) -> Result<()> {
        let target = SocketAddrV4::from_str(addr)
            .context(format!("Invalid feedback target: {}", addr))?;
        if !self.targets.contains(&target) {
            self.targets.push(target);
            log::info!("OSC feedback target added: {}", addr);
        }
        Ok(())
    }

    /// Send a parameter change to all feedback targets.
    /// `path` is the param router path (e.g. `deck/abc123/opacity`).
    pub fn send_param(&self, path: &str, value: f32) {
        let addr = format!("{}{}", OSC_PREFIX, path);
        let msg = OscMessage {
            addr,
            args: vec![OscType::Float(value)],
        };
        let packet = OscPacket::Message(msg);
        if let Ok(buf) = rosc::encoder::encode(&packet) {
            for target in &self.targets {
                let _ = self.socket.send_to(&buf, target);
            }
        }
    }

    /// Whether any feedback targets are registered.
    pub fn has_targets(&self) -> bool {
        !self.targets.is_empty()
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_param_crossfader() {
        let input = parse_osc_message("/varda/crossfader", &[OscType::Float(0.5)]);
        assert_eq!(input, OscInput::Param { path: "crossfader".into(), value: 0.5 });
    }

    #[test]
    fn parse_param_deck_opacity() {
        let input = parse_osc_message("/varda/deck/abc123/opacity", &[OscType::Float(0.8)]);
        assert_eq!(input, OscInput::Param { path: "deck/abc123/opacity".into(), value: 0.8 });
    }

    #[test]
    fn parse_param_mod_frequency() {
        let input = parse_osc_message("/varda/mod/2/frequency", &[OscType::Float(0.5)]);
        assert_eq!(input, OscInput::Param { path: "mod/2/frequency".into(), value: 0.5 });
    }

    #[test]
    fn parse_param_action_undo() {
        let input = parse_osc_message("/varda/action/undo", &[OscType::Float(1.0)]);
        assert_eq!(input, OscInput::Param { path: "action/undo".into(), value: 1.0 });
    }

    #[test]
    fn parse_clock_bpm_raw() {
        let input = parse_osc_message("/varda/clock/bpm", &[OscType::Float(120.0)]);
        assert_eq!(input, OscInput::ClockBpm(120.0));
    }

    #[test]
    fn parse_clock_beat_phase() {
        let input = parse_osc_message("/varda/clock/beat", &[OscType::Float(0.75)]);
        assert_eq!(input, OscInput::ClockBeat(0.75));
    }

    #[test]
    fn parse_clock_beat_clamped() {
        let input = parse_osc_message("/varda/clock/beat", &[OscType::Float(1.5)]);
        assert_eq!(input, OscInput::ClockBeat(1.0));
    }

    #[test]
    fn parse_unknown_no_prefix() {
        let input = parse_osc_message("/deck/0/opacity", &[OscType::Float(0.5)]);
        assert!(matches!(input, OscInput::Unknown(_)));
    }

    #[test]
    fn parse_unknown_wrong_prefix() {
        let input = parse_osc_message("/other/crossfader", &[OscType::Float(0.5)]);
        assert!(matches!(input, OscInput::Unknown(_)));
    }

    #[test]
    fn parse_int_arg_as_float() {
        let input = parse_osc_message("/varda/crossfader", &[OscType::Int(64)]);
        assert_eq!(input, OscInput::Param { path: "crossfader".into(), value: 64.0 });
    }

    #[test]
    fn parse_bool_arg_true() {
        let input = parse_osc_message("/varda/deck/abc/mute", &[OscType::Bool(true)]);
        assert_eq!(input, OscInput::Param { path: "deck/abc/mute".into(), value: 1.0 });
    }

    #[test]
    fn parse_bool_arg_false() {
        let input = parse_osc_message("/varda/deck/abc/mute", &[OscType::Bool(false)]);
        assert_eq!(input, OscInput::Param { path: "deck/abc/mute".into(), value: 0.0 });
    }

    #[test]
    fn parse_no_args_is_unknown() {
        let input = parse_osc_message("/varda/crossfader", &[]);
        assert!(matches!(input, OscInput::Unknown(_)));
    }

    #[test]
    fn parse_deep_path_effect_param() {
        let input = parse_osc_message(
            "/varda/deck/abc/effect/0/param/brightness",
            &[OscType::Float(0.7)],
        );
        assert_eq!(
            input,
            OscInput::Param { path: "deck/abc/effect/0/param/brightness".into(), value: 0.7 },
        );
    }

    #[test]
    fn parse_master_effect_param() {
        let input = parse_osc_message(
            "/varda/master/effect/1/param/amount",
            &[OscType::Float(0.3)],
        );
        assert_eq!(
            input,
            OscInput::Param { path: "master/effect/1/param/amount".into(), value: 0.3 },
        );
    }

    #[test]
    fn parse_channel_opacity() {
        let input = parse_osc_message("/varda/ch/def456/opacity", &[OscType::Float(0.9)]);
        assert_eq!(input, OscInput::Param { path: "ch/def456/opacity".into(), value: 0.9 });
    }

    #[test]
    fn parse_step_sequencer_step() {
        let input = parse_osc_message("/varda/mod/0/step/3", &[OscType::Float(0.6)]);
        assert_eq!(input, OscInput::Param { path: "mod/0/step/3".into(), value: 0.6 });
    }

    #[test]
    fn feedback_sender_no_targets() {
        let sender = OscFeedbackSender::new().unwrap();
        assert!(!sender.has_targets());
        // send_param should be a no-op without targets
        sender.send_param("crossfader", 0.5);
    }

    #[test]
    fn osc_config_default() {
        let config = OscConfig::default();
        assert!(config.enabled);
        assert_eq!(config.in_port, 9000);
        assert!(config.feedback_targets.is_empty());
    }

    #[test]
    fn osc_config_roundtrip() {
        let config = OscConfig {
            enabled: true,
            in_port: 8000,
            feedback_targets: vec!["192.168.1.10:9001".into(), "192.168.1.20:9002".into()],
        };
        let json = serde_json::to_string(&config).unwrap();
        let loaded: OscConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.in_port, 8000);
        assert_eq!(loaded.feedback_targets.len(), 2);
        assert_eq!(loaded.feedback_targets[0], "192.168.1.10:9001");
    }

    #[test]
    fn osc_config_deserialize_minimal() {
        // Only provide in_port; enabled and feedback_targets should use defaults
        let json = r#"{"in_port": 7000}"#;
        let config: OscConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert_eq!(config.in_port, 7000);
        assert!(config.feedback_targets.is_empty());
    }

    #[test]
    fn osc_config_disabled() {
        let json = r#"{"enabled": false}"#;
        let config: OscConfig = serde_json::from_str(json).unwrap();
        assert!(!config.enabled);
        assert_eq!(config.in_port, 9000); // default
    }

    #[test]
    fn osc_config_save_load_file() {
        let dir = std::env::temp_dir().join("varda_osc_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("osc_test.json");

        let config = OscConfig {
            enabled: false,
            in_port: 8888,
            feedback_targets: vec!["10.0.0.1:9000".into()],
        };
        config.save(&path).unwrap();

        let loaded = OscConfig::load(&path).unwrap();
        assert!(!loaded.enabled);
        assert_eq!(loaded.in_port, 8888);
        assert_eq!(loaded.feedback_targets, vec!["10.0.0.1:9000"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn feedback_sender_add_target() {
        let mut sender = OscFeedbackSender::new().unwrap();
        sender.add_target("127.0.0.1:9001").unwrap();
        assert!(sender.has_targets());
        // Adding same target again is a no-op
        sender.add_target("127.0.0.1:9001").unwrap();
        assert_eq!(sender.targets.len(), 1);
    }
}
