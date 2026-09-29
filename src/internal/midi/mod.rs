//! MIDI input and output via midir (CoreMIDI/ALSA/JACK/WinMM).
//!
//! Supports any number of devices, each with its own `DeviceId`. Mappings are
//! per device, so two controllers can map the same CC# to different parameters.

pub mod auto_map;
pub mod controller_profile;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use anyhow::Context as _;
use midir::{MidiInput, MidiOutput};

pub use auto_map::AutoMapEngine;
pub use controller_profile::{ControllerLedManager, ControllerProfileData, ProfileRegistry};

pub use crate::engine::value::midi::{DeviceId, MidiMessage};

impl MidiMessage {
    /// Parses raw MIDI bytes into a `MidiMessage` tagged with `device_id`.
    pub fn from_bytes(data: &[u8], device_id: DeviceId) -> Option<Self> {
        if data.is_empty() {
            return None;
        }
        let status = data[0];

        // System real-time messages (one byte, no channel), before channel masking.
        match status {
            0xF8 => return Some(MidiMessage::ClockTick { device_id }),
            0xFA => return Some(MidiMessage::ClockStart { device_id }),
            0xFB => return Some(MidiMessage::ClockContinue { device_id }),
            0xFC => return Some(MidiMessage::ClockStop { device_id }),
            0xF1 if data.len() >= 2 => {
                return Some(MidiMessage::MtcQuarterFrame {
                    device_id,
                    data: data[1],
                });
            }
            _ => {}
        }

        // Universal real-time SysEx `F0 7F <dev> 01 01 hh mm ss ff F7`: a
        // timecode locate. Only the shape is checked here; the timecode
        // receiver decodes the bytes.
        if status == 0xF0
            && data.len() >= 10
            && data[1] == 0x7F
            && data[3] == 0x01
            && data[4] == 0x01
        {
            return Some(MidiMessage::MtcFullFrame {
                device_id,
                payload: [data[5], data[6], data[7], data[8]],
            });
        }

        let msg_type = status & 0xF0;
        let channel = status & 0x0F;
        match msg_type {
            0x90 if data.len() >= 3 && data[2] > 0 => Some(MidiMessage::NoteOn {
                device_id,
                channel,
                note: data[1],
                velocity: data[2],
            }),
            0x90 if data.len() >= 3 => Some(MidiMessage::NoteOff {
                device_id,
                channel,
                note: data[1],
                velocity: 0,
            }),
            0x80 if data.len() >= 3 => Some(MidiMessage::NoteOff {
                device_id,
                channel,
                note: data[1],
                velocity: data[2],
            }),
            0xB0 if data.len() >= 3 => Some(MidiMessage::ControlChange {
                device_id,
                channel,
                cc: data[1],
                value: data[2],
            }),
            _ => None,
        }
    }

    pub fn device_id(&self) -> DeviceId {
        match self {
            MidiMessage::ControlChange { device_id, .. }
            | MidiMessage::NoteOn { device_id, .. }
            | MidiMessage::NoteOff { device_id, .. }
            | MidiMessage::ClockTick { device_id }
            | MidiMessage::ClockStart { device_id }
            | MidiMessage::ClockContinue { device_id }
            | MidiMessage::ClockStop { device_id }
            | MidiMessage::MtcQuarterFrame { device_id, .. }
            | MidiMessage::MtcFullFrame { device_id, .. } => *device_id,
        }
    }

    /// Mapping key: device + message type + channel + CC/note. `None` for
    /// clock and timecode messages.
    pub fn mapping_key(&self) -> Option<MidiKey> {
        match self {
            MidiMessage::ControlChange {
                device_id,
                channel,
                cc,
                ..
            } => Some(MidiKey::CC(*device_id, *channel, *cc)),
            MidiMessage::NoteOn {
                device_id,
                channel,
                note,
                ..
            }
            | MidiMessage::NoteOff {
                device_id,
                channel,
                note,
                ..
            } => Some(MidiKey::Note(*device_id, *channel, *note)),
            // Clock and timecode drive the engine; they aren't mappable controls.
            MidiMessage::ClockTick { .. }
            | MidiMessage::ClockStart { .. }
            | MidiMessage::ClockContinue { .. }
            | MidiMessage::ClockStop { .. }
            | MidiMessage::MtcQuarterFrame { .. }
            | MidiMessage::MtcFullFrame { .. } => None,
        }
    }

    /// Normalized value (0.0–1.0). Clock messages return 0.
    pub fn normalized_value(&self) -> f32 {
        match self {
            MidiMessage::ControlChange { value, .. } => f32::from(*value) / 127.0,
            MidiMessage::NoteOn { velocity, .. } => f32::from(*velocity) / 127.0,
            MidiMessage::NoteOff { .. }
            | MidiMessage::ClockTick { .. }
            | MidiMessage::ClockStart { .. }
            | MidiMessage::ClockContinue { .. }
            | MidiMessage::ClockStop { .. }
            | MidiMessage::MtcQuarterFrame { .. }
            | MidiMessage::MtcFullFrame { .. } => 0.0,
        }
    }
}

