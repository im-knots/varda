//! sACN (ANSI E1.31) data packet encoding.
//!
//! Hand-rolled for the same reason as [`super::artnet`]: the byte layout is the thing under
//! test, and /spec/dmx-output.md § Testing requires vectors derived from the published spec.
//!
//! Reference: ANSI E1.31-2016, <https://tsp.esta.org/tsp/documents/docs/E1-31-2016.pdf>
//!
//! An E1.31 data packet is three nested PDUs, each opening with a 16-bit flags-and-length field:
//!
//! ```text
//!   0            16                  38                 115              638
//!   | Root Layer | Framing Layer     | DMP Layer         | 513 data bytes |
//! ```

use super::universe::{UNIVERSE_SIZE, UniverseId};

/// ACN packet identifier: "ASC-E1.17" padded to 12 bytes.
pub const ACN_PID: &[u8; 12] = b"ASC-E1.17\0\0\0";

/// Standard sACN UDP port.
pub const SACN_PORT: u16 = 5568;

/// Default per-universe priority. E1.31 range is 0..=200; 100 is the specified default.
pub const DEFAULT_PRIORITY: u8 = 100;

/// Source name field width in bytes, NUL padded.
pub const SOURCE_NAME_LEN: usize = 64;

/// Total data-packet length carrying a full 512-slot universe.
pub const DATA_PACKET_LEN: usize = 638;

const VECTOR_ROOT_E131_DATA: u32 = 0x0000_0004;
const VECTOR_E131_DATA_PACKET: u32 = 0x0000_0002;
const VECTOR_DMP_SET_PROPERTY: u8 = 0x02;

/// Options field bit 6. E1.31 § 6.2.6.
///
/// A receiver that merely stops hearing a source holds its last value until the 2.5 second
/// network-data-loss timeout expires. Setting this releases it immediately, which is why
/// shutdown sends it rather than just going silent. See /spec/dmx-output.md § C6.
pub const OPT_STREAM_TERMINATED: u8 = 0x40;
/// Options field bit 7: preview data, which receivers must not act on.
pub const OPT_PREVIEW_DATA: u8 = 0x80;

/// How many terminated packets E1.31 requires a source to send when it stops a universe.
pub const TERMINATION_PACKET_COUNT: usize = 3;

/// PDU flags nibble: 0x7 in the high 4 bits, length in the low 12.
fn flags_and_length(pdu_len: usize) -> [u8; 2] {
    debug_assert!(pdu_len <= 0x0FFF);
    #[allow(clippy::cast_possible_truncation)]
    let v = 0x7000u16 | (pdu_len as u16 & 0x0FFF);
    v.to_be_bytes()
}

/// The IPv4 multicast group for a universe: `239.255.<high>.<low>`. E1.31 Appendix A.
#[must_use]
pub fn multicast_addr(universe: UniverseId) -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::new(239, 255, (universe >> 8) as u8, (universe & 0xFF) as u8)
}

/// Everything about a source that does not change frame to frame.
#[derive(Debug, Clone)]
pub struct SourceConfig {
    /// Component Identifier: a stable 16-byte UUID for this sender.
    ///
    /// Must persist across restarts. A source that invents a new CID each launch appears to
    /// receivers as a second, competing source, and merging consoles will treat the two as
    /// rivals for the same universe.
    pub cid: [u8; 16],
    pub source_name: String,
    pub priority: u8,
}

impl SourceConfig {
    #[must_use]
    pub fn new(cid: [u8; 16], source_name: impl Into<String>) -> Self {
        Self {
            cid,
            source_name: source_name.into(),
            priority: DEFAULT_PRIORITY,
        }
    }
}

