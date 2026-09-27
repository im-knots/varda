//! The deck source types this build offers, and the device managers they share
//! with the rest of the engine.
//!
//! Adding a deck source means writing a provider (see `crate::source`) and one
//! line in [`source_providers`]. Nothing else in the engine, the GUI or the API
//! names a source type. See /spec/deck-source-providers.md.

use super::{AppConfig, DeckSources};
use crate::engine::value::source::SourceTypeSnapshot;
use crate::renderer::GpuContext;
use crate::source::{Services, SourceEnv, SourceRegistry};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a source type listing is reused before it is rebuilt. Covers the
/// library changes no command announces: a stream connecting, a sender
/// appearing on the network, a shader file saved.
const TYPE_LISTING_TTL: Duration = Duration::from_millis(250);

/// The last listing of every source type and when it was built. Listing walks
/// every library (the whole shader library among them), so snapshots reuse it
/// rather than rebuilding it every frame.
#[derive(Default)]
pub(crate) struct TypeCache(Mutex<Option<(Instant, Arc<Vec<SourceTypeSnapshot>>)>>);

impl TypeCache {
    /// Forget the listing, so the next snapshot rebuilds it.
    pub(crate) fn invalidate(&self) {
        if let Ok(mut cached) = self.0.lock() {
            *cached = None;
        }
    }
}

/// Every deck source type, in the order the library lists them.
pub(crate) fn source_providers() -> SourceRegistry {
    let mut r = SourceRegistry::new();
    r.register(crate::generator::ShaderProvider)
        .register(crate::still::ImageProvider)
        .register(crate::video::provider::VideoProvider)
        .register(crate::camera::provider::CameraProvider::new())
        .register(crate::depth::provider::DepthSensorProvider)
        .register(crate::screen_capture::provider::ScreenCaptureProvider::new())
        .register(crate::tap::TapProvider)
        .register(crate::ndi::provider::NdiProvider)
        .register(crate::stream::provider::StreamProvider::new(
            crate::stream::provider::StreamKind::Srt,
        ))
        .register(crate::stream::provider::StreamProvider::new(
            crate::stream::provider::StreamKind::Hls,
        ))
        .register(crate::stream::provider::StreamProvider::new(
            crate::stream::provider::StreamKind::Dash,
        ))
        .register(crate::stream::provider::StreamProvider::new(
            crate::stream::provider::StreamKind::Rtmp,
        ))
        .register(crate::html::provider::HtmlProvider::new())
        .register(crate::spout::provider())
        .register(crate::solid_color::SolidColorProvider);
    #[cfg(target_os = "macos")]
    r.register(crate::syphon::provider());
    r
}

/// The device managers that providers and other features share: cameras (the
/// stage editor snapshots them), depth sensors (shader preprocessors read
/// them), and the NDI, Syphon and Spout runtimes (outputs send through them).
/// The `--no-*` flags build a manager disabled rather than leaving it out, so
/// its source type reports why it is unavailable.
pub(crate) fn source_services(config: &AppConfig) -> Services {
    let services = Services::new()
        .with(crate::camera::CameraManager::new())
        .with(crate::depth::DepthSensorManager::new())
        // Constructed disabled rather than merely inert, so
        // `--no-screen-capture` never triggers the macOS TCC prompt.
        .with(if config.screen_capture_disabled {
            crate::screen_capture::ScreenCaptureManager::new_disabled()
        } else {
            crate::screen_capture::ScreenCaptureManager::new()
        })
        .with(if config.ndi_disabled {
            log::info!("NDI disabled by CLI flag");
            crate::ndi::NdiManager::new_disabled()
        } else {
            crate::ndi::NdiManager::new()
        })
        .with(if config.spout_disabled {
            log::info!("Spout disabled by CLI flag");
            crate::spout::SpoutManager::new_disabled()
        } else {
            crate::spout::SpoutManager::new()
        })
        .with(crate::stream::StreamManager::new())
        .with(if config.html_disabled {
            crate::html::HtmlManager::new_disabled()
        } else {
            crate::html::HtmlManager::new()
        });
    #[cfg(target_os = "macos")]
    let services = services.with(if config.syphon_disabled {
        log::info!("Syphon disabled by CLI flag");
        crate::syphon::SyphonManager::new_disabled()
    } else {
        crate::syphon::SyphonManager::new()
    });
    services
}