/// Identifies a MIDI control for mapping. Includes `device_id` so the same CC#
/// on different devices maps independently.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub enum MidiKey {
    /// CC message: (`device_id`, channel, `cc_number`)
    CC(DeviceId, u8, u8),
    /// Note message: (`device_id`, channel, `note_number`)
    Note(DeviceId, u8, u8),
}

impl MidiKey {
    pub fn device_id(&self) -> DeviceId {
        match self {
            MidiKey::CC(d, _, _) | MidiKey::Note(d, _, _) => *d,
        }
    }
}

impl std::fmt::Display for MidiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MidiKey::CC(dev, ch, cc) => write!(f, "[dev{}] CC ch{} #{}", dev, ch + 1, cc),
            MidiKey::Note(dev, ch, note) => write!(f, "[dev{}] Note ch{} #{}", dev, ch + 1, note),
        }
    }
}

// ── MIDI Device Info ────────────────────────────────────────────────

/// A connected MIDI device.
#[derive(Debug, Clone)]
pub struct MidiDeviceInfo {
    /// Stable for the session; assigned on scan.
    pub id: DeviceId,
    pub name: String,
    /// Enabled for input.
    pub enabled: bool,
    /// Has a matching output port.
    pub has_output: bool,
    /// Controller profile matched by device name, if any.
    pub profile: Option<Arc<ControllerProfileData>>,
}

impl MidiDeviceInfo {
    /// Profile display name for the UI.
    pub fn profile_name(&self) -> &str {
        self.profile
            .as_ref()
            .map_or("Generic", |p| p.profile.name.as_str())
    }
}

// ── MIDI Device Manager ────────────────────────────────────────────

/// MIDI devices: discovery, input, output, and message routing.
pub struct MidiDeviceManager {
    receiver: Receiver<MidiMessage>,
    sender: Sender<MidiMessage>,
    pub devices: HashMap<DeviceId, MidiDeviceInfo>,
    next_device_id: DeviceId,
    /// Kept alive so callbacks keep firing.
    input_connections: Vec<midir::MidiInputConnection<()>>,
    /// Output connections by `DeviceId`. Mutex because
    /// `MidiOutputConnection::send` needs `&mut self` and `send_raw` takes `&self`.
    output_connections: HashMap<DeviceId, Mutex<midir::MidiOutputConnection>>,
    pub profile_registry: ProfileRegistry,
}

/// Strips a directional suffix from a port name, to pair the input and output
/// ports of multi-port USB devices:
/// - "Tascam Model 12 MIDI In" → "Tascam Model 12 MIDI"
/// - "Tascam Model 12 MIDI Out" → "Tascam Model 12 MIDI"
/// - "APC MINI" → "APC MINI"
fn strip_port_suffix(name: &str) -> &str {
    let lower = name.to_lowercase();
    // Longer suffixes first to avoid partial matches.
    let suffixes = [" input", " output", " in", " out"];
    for suffix in &suffixes {
        if lower.ends_with(suffix) {
            return &name[..name.len() - suffix.len()];
        }
    }
    name
}

impl MidiDeviceManager {
    /// Creates a manager and scans for devices.
    ///
    /// # Errors
    ///
    /// Returns an error if the initial scan fails; see [`scan_devices`](Self::scan_devices).
    pub fn new() -> anyhow::Result<Self> {
        let (sender, receiver) = channel();

        let mut mgr = Self {
            receiver,
            sender,
            devices: HashMap::new(),
            next_device_id: 0,
            input_connections: Vec::new(),
            output_connections: HashMap::new(),
            profile_registry: ProfileRegistry::new(),
        };
        mgr.scan_devices()?;
        Ok(mgr)
    }

