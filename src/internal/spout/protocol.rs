//! Spout's shared-memory wire format, reimplemented rather than linked.
//!
//! Spout is a C++ library with no stable ABI, and its DX12 support would add a
//! C++ build step to the Windows target. Its actual protocol is small: a table of
//! sender names and one 280-byte record per sender. Reimplementing it keeps the
//! Windows build pure Rust.
//!
//! The cost of reimplementing rather than linking is compatibility with every
//! existing Spout application, and this module is where that risk lives. So it is
//! deliberately pure: no Windows API, no D3D, no `unsafe`. It compiles and is
//! tested on every platform, including the Mac this was developed on, which is the
//! only part of the Spout work that can be pinned down without Windows.
//!
//! See /spec/spout-output.md § Wire protocol.

/// Bytes in one `SharedTextureInfo` record, fixed by Spout.
pub const SHARED_TEXTURE_INFO_BYTES: usize = 280;

/// Bytes in one sender-name slot in the name table, fixed by Spout.
pub const SENDER_NAME_BYTES: usize = 256;

/// Sender slots when the registry says nothing.
///
/// Spout reads `HKCU\Software\Leading Edge\Spout\MaxSenders` and falls back to
/// this. Varda reads the same key on Windows so that a user who raised the limit
/// for another application sees the same senders here.
pub const DEFAULT_MAX_SENDERS: usize = 10;

/// A sender's published texture, as it appears in shared memory.
///
/// Field order and widths are Spout's, not ours. `share_handle` is a `u32` even on
/// 64-bit Windows because it carries a **legacy** DXGI shared handle, which is not
/// pointer sized. That single field is why the Windows backend needs a `D3D11On12`
/// bridge: `ID3D12Device::OpenSharedHandle` takes NT handles only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedTextureInfo {
    pub share_handle: u32,
    pub width: u32,
    pub height: u32,
    /// A `DXGI_FORMAT` value. See [`DxgiFormat`].
    pub format: u32,
    pub usage: u32,
    pub partner_id: u32,
}

impl SharedTextureInfo {
    /// Encode into Spout's exact 280-byte record.
    ///
    /// The 256-byte description is written as zeroes. Spout uses it for
    /// development notes and no receiver depends on it.
    #[must_use]
    pub fn encode(&self) -> [u8; SHARED_TEXTURE_INFO_BYTES] {
        let mut out = [0u8; SHARED_TEXTURE_INFO_BYTES];
        out[0..4].copy_from_slice(&self.share_handle.to_le_bytes());
        out[4..8].copy_from_slice(&self.width.to_le_bytes());
        out[8..12].copy_from_slice(&self.height.to_le_bytes());
        out[12..16].copy_from_slice(&self.format.to_le_bytes());
        out[16..20].copy_from_slice(&self.usage.to_le_bytes());
        // description[256] stays zeroed: bytes 20..276
        out[276..280].copy_from_slice(&self.partner_id.to_le_bytes());
        out
    }

    /// Decode a record written by any Spout sender.
    ///
    /// Returns `None` for a short buffer rather than panicking: this reads memory
    /// another process owns, and a truncated or torn map is a runtime condition
    /// rather than a bug in Varda.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < SHARED_TEXTURE_INFO_BYTES {
            return None;
        }
        let word = |at: usize| -> u32 {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(&bytes[at..at + 4]);
            u32::from_le_bytes(buf)
        };
        Some(Self {
            share_handle: word(0),
            width: word(4),
            height: word(8),
            format: word(12),
            usage: word(16),
            partner_id: word(276),
        })
    }

    /// Whether this record describes a texture worth opening.
    ///
    /// A sender that has registered its name but not yet published leaves zeroes
    /// behind, and a zero handle or a zero dimension is that state rather than a
    /// texture.
    #[must_use]
    pub fn is_publishable(&self) -> bool {
        self.share_handle != 0 && self.width > 0 && self.height > 0
    }
}

/// The `DXGI_FORMAT` values Spout senders use.
///
/// Spout's own documentation lists these as the formats `SendTexture` accepts.
/// Numeric values are DXGI's and are part of the wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxgiFormat {
    /// `DXGI_FORMAT_B8G8R8A8_UNORM`, Spout's default and the interoperable choice.
    Bgra8Unorm,
    /// `DXGI_FORMAT_R8G8B8A8_UNORM`.
    Rgba8Unorm,
    /// `DXGI_FORMAT_R8G8B8A8_UNORM_SRGB`.
    Rgba8UnormSrgb,
    /// `DXGI_FORMAT_R10G10B10A2_UNORM`.
    Rgb10a2Unorm,
    /// `DXGI_FORMAT_R16G16B16A16_FLOAT`.
    Rgba16Float,
}

