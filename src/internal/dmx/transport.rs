//! DMX transports: where a universe actually goes.
//!
//! One trait, two implementations. Adding USB serial later is a third and requires no change
//! above this line. Multiple transports may be active at once, because a rig with an Art-Net
//! node and an sACN console on the same network is ordinary.
//!
//! See /spec/dmx-output.md § C8.

use super::artnet;
use super::sacn;
use super::universe::{UNIVERSE_SIZE, UniverseId};
use std::collections::HashMap;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};

/// Why a transport could not deliver.
#[derive(Debug)]
pub enum DmxError {
    Bind(std::io::Error),
    Send(std::io::Error),
    Config(String),
}

impl fmt::Display for DmxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(e) => write!(f, "bind failed: {e}"),
            Self::Send(e) => write!(f, "send failed: {e}"),
            Self::Config(m) => write!(f, "configuration error: {m}"),
        }
    }
}

impl std::error::Error for DmxError {}

/// A destination for DMX universes.
///
/// The transport owns its own per-universe sequence counters, because Art-Net and sACN number
/// differently: Art-Net reserves 0 to mean "sequencing disabled" and must skip it, while 0 is
/// an ordinary value in E1.31.
pub trait DmxTransport: Send {
    /// Human-readable label for status reporting.
    fn label(&self) -> String;

    /// Transmit one universe.
    ///
    /// # Errors
    ///
    /// Returns [`DmxError::Send`] when the socket rejects the datagram.
    fn send(&mut self, universe: UniverseId, data: &[u8; UNIVERSE_SIZE]) -> Result<(), DmxError>;

    /// Signal that this source is releasing a universe.
    ///
    /// sACN sends three packets with `Stream_Terminated` set, which releases receivers
    /// immediately instead of leaving them holding the last value for 2.5 seconds. Art-Net has
    /// no equivalent flag, so a dark frame is the mechanism there and this is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`DmxError::Send`] when the socket rejects the datagram.
    fn terminate(&mut self, universe: UniverseId) -> Result<(), DmxError>;
}

// ── Art-Net ─────────────────────────────────────────────────────────────────

/// Art-Net sender. Unicast to a node, or broadcast to a subnet.
pub struct ArtNetSender {
    socket: UdpSocket,
    target: SocketAddr,
    sequence: HashMap<UniverseId, u8>,
}

impl ArtNetSender {
    /// Bind a sender aimed at `target`.
    ///
    /// # Errors
    ///
    /// Returns [`DmxError::Bind`] when the socket cannot be bound or broadcast cannot be
    /// enabled for a broadcast target.
    pub fn new(target: SocketAddr, broadcast: bool) -> Result<Self, DmxError> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(DmxError::Bind)?;
        if broadcast {
            socket.set_broadcast(true).map_err(DmxError::Bind)?;
        }
        Ok(Self {
            socket,
            target,
            sequence: HashMap::new(),
        })
    }

    /// The local address the sender bound to. Useful in tests and status reporting.
    ///
    /// # Errors
    ///
    /// Returns [`DmxError::Bind`] when the socket cannot report its address.
    pub fn local_addr(&self) -> Result<SocketAddr, DmxError> {
        self.socket.local_addr().map_err(DmxError::Bind)
    }
}

impl DmxTransport for ArtNetSender {
    fn label(&self) -> String {
        format!("art-net {}", self.target)
    }

    fn send(&mut self, universe: UniverseId, data: &[u8; UNIVERSE_SIZE]) -> Result<(), DmxError> {
        let seq = self.sequence.entry(universe).or_insert(0);
        *seq = artnet::next_sequence(*seq);
        let packet = artnet::encode_artdmx(universe, *seq, 0, data);
        self.socket
            .send_to(&packet, self.target)
            .map_err(DmxError::Send)?;
        Ok(())
    }

    fn terminate(&mut self, _universe: UniverseId) -> Result<(), DmxError> {
        // Art-Net has no termination flag. The driver's dark frame is the mechanism.
        Ok(())
    }
}

// ── sACN ────────────────────────────────────────────────────────────────────

/// Where an sACN sender puts its packets.
#[derive(Debug, Clone)]
pub enum SacnDestination {
    /// Standard E1.31 multicast: `239.255.<high>.<low>` per universe.
    Multicast,
    /// Unicast to one receiver, for networks where multicast is filtered.
    Unicast(SocketAddr),
}

/// sACN (E1.31) sender.
pub struct SacnSender {
    socket: UdpSocket,
    config: sacn::SourceConfig,
    destination: SacnDestination,
    sequence: HashMap<UniverseId, u8>,
}