    /// A manager with no hardware, for tests. Skips scanning so local devices
    /// can't leak into tests.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        let (sender, receiver) = channel();
        Self {
            receiver,
            sender,
            devices: HashMap::new(),
            next_device_id: 0,
            input_connections: Vec::new(),
            output_connections: HashMap::new(),
            profile_registry: ProfileRegistry::new(),
        }
    }

    /// Queues a message as if `device_id` had sent it, registering the device so
    /// [`try_recv`](Self::try_recv) doesn't skip it.
    #[cfg(test)]
    pub(crate) fn inject(&mut self, msg: MidiMessage) {
        let id = msg.device_id();
        self.devices.entry(id).or_insert_with(|| MidiDeviceInfo {
            id,
            name: format!("Test device {id}"),
            enabled: true,
            has_output: false,
            profile: None,
        });
        let _ = self.sender.send(msg);
    }

    /// Loads user controller profiles from a directory (e.g. `.varda/controller-profiles/`).
    pub fn load_user_profiles(&mut self, dir: &std::path::Path) {
        self.profile_registry.load_user_profiles(dir);
    }

    /// Scans for MIDI devices. Call again to rescan after hot-plug.
    ///
    /// # Errors
    ///
    /// Returns an error if the MIDI backend can't create the `MidiInput`/
    /// `MidiOutput` clients used to list ports. Per-port connection failures
    /// are logged and skipped.
    pub fn scan_devices(&mut self) -> anyhow::Result<()> {
        // Serialize scans process-wide. Concurrent port enumeration crashes
        // with STATUS_ACCESS_VIOLATION inside midir 0.11's winmm backend, which
        // calls the driver directly. The lock covers the whole function because
        // `connect_input`/`connect_output` enumerate again. Uncontended in
        // production (scans run on the app thread). Poisoning is ignored: a
        // panic elsewhere says nothing about driver state.
        static ENUMERATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENUMERATION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        self.input_connections.clear();
        self.output_connections.clear();
        self.devices.clear();
        self.next_device_id = 0;

        // Output port names, from a throwaway MidiOutput.
        let output_port_names: Vec<String> = {
            let midi_out = MidiOutput::new("Varda scan")
                .map_err(|e| anyhow::anyhow!("Failed to create MidiOutput for scan: {e}"))?;
            let ports = midi_out.ports();
            ports
                .iter()
                .map(|p| midi_out.port_name(p).unwrap_or_default())
                .collect()
        };

        // Input port names, from a throwaway MidiInput.
        let input_port_names: Vec<String> = {
            let midi_in = MidiInput::new("Varda scan")
                .map_err(|e| anyhow::anyhow!("Failed to create MidiInput for scan: {e}"))?;
            let ports = midi_in.ports();
            ports
                .iter()
                .map(|p| midi_in.port_name(p).unwrap_or_default())
                .collect()
        };

        log::info!(
            "MIDI scan: {} input(s), {} output(s)",
            input_port_names.len(),
            output_port_names.len()
        );

        let mut matched_outputs: Vec<bool> = vec![false; output_port_names.len()];

        // For each input: assign a DeviceId, find its output, connect.
        for in_name in &input_port_names {
            let device_id = self.next_device_id;
            self.next_device_id += 1;

            // Output matching: first an exact case-insensitive name (APC Mini),
            // then the stem without In/Out suffixes (Tascam Model 12 MIDI In ↔ Out).
            let matching_out_idx = output_port_names
                .iter()
                .enumerate()
                .position(|(j, out_name)| {
                    !matched_outputs[j] && out_name.to_lowercase() == in_name.to_lowercase()
                })
                .or_else(|| {
                    let in_stem = strip_port_suffix(in_name);
                    output_port_names
                        .iter()
                        .enumerate()
                        .position(|(j, out_name)| {
                            !matched_outputs[j]
                                && strip_port_suffix(out_name).eq_ignore_ascii_case(in_stem)
                        })
                });
            let has_output = matching_out_idx.is_some();

            if let Some(out_idx) = matching_out_idx {
                matched_outputs[out_idx] = true;
                self.connect_output(device_id, &output_port_names[out_idx]);
            }

            let profile = self.profile_registry.detect(in_name);
            let profile_name = profile
                .as_ref()
                .map_or("Generic", |p| p.profile.name.as_str());
            log::info!(
                "MIDI device [{device_id}]: {in_name} (profile={profile_name}, output={has_output})"
            );

            self.devices.insert(
                device_id,
                MidiDeviceInfo {
                    id: device_id,
                    name: in_name.clone(),
                    enabled: true,
                    has_output,
                    profile,
                },
            );

            // A fresh MidiInput per port, since midir consumes it on connect.
            // Match by name, not index: each MidiInput::new() is a new CoreMIDI
            // client and port order can differ.
            let tx = self.sender.clone();
            let dev_id = device_id;
            let port_label = format!("Varda In {device_id}");
            match MidiInput::new(&port_label) {
                Ok(midi_in) => {
                    let ports = midi_in.ports();
                    let target_port = ports
                        .iter()
                        .find(|p| midi_in.port_name(p).is_ok_and(|n| n == *in_name));
                    if let Some(port) = target_port {
                        match midi_in.connect(
                            port,
                            &port_label,
                            move |_ts, data, ()| {
                                // Raw bytes; enable with RUST_LOG=varda::midi=debug.
                                log::debug!(
                                    "[MIDI-RAW] dev={} len={} bytes: {:02X?}",
                                    dev_id,
                                    data.len(),
                                    data
                                );
                                if let Some(msg) = MidiMessage::from_bytes(data, dev_id) {
                                    let _ = tx.send(msg);
                                }
                            },
                            (),
                        ) {
                            Ok(conn) => {
                                log::debug!(
                                    "[MIDI] Connected input: '{in_name}' (dev={device_id})"
                                );
                                self.input_connections.push(conn);
                            }
                            Err(e) => log::warn!("Failed to connect MIDI input {in_name}: {e}"),
                        }
                    } else {
                        log::warn!(
                            "MIDI input port '{in_name}' not found during connect (port list changed?)"
                        );
                    }
                }
                Err(e) => log::warn!("Failed to create MidiInput for {in_name}: {e}"),
            }
        }

        // Unmatched outputs become output-only devices.
        for (j, out_name) in output_port_names.iter().enumerate() {
            if matched_outputs[j] {
                continue;
            }
            let device_id = self.next_device_id;
            self.next_device_id += 1;
            let profile = self.profile_registry.detect(out_name);
            let profile_name = profile
                .as_ref()
                .map_or("Generic", |p| p.profile.name.as_str());
            log::info!(
                "MIDI output-only device [{device_id}]: {out_name} (profile={profile_name})"
            );
            self.connect_output(device_id, out_name);
            self.devices.insert(
                device_id,
                MidiDeviceInfo {
                    id: device_id,
                    name: out_name.clone(),
                    enabled: true,
                    has_output: true,
                    profile,
                },
            );
        }

        Ok(())
    }

    /// Connects an output port by name and stores the connection.
    fn connect_output(&mut self, device_id: DeviceId, port_name: &str) {
        match MidiOutput::new(&format!("Varda Out {device_id}")) {
            Ok(midi_out) => {
                let ports = midi_out.ports();
                let port = ports
                    .iter()
                    .find(|p| midi_out.port_name(p).is_ok_and(|n| n == port_name));
                if let Some(port) = port {
                    match midi_out.connect(port, &format!("Varda Out {device_id}")) {
                        Ok(conn) => {
                            self.output_connections.insert(device_id, Mutex::new(conn));
                        }
                        Err(e) => log::warn!("Failed to connect MIDI output {port_name}: {e}"),
                    }
                }
            }
            Err(e) => log::warn!("Failed to create MidiOutput for {port_name}: {e}"),
        }
    }

    /// Next MIDI message, non-blocking. Skips messages from disabled devices.
    pub fn try_recv(&self) -> Option<MidiMessage> {
        loop {
            match self.receiver.try_recv() {
                Ok(msg) => {
                    let dev_id = msg.device_id();
                    if let Some(info) = self.devices.get(&dev_id)
                        && info.enabled
                    {
                        return Some(msg);
                    }
                    // Disabled or unknown device: skip.
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
            }
        }
    }

    /// Sends Note On to `device_id`.
    pub fn send_note_on(&self, device_id: DeviceId, channel: u8, note: u8, velocity: u8) {
        let status = 0x90 | (channel & 0x0F);
        self.send_raw(device_id, &[status, note, velocity]);
    }

    /// Sends raw MIDI bytes to `device_id`.
    pub fn send_raw(&self, device_id: DeviceId, bytes: &[u8]) {
        if let Some(conn_mutex) = self.output_connections.get(&device_id)
            && let Ok(mut conn) = conn_mutex.lock()
            && let Err(e) = conn.send(bytes)
        {
            log::warn!("Failed to send MIDI to device {device_id}: {e}");
        }
    }

    pub fn device(&self, id: DeviceId) -> Option<&MidiDeviceInfo> {
        self.devices.get(&id)
    }

    pub fn set_device_enabled(&mut self, id: DeviceId, enabled: bool) {
        if let Some(info) = self.devices.get_mut(&id) {
            info.enabled = enabled;
            log::info!(
                "MIDI device [{}] {} → {}",
                id,
                info.name,
                if enabled { "enabled" } else { "disabled" }
            );
        }
    }

    /// Every device, sorted, for the UI.
    pub fn device_list(&self) -> Vec<MidiDeviceInfo> {
        let mut list: Vec<_> = self.devices.values().cloned().collect();
        list.sort_by_key(|d| d.id);
        list
    }
}