impl DxgiFormat {
    /// The `DXGI_FORMAT` number written into shared memory.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Rgba16Float => 10,
            Self::Rgb10a2Unorm => 24,
            Self::Rgba8Unorm => 28,
            Self::Rgba8UnormSrgb => 29,
            Self::Bgra8Unorm => 87,
        }
    }

    /// Interpret a sender's format field.
    ///
    /// `None` for anything Varda cannot sample, including zero, which older
    /// senders write to mean "the default". Callers treat `None` as
    /// [`Self::Bgra8Unorm`] only where Spout itself documents that fallback.
    #[must_use]
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            10 => Some(Self::Rgba16Float),
            24 => Some(Self::Rgb10a2Unorm),
            28 => Some(Self::Rgba8Unorm),
            29 => Some(Self::Rgba8UnormSrgb),
            87 => Some(Self::Bgra8Unorm),
            _ => None,
        }
    }

    /// What a sender's format field means when it is unset.
    ///
    /// Spout documents `B8G8R8A8_UNORM` as the default, and senders predating the
    /// format field leave a zero. Reading that as BGRA8 is what every other
    /// receiver does.
    #[must_use]
    pub const fn from_u32_or_default(value: u32) -> Option<Self> {
        if value == 0 {
            Some(Self::Bgra8Unorm)
        } else {
            Self::from_u32(value)
        }
    }

    /// The wgpu format to sample this as.
    ///
    /// Spout carries display-encoded pixels with no transfer signalling, so an
    /// `_UNORM` surface is sampled as sRGB: that is the convention every Spout
    /// application follows, and the same reading Varda applies to Syphon's
    /// `BGRA8Unorm` surfaces (/spec/syphon-zero-copy.md).
    #[must_use]
    pub const fn wgpu_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Bgra8Unorm => wgpu::TextureFormat::Bgra8UnormSrgb,
            Self::Rgba8Unorm | Self::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            Self::Rgb10a2Unorm => wgpu::TextureFormat::Rgb10a2Unorm,
            Self::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        }
    }
}

/// Trim and bound a sender name to what Spout's name table can hold.
///
/// Slots are 256 bytes including the terminator, so 255 bytes of name. Truncation
/// is on a character boundary: a name cut mid-codepoint would be written into
/// shared memory that other applications read as text.
#[must_use]
pub fn clamp_sender_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.len() < SENDER_NAME_BYTES {
        return trimmed.to_string();
    }
    let mut end = SENDER_NAME_BYTES - 1;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed[..end].to_string()
}

/// Read the sender-name table.
///
/// Slots are fixed width and NUL padded, and a released sender leaves its slot
/// zeroed rather than compacting the table, so empty slots are skipped instead of
/// ending the scan. Names other processes wrote are not guaranteed to be UTF-8,
/// so invalid slots are dropped rather than replaced with a lossy rendering that
/// would not match back when the user selected it.
#[must_use]
pub fn decode_sender_names(bytes: &[u8], max_senders: usize) -> Vec<String> {
    let mut names = Vec::new();
    for slot in 0..max_senders {
        let start = slot * SENDER_NAME_BYTES;
        let Some(raw) = bytes.get(start..start + SENDER_NAME_BYTES) else {
            break;
        };
        let end = raw
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(SENDER_NAME_BYTES);
        if end == 0 {
            continue;
        }
        if let Ok(name) = std::str::from_utf8(&raw[..end]) {
            names.push(name.to_string());
        }
    }
    names
}