impl SacnSender {
    /// Bind a sender.
    ///
    /// `interface` selects the outgoing interface for multicast; pass
    /// [`Ipv4Addr::UNSPECIFIED`] to let the OS choose.
    ///
    /// # Errors
    ///
    /// Returns [`DmxError::Bind`] when the socket cannot be bound or configured.
    pub fn new(
        config: sacn::SourceConfig,
        destination: SacnDestination,
        interface: Ipv4Addr,
    ) -> Result<Self, DmxError> {
        let socket = UdpSocket::bind((interface, 0)).map_err(DmxError::Bind)?;
        if matches!(destination, SacnDestination::Multicast) {
            // One hop past the local segment by default: enough for a switch, not enough to
            // flood a routed campus network with lighting data nobody asked for.
            socket.set_multicast_ttl_v4(1).map_err(DmxError::Bind)?;
        }
        Ok(Self {
            socket,
            config,
            destination,
            sequence: HashMap::new(),
        })
    }

    fn address(&self, universe: UniverseId) -> SocketAddr {
        match self.destination {
            SacnDestination::Multicast => SocketAddr::V4(SocketAddrV4::new(
                sacn::multicast_addr(universe),
                sacn::SACN_PORT,
            )),
            SacnDestination::Unicast(addr) => addr,
        }
    }

    fn next_seq(&mut self, universe: UniverseId) -> u8 {
        let seq = self.sequence.entry(universe).or_insert(0);
        *seq = sacn::next_sequence(*seq);
        *seq
    }

    /// The local address the sender bound to.
    ///
    /// # Errors
    ///
    /// Returns [`DmxError::Bind`] when the socket cannot report its address.
    pub fn local_addr(&self) -> Result<SocketAddr, DmxError> {
        self.socket.local_addr().map_err(DmxError::Bind)
    }
}

impl DmxTransport for SacnSender {
    fn label(&self) -> String {
        match self.destination {
            SacnDestination::Multicast => "sacn multicast".to_string(),
            SacnDestination::Unicast(a) => format!("sacn unicast {a}"),
        }
    }

    fn send(&mut self, universe: UniverseId, data: &[u8; UNIVERSE_SIZE]) -> Result<(), DmxError> {
        let seq = self.next_seq(universe);
        let packet = sacn::encode_data_packet(&self.config, universe, seq, data, false);
        self.socket
            .send_to(&packet, self.address(universe))
            .map_err(DmxError::Send)?;
        Ok(())
    }

