//! DMX lighting output over Art-Net and sACN.
//!
//! Implements /spec/dmx-output.md. The performance-mode abstraction that feeds this
//! (looks, lighting decks, merge, palettes) lives in /spec/lighting-routing.md.
//!
//! Layering, bottom up:
//!
//! ```text
//!   role      semantic channel meaning, normalized f32
//!   profile   on-disk fixture definitions normalized to FixtureProfile
//!   patch     fixtures at addresses, fail-closed validation
//!   universe  slot writes assembled into 512-byte frames
//!   artnet    ArtDmx packet encoding
//!   sacn      E1.31 data packet encoding
//! ```
//!
//! Everything here is pure and transport-agnostic apart from the encoders' own byte layout,
//! so the whole module is testable without a rig.

pub mod artnet;
pub mod config;
pub mod driver;
pub mod guard;
pub mod library;
pub mod look;
pub mod merge;
pub mod ofl;
pub mod palette;
pub mod patch;
pub mod profile;
pub mod role;
pub mod router;
pub mod runtime;
pub mod sacn;
pub mod sample;
pub mod show;
pub mod smoother;
pub mod snapshot;
pub mod spread;
pub mod transport;
pub mod universe;
pub mod values;
pub mod watchdog;

pub use config::{LightingConfig, SacnDestinationConfig, TransportConfig};
pub use driver::{DmxDriver, DriverStatus, Mailbox, RefreshTracker, SharedStatus};
pub use guard::{WhiteGuardConfig, apply as apply_white_guard};
pub use library::ProfileLibrary;
pub use look::{
    AttrValue, Group, GroupSource, Look, LookAssignment, ModulatedBinding, SampledBinding,
};
pub use merge::{LightingBlend, LightingChannel, LightingDeck, LtpTransition, merge};
pub use palette::{OverrideKey, Palette, PaletteKind, PaletteSet};
pub use patch::{Fixture, PatchMessage, PatchedFixture, ResolvedRig, Severity, SlotOwner};
pub use profile::{ChannelRange, FixtureProfile, ProfileChannel, ProfileError, ProfileMode};
pub use role::{Role, RoleGroup, SmoothingClass};
pub use router::{LightingRouteError, apply as apply_lighting_param, is_lighting_path};
pub use runtime::LightingRuntime;
pub use sample::{PROGRAM_KEY, SAMPLE_EDGE, SampledFrame, SampledFrames};
pub use show::{ChannelDecks, LightingShow};
pub use smoother::RoleSmoother;
pub use snapshot::{
    FixtureView, LightingSnapshot, PaletteView, PatchEntryView, SlotOwnerView, UniverseView,
};
pub use spread::{Spread, SpreadMode};
pub use transport::{ArtNetSender, DmxError, DmxTransport, SacnDestination, SacnSender};
pub use universe::{
    Resolution, SlotWrite, UNIVERSE_SIZE, UniverseId, UniverseSet, quantize_8, quantize_16,
};
pub use values::RoleValues;
pub use watchdog::{FixtureWatchdog, WatchdogFlag, WatchdogKind};

/// Directories searched for bundled fixture profiles, in precedence order.
///
/// Workspace first, then the bundled library, so an operator correcting a channel map for
/// tonight's rig always wins over the shipped definition without waiting for a release.
///
/// The bundled location mirrors how shaders resolve (`registry::get_bundled_shader_path`),
/// because fixtures ship the same way: loose files beside the executable, per platform.
#[must_use]
pub fn bundled_profile_dirs(workspace: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![workspace.join(".varda").join("fixtures")];
    if let Some(bundled) = bundled_fixture_path() {
        dirs.push(bundled);
    }
    // Running from a source checkout: `fixtures/` sits at the repo root beside `shaders/`.
    let repo_local = std::path::PathBuf::from("fixtures");
    if repo_local.is_dir() {
        dirs.push(repo_local);
    }
    dirs
}

/// The bundled fixture library relative to the executable, when Varda is packaged.
#[must_use]
pub fn bundled_fixture_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // macOS .app: exe at Contents/MacOS/varda, fixtures at Contents/Resources/fixtures.
    #[cfg(target_os = "macos")]
    {
        let app_resources = exe_dir.join("../Resources/fixtures");
        if app_resources.is_dir() {
            return Some(app_resources);
        }
    }

    // Linux portable tarball: exe at bin/varda, fixtures at fixtures/.
    #[cfg(target_os = "linux")]
    {
        let tarball = exe_dir.join("../fixtures");
        if tarball.is_dir() {
            return Some(tarball);
        }
    }

    // Windows portable, and the plain "next to the binary" case everywhere.
    let beside_exe = exe_dir.join("fixtures");
    if beside_exe.is_dir() {
        return Some(beside_exe);
    }

    None
}
