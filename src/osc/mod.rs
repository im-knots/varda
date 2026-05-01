//! OSC (Open Sound Control) support for Varda
//! Enables external control from MIDI controllers, TouchOSC, etc.

use anyhow::{Context, Result};
use rosc::{OscMessage, OscPacket, OscType};
use std::net::{SocketAddrV4, UdpSocket};
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;

/// An OSC control message
#[derive(Debug, Clone)]
pub enum OscControl {
    /// Set a float parameter: (deck_index, param_name, value)
    SetFloat(usize, String, f32),
    /// Set deck opacity: (deck_index, value)
    SetOpacity(usize, f32),
    /// Set deck blend mode: (deck_index, mode_name)
    SetBlendMode(usize, String),
    /// Toggle deck solo: (deck_index, enabled)
    SetSolo(usize, bool),
    /// Toggle deck mute: (deck_index, enabled)
    SetMute(usize, bool),
    /// Set master effect parameter: (effect_index, param_name, value)
    SetMasterEffect(usize, String, f32),
    /// Trigger a one-shot action
    Trigger(String),
    /// Unknown message
    Unknown(String, Vec<OscType>),
}

/// OSC receiver for incoming control messages
pub struct OscReceiver {
    receiver: Receiver<OscControl>,
    _thread: thread::JoinHandle<()>,
}

impl OscReceiver {
    /// Create a new OSC receiver listening on the given port
    pub fn new(port: u16) -> Result<Self> {
        let addr = SocketAddrV4::from_str(&format!("0.0.0.0:{}", port))
            .context("Invalid address")?;
        let socket = UdpSocket::bind(addr)
            .context(format!("Failed to bind to port {}", port))?;
        
        // Set non-blocking so we can check for shutdown
        socket.set_read_timeout(Some(std::time::Duration::from_millis(100)))?;
        
        let (sender, receiver) = channel();
        
        log::info!("OSC receiver listening on port {}", port);
        
        let _thread = thread::spawn(move || {
            Self::receive_loop(socket, sender);
        });
        
        Ok(Self { receiver, _thread })
    }
    
    fn receive_loop(socket: UdpSocket, sender: Sender<OscControl>) {
        let mut buf = [0u8; rosc::decoder::MTU];
        
        loop {
            match socket.recv_from(&mut buf) {
                Ok((size, _addr)) => {
                    if let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..size]) {
                        Self::handle_packet(packet, &sender);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Timeout, continue
                }
                Err(e) => {
                    log::error!("OSC receive error: {}", e);
                    break;
                }
            }
        }
    }
    
    fn handle_packet(packet: OscPacket, sender: &Sender<OscControl>) {
        match packet {
            OscPacket::Message(msg) => {
                if let Some(control) = Self::parse_message(msg) {
                    let _ = sender.send(control);
                }
            }
            OscPacket::Bundle(bundle) => {
                for p in bundle.content {
                    Self::handle_packet(p, sender);
                }
            }
        }
    }
    
    fn parse_message(msg: OscMessage) -> Option<OscControl> {
        let parts: Vec<&str> = msg.addr.split('/').filter(|s| !s.is_empty()).collect();
        
        match parts.as_slice() {
            // /deck/0/opacity 0.5
            ["deck", idx, "opacity"] => {
                let deck = idx.parse().ok()?;
                let val = msg.args.first()?.clone().float()?;
                Some(OscControl::SetOpacity(deck, val))
            }
            // /deck/0/solo 1
            ["deck", idx, "solo"] => {
                let deck = idx.parse().ok()?;
                let val = match msg.args.first()? {
                    OscType::Int(i) => *i != 0,
                    OscType::Float(f) => *f > 0.5,
                    OscType::Bool(b) => *b,
                    _ => return None,
                };
                Some(OscControl::SetSolo(deck, val))
            }
            // /deck/0/mute 1
            ["deck", idx, "mute"] => {
                let deck = idx.parse().ok()?;
                let val = match msg.args.first()? {
                    OscType::Int(i) => *i != 0,
                    OscType::Float(f) => *f > 0.5,
                    OscType::Bool(b) => *b,
                    _ => return None,
                };
                Some(OscControl::SetMute(deck, val))
            }
            // /deck/0/param/name 0.5
            ["deck", idx, "param", name] => {
                let deck = idx.parse().ok()?;
                let val = msg.args.first()?.clone().float()?;
                Some(OscControl::SetFloat(deck, name.to_string(), val))
            }
            // /trigger/name
            ["trigger", name] => {
                Some(OscControl::Trigger(name.to_string()))
            }
            _ => {
                Some(OscControl::Unknown(msg.addr, msg.args))
            }
        }
    }
    
    /// Get the next OSC control message (non-blocking)
    pub fn try_recv(&self) -> Option<OscControl> {
        match self.receiver.try_recv() {
            Ok(ctrl) => Some(ctrl),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }
}

/// OSC sender for outputting audio analysis data and other state
pub struct OscSender {
    socket: UdpSocket,
    target: SocketAddrV4,
}

impl OscSender {
    /// Create a new OSC sender that sends to the given target address
    pub fn new(target_addr: &str) -> Result<Self> {
        let target = SocketAddrV4::from_str(target_addr)
            .context(format!("Invalid target address: {}", target_addr))?;

        // Bind to any available port
        let socket = UdpSocket::bind("0.0.0.0:0")
            .context("Failed to create UDP socket for OSC output")?;

        log::info!("OSC sender created, target: {}", target_addr);

        Ok(Self { socket, target })
    }

    /// Send audio analysis data
    pub fn send_audio_data(&self, level: f32, bass: f32, mid: f32, treble: f32, bpm: f32, beat_phase: f32) {
        // Send individual messages for each metric
        let _ = self.send_float("/audio/level", level);
        let _ = self.send_float("/audio/bass", bass);
        let _ = self.send_float("/audio/mid", mid);
        let _ = self.send_float("/audio/treble", treble);
        let _ = self.send_float("/audio/bpm", bpm);
        let _ = self.send_float("/audio/beat_phase", beat_phase);
    }

    /// Send a float value to the given OSC address
    pub fn send_float(&self, addr: &str, value: f32) -> Result<()> {
        let msg = OscMessage {
            addr: addr.to_string(),
            args: vec![OscType::Float(value)],
        };
        let packet = OscPacket::Message(msg);
        let buf = rosc::encoder::encode(&packet)?;
        self.socket.send_to(&buf, self.target)?;
        Ok(())
    }

    /// Send a boolean value to the given OSC address
    pub fn send_bool(&self, addr: &str, value: bool) -> Result<()> {
        let msg = OscMessage {
            addr: addr.to_string(),
            args: vec![OscType::Bool(value)],
        };
        let packet = OscPacket::Message(msg);
        let buf = rosc::encoder::encode(&packet)?;
        self.socket.send_to(&buf, self.target)?;
        Ok(())
    }

    /// Send an integer value to the given OSC address
    pub fn send_int(&self, addr: &str, value: i32) -> Result<()> {
        let msg = OscMessage {
            addr: addr.to_string(),
            args: vec![OscType::Int(value)],
        };
        let packet = OscPacket::Message(msg);
        let buf = rosc::encoder::encode(&packet)?;
        self.socket.send_to(&buf, self.target)?;
        Ok(())
    }
}