/// Encode one E1.31 data packet.
///
/// `terminated` sets the `Stream_Terminated` option bit. A terminated packet still carries its
/// data payload; the spec defines the flag as the signal, not an empty frame, so the driver
/// sends zeroes alongside it.
#[must_use]
pub fn encode_data_packet(
    cfg: &SourceConfig,
    universe: UniverseId,
    sequence: u8,
    data: &[u8; UNIVERSE_SIZE],
    terminated: bool,
) -> Vec<u8> {
    let mut p = Vec::with_capacity(DATA_PACKET_LEN);

    // ── Root layer ──────────────────────────────────────────────────────────
    p.extend_from_slice(&0x0010u16.to_be_bytes()); // preamble size
    p.extend_from_slice(&0x0000u16.to_be_bytes()); // post-amble size
    p.extend_from_slice(ACN_PID);
    p.extend_from_slice(&flags_and_length(DATA_PACKET_LEN - 16));
    p.extend_from_slice(&VECTOR_ROOT_E131_DATA.to_be_bytes());
    p.extend_from_slice(&cfg.cid);

    // ── Framing layer ───────────────────────────────────────────────────────
    p.extend_from_slice(&flags_and_length(DATA_PACKET_LEN - 38));
    p.extend_from_slice(&VECTOR_E131_DATA_PACKET.to_be_bytes());
    let mut name = [0u8; SOURCE_NAME_LEN];
    let bytes = cfg.source_name.as_bytes();
    // Truncate on a UTF-8 boundary and always leave room for the NUL terminator, so a long or
    // multi-byte name cannot emit a half character or overrun the fixed field.
    let mut take = bytes.len().min(SOURCE_NAME_LEN - 1);
    while take > 0 && !cfg.source_name.is_char_boundary(take) {
        take -= 1;
    }
    name[..take].copy_from_slice(&bytes[..take]);
    p.extend_from_slice(&name);
    p.push(cfg.priority.min(200));
    p.extend_from_slice(&0u16.to_be_bytes()); // synchronization address: unused
    p.push(sequence);
    p.push(if terminated { OPT_STREAM_TERMINATED } else { 0 });
    p.extend_from_slice(&universe.to_be_bytes());

    // ── DMP layer ───────────────────────────────────────────────────────────
    p.extend_from_slice(&flags_and_length(DATA_PACKET_LEN - 115));
    p.push(VECTOR_DMP_SET_PROPERTY);
    p.push(0xa1); // address type and data type
    p.extend_from_slice(&0x0000u16.to_be_bytes()); // first property address
    p.extend_from_slice(&0x0001u16.to_be_bytes()); // address increment
    #[allow(clippy::cast_possible_truncation)]
    let count = (UNIVERSE_SIZE + 1) as u16; // start code + 512 slots
    p.extend_from_slice(&count.to_be_bytes());
    p.push(0x00); // DMX512 null start code
    p.extend_from_slice(data);

    debug_assert_eq!(p.len(), DATA_PACKET_LEN);
    p
}