impl DeckSources {
    /// A shared device manager.
    ///
    /// # Panics
    ///
    /// Panics if `T` was never registered, which [`source_services`] rules out
    /// for every manager the engine reaches for.
    pub(crate) fn service<T: std::any::Any>(&self) -> &T {
        self.services
            .get::<T>()
            .expect("device manager registered at startup")
    }

    /// A shared device manager, mutably. Panics as [`Self::service`] does.
    pub(crate) fn service_mut<T: std::any::Any>(&mut self) -> &mut T {
        self.services
            .get_mut::<T>()
            .expect("device manager registered at startup")
    }

    /// The registry and an environment over this group's services, split so a
    /// provider can be called with both.
    pub(crate) fn env<'a>(
        &'a mut self,
        gpu: &'a GpuContext,
        width: u32,
        height: u32,
        channels: &'a [(String, String)],
    ) -> (&'a mut SourceRegistry, SourceEnv<'a>) {
        (
            &mut self.providers,
            SourceEnv {
                gpu,
                width,
                height,
                services: &mut self.services,
                shaders: &self.registry,
                channels,
            },
        )
    }

    /// Every source type, as snapshots list them: rebuilt after a command or
    /// once [`TYPE_LISTING_TTL`] has passed, otherwise shared.
    pub(crate) fn type_snapshots(
        &self,
        channels: &[(String, String)],
    ) -> Arc<Vec<SourceTypeSnapshot>> {
        let now = Instant::now();
        let Ok(mut cached) = self.type_cache.0.lock() else {
            return Arc::new(self.providers.type_snapshots(&self.query(channels)));
        };
        if let Some((built, types)) = cached.as_ref()
            && now.duration_since(*built) < TYPE_LISTING_TTL
        {
            return Arc::clone(types);
        }
        let types = Arc::new(self.providers.type_snapshots(&self.query(channels)));
        *cached = Some((now, Arc::clone(&types)));
        types
    }

    /// The read-only view providers answer snapshots from.
    pub(crate) fn query<'a>(
        &'a self,
        channels: &'a [(String, String)],
    ) -> crate::source::SourceQuery<'a> {
        crate::source::SourceQuery {
            services: &self.services,
            shaders: &self.registry,
            channels,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_registers_once_under_its_saved_tag() {
        let registry = source_providers();
        let ids: Vec<&str> = registry
            .iter()
            .map(crate::source::DeckSourceProvider::id)
            .collect();
        for id in [
            "Shader",
            "Image",
            "Video",
            "Camera",
            "DepthSensor",
            "ScreenCapture",
            "Tap",
            "Ndi",
            "Srt",
            "Hls",
            "Dash",
            "Rtmp",
            "Html",
            "Spout",
            "SolidColor",
        ] {
            assert!(ids.contains(&id), "{id} is not registered");
        }
        #[cfg(target_os = "macos")]
        assert!(ids.contains(&"Syphon"), "Syphon is not registered");
    }

    fn headless_app() -> Option<super::super::VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        super::super::VardaApp::new(gpu, &crate::testing::headless_config()).ok()
    }

    #[test]
    fn snapshots_share_one_listing_until_something_changes_it() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let first = app.build_engine_state().sources;
        let second = app.build_engine_state().sources;
        assert!(
            Arc::ptr_eq(&first, &second),
            "a frame later, nothing to rebuild"
        );

        // A fader sweep cannot change a library, so it keeps the listing.
        let channel = app.mixer_ref().channels()[0].uuid().to_string();
        app.execute_command(crate::engine::EngineCommand::SetChannelOpacity {
            channel_uuid: channel,
            opacity: 0.5,
        });
        assert!(Arc::ptr_eq(&first, &app.build_engine_state().sources));

        // Saving a URL does, and the very next snapshot lists it.
        let entry =
            crate::source::SourceConfig::new("Hls").with("url", "https://example.invalid/a.m3u8");
        app.execute_command(crate::engine::EngineCommand::AddSourceLibraryEntry {
            entry: entry.clone(),
        });
        let after = app.build_engine_state().sources;
        assert!(!Arc::ptr_eq(&first, &after));
        let hls = after
            .iter()
            .find(|t| t.source_type == "Hls")
            .expect("HLS type");
        assert!(
            hls.library
                .entries
                .iter()
                .any(|e| e.config.str("url") == entry.str("url"))
        );
    }
}