    fn terminate(&mut self, universe: UniverseId) -> Result<(), DmxError> {
        let dark = [0u8; UNIVERSE_SIZE];
        let addr = self.address(universe);
        // E1.31 requires three terminated packets. A receiver that only sees silence holds its
        // last value until its 2.5 second data-loss timeout expires.
        for _ in 0..sacn::TERMINATION_PACKET_COUNT {
            let seq = self.next_seq(universe);
            let packet = sacn::encode_data_packet(&self.config, universe, seq, &dark, true);
            self.socket.send_to(&packet, addr).map_err(DmxError::Send)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn loopback_receiver() -> (UdpSocket, SocketAddr) {
        let s = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind receiver");
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let addr = s.local_addr().unwrap();
        (s, addr)
    }

    fn cid() -> [u8; 16] {
        [0x11; 16]
    }

    #[test]
    fn artnet_sends_a_well_formed_packet_over_the_wire() {
        let (rx, addr) = loopback_receiver();
        let mut tx = ArtNetSender::new(addr, false).expect("bind sender");
        let mut data = [0u8; UNIVERSE_SIZE];
        data[0] = 42;
        data[511] = 7;
        tx.send(3, &data).expect("send");

        let mut buf = [0u8; 1024];
        let (n, _) = rx.recv_from(&mut buf).expect("receive");
        assert_eq!(n, artnet::ARTDMX_LEN);
        assert_eq!(&buf[0..8], artnet::ARTNET_ID);
        assert_eq!(buf[14], 3, "universe low byte");
        assert_eq!(buf[18], 42, "slot 1");
        assert_eq!(buf[18 + 511], 7, "slot 512");
    }

    #[test]
    fn artnet_sequence_advances_per_universe_and_skips_zero() {
        let (rx, addr) = loopback_receiver();
        let mut tx = ArtNetSender::new(addr, false).unwrap();
        let data = [0u8; UNIVERSE_SIZE];
        let mut buf = [0u8; 1024];

        let mut seqs = Vec::new();
        for _ in 0..3 {
            tx.send(1, &data).unwrap();
            rx.recv_from(&mut buf).unwrap();
            seqs.push(buf[12]);
        }
        assert_eq!(seqs, vec![1, 2, 3]);
        assert!(!seqs.contains(&0), "0 means sequencing disabled");

        // A different universe counts independently.
        tx.send(2, &data).unwrap();
        rx.recv_from(&mut buf).unwrap();
        assert_eq!(buf[12], 1, "universe 2 starts its own count");
    }

    #[test]
    fn artnet_terminate_is_a_noop() {
        let (rx, addr) = loopback_receiver();
        let mut tx = ArtNetSender::new(addr, false).unwrap();
        tx.terminate(1).expect("terminate");
        rx.set_read_timeout(Some(Duration::from_millis(120)))
            .unwrap();
        let mut buf = [0u8; 1024];
        assert!(
            rx.recv_from(&mut buf).is_err(),
            "Art-Net has no termination packet"
        );
    }

    #[test]
    fn sacn_unicast_sends_a_well_formed_packet() {
        let (rx, addr) = loopback_receiver();
        let cfg = sacn::SourceConfig::new(cid(), "Varda");
        let mut tx = SacnSender::new(cfg, SacnDestination::Unicast(addr), Ipv4Addr::LOCALHOST)
            .expect("bind sender");
        let mut data = [0u8; UNIVERSE_SIZE];
        data[0] = 99;
        tx.send(5, &data).expect("send");

        let mut buf = [0u8; 1024];
        let (n, _) = rx.recv_from(&mut buf).expect("receive");
        assert_eq!(n, sacn::DATA_PACKET_LEN);
        assert_eq!(&buf[4..16], sacn::ACN_PID);
        assert_eq!(&buf[113..115], &[0, 5], "universe");
        assert_eq!(buf[112], 0, "not terminated");
        assert_eq!(buf[126], 99, "slot 1 follows the start code");
    }

    /// Contract C6. Three packets with the terminated bit, carrying zeroes.
    #[test]
    fn sacn_terminate_sends_three_terminated_dark_packets() {
        let (rx, addr) = loopback_receiver();
        let cfg = sacn::SourceConfig::new(cid(), "Varda");
        let mut tx =
            SacnSender::new(cfg, SacnDestination::Unicast(addr), Ipv4Addr::LOCALHOST).unwrap();
        tx.terminate(9).expect("terminate");

        let mut buf = [0u8; 1024];
        for i in 0..sacn::TERMINATION_PACKET_COUNT {
            let (n, _) = rx.recv_from(&mut buf).unwrap_or_else(|e| {
                panic!("termination packet {i} missing: {e}");
            });
            assert_eq!(n, sacn::DATA_PACKET_LEN);
            assert_eq!(
                buf[112] & sacn::OPT_STREAM_TERMINATED,
                sacn::OPT_STREAM_TERMINATED,
                "packet {i} must set Stream_Terminated"
            );
            assert_eq!(&buf[113..115], &[0, 9], "universe");
            assert!(
                buf[126..126 + UNIVERSE_SIZE].iter().all(|&b| b == 0),
                "a termination frame must be dark"
            );
        }
        rx.set_read_timeout(Some(Duration::from_millis(120)))
            .unwrap();
        assert!(
            rx.recv_from(&mut buf).is_err(),
            "exactly three termination packets"
        );
    }

    #[test]
    fn sacn_sequence_wraps_through_zero() {
        let (rx, addr) = loopback_receiver();
        let cfg = sacn::SourceConfig::new(cid(), "Varda");
        let mut tx =
            SacnSender::new(cfg, SacnDestination::Unicast(addr), Ipv4Addr::LOCALHOST).unwrap();
        let data = [0u8; UNIVERSE_SIZE];
        let mut buf = [0u8; 1024];
        let mut seen_zero = false;
        for _ in 0..258 {
            tx.send(1, &data).unwrap();
            rx.recv_from(&mut buf).unwrap();
            if buf[111] == 0 {
                seen_zero = true;
            }
        }
        assert!(seen_zero, "0 is a legal E1.31 sequence value");
    }

    #[test]
    fn multicast_destination_targets_the_specified_group() {
        let cfg = sacn::SourceConfig::new(cid(), "Varda");
        let tx =
            SacnSender::new(cfg, SacnDestination::Multicast, Ipv4Addr::LOCALHOST).expect("bind");
        assert_eq!(
            tx.address(1),
            SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::new(239, 255, 0, 1),
                sacn::SACN_PORT
            ))
        );
    }

    #[test]
    fn labels_identify_the_destination() {
        let (_rx, addr) = loopback_receiver();
        let art = ArtNetSender::new(addr, false).unwrap();
        assert!(art.label().starts_with("art-net"));
        let cfg = sacn::SourceConfig::new(cid(), "Varda");
        let mc = SacnSender::new(cfg, SacnDestination::Multicast, Ipv4Addr::LOCALHOST).unwrap();
        assert_eq!(mc.label(), "sacn multicast");
    }
}