/// Advance an E1.31 sequence counter. Unlike Art-Net, 0 is a legal value here.
#[must_use]
pub fn next_sequence(current: u8) -> u8 {
    current.wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SourceConfig {
        SourceConfig::new([0xAB; 16], "Varda")
    }
    fn zeros() -> [u8; UNIVERSE_SIZE] {
        [0u8; UNIVERSE_SIZE]
    }

    #[test]
    fn packet_length_matches_the_spec() {
        assert_eq!(encode_data_packet(&cfg(), 1, 0, &zeros(), false).len(), 638);
    }

    #[test]
    fn root_layer_is_byte_exact() {
        let p = encode_data_packet(&cfg(), 1, 0, &zeros(), false);
        assert_eq!(&p[0..2], &[0x00, 0x10], "preamble size");
        assert_eq!(&p[2..4], &[0x00, 0x00], "post-amble size");
        assert_eq!(&p[4..16], b"ASC-E1.17\0\0\0");
        assert_eq!(&p[16..18], &[0x72, 0x6E], "root flags+len = 0x7000|622");
        assert_eq!(&p[18..22], &[0, 0, 0, 4], "VECTOR_ROOT_E131_DATA");
        assert_eq!(&p[22..38], &[0xAB; 16], "CID");
    }

    #[test]
    fn framing_layer_is_byte_exact() {
        let p = encode_data_packet(&cfg(), 0x1234, 42, &zeros(), false);
        assert_eq!(&p[38..40], &[0x72, 0x58], "framing flags+len = 0x7000|600");
        assert_eq!(&p[40..44], &[0, 0, 0, 2], "VECTOR_E131_DATA_PACKET");
        assert_eq!(&p[44..49], b"Varda");
        assert!(p[49..108].iter().all(|&b| b == 0), "name is NUL padded");
        assert_eq!(p[108], 100, "default priority");
        assert_eq!(&p[109..111], &[0, 0], "sync address unused");
        assert_eq!(p[111], 42, "sequence");
        assert_eq!(p[112], 0, "options");
        assert_eq!(&p[113..115], &[0x12, 0x34], "universe big-endian");
    }

    #[test]
    fn dmp_layer_is_byte_exact() {
        let p = encode_data_packet(&cfg(), 1, 0, &zeros(), false);
        assert_eq!(&p[115..117], &[0x72, 0x0B], "DMP flags+len = 0x7000|523");
        assert_eq!(p[117], 0x02, "VECTOR_DMP_SET_PROPERTY");
        assert_eq!(p[118], 0xa1, "address and data type");
        assert_eq!(&p[119..121], &[0x00, 0x00], "first property address");
        assert_eq!(&p[121..123], &[0x00, 0x01], "address increment");
        assert_eq!(&p[123..125], &[0x02, 0x01], "513 property values");
        assert_eq!(p[125], 0x00, "DMX null start code");
    }

    #[test]
    fn every_pdu_carries_the_seven_flag_nibble() {
        let p = encode_data_packet(&cfg(), 1, 0, &zeros(), false);
        for off in [16usize, 38, 115] {
            assert_eq!(p[off] >> 4, 0x7, "PDU at {off} must carry flags 0x7");
        }
    }

    #[test]
    fn payload_follows_the_start_code_verbatim() {
        let mut d = zeros();
        d[0] = 1;
        d[511] = 255;
        let p = encode_data_packet(&cfg(), 1, 0, &d, false);
        assert_eq!(p[125], 0x00, "start code precedes the data");
        assert_eq!(&p[126..], &d[..]);
    }

    /// Contract C6. Without this bit a receiver holds its last value for 2.5 seconds after the
    /// source goes quiet, which on a shutdown means the rig stays lit after Varda exits.
    #[test]
    fn stream_terminated_sets_options_bit_six() {
        let p = encode_data_packet(&cfg(), 1, 0, &zeros(), true);
        assert_eq!(p[112], 0x40);
        assert_eq!(p[112] & OPT_STREAM_TERMINATED, OPT_STREAM_TERMINATED);
        assert_eq!(p[112] & OPT_PREVIEW_DATA, 0, "preview must stay clear");
    }

    #[test]
    fn terminated_packet_is_otherwise_a_normal_packet() {
        let plain = encode_data_packet(&cfg(), 9, 5, &zeros(), false);
        let term = encode_data_packet(&cfg(), 9, 5, &zeros(), true);
        assert_eq!(plain.len(), term.len());
        for i in 0..plain.len() {
            if i != 112 {
                assert_eq!(plain[i], term[i], "byte {i} must be unchanged");
            }
        }
    }

    #[test]
    fn multicast_address_follows_appendix_a() {
        assert_eq!(
            multicast_addr(1),
            std::net::Ipv4Addr::new(239, 255, 0, 1),
            "universe 1"
        );
        assert_eq!(
            multicast_addr(0x1234),
            std::net::Ipv4Addr::new(239, 255, 0x12, 0x34)
        );
        assert_eq!(
            multicast_addr(63_999),
            std::net::Ipv4Addr::new(239, 255, 0xF9, 0xFF)
        );
    }

    #[test]
    fn long_source_name_is_truncated_and_still_nul_terminated() {
        let mut c = cfg();
        c.source_name = "x".repeat(200);
        let p = encode_data_packet(&c, 1, 0, &zeros(), false);
        assert_eq!(p.len(), DATA_PACKET_LEN);
        assert_eq!(p[44 + SOURCE_NAME_LEN - 1], 0, "field stays NUL terminated");
    }

    #[test]
    fn multibyte_source_name_never_splits_a_character() {
        let mut c = cfg();
        // 63 bytes of 'a' then a 3-byte character would straddle the 63-byte limit.
        c.source_name = format!("{}★★★", "a".repeat(62));
        let p = encode_data_packet(&c, 1, 0, &zeros(), false);
        let name = &p[44..44 + SOURCE_NAME_LEN];
        let end = name.iter().position(|&b| b == 0).unwrap();
        std::str::from_utf8(&name[..end]).expect("truncation must leave valid UTF-8");
    }

    #[test]
    fn priority_is_clamped_to_the_legal_range() {
        let mut c = cfg();
        c.priority = 255;
        let p = encode_data_packet(&c, 1, 0, &zeros(), false);
        assert_eq!(p[108], 200, "E1.31 priority tops out at 200");
    }

    #[test]
    fn sequence_wraps_through_zero() {
        assert_eq!(next_sequence(255), 0, "0 is legal in E1.31, unlike Art-Net");
    }
}