// ── MIDI Mapping Store ──────────────────────────────────────────────

/// Mappings from MIDI controls to parameter paths (UUIDs are 8-char hex):
///   crossfader                              → mixer crossfader position
///   deck/<uuid>/opacity                     → deck opacity
///   deck/<uuid>/param/<name>                → generator param (float)
///   deck/<uuid>/effect/<`effect_uuid>/param`/<name> → deck effect param (float)
///   ch/<uuid>/opacity                       → channel opacity
///   ch/<uuid>/effect/<`effect_uuid>/param`/<name>   → channel effect param (float)
///   master/effect/<`effect_uuid>/param`/<name>      → master effect param (float)
///   mod/<`mod_uuid`>/<`param_name`>             → modulation source param
#[derive(Debug, Clone)]
pub struct MidiMappingStore {
    pub mappings: HashMap<MidiKey, String>,
    pub learn_mode: bool,
    /// Parameter path waiting for the next MIDI input.
    pub learn_target: Option<String>,
}

impl Default for MidiMappingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiMappingStore {
    pub fn new() -> Self {
        Self {
            mappings: HashMap::new(),
            learn_mode: false,
            learn_target: None,
        }
    }

    pub fn set(&mut self, key: MidiKey, path: String) {
        log::info!("MIDI mapped {key} → {path}");
        self.mappings.insert(key, path);
    }

    pub fn remove(&mut self, key: &MidiKey) {
        self.mappings.remove(key);
    }

    pub fn get(&self, key: &MidiKey) -> Option<&String> {
        self.mappings.get(key)
    }

    /// Toggles learn mode. Turning it off clears the learn target.
    pub fn toggle_learn(&mut self) {
        self.learn_mode = !self.learn_mode;
        if !self.learn_mode {
            self.learn_target = None;
        }
        log::info!(
            "MIDI learn mode: {}",
            if self.learn_mode { "ON" } else { "OFF" }
        );
    }

