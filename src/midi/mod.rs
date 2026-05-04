//! MIDI input/output support for Varda
//! Uses midir for cross-platform MIDI (CoreMIDI/ALSA/JACK/WinMM).
//!
//! Supports N simultaneous MIDI devices. Each device gets a unique `DeviceId`.
//! MIDI mappings are device-specific so two controllers can have the same CC#
//! mapped to different parameters.

pub mod apc_mini;

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Mutex;

use midir::{MidiInput, MidiOutput};

use crate::mixer::Mixer;
use crate::modulation::ModulationSource;
use crate::params::ParamValue;

/// Stable identifier for a MIDI device within a session.
pub type DeviceId = u32;

/// Parsed MIDI message types we care about
#[derive(Debug, Clone)]
pub enum MidiMessage {
    /// Control Change: channel, cc number, value (0–127)
    ControlChange { device_id: DeviceId, channel: u8, cc: u8, value: u8 },
    /// Note On: channel, note, velocity
    NoteOn { device_id: DeviceId, channel: u8, note: u8, velocity: u8 },
    /// Note Off: channel, note, velocity
    NoteOff { device_id: DeviceId, channel: u8, note: u8, velocity: u8 },
}

impl MidiMessage {
    /// Parse raw MIDI bytes into a MidiMessage, tagged with a device ID.
    pub fn from_bytes(data: &[u8], device_id: DeviceId) -> Option<Self> {
        if data.is_empty() {
            return None;
        }
        let status = data[0];
        let msg_type = status & 0xF0;
        let channel = status & 0x0F;
        match msg_type {
            0x90 if data.len() >= 3 && data[2] > 0 => Some(MidiMessage::NoteOn {
                device_id, channel, note: data[1], velocity: data[2],
            }),
            0x90 if data.len() >= 3 => Some(MidiMessage::NoteOff {
                device_id, channel, note: data[1], velocity: 0,
            }),
            0x80 if data.len() >= 3 => Some(MidiMessage::NoteOff {
                device_id, channel, note: data[1], velocity: data[2],
            }),
            0xB0 if data.len() >= 3 => Some(MidiMessage::ControlChange {
                device_id, channel, cc: data[1], value: data[2],
            }),
            _ => None,
        }
    }

    /// The device this message came from.
    pub fn device_id(&self) -> DeviceId {
        match self {
            MidiMessage::ControlChange { device_id, .. } => *device_id,
            MidiMessage::NoteOn { device_id, .. } => *device_id,
            MidiMessage::NoteOff { device_id, .. } => *device_id,
        }
    }

    /// Unique key for mapping: encodes device + message type + channel + cc/note
    pub fn mapping_key(&self) -> MidiKey {
        match self {
            MidiMessage::ControlChange { device_id, channel, cc, .. } => MidiKey::CC(*device_id, *channel, *cc),
            MidiMessage::NoteOn { device_id, channel, note, .. } => MidiKey::Note(*device_id, *channel, *note),
            MidiMessage::NoteOff { device_id, channel, note, .. } => MidiKey::Note(*device_id, *channel, *note),
        }
    }

    /// Normalized value (0.0–1.0)
    pub fn normalized_value(&self) -> f32 {
        match self {
            MidiMessage::ControlChange { value, .. } => *value as f32 / 127.0,
            MidiMessage::NoteOn { velocity, .. } => *velocity as f32 / 127.0,
            MidiMessage::NoteOff { .. } => 0.0,
        }
    }
}

/// Unique identifier for a MIDI control (for mapping).
/// Includes device_id so the same CC# on different devices maps independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MidiKey {
    /// CC message: (device_id, channel, cc_number)
    CC(DeviceId, u8, u8),
    /// Note message: (device_id, channel, note_number)
    Note(DeviceId, u8, u8),
}

