//! Sampling video into lighting: the edge that joins Varda's two graphs.
//!
//! A lighting deck whose source is video is what every other tool ships as a separate subsystem
//! — Resolume's Lumiverse, `MadMapper`'s `MadLight`, Millumin's DMX layer. Here it is one more list
//! on a Look, merging at the same level, under the same master, through the same crossfader.
//!
//! The frames are deliberately tiny. A rig is tens of fixtures, not millions of pixels, and each
//! fixture wants the *average* of the area it covers rather than one texel of it — so a 64×64
//! downsample is both cheaper than a full readback and a better answer than sampling full
//! resolution would be.
//! See /spec/lighting-routing.md § Sampled.

use std::collections::HashMap;

/// Edge length of the downsampled frame each sampled source is read back at.
///
/// Large enough that neighbouring fixtures on a truss read different pixels, small enough that
/// the readback is a rounding error: 64×64 RGBA is 16 KiB per sampled source per frame.
pub const SAMPLE_EDGE: u32 = 64;

/// One source's video, downsampled and read back to the CPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampledFrame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl SampledFrame {
    /// A frame of the given size with nothing in it.
    #[must_use]
    pub fn blank(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// Linear RGB at a normalized point, clamped to the frame.
    ///
    /// Nearest-texel rather than bilinear: the frame is already an area average of the source,
    /// so filtering it again would only blur away the spatial detail that makes a pixel map read
    /// as a map. Returns `None` for an empty frame so a sampled role stays *inert* rather than
    /// going dark before the first readback lands — the same rule a dangling palette follows.
    #[must_use]
    pub fn sample(&self, point: [f32; 2]) -> Option<[f32; 3]> {
        if self.width == 0 || self.height == 0 || self.rgba.is_empty() {
            return None;
        }
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let x = ((point[0].clamp(0.0, 1.0) * (self.width - 1) as f32).round() as u32)
            .min(self.width - 1);
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let y = ((point[1].clamp(0.0, 1.0) * (self.height - 1) as f32).round() as u32)
            .min(self.height - 1);
        let i = ((y * self.width + x) as usize) * 4;
        let px = self.rgba.get(i..i + 3)?;
        Some([
            f32::from(px[0]) / 255.0,
            f32::from(px[1]) / 255.0,
            f32::from(px[2]) / 255.0,
        ])
    }
}

/// Every sampled source's latest frame, keyed the way [`super::look::SampleSource`] names them.
///
/// Channels are keyed by UUID; the program has one entry under [`PROGRAM_KEY`], which is not a
/// legal channel UUID, so the two cannot collide.
pub type SampledFrames = HashMap<String, SampledFrame>;

/// Key under which the program's frame is stored.
///
/// Not a legal short channel UUID — those are eight hex characters — so it cannot be shadowed by
/// a real channel.
pub const PROGRAM_KEY: &str = "__program";