    /// Sets the learn target. Only works in learn mode.
    pub fn select_learn_target(&mut self, param_path: String) {
        if self.learn_mode {
            log::info!("MIDI learn target: {param_path}");
            self.learn_target = Some(param_path);
        }
    }

    pub fn cancel_learn(&mut self) {
        self.learn_mode = false;
        self.learn_target = None;
    }

    /// Handles a message in learn mode. Returns true if a mapping was created.
    /// Stays in learn mode and clears the target so another param can be picked.
    pub fn process_learn(&mut self, key: MidiKey) -> bool {
        if let Some(path) = self.learn_target.take() {
            self.set(key, path);
            true
        } else {
            false
        }
    }

    pub fn clear_all(&mut self) {
        self.mappings.clear();
        log::info!("MIDI mappings cleared");
    }

    /// All mappings, sorted by device ID.
    pub fn sorted_mappings(&self) -> Vec<(MidiKey, String)> {
        let mut list: Vec<_> = self.mappings.iter().map(|(k, v)| (*k, v.clone())).collect();
        list.sort_by_key(|(k, _)| k.device_id());
        list
    }

    /// Exports mappings keyed by device name instead of ID. Skips keys the
    /// auto-map engine handles, so `midi.json` holds only manual mappings.
    pub fn to_config(
        &self,
        devices: &HashMap<DeviceId, MidiDeviceInfo>,
        auto_map: &AutoMapEngine,
    ) -> MidiConfig {
        let mappings = self
            .mappings
            .iter()
            .filter(|(key, _)| !auto_map.handles_key(key.device_id(), key))
            .map(|(key, path)| {
                let device_name = devices.get(&key.device_id()).map_or_else(
                    || format!("unknown_{}", key.device_id()),
                    |d| d.name.clone(),
                );
                let (msg_type, channel, number) = match key {
                    MidiKey::CC(_, ch, cc) => ("cc".to_string(), *ch, *cc),
                    MidiKey::Note(_, ch, note) => ("note".to_string(), *ch, *note),
                };
                MidiMappingEntry {
                    device_name,
                    msg_type,
                    channel,
                    number,
                    param_path: path.clone(),
                }
            })
            .collect();
        MidiConfig {
            version: 1,
            mappings,
        }
    }

    /// Imports mappings, resolving device names to current device IDs.
    pub fn load_from_config(
        &mut self,
        config: &MidiConfig,
        devices: &HashMap<DeviceId, MidiDeviceInfo>,
    ) {
        let name_to_id: HashMap<&str, DeviceId> = devices
            .iter()
            .map(|(id, info)| (info.name.as_str(), *id))
            .collect();

        for entry in &config.mappings {
            let device_id = if let Some(id) = name_to_id.get(entry.device_name.as_str()) {
                *id
            } else {
                log::warn!(
                    "MIDI mapping references unknown device '{}', skipping: {} -> {}",
                    entry.device_name,
                    entry.device_name,
                    entry.param_path
                );
                continue;
            };
            let key = match entry.msg_type.as_str() {
                "cc" => MidiKey::CC(device_id, entry.channel, entry.number),
                "note" => MidiKey::Note(device_id, entry.channel, entry.number),
                _ => {
                    log::warn!("Unknown MIDI message type '{}', skipping", entry.msg_type);
                    continue;
                }
            };
            self.set(
                key,
                crate::engine::value::param::canonical_path(&entry.param_path),
            );
        }
    }
}

// ── MIDI Persistence Config ─────────────────────────────────────────

/// MIDI config for `.varda/midi.json`. Keyed by device name, not ID, so
/// mappings survive re-enumeration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MidiConfig {
    #[serde(default = "default_midi_version")]
    pub version: u32,
    #[serde(default)]
    pub mappings: Vec<MidiMappingEntry>,
}

fn default_midi_version() -> u32 {
    1
}

/// One mapping: device name + CC/note → parameter path.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MidiMappingEntry {
    pub device_name: String,
    pub msg_type: String, // "cc" or "note"
    pub channel: u8,
    pub number: u8,
    pub param_path: String,
}

impl MidiMappingEntry {
    /// Checks one entry. Returns errors; empty means valid.
    pub fn validate(&self, prefix: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if self.device_name.trim().is_empty() {
            errors.push(format!("{prefix}: device_name is empty"));
        }
        if self.msg_type != "cc" && self.msg_type != "note" {
            errors.push(format!(
                "{}: msg_type '{}' is invalid (expected \"cc\" or \"note\")",
                prefix, self.msg_type
            ));
        }
        if self.channel > 15 {
            errors.push(format!(
                "{}: channel {} exceeds MIDI range 0-15",
                prefix, self.channel
            ));
        }
        if self.number > 127 {
            errors.push(format!(
                "{}: number {} exceeds MIDI range 0-127",
                prefix, self.number
            ));
        }
        if self.param_path.trim().is_empty() {
            errors.push(format!("{prefix}: param_path is empty"));
        }
        errors
    }
}