impl MidiKey {
    pub fn device_id(&self) -> DeviceId {
        match self {
            MidiKey::CC(d, _, _) => *d,
            MidiKey::Note(d, _, _) => *d,
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

/// Information about a connected MIDI device.
#[derive(Debug, Clone)]
pub struct MidiDeviceInfo {
    /// Stable ID for this session (assigned on scan).
    pub id: DeviceId,
    /// Human-readable device name.
    pub name: String,
    /// Whether this device is enabled for input.
    pub enabled: bool,
    /// Whether this device supports output (has a matching destination).
    pub has_output: bool,
    /// Controller profile type detected from name.
    pub profile: ControllerProfile,
}

/// Known controller profiles (detected by name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerProfile {
    ApcMini,
    Generic,
}

impl ControllerProfile {
    fn detect(name: &str) -> Self {
        let lower = name.to_lowercase();
        if lower.contains("apc mini") {
            ControllerProfile::ApcMini
        } else {
            ControllerProfile::Generic
        }
    }
}

// ── MIDI Device Manager ────────────────────────────────────────────

/// Manages N MIDI devices — input, output, device discovery, and message routing.
pub struct MidiDeviceManager {
    receiver: Receiver<MidiMessage>,
    sender: Sender<MidiMessage>,
    /// All known devices (by DeviceId).
    pub devices: HashMap<DeviceId, MidiDeviceInfo>,
    /// Next device ID to assign.
    next_device_id: DeviceId,
    /// Held alive so callbacks keep firing.
    input_connections: Vec<midir::MidiInputConnection<()>>,
    /// Output connections keyed by DeviceId. Mutex for interior mutability
    /// (MidiOutputConnection::send requires &mut self, but send_raw takes &self).
    output_connections: HashMap<DeviceId, Mutex<midir::MidiOutputConnection>>,
}

impl MidiDeviceManager {
    /// Create a new device manager and scan for connected devices.
    pub fn new() -> anyhow::Result<Self> {
        let (sender, receiver) = channel();

        let mut mgr = Self {
            receiver,
            sender,
            devices: HashMap::new(),
            next_device_id: 0,
            input_connections: Vec::new(),
            output_connections: HashMap::new(),
        };
        mgr.scan_devices()?;
        Ok(mgr)
    }

    /// Scan for MIDI devices. Can be called again to rescan (hot-plug).
    pub fn scan_devices(&mut self) -> anyhow::Result<()> {
        // Disconnect existing connections by dropping them
        self.input_connections.clear();
        self.output_connections.clear();
        self.devices.clear();
        self.next_device_id = 0;

        // Snapshot output port names (throwaway MidiOutput, collect names, drop)
        let output_port_names: Vec<String> = {
            let midi_out = MidiOutput::new("Varda scan")
                .map_err(|e| anyhow::anyhow!("Failed to create MidiOutput for scan: {}", e))?;
            let ports = midi_out.ports();
            ports.iter()
                .map(|p| midi_out.port_name(p).unwrap_or_default())
                .collect()
        };

        // Snapshot input port names (throwaway MidiInput, collect names, drop)
        let input_port_names: Vec<String> = {
            let midi_in = MidiInput::new("Varda scan")
                .map_err(|e| anyhow::anyhow!("Failed to create MidiInput for scan: {}", e))?;
            let ports = midi_in.ports();
            ports.iter()
                .map(|p| midi_in.port_name(p).unwrap_or_default())
                .collect()
        };

        log::info!("MIDI scan: {} input(s), {} output(s)", input_port_names.len(), output_port_names.len());

        // Track which output names have been matched to an input
        let mut matched_outputs: Vec<bool> = vec![false; output_port_names.len()];

        // For each input: assign DeviceId, check matching output, connect
        for (i, in_name) in input_port_names.iter().enumerate() {
            let device_id = self.next_device_id;
            self.next_device_id += 1;

            // Check for a matching output port by name
            let matching_out_idx = output_port_names.iter().position(|out_name| {
                out_name.to_lowercase() == in_name.to_lowercase()
            });
            let has_output = matching_out_idx.is_some();

            if let Some(out_idx) = matching_out_idx {
                matched_outputs[out_idx] = true;
                self.connect_output(device_id, &output_port_names[out_idx]);
            }

            let profile = ControllerProfile::detect(in_name);
            log::info!("MIDI device [{}]: {} (profile={:?}, output={})",
                device_id, in_name, profile, has_output);

            self.devices.insert(device_id, MidiDeviceInfo {
                id: device_id,
                name: in_name.clone(),
                enabled: true,
                has_output,
                profile,
            });

            // Connect input (fresh MidiInput per port — midir consumes it on connect)
            let tx = self.sender.clone();
            let dev_id = device_id;
            let port_label = format!("Varda In {}", device_id);
            match MidiInput::new(&port_label) {
                Ok(midi_in) => {
                    let ports = midi_in.ports();
                    if i < ports.len() {
                        match midi_in.connect(
                            &ports[i],
                            &port_label,
                            move |_ts, data, _| {
                                if let Some(msg) = MidiMessage::from_bytes(data, dev_id) {
                                    let _ = tx.send(msg);
                                }
                            },
                            (),
                        ) {
                            Ok(conn) => self.input_connections.push(conn),
                            Err(e) => log::warn!("Failed to connect MIDI input {}: {}", in_name, e),
                        }
                    }
                }
                Err(e) => log::warn!("Failed to create MidiInput for {}: {}", in_name, e),
            }
        }

        // Register unmatched outputs as output-only devices
        for (j, out_name) in output_port_names.iter().enumerate() {
            if matched_outputs[j] {
                continue;
            }
            let device_id = self.next_device_id;
            self.next_device_id += 1;
            let profile = ControllerProfile::detect(out_name);
            log::info!("MIDI output-only device [{}]: {} (profile={:?})", device_id, out_name, profile);
            self.connect_output(device_id, out_name);
            self.devices.insert(device_id, MidiDeviceInfo {
                id: device_id,
                name: out_name.clone(),
                enabled: true,
                has_output: true,
                profile,
            });
        }

        Ok(())
    }

    /// Connect an output port by name and store the connection.
    fn connect_output(&mut self, device_id: DeviceId, port_name: &str) {
        match MidiOutput::new(&format!("Varda Out {}", device_id)) {
            Ok(midi_out) => {
                let ports = midi_out.ports();
                let port = ports.iter().find(|p| {
                    midi_out.port_name(p).map(|n| n == port_name).unwrap_or(false)
                });
                if let Some(port) = port {
                    match midi_out.connect(port, &format!("Varda Out {}", device_id)) {
                        Ok(conn) => {
                            self.output_connections.insert(device_id, Mutex::new(conn));
                        }
                        Err(e) => log::warn!("Failed to connect MIDI output {}: {}", port_name, e),
                    }
                }
            }
            Err(e) => log::warn!("Failed to create MidiOutput for {}: {}", port_name, e),
        }
    }

    /// Get the next MIDI message (non-blocking). Skips messages from disabled devices.
    pub fn try_recv(&self) -> Option<MidiMessage> {
        loop {
            match self.receiver.try_recv() {
                Ok(msg) => {
                    let dev_id = msg.device_id();
                    if let Some(info) = self.devices.get(&dev_id) {
                        if info.enabled {
                            return Some(msg);
                        }
                    }
                    // Device disabled or unknown — skip
                    continue;
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }

    /// Send a Note On message to a specific device (by device_id).
    pub fn send_note_on(&self, device_id: DeviceId, channel: u8, note: u8, velocity: u8) {
        let status = 0x90 | (channel & 0x0F);
        self.send_raw(device_id, &[status, note, velocity]);
    }

    /// Send raw MIDI bytes to a specific device.
    pub fn send_raw(&self, device_id: DeviceId, bytes: &[u8]) {
        if let Some(conn_mutex) = self.output_connections.get(&device_id) {
            if let Ok(mut conn) = conn_mutex.lock() {
                if let Err(e) = conn.send(bytes) {
                    log::warn!("Failed to send MIDI to device {}: {}", device_id, e);
                }
            }
        }
    }

    /// Get device info by ID.
    pub fn device(&self, id: DeviceId) -> Option<&MidiDeviceInfo> {
        self.devices.get(&id)
    }

    /// Toggle a device's enabled state.
    pub fn set_device_enabled(&mut self, id: DeviceId, enabled: bool) {
        if let Some(info) = self.devices.get_mut(&id) {
            info.enabled = enabled;
            log::info!("MIDI device [{}] {} → {}", id, info.name,
                if enabled { "enabled" } else { "disabled" });
        }
    }

    /// Get all device IDs matching a controller profile.
    pub fn devices_with_profile(&self, profile: ControllerProfile) -> Vec<DeviceId> {
        self.devices.iter()
            .filter(|(_, info)| info.profile == profile && info.enabled)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get a sorted list of all device infos (for UI display).
    pub fn device_list(&self) -> Vec<MidiDeviceInfo> {
        let mut list: Vec<_> = self.devices.values().cloned().collect();
        list.sort_by_key(|d| d.id);
        list
    }
}

// ── MIDI Mapping Store ──────────────────────────────────────────────

/// Persistent mapping from MIDI controls to parameter paths.
///
/// Parameter path format:
///   crossfader                         → mixer crossfader position
///   ch/<n>/opacity                     → channel opacity
///   ch/<n>/deck/<m>/opacity            → deck opacity
///   ch/<n>/deck/<m>/param/<name>       → generator param (float)
///   ch/<n>/deck/<m>/effect/<k>/param/<name> → effect param (float)
///   master/effect/<k>/param/<name>     → master effect param (float)
#[derive(Debug, Clone)]
pub struct MidiMappingStore {
    /// MidiKey → parameter path
    pub mappings: HashMap<MidiKey, String>,
    /// Whether learn mode is active
    pub learn_mode: bool,
    /// The parameter path waiting for the next MIDI input (learn target)
    pub learn_target: Option<String>,
}

impl MidiMappingStore {
    pub fn new() -> Self {
        Self {
            mappings: HashMap::new(),
            learn_mode: false,
            learn_target: None,
        }
    }

    /// Set a mapping from a MIDI key to a parameter path
    pub fn set(&mut self, key: MidiKey, path: String) {
        log::info!("MIDI mapped {} → {}", key, path);
        self.mappings.insert(key, path);
    }

    /// Remove a mapping
    pub fn remove(&mut self, key: &MidiKey) {
        self.mappings.remove(key);
    }

    /// Get the parameter path for a MIDI key
    pub fn get(&self, key: &MidiKey) -> Option<&String> {
        self.mappings.get(key)
    }

    /// Toggle learn mode on/off. Clears learn target when turning off.
    pub fn toggle_learn(&mut self) {
        self.learn_mode = !self.learn_mode;
        if !self.learn_mode {
            self.learn_target = None;
        }
        log::info!("MIDI learn mode: {}", if self.learn_mode { "ON" } else { "OFF" });
    }

    /// Select a parameter path as the learn target (must be in learn mode).
    pub fn select_learn_target(&mut self, param_path: String) {
        if self.learn_mode {
            log::info!("MIDI learn target: {}", param_path);
            self.learn_target = Some(param_path);
        }
    }

    /// Enter learn mode for a specific parameter path (legacy — used by main loop).
    pub fn start_learn(&mut self, param_path: String) {
        self.learn_mode = true;
        self.learn_target = Some(param_path);
        log::info!("MIDI learn mode: waiting for input...");
    }

    /// Cancel learn mode
    pub fn cancel_learn(&mut self) {
        self.learn_mode = false;
        self.learn_target = None;
    }

    /// Process a MIDI message in learn mode. Returns true if a mapping was created.
    /// Stays in learn mode — clears target so user can select another param.
    pub fn process_learn(&mut self, key: MidiKey) -> bool {
        if let Some(path) = self.learn_target.take() {
            self.set(key, path);
            // Stay in learn mode — user can select another param
            true
        } else {
            false
        }
    }

    /// Remove all mappings.
    pub fn clear_all(&mut self) {
        self.mappings.clear();
        log::info!("MIDI mappings cleared");
    }

    /// Get all mappings sorted by device ID for display.
    pub fn sorted_mappings(&self) -> Vec<(MidiKey, String)> {
        let mut list: Vec<_> = self.mappings.iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        list.sort_by_key(|(k, _)| k.device_id());
        list
    }
}

// ── Parameter Resolution ────────────────────────────────────────────

/// Apply a normalized MIDI value (0.0–1.0) to the parameter at the given path.
/// Returns true if the path resolved successfully.
pub fn apply_midi_to_param(mixer: &mut Mixer, path: &str, value: f32) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        // crossfader
        ["crossfader"] => {
            mixer.snap_crossfader(value);
            true
        }
        // ch/<n>/opacity
        ["ch", ch_s, "opacity"] => {
            if let Ok(ch) = ch_s.parse::<usize>() {
                if let Some(channel) = mixer.channel_mut(ch) {
                    channel.opacity = value.clamp(0.0, 1.0);
                    return true;
                }
            }
            false
        }
        // ch/<n>/deck/<m>/opacity
        ["ch", ch_s, "deck", dk_s, "opacity"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if dk < channel.decks.len() {
                        channel.decks[dk].opacity = value.clamp(0.0, 1.0);
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/mute — toggle on any note-on (value > 0.5)
        ["ch", ch_s, "deck", dk_s, "mute"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if dk < channel.decks.len() {
                        if value > 0.5 {
                            channel.decks[dk].mute = !channel.decks[dk].mute;
                        }
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/solo — toggle on any note-on (value > 0.5)
        ["ch", ch_s, "deck", dk_s, "solo"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if dk < channel.decks.len() {
                        if value > 0.5 {
                            channel.decks[dk].solo = !channel.decks[dk].solo;
                        }
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/trigger — activate deck (set opacity to 1.0 on note-on)
        ["ch", ch_s, "deck", dk_s, "trigger"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if dk < channel.decks.len() && value > 0.5 {
                        channel.decks[dk].opacity = 1.0;
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/param/<name>
        ["ch", ch_s, "deck", dk_s, "param", name] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if dk < channel.decks.len() {
                        apply_float_param_scaled(&mut channel.decks[dk].deck.generator_params, name, value);
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/effect/<k>/param/<name>
        ["ch", ch_s, "deck", dk_s, "effect", ek_s, "param", name] => {
            if let (Ok(ch), Ok(dk), Ok(ek)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>(), ek_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if dk < channel.decks.len() && ek < channel.decks[dk].deck.effects.len() {
                        apply_float_param_scaled(&mut channel.decks[dk].deck.effects[ek].params, name, value);
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/effect/<k>/param/<name>
        ["ch", ch_s, "effect", ek_s, "param", name] => {
            if let (Ok(ch), Ok(ek)) = (ch_s.parse::<usize>(), ek_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if ek < channel.effects.len() {
                        apply_float_param_scaled(&mut channel.effects[ek].params, name, value);
                        return true;
                    }
                }
            }
            false
        }
        // master/effect/<k>/param/<name>
        ["master", "effect", ek_s, "param", name] => {
            if let Ok(ek) = ek_s.parse::<usize>() {
                if ek < mixer.master_effects.len() {
                    apply_float_param_scaled(&mut mixer.master_effects[ek].params, name, value);
                    return true;
                }
            }
            false
        }
        // mod/<idx>/<param_name> — modulation source params
        ["mod", idx_s, param_name] => {
            if let Ok(idx) = idx_s.parse::<usize>() {
                if idx < mixer.modulation.sources.len() {
                    apply_mod_param(&mut mixer.modulation.sources[idx], param_name, value);
                    return true;
                }
            }
            false
        }
        // mod/<idx>/step/<step_idx> — step sequencer step values
        ["mod", idx_s, "step", step_s] => {
            if let (Ok(idx), Ok(step_idx)) = (idx_s.parse::<usize>(), step_s.parse::<usize>()) {
                if idx < mixer.modulation.sources.len() {
                    if let ModulationSource::StepSequencer { steps, .. } = &mut mixer.modulation.sources[idx] {
                        if step_idx < steps.len() {
                            steps[step_idx] = value.clamp(0.0, 1.0);
                            return true;
                        }
                    }
                }
            }
            false
        }
        _ => {
            log::warn!("Unknown MIDI parameter path: {}", path);
            false
        }
    }
}

/// Apply a normalized MIDI value to a modulation source parameter.
fn apply_mod_param(source: &mut ModulationSource, param_name: &str, value: f32) {
    match source {
        ModulationSource::LFO { frequency, amplitude, phase, .. } => {
            match param_name {
                "frequency" => *frequency = 0.01 + value * 9.99, // 0.01–10.0 Hz
                "amplitude" => *amplitude = value.clamp(0.0, 1.0),
                "phase" => *phase = value.clamp(0.0, 1.0),
                _ => log::warn!("Unknown LFO param: {}", param_name),
            }
        }
        ModulationSource::AudioBand { smoothing, .. } => {
            match param_name {
                "smoothing" => *smoothing = (value * 0.99).clamp(0.0, 0.99),
                _ => log::warn!("Unknown Audio param: {}", param_name),
            }
        }
        ModulationSource::ADSR { attack, decay, sustain, release, .. } => {
            match param_name {
                "attack" => *attack = 0.001 + value * 4.999,   // 0.001–5.0s
                "decay" => *decay = 0.001 + value * 4.999,
                "sustain" => *sustain = value.clamp(0.0, 1.0),
                "release" => *release = 0.001 + value * 4.999,
                _ => log::warn!("Unknown ADSR param: {}", param_name),
            }
        }
        ModulationSource::StepSequencer { rate, .. } => {
            match param_name {
                "rate" => *rate = 0.1 + value * 19.9, // 0.1–20.0 Hz
                _ => log::warn!("Unknown StepSeq param: {}", param_name),
            }
        }
    }
}

/// Apply a normalized 0.0–1.0 value to a float param, scaling to the param's min/max range.
fn apply_float_param_scaled(params: &mut crate::ShaderParams, name: &str, normalized: f32) {
    // Look up min/max from the ISF definition
    if let Some(def) = params.definitions.get(name) {
        let min = def.min.unwrap_or(0.0);
        let max = def.max.unwrap_or(1.0);
        let scaled = min + normalized * (max - min);
        params.set(name, ParamValue::Float(scaled));
    } else {
        // No definition — assume 0.0–1.0 range
        params.set(name, ParamValue::Float(normalized));
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
            MidiMessage::NoteOn { device_id, channel, note, velocity } => {
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
        let key1 = msg1.mapping_key();
        let key2 = msg2.mapping_key();
        // Same CC on different devices should be different keys
        assert_ne!(key1, key2);
        assert_eq!(key1.device_id(), 0);
        assert_eq!(key2.device_id(), 1);
    }

    #[test]
    fn test_midi_key_same_device_same_control() {
        let msg1 = MidiMessage::from_bytes(&[0xB0, 48, 64], 5).unwrap();
        let msg2 = MidiMessage::from_bytes(&[0xB0, 48, 100], 5).unwrap();
        // Same device, same CC — keys should match (different values don't matter)
        assert_eq!(msg1.mapping_key(), msg2.mapping_key());
    }

    #[test]
    fn test_midi_key_display_with_device() {
        let key = MidiKey::CC(3, 0, 48);
        let display = format!("{}", key);
        assert!(display.contains("dev3"));
        assert!(display.contains("48"));
    }

    #[test]
    fn test_mapping_store_clear_all() {
        let mut store = MidiMappingStore::new();
        store.set(MidiKey::CC(0, 0, 48), "crossfader".to_string());
        store.set(MidiKey::Note(1, 0, 36), "ch/0/opacity".to_string());
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
    fn test_controller_profile_detect() {
        assert_eq!(ControllerProfile::detect("APC MINI"), ControllerProfile::ApcMini);
        assert_eq!(ControllerProfile::detect("Apc Mini mk2"), ControllerProfile::ApcMini);
        assert_eq!(ControllerProfile::detect("Novation Launchpad"), ControllerProfile::Generic);
    }
}