/// The component of a sampled color a role reads.
///
/// A fixture is not a pixel: it has a dimmer, and maybe red, green and blue, and maybe only
/// white. Mapping is therefore per role rather than "write the RGB triple", which is what lets a
/// pixel map drive a rig of mixed fixture types without the performer patching around it.
#[must_use]
pub fn component(role: super::role::Role, rgb: [f32; 3]) -> Option<f32> {
    use super::role::Role;
    let [r, g, b] = rgb;
    Some(match role {
        Role::Red => r,
        Role::Green => g,
        Role::Blue => b,
        // Luminance, so a dimmer-only fixture in a pixel map still follows the image. Rec. 709,
        // matching the coefficients the rest of Varda's color path uses.
        Role::Dimmer | Role::White => 0.2126 * r + 0.7152 * g + 0.0722 * b,
        // The complements, so an RGBW or CMY fixture is not left dark in a map.
        Role::Cyan => 1.0 - r,
        Role::Magenta => 1.0 - g,
        Role::Yellow => 1.0 - b,
        Role::Amber => f32::midpoint(r, g),
        // Everything else — position, gobo, prism, control — has no meaning as a color sample.
        // Refusing is what keeps a pixel map from swinging a moving head at the video.
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dmx::Role;

    fn frame_of(pixels: &[[u8; 4]], width: u32) -> SampledFrame {
        SampledFrame {
            width,
            #[allow(clippy::cast_possible_truncation)]
            height: (pixels.len() as u32) / width,
            rgba: pixels.iter().flatten().copied().collect(),
        }
    }

    #[test]
    fn a_blank_frame_is_the_right_size() {
        let f = SampledFrame::blank(4, 2);
        assert_eq!(f.rgba.len(), 4 * 2 * 4);
        assert_eq!(f.sample([0.5, 0.5]), Some([0.0, 0.0, 0.0]));
    }

    /// Before the first readback lands there is no frame. A sampled role must stay inert rather
    /// than going dark, the same rule a dangling palette reference follows: a rig blacking out
    /// because a readback has not arrived yet is the worst possible failure on stage.
    #[test]
    fn an_empty_frame_is_inert_not_dark() {
        assert_eq!(SampledFrame::blank(0, 0).sample([0.5, 0.5]), None);
    }

    #[test]
    fn sampling_picks_the_texel_under_the_point() {
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let f = frame_of(&[red, green], 2);
        assert_eq!(f.sample([0.0, 0.0]), Some([1.0, 0.0, 0.0]));
        assert_eq!(f.sample([1.0, 0.0]), Some([0.0, 1.0, 0.0]));
    }

    /// A point outside the frame clamps rather than wrapping or panicking: a fixture placed past
    /// the edge of the plot reads the nearest edge, which is what an operator means by putting it
    /// there.
    #[test]
    fn a_point_outside_the_frame_clamps() {
        let f = frame_of(&[[255, 0, 0, 255], [0, 255, 0, 255]], 2);
        assert_eq!(f.sample([-5.0, -5.0]), Some([1.0, 0.0, 0.0]));
        assert_eq!(f.sample([5.0, 5.0]), Some([0.0, 1.0, 0.0]));
    }

    #[test]
    fn color_roles_read_their_own_component() {
        let rgb = [0.2, 0.4, 0.6];
        assert_eq!(component(Role::Red, rgb), Some(0.2));
        assert_eq!(component(Role::Green, rgb), Some(0.4));
        assert_eq!(component(Role::Blue, rgb), Some(0.6));
    }

    /// A dimmer-only par must still follow a pixel map, or half the rigs in the world cannot use
    /// the feature.
    #[test]
    fn a_dimmer_reads_luminance() {
        let white = component(Role::Dimmer, [1.0, 1.0, 1.0]).unwrap();
        assert!((white - 1.0).abs() < 1e-5);
        let black = component(Role::Dimmer, [0.0, 0.0, 0.0]).unwrap();
        assert!(black.abs() < 1e-5);
        // Green weighs most, per Rec. 709.
        let g = component(Role::Dimmer, [0.0, 1.0, 0.0]).unwrap();
        let r = component(Role::Dimmer, [1.0, 0.0, 0.0]).unwrap();
        assert!(g > r);
    }

    /// Position, gobo and control roles have no meaning as a color, and mapping them would aim
    /// moving heads at the video.
    #[test]
    fn non_color_roles_refuse_to_be_sampled() {
        for role in [
            Role::Pan,
            Role::Tilt,
            Role::GoboWheel,
            Role::Prism,
            Role::Reset,
        ] {
            assert_eq!(component(role, [1.0, 1.0, 1.0]), None, "{role:?}");
        }
    }

    /// The program key cannot collide with a channel: channel uuids are eight hex characters.
    #[test]
    fn the_program_key_is_not_a_channel_uuid() {
        assert!(PROGRAM_KEY.len() != 8 || !PROGRAM_KEY.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
