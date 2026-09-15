//! Art-Net 4 `ArtDmx` packet encoding.
//!
//! Hand-rolled over `std::net::UdpSocket`, following `src/internal/spout/protocol.rs`. The
//! published crate would add a dependency for roughly 100 lines of encoding while making the
//! byte-exact tests /spec/dmx-output.md § Testing demands harder to write.
//!
//! Reference: Art-Net 4 specification, <https://art-net.org.uk/downloads/art-net.pdf>

use super::universe::{UNIVERSE_SIZE, UniverseId};

/// Art-Net identifier, 8 bytes including the NUL terminator. Spec: "Art-Net\0".
pub const ARTNET_ID: &[u8; 8] = b"Art-Net\0";

/// `OpOutput` / `OpDmx`, transmitted little-endian on the wire.
pub const OP_OUTPUT: u16 = 0x5000;

/// Protocol version the spec requires in `ArtDmx`, transmitted **big-endian**.
///
/// The mixed endianness is not a mistake in this code: Art-Net sends `OpCode` low byte first
/// and `ProtVer` high byte first. Getting this backwards is the classic Art-Net bug and is why
/// [`tests::opcode_is_little_endian_but_protver_is_big_endian`] exists.
pub const PROT_VER: u16 = 14;

/// Standard Art-Net UDP port.
pub const ARTNET_PORT: u16 = 6454;

/// Total `ArtDmx` packet length for a full 512-slot universe.
pub const ARTDMX_LEN: usize = 18 + UNIVERSE_SIZE;

/// Encode one `ArtDmx` packet.
///
/// `universe` is the 15-bit Port-Address (Net, Sub-Net, and Universe combined). `sequence` is
/// the per-universe counter; 0 is reserved by the spec to mean "sequencing disabled", which the
/// driver avoids by skipping it on wrap.
///
/// Always emits the full 512 slots. Sending fewer is legal Art-Net but some nodes size their
/// output buffer from the first packet they see, so a short frame followed by a full one can
/// truncate.
#[must_use]
pub fn encode_artdmx(
    universe: UniverseId,
    sequence: u8,
    physical: u8,
    data: &[u8; UNIVERSE_SIZE],
) -> Vec<u8> {
    let mut p = Vec::with_capacity(ARTDMX_LEN);
    p.extend_from_slice(ARTNET_ID); // 0..8   ID
    p.extend_from_slice(&OP_OUTPUT.to_le_bytes()); // 8..10  OpCode, little-endian
    p.extend_from_slice(&PROT_VER.to_be_bytes()); // 10..12 ProtVer, big-endian
    p.push(sequence); // 12     Sequence
    p.push(physical); // 13     Physical (informational only)
    let addr = universe & 0x7FFF; // 15-bit Port-Address
    p.push((addr & 0x00FF) as u8); // 14     SubUni (low byte)
    p.push(((addr >> 8) & 0x007F) as u8); // 15     Net (high 7 bits)
    #[allow(clippy::cast_possible_truncation)]
    let len = UNIVERSE_SIZE as u16;
    p.extend_from_slice(&len.to_be_bytes()); // 16..18 Length, big-endian
    p.extend_from_slice(data); // 18..530 Data
    debug_assert_eq!(p.len(), ARTDMX_LEN);
    p
}

/// Advance an Art-Net sequence counter, skipping 0.
///
/// The spec reserves 0 to mean sequencing is disabled, so a counter that wraps through it would
/// tell the node to stop detecting out-of-order packets exactly once every 256 frames.
#[must_use]
pub fn next_sequence(current: u8) -> u8 {
    match current.wrapping_add(1) {
        0 => 1,
        n => n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zeros() -> [u8; UNIVERSE_SIZE] {
        [0u8; UNIVERSE_SIZE]
    }

    #[test]
    fn packet_has_the_specified_length() {
        assert_eq!(encode_artdmx(1, 1, 0, &zeros()).len(), 530);
        assert_eq!(ARTDMX_LEN, 530);
    }

    #[test]
    fn header_is_byte_exact() {
        let p = encode_artdmx(1, 7, 0, &zeros());
        assert_eq!(&p[0..8], b"Art-Net\0");
        assert_eq!(&p[8..10], &[0x00, 0x50], "OpOutput little-endian");
        assert_eq!(&p[10..12], &[0x00, 0x0E], "ProtVer 14 big-endian");
        assert_eq!(p[12], 7, "sequence");
        assert_eq!(p[13], 0, "physical");
        assert_eq!(&p[16..18], &[0x02, 0x00], "length 512 big-endian");
    }

    /// The classic Art-Net bug. `OpCode` is low byte first, `ProtVer` is high byte first, in the
    /// same header. A single `to_le_bytes` or `to_be_bytes` for both is wrong either way.
    #[test]
    fn opcode_is_little_endian_but_protver_is_big_endian() {
        let p = encode_artdmx(1, 1, 0, &zeros());
        let op = u16::from_le_bytes([p[8], p[9]]);
        let ver = u16::from_be_bytes([p[10], p[11]]);
        assert_eq!(op, OP_OUTPUT);
        assert_eq!(ver, PROT_VER);
        assert_ne!(p[8], p[9], "a palindromic encoding would hide the bug");
    }

    #[test]
    fn port_address_splits_into_subuni_and_net() {
        // Universe 1 -> SubUni 1, Net 0.
        let p = encode_artdmx(1, 1, 0, &zeros());
        assert_eq!(p[14], 1);
        assert_eq!(p[15], 0);

        // Universe 0x1234 -> SubUni 0x34, Net 0x12.
        let p = encode_artdmx(0x1234, 1, 0, &zeros());
        assert_eq!(p[14], 0x34);
        assert_eq!(p[15], 0x12);
    }

    #[test]
    fn port_address_is_masked_to_fifteen_bits() {
        // The high bit must never leak into the Net field, which is 7 bits wide.
        let p = encode_artdmx(0xFFFF, 1, 0, &zeros());
        assert_eq!(p[14], 0xFF);
        assert_eq!(p[15], 0x7F, "Net field is 7 bits");
    }

    #[test]
    fn payload_is_copied_verbatim() {
        let mut d = zeros();
        d[0] = 1;
        d[255] = 128;
        d[511] = 255;
        let p = encode_artdmx(1, 1, 0, &d);
        assert_eq!(p[18], 1);
        assert_eq!(p[18 + 255], 128);
        assert_eq!(p[18 + 511], 255);
        assert_eq!(&p[18..], &d[..]);
    }

    #[test]
    fn sequence_skips_zero_on_wrap() {
        assert_eq!(next_sequence(1), 2);
        assert_eq!(next_sequence(254), 255);
        assert_eq!(
            next_sequence(255),
            1,
            "0 means sequencing disabled and must be skipped"
        );
    }

    #[test]
    fn sequence_never_yields_zero_across_a_full_cycle() {
        let mut s = 1u8;
        for _ in 0..1000 {
            s = next_sequence(s);
            assert_ne!(s, 0);
        }
    }
}