/// Write the sender-name table.
///
/// Returns `None` when the names do not fit, which is Spout's own behaviour:
/// registering past `MaxSenders` fails rather than evicting somebody else's
/// sender.
#[must_use]
pub fn encode_sender_names(names: &[String], max_senders: usize) -> Option<Vec<u8>> {
    if names.len() > max_senders {
        return None;
    }
    let mut out = vec![0u8; max_senders * SENDER_NAME_BYTES];
    for (slot, name) in names.iter().enumerate() {
        let clamped = clamp_sender_name(name);
        let bytes = clamped.as_bytes();
        let start = slot * SENDER_NAME_BYTES;
        out[start..start + bytes.len()].copy_from_slice(bytes);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These pin Varda's reading of a format another project owns, so they are
    // written against the numbers and offsets in Spout's own headers rather than
    // against this implementation. See /spec/spout-output.md § Wire protocol.

    #[test]
    fn a_record_is_exactly_the_size_spout_documents() {
        // "280 bytes total" is a comment in SpoutSenderNames.h and a contract:
        // Spout maps a fixed-size view and reads fields at fixed offsets.
        assert_eq!(SHARED_TEXTURE_INFO_BYTES, 280);
        assert_eq!(4 * 5 + 256 + 4, SHARED_TEXTURE_INFO_BYTES);
    }

    #[test]
    fn fields_land_on_spouts_offsets() {
        // Distinct values so a transposed pair cannot pass.
        let info = SharedTextureInfo {
            share_handle: 0x1111_1111,
            width: 0x2222_2222,
            height: 0x3333_3333,
            format: 0x4444_4444,
            usage: 0x5555_5555,
            partner_id: 0x6666_6666,
        };
        let bytes = info.encode();
        let at =
            |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        assert_eq!(at(0), 0x1111_1111, "shareHandle");
        assert_eq!(at(4), 0x2222_2222, "width");
        assert_eq!(at(8), 0x3333_3333, "height");
        assert_eq!(at(12), 0x4444_4444, "format");
        assert_eq!(at(16), 0x5555_5555, "usage");
        assert_eq!(at(276), 0x6666_6666, "partnerId, after description[256]");
        assert!(
            bytes[20..276].iter().all(|b| *b == 0),
            "the description field must be written as zeroes, not left as stack noise"
        );
    }

    #[test]
    fn a_record_survives_a_round_trip() {
        let info = SharedTextureInfo {
            share_handle: 0x0004_2A1C,
            width: 1920,
            height: 1080,
            format: DxgiFormat::Bgra8Unorm.as_u32(),
            usage: 0,
            partner_id: 0,
        };
        assert_eq!(SharedTextureInfo::decode(&info.encode()), Some(info));
    }

    #[test]
    fn a_short_map_is_refused_rather_than_read() {
        // This memory belongs to another process. A truncated view is a runtime
        // condition, not a reason to panic inside the render loop.
        assert_eq!(SharedTextureInfo::decode(&[0u8; 279]), None);
        assert_eq!(SharedTextureInfo::decode(&[]), None);
    }

    #[test]
    fn a_registered_but_unpublished_sender_is_not_openable() {
        // Spout registers the name first and publishes the texture later, so this
        // state is normal and must not be read as a texture at handle zero.
        let empty = SharedTextureInfo {
            share_handle: 0,
            width: 0,
            height: 0,
            format: 0,
            usage: 0,
            partner_id: 0,
        };
        assert!(!empty.is_publishable());
        assert!(
            !SharedTextureInfo {
                share_handle: 42,
                width: 0,
                height: 1080,
                ..empty
            }
            .is_publishable(),
            "a zero dimension is not a texture"
        );
        assert!(
            SharedTextureInfo {
                share_handle: 42,
                width: 1920,
                height: 1080,
                ..empty
            }
            .is_publishable()
        );
    }

    #[test]
    fn dxgi_numbers_match_the_values_spout_publishes() {
        // From spoutDX::SendTexture's documented format list. These are DXGI's
        // numbers and are part of the wire contract, not ours to choose.
        assert_eq!(DxgiFormat::Rgba16Float.as_u32(), 10);
        assert_eq!(DxgiFormat::Rgb10a2Unorm.as_u32(), 24);
        assert_eq!(DxgiFormat::Rgba8Unorm.as_u32(), 28);
        assert_eq!(DxgiFormat::Rgba8UnormSrgb.as_u32(), 29);
        assert_eq!(DxgiFormat::Bgra8Unorm.as_u32(), 87);
        for f in [
            DxgiFormat::Rgba16Float,
            DxgiFormat::Rgb10a2Unorm,
            DxgiFormat::Rgba8Unorm,
            DxgiFormat::Rgba8UnormSrgb,
            DxgiFormat::Bgra8Unorm,
        ] {
            assert_eq!(DxgiFormat::from_u32(f.as_u32()), Some(f));
        }
    }

    #[test]
    fn an_unset_format_reads_as_spouts_documented_default() {
        // Senders predating the format field leave a zero, and every other
        // receiver reads that as BGRA8.
        assert_eq!(
            DxgiFormat::from_u32_or_default(0),
            Some(DxgiFormat::Bgra8Unorm)
        );
        assert_eq!(
            DxgiFormat::from_u32(0),
            None,
            "but zero is not itself a format"
        );
    }

    #[test]
    fn an_unknown_format_is_refused_rather_than_guessed() {
        // Sampling an unknown layout produces a plausible wrong picture, which is
        // worse than declining the source.
        assert_eq!(DxgiFormat::from_u32(2), None);
        assert_eq!(DxgiFormat::from_u32(u32::MAX), None);
    }

    #[test]
    fn eight_bit_formats_are_sampled_as_srgb() {
        // Spout carries display-encoded pixels with no transfer signalling. The
        // same reading Varda applies to Syphon, and getting it wrong there cost a
        // release (see /spec/syphon-zero-copy.md).
        assert_eq!(
            DxgiFormat::Bgra8Unorm.wgpu_format(),
            wgpu::TextureFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            DxgiFormat::Rgba8Unorm.wgpu_format(),
            wgpu::TextureFormat::Rgba8UnormSrgb
        );
    }

    #[test]
    fn names_round_trip_through_the_fixed_slot_table() {
        let names = vec!["Varda Main".to_string(), "Resolume Arena".to_string()];
        let table = encode_sender_names(&names, DEFAULT_MAX_SENDERS).expect("fits");
        assert_eq!(table.len(), DEFAULT_MAX_SENDERS * SENDER_NAME_BYTES);
        assert_eq!(decode_sender_names(&table, DEFAULT_MAX_SENDERS), names);
    }

    #[test]
    fn a_released_sender_leaves_a_hole_rather_than_ending_the_table() {
        // Spout zeroes a slot on release without compacting. Stopping at the first
        // empty slot would hide every sender registered after it.
        let mut table = encode_sender_names(
            &[
                "First".to_string(),
                "Second".to_string(),
                "Third".to_string(),
            ],
            DEFAULT_MAX_SENDERS,
        )
        .expect("fits");
        table[SENDER_NAME_BYTES..2 * SENDER_NAME_BYTES].fill(0);
        assert_eq!(
            decode_sender_names(&table, DEFAULT_MAX_SENDERS),
            vec!["First".to_string(), "Third".to_string()]
        );
    }

    #[test]
    fn a_name_written_by_another_process_need_not_be_utf8() {
        // Dropping the slot beats a lossy rendering: a name with a replacement
        // character would not match back when the user selected it.
        let mut table = encode_sender_names(&["Good".to_string()], 2).expect("fits");
        table[SENDER_NAME_BYTES] = 0xFF;
        table[SENDER_NAME_BYTES + 1] = 0xFE;
        assert_eq!(
            decode_sender_names(&table, 2),
            vec!["Good".to_string()],
            "invalid slots are skipped, valid ones survive"
        );
    }

    #[test]
    fn the_table_refuses_to_evict_another_applications_sender() {
        // Spout fails registration past MaxSenders rather than overwriting.
        let names: Vec<String> = (0..3).map(|i| format!("S{i}")).collect();
        assert!(encode_sender_names(&names, 2).is_none());
        assert!(encode_sender_names(&names, 3).is_some());
    }

    #[test]
    fn a_long_name_is_cut_on_a_character_boundary() {
        // The result is written into memory other applications read as text.
        let long = "é".repeat(400);
        let clamped = clamp_sender_name(&long);
        assert!(
            clamped.len() < SENDER_NAME_BYTES,
            "must leave room for the NUL"
        );
        assert!(
            clamped.chars().all(|c| c == 'é'),
            "truncation must not split a codepoint: {clamped:?}"
        );
        // And it still fits its slot once encoded.
        let table = encode_sender_names(&[long], 1).expect("fits after clamping");
        assert_eq!(decode_sender_names(&table, 1), vec![clamped]);
    }

    #[test]
    fn names_are_trimmed_so_a_stray_space_is_not_a_different_sender() {
        assert_eq!(clamp_sender_name("  Varda Main  "), "Varda Main");
    }
}
