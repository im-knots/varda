//! NDI C FFI type definitions.
//!
//! These `#[repr(C)]` structs mirror the NDI SDK headers exactly.
//! Only the subset used by Varda is declared here.

use std::os::raw::{c_char, c_int, c_float};

/// Opaque handle returned by NDIlib_find_create_v2.
pub type NDIlib_find_instance_t = *mut std::ffi::c_void;

/// Opaque handle returned by NDIlib_recv_create_v3.
pub type NDIlib_recv_instance_t = *mut std::ffi::c_void;

/// Opaque handle returned by NDIlib_send_create.
pub type NDIlib_send_instance_t = *mut std::ffi::c_void;

/// NDI source descriptor (returned by find).
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct NDIlib_source_t {
    /// Source name as a null-terminated UTF-8 C string.
    pub p_ndi_name: *const c_char,
    /// URL/address (can be null for default).
    pub p_url_address: *const c_char,
}

/// Settings for NDIlib_find_create_v2.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct NDIlib_find_create_t {
    /// Show local sources? (true = yes)
    pub show_local_sources: bool,
    /// Comma-separated list of groups to search (null = default).
    pub p_groups: *const c_char,
    /// Comma-separated list of extra IPs to search (null = none).
    pub p_extra_ips: *const c_char,
}

impl Default for NDIlib_find_create_t {
    fn default() -> Self {
        Self {
            show_local_sources: true,
            p_groups: std::ptr::null(),
            p_extra_ips: std::ptr::null(),
        }
    }
}

/// NDI FourCC pixel formats.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct NDIlib_FourCC_video_type_e(pub u32);

impl NDIlib_FourCC_video_type_e {
    /// UYVY 4:2:2 (most common NDI format)
    pub const UYVY: Self = Self(u32::from_le_bytes(*b"UYVY"));
    /// RGBA 8-bit
    pub const RGBA: Self = Self(u32::from_le_bytes(*b"RGBA"));
    /// BGRA 8-bit
    pub const BGRA: Self = Self(u32::from_le_bytes(*b"BGRA"));
    /// BGRX 8-bit (like BGRA but alpha is undefined)
    pub const BGRX: Self = Self(u32::from_le_bytes(*b"BGRX"));
}

/// NDI video frame descriptor.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct NDIlib_video_frame_v2_t {
    /// Width in pixels.
    pub xres: c_int,
    /// Height in pixels.
    pub yres: c_int,
    /// FourCC pixel format.
    pub FourCC: NDIlib_FourCC_video_type_e,
    /// Frame rate numerator.
    pub frame_rate_N: c_int,
    /// Frame rate denominator.
    pub frame_rate_D: c_int,
    /// Aspect ratio (0 = default from resolution).
    pub picture_aspect_ratio: c_float,
    /// Progressive or interlaced.
    pub frame_format_type: c_int,
    /// Timecode (100ns units, 0 = auto).
    pub timecode: i64,
    /// Pointer to frame pixel data.
    pub p_data: *mut u8,
    /// Line stride in bytes.
    pub line_stride_in_bytes: c_int,
    /// Metadata XML string (null = none).
    pub p_metadata: *const c_char,
    /// Timestamp (100ns units, 0 = auto).
    pub timestamp: i64,
}

/// Frame type returned by NDIlib_recv_capture_v3.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct NDIlib_frame_type_e(pub c_int);

impl NDIlib_frame_type_e {
    pub const NONE: Self = Self(0);
    pub const VIDEO: Self = Self(1);
    pub const AUDIO: Self = Self(2);
    pub const METADATA: Self = Self(3);
    pub const ERROR: Self = Self(4);
    pub const STATUS_CHANGE: Self = Self(100);
}

/// Settings for NDIlib_recv_create_v3.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct NDIlib_recv_create_v3_t {
    /// Source to connect to.
    pub source_to_connect_to: NDIlib_source_t,
    /// Color format preference (0 = BGRA, 100 = fastest).
    pub color_format: c_int,
    /// Bandwidth: 0 = metadata only, 10 = audio only, 100 = highest.
    pub bandwidth: c_int,
    /// Allow video fields? (false = always progressive).
    pub allow_video_fields: bool,
    /// Receiver name (displayed on sender side).
    pub p_ndi_recv_name: *const c_char,
}

/// Settings for NDIlib_send_create_t.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct NDIlib_send_create_t {
    /// Sender name visible on the network.
    pub p_ndi_name: *const c_char,
    /// Groups to join (null = default).
    pub p_groups: *const c_char,
    /// Whether to clock video (pace to frame rate).
    pub clock_video: bool,
    /// Whether to clock audio.
    pub clock_audio: bool,
}
