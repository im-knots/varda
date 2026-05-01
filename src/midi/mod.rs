//! MIDI input support for Varda
//! Uses coremidi on macOS. Linux backend to be added later.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use crate::mixer::Mixer;
use crate::params::ParamValue;

/// Parsed MIDI message types we care about
#[derive(Debug, Clone)]
pub enum MidiMessage {
    /// Control Change: channel, cc number, value (0–127)
    ControlChange { channel: u8, cc: u8, value: u8 },
    /// Note On: channel, note, velocity
    NoteOn { channel: u8, note: u8, velocity: u8 },
    /// Note Off: channel, note, velocity
    NoteOff { channel: u8, note: u8, velocity: u8 },
}

impl MidiMessage {
    /// Parse raw MIDI bytes into a MidiMessage
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.is_empty() {
            return None;
        }
        let status = data[0];
        let msg_type = status & 0xF0;
        let channel = status & 0x0F;
        match msg_type {
            0x90 if data.len() >= 3 && data[2] > 0 => Some(MidiMessage::NoteOn {
                channel,
                note: data[1],
                velocity: data[2],
            }),
            0x90 if data.len() >= 3 => Some(MidiMessage::NoteOff {
                channel,
                note: data[1],
                velocity: 0,
            }),
            0x80 if data.len() >= 3 => Some(MidiMessage::NoteOff {
                channel,
                note: data[1],
                velocity: data[2],
            }),
            0xB0 if data.len() >= 3 => Some(MidiMessage::ControlChange {
                channel,
                cc: data[1],
                value: data[2],
            }),
            _ => None,
        }
    }

    /// Unique key for mapping: encodes message type + channel + cc/note
    pub fn mapping_key(&self) -> MidiKey {
        match self {
            MidiMessage::ControlChange { channel, cc, .. } => MidiKey::CC(*channel, *cc),
            MidiMessage::NoteOn { channel, note, .. } => MidiKey::Note(*channel, *note),
            MidiMessage::NoteOff { channel, note, .. } => MidiKey::Note(*channel, *note),
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

/// Unique identifier for a MIDI control (for mapping)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MidiKey {
    /// CC message: (channel, cc_number)
    CC(u8, u8),
    /// Note message: (channel, note_number)
    Note(u8, u8),
}

impl std::fmt::Display for MidiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MidiKey::CC(ch, cc) => write!(f, "CC ch{} #{}", ch + 1, cc),
            MidiKey::Note(ch, note) => write!(f, "Note ch{} #{}", ch + 1, note),
        }
    }
}

/// MIDI input receiver — connects to available MIDI sources and forwards messages
pub struct MidiInput {
    receiver: Receiver<MidiMessage>,
    #[cfg(target_os = "macos")]
    _client: coremidi::Client,
    #[cfg(target_os = "macos")]
    _ports: Vec<coremidi::InputPort>,
}

impl MidiInput {
    /// Create a new MIDI input that listens to all available MIDI sources
    pub fn new() -> anyhow::Result<Self> {
        let (sender, receiver) = channel();

        #[cfg(target_os = "macos")]
        {
            let client = coremidi::Client::new("Varda MIDI")
                .map_err(|e| anyhow::anyhow!("Failed to create MIDI client: {:?}", e))?;

            let mut ports = Vec::new();
            let source_count = coremidi::Sources::count();
            log::info!("Found {} MIDI source(s)", source_count);

            for i in 0..source_count {
                if let Some(source) = coremidi::Source::from_index(i) {
                    let name = source.display_name().unwrap_or_else(|| format!("Source {}", i));
                    log::info!("Connecting to MIDI source: {}", name);

                    let tx = sender.clone();
                    let port_name = format!("Varda Input {}", i);
                    let port = client
                        .input_port(&port_name, move |packet_list| {
                            for packet in packet_list.iter() {
                                if let Some(msg) = MidiMessage::from_bytes(packet.data()) {
                                    let _ = tx.send(msg);
                                }
                            }
                        })
                        .map_err(|e| anyhow::anyhow!("Failed to create MIDI port: {:?}", e))?;

                    port.connect_source(&source)
                        .map_err(|e| anyhow::anyhow!("Failed to connect to MIDI source: {:?}", e))?;

                    ports.push(port);
                }
            }

            Ok(Self {
                receiver,
                _client: client,
                _ports: ports,
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            drop(sender);
            log::warn!("MIDI input not yet supported on this platform");
            Ok(Self { receiver })
        }
    }

    /// Get the next MIDI message (non-blocking)
    pub fn try_recv(&self) -> Option<MidiMessage> {
        match self.receiver.try_recv() {
            Ok(msg) => Some(msg),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
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

    /// Enter learn mode for a specific parameter path
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
    pub fn process_learn(&mut self, key: MidiKey) -> bool {
        if let Some(path) = self.learn_target.take() {
            self.set(key, path);
            self.learn_mode = false;
            true
        } else {
            false
        }
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
        _ => {
            log::warn!("Unknown MIDI parameter path: {}", path);
            false
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