impl MidiConfig {
    /// Checks the config for semantic errors. Empty means valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for (i, entry) in self.mappings.iter().enumerate() {
            errors.extend(entry.validate(&format!("mappings[{i}]")));
        }
        errors
    }

    /// Loads from a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file can't be read or isn't valid MIDI-config
    /// JSON. Semantic issues are logged as warnings, not returned.
    pub fn load<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read MIDI config: {}", path.as_ref().display()))?;
        let config: MidiConfig = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse MIDI config: {}", path.as_ref().display()))?;
        let warnings = config.validate();
        for w in &warnings {
            log::warn!("MIDI config {}: {}", path.as_ref().display(), w);
        }
        Ok(config)
    }

    /// Saves to a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the atomic write to `path` fails.
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> anyhow::Result<()> {
        let errors = self.validate();
        for e in &errors {
            log::error!("MIDI config save: {e}");
        }
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize MIDI config")?;
        crate::files::atomic_write(path.as_ref(), &content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midi_message_from_bytes_with_device_id() {
        let msg = MidiMessage::from_bytes(&[0x90, 60, 100], 42).unwrap();
        assert_eq!(msg.device_id(), 42);
        match msg {
            MidiMessage::NoteOn {
                device_id,
                channel,
                note,
                velocity,
            } => {
                assert_eq!(device_id, 42);
                assert_eq!(channel, 0);
                assert_eq!(note, 60);
                assert_eq!(velocity, 100);
            }
            _ => panic!("Expected NoteOn"),
        }
    }

    #[test]
    fn test_midi_key_includes_device_id() {
        let msg1 = MidiMessage::from_bytes(&[0xB0, 48, 64], 0).unwrap();
        let msg2 = MidiMessage::from_bytes(&[0xB0, 48, 64], 1).unwrap();
        let key1 = msg1.mapping_key().unwrap();
        let key2 = msg2.mapping_key().unwrap();
        // Same CC on different devices gives different keys.
        assert_ne!(key1, key2);
        assert_eq!(key1.device_id(), 0);
        assert_eq!(key2.device_id(), 1);
    }

    #[test]
    fn test_midi_key_same_device_same_control() {
        let msg1 = MidiMessage::from_bytes(&[0xB0, 48, 64], 5).unwrap();
        let msg2 = MidiMessage::from_bytes(&[0xB0, 48, 100], 5).unwrap();
        // Same device and CC give the same key, whatever the value.
        assert_eq!(msg1.mapping_key(), msg2.mapping_key());
    }

    #[test]
    fn test_clock_tick_parsed() {
        let msg = MidiMessage::from_bytes(&[0xF8], 7).unwrap();
        assert!(matches!(msg, MidiMessage::ClockTick { device_id: 7 }));
        assert!(msg.mapping_key().is_none());
    }

    #[test]
    fn test_clock_start_stop_continue() {
        let start = MidiMessage::from_bytes(&[0xFA], 0).unwrap();
        assert!(matches!(start, MidiMessage::ClockStart { .. }));

        let cont = MidiMessage::from_bytes(&[0xFB], 0).unwrap();
        assert!(matches!(cont, MidiMessage::ClockContinue { .. }));

        let stop = MidiMessage::from_bytes(&[0xFC], 0).unwrap();
        assert!(matches!(stop, MidiMessage::ClockStop { .. }));
    }

    /// A quarter frame must parse as timecode, not as a mappable control change.
    #[test]
    fn a_quarter_frame_is_read_as_timecode_and_kept_off_the_mapping_table() {
        let msg = MidiMessage::from_bytes(&[0xF1, 0x37], 4).expect("a quarter frame");
        assert!(matches!(
            msg,
            MidiMessage::MtcQuarterFrame {
                device_id: 4,
                data: 0x37
            }
        ));
        assert_eq!(msg.device_id(), 4);
        assert!(
            msg.mapping_key().is_none(),
            "timecode is an engine signal, not a control"
        );
        assert!((msg.normalized_value() - 0.0).abs() < f32::EPSILON);

        assert!(
            MidiMessage::from_bytes(&[0xF1], 4).is_none(),
            "a status byte with its data byte lost carries no nibble"
        );
    }

    /// Full-frame locate: the payload is extracted here and decoded later, so
    /// only the shape is checked.
    #[test]
    fn a_locate_arrives_as_universal_real_time_sysex() {
        let bytes = [0xF0, 0x7F, 0x7F, 0x01, 0x01, 0x21, 0x0A, 0x1E, 0x0C, 0xF7];
        let msg = MidiMessage::from_bytes(&bytes, 2).expect("a full frame");
        assert!(matches!(
            msg,
            MidiMessage::MtcFullFrame {
                device_id: 2,
                payload: [0x21, 0x0A, 0x1E, 0x0C]
            }
        ));
    }

    /// Other system-exclusive messages (e.g. a synth patch dump) must not parse as a locate.
    #[test]
    fn other_sysex_traffic_is_not_mistaken_for_a_locate() {
        // Device enquiry: same universal prefix, different sub-id.
        let enquiry = [0xF0, 0x7F, 0x7F, 0x06, 0x01, 0x00, 0x00, 0x00, 0x00, 0xF7];
        assert!(MidiMessage::from_bytes(&enquiry, 0).is_none());

        // Manufacturer dump, not universal.
        let dump = [0xF0, 0x43, 0x00, 0x01, 0x01, 0x10, 0x20, 0x30, 0x40, 0xF7];
        assert!(MidiMessage::from_bytes(&dump, 0).is_none());

        // Truncated address.
        let short = [0xF0, 0x7F, 0x7F, 0x01, 0x01, 0x21, 0xF7];
        assert!(MidiMessage::from_bytes(&short, 0).is_none());
    }

    #[test]
    fn test_midi_key_display_with_device() {
        let key = MidiKey::CC(3, 0, 48);
        let display = format!("{key}");
        assert!(display.contains("dev3"));
        assert!(display.contains("48"));
    }

    #[test]
    fn test_mapping_store_clear_all() {
        let mut store = MidiMappingStore::new();
        store.set(MidiKey::CC(0, 0, 48), "crossfader".to_string());
        store.set(MidiKey::Note(1, 0, 36), "ch/aabbccdd/opacity".to_string());
        assert_eq!(store.mappings.len(), 2);
        store.clear_all();
        assert_eq!(store.mappings.len(), 0);
    }

    #[test]
    fn test_mapping_store_sorted_by_device() {
        let mut store = MidiMappingStore::new();
        store.set(MidiKey::CC(2, 0, 48), "b".to_string());
        store.set(MidiKey::CC(0, 0, 48), "a".to_string());
        store.set(MidiKey::Note(1, 0, 36), "c".to_string());
        let sorted = store.sorted_mappings();
        assert_eq!(sorted[0].0.device_id(), 0);
        assert_eq!(sorted[1].0.device_id(), 1);
        assert_eq!(sorted[2].0.device_id(), 2);
    }

    #[test]
    fn test_profile_registry_detect() {
        let registry = ProfileRegistry::new();
        let apc = registry.detect("APC MINI");
        assert!(apc.is_some());
        assert_eq!(apc.unwrap().profile.name, "Akai APC Mini mk1");

        let apc2 = registry.detect("Apc Mini mk2");
        assert!(apc2.is_some());

        let generic = registry.detect("Novation Launchpad");
        assert!(generic.is_none());
    }

    #[test]
    fn test_strip_port_suffix_midi_in_out() {
        assert_eq!(
            strip_port_suffix("Tascam Model 12 MIDI In"),
            "Tascam Model 12 MIDI"
        );
        assert_eq!(
            strip_port_suffix("Tascam Model 12 MIDI Out"),
            "Tascam Model 12 MIDI"
        );
    }

    #[test]
    fn test_strip_port_suffix_daw_control() {
        assert_eq!(
            strip_port_suffix("Tascam Model 12 DAW CONTROL MIDI In"),
            "Tascam Model 12 DAW CONTROL MIDI"
        );
        assert_eq!(
            strip_port_suffix("Tascam Model 12 DAW CONTROL MIDI Out"),
            "Tascam Model 12 DAW CONTROL MIDI"
        );
    }

    #[test]
    fn test_strip_port_suffix_simple_in_out() {
        assert_eq!(strip_port_suffix("Digitakt In"), "Digitakt");
        assert_eq!(strip_port_suffix("Digitakt Out"), "Digitakt");
    }

    #[test]
    fn test_strip_port_suffix_no_suffix() {
        assert_eq!(strip_port_suffix("APC MINI"), "APC MINI");
        assert_eq!(strip_port_suffix("Launchpad X"), "Launchpad X");
    }

    #[test]
    fn test_strip_port_suffix_input_output() {
        assert_eq!(strip_port_suffix("Device MIDI Input"), "Device MIDI");
        assert_eq!(strip_port_suffix("Device MIDI Output"), "Device MIDI");
    }

    #[test]
    fn test_stem_pairing_matches() {
        // Ports differing only by In/Out suffix share a stem.
        let in_stem = strip_port_suffix("Tascam Model 12 MIDI In");
        let out_stem = strip_port_suffix("Tascam Model 12 MIDI Out");
        assert_eq!(in_stem, out_stem);

        // DAW CONTROL ports pair separately.
        let daw_in = strip_port_suffix("Tascam Model 12 DAW CONTROL MIDI In");
        let daw_out = strip_port_suffix("Tascam Model 12 DAW CONTROL MIDI Out");
        assert_eq!(daw_in, daw_out);

        // Main and DAW don't match.
        assert_ne!(in_stem, daw_in);
    }

    #[test]
    fn test_midi_config_validate_valid() {
        let config = MidiConfig {
            version: 1,
            mappings: vec![MidiMappingEntry {
                device_name: "APC Mini".into(),
                msg_type: "cc".into(),
                channel: 0,
                number: 48,
                param_path: "ch/aabbccdd/opacity".into(),
            }],
        };
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_midi_config_validate_empty_device() {
        let entry = MidiMappingEntry {
            device_name: String::new(),
            msg_type: "cc".into(),
            channel: 0,
            number: 0,
            param_path: "ch/aabbccdd/opacity".into(),
        };
        assert!(
            entry
                .validate("m[0]")
                .iter()
                .any(|e| e.contains("device_name"))
        );
    }

    #[test]
    fn test_midi_config_validate_bad_msg_type() {
        let entry = MidiMappingEntry {
            device_name: "dev".into(),
            msg_type: "sysex".into(),
            channel: 0,
            number: 0,
            param_path: "ch/aabbccdd/opacity".into(),
        };
        assert!(
            entry
                .validate("m[0]")
                .iter()
                .any(|e| e.contains("msg_type"))
        );
    }

    #[test]
    fn test_midi_config_validate_channel_out_of_range() {
        let entry = MidiMappingEntry {
            device_name: "dev".into(),
            msg_type: "note".into(),
            channel: 16,
            number: 60,
            param_path: "ch/aabbccdd/opacity".into(),
        };
        assert!(entry.validate("m[0]").iter().any(|e| e.contains("channel")));
    }

    #[test]
    fn test_midi_config_validate_empty_param_path() {
        let entry = MidiMappingEntry {
            device_name: "dev".into(),
            msg_type: "cc".into(),
            channel: 0,
            number: 0,
            param_path: String::new(),
        };
        assert!(
            entry
                .validate("m[0]")
                .iter()
                .any(|e| e.contains("param_path"))
        );
    }

    // ── Helper for building an AutoMapEngine with a registered device ──

    fn make_auto_map_engine(
        device_id: DeviceId,
        grid_range: [u8; 2],
        fader_range: [u8; 2],
    ) -> AutoMapEngine {
        use crate::midi::controller_profile::*;
        use std::sync::Arc;

        let profile = ControllerProfileData {
            profile: ProfileMeta {
                name: "test".into(),
                name_match: "test".into(),
            },
            leds: None,
            controls: vec![
                ControlDef {
                    name: "pads".into(),
                    control_type: "button".into(),
                    midi_type: "note".into(),
                    channel: 0,
                    range: grid_range,
                    has_led: false,
                },
                ControlDef {
                    name: "faders".into(),
                    control_type: "fader".into(),
                    midi_type: "cc".into(),
                    channel: 0,
                    range: fader_range,
                    has_led: false,
                },
            ],
            auto_map: Some(AutoMapConfig {
                strategy: "grid".into(),
                grid_control: "pads".into(),
                fader_control: "faders".into(),
                shift_control: None,
                page_buttons_control: None,
                columns: 8,
                rows: 8,
                tap_hold_threshold_ms: 300,
                tap_action: "toggle_mute".into(),
                hold_action: "solo".into(),
                fader_target: "opacity".into(),
                last_fader_target: None,
                led_rules: AutoMapLedRules {
                    active: "green".into(),
                    muted: "red".into(),
                    zero_opacity: "yellow".into(),
                    soloed: "blue".into(),
                    empty: "off".into(),
                },
            }),
        };
        let mut engine = AutoMapEngine::new();
        engine.register_device(device_id, Arc::new(profile));
        engine
    }

    #[test]
    fn test_to_config_filters_auto_mapped_keys() {
        let dev_id: DeviceId = 1;
        // Grid pads: notes 0–63; faders: CC 48–55.
        let auto_map = make_auto_map_engine(dev_id, [0, 63], [48, 55]);

        let mut store = MidiMappingStore::new();
        // Auto-mapped grid note: filtered.
        store
            .mappings
            .insert(MidiKey::Note(dev_id, 0, 10), "deck/aabbccdd/trigger".into());
        // Auto-mapped fader CC: filtered.
        store
            .mappings
            .insert(MidiKey::CC(dev_id, 0, 50), "ch/aabbccdd/opacity".into());
        // CC outside the auto-map range: kept.
        store
            .mappings
            .insert(MidiKey::CC(dev_id, 0, 99), "ch/eeff0011/opacity".into());

        let mut devices = HashMap::new();
        devices.insert(
            dev_id,
            MidiDeviceInfo {
                id: dev_id,
                name: "TestDev".into(),
                enabled: true,
                has_output: false,
                profile: None,
            },
        );

        let config = store.to_config(&devices, &auto_map);
        assert_eq!(config.mappings.len(), 1);
        assert_eq!(config.mappings[0].param_path, "ch/eeff0011/opacity");
        assert_eq!(config.mappings[0].number, 99);
    }

    #[test]
    fn test_to_config_no_automap_passes_all() {
        let dev_id: DeviceId = 1;
        let auto_map = AutoMapEngine::new(); // empty — no devices registered

        let mut store = MidiMappingStore::new();
        store
            .mappings
            .insert(MidiKey::Note(dev_id, 0, 10), "deck/aabbccdd/trigger".into());
        store
            .mappings
            .insert(MidiKey::CC(dev_id, 0, 50), "ch/aabbccdd/opacity".into());

        let mut devices = HashMap::new();
        devices.insert(
            dev_id,
            MidiDeviceInfo {
                id: dev_id,
                name: "TestDev".into(),
                enabled: true,
                has_output: false,
                profile: None,
            },
        );

        let config = store.to_config(&devices, &auto_map);
        assert_eq!(config.mappings.len(), 2);
    }
}
