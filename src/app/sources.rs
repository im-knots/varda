//! The deck source types this build offers, and the device managers they share
//! with the rest of the engine.
//!
//! To add a deck source, write a provider (see `crate::source`) and add one
//! line to [`source_providers`]. Nothing else names a source type.

use super::{AppConfig, DeckSources};
use crate::engine::value::provider::ProviderTypeSnapshot;
use crate::renderer::GpuContext;
use crate::source::{Services, SourceEnv, SourceRegistry};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a source type listing is reused. Bounds staleness for library
/// changes no command announces (a stream connecting, a shader file saved).
const TYPE_LISTING_TTL: Duration = Duration::from_millis(250);

/// The last listing of every source type and when it was built. Listing walks
/// every library, so snapshots reuse it.
#[derive(Default)]
pub(crate) struct TypeCache(Mutex<Option<(Instant, Arc<Vec<ProviderTypeSnapshot>>)>>);

impl TypeCache {
    /// The cached listing, or `build`'s when it is missing or stale.
    pub(crate) fn get_or_build(
        &self,
        build: impl FnOnce() -> Vec<ProviderTypeSnapshot>,
    ) -> Arc<Vec<ProviderTypeSnapshot>> {
        let now = Instant::now();
        let Ok(mut cached) = self.0.lock() else {
            return Arc::new(build());
        };
        if let Some((built, types)) = cached.as_ref()
            && now.duration_since(*built) < TYPE_LISTING_TTL
        {
            return Arc::clone(types);
        }
        let types = Arc::new(build());
        *cached = Some((now, Arc::clone(&types)));
        types
    }

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
        .register(crate::spout::provider());
    #[cfg(target_os = "macos")]
    r.register(crate::syphon::provider());
    r.register(crate::tap::TapProvider)
        .register(crate::solid_color::SolidColorProvider)
        .register(crate::text::TextProvider);
    r
}

/// Every output sink type, in the order the output panel lists them.
pub(crate) fn output_sinks() -> crate::output::SinkRegistry {
    use crate::output::ffmpeg::{FfmpegKind, FfmpegProvider};
    let mut r = crate::output::SinkRegistry::new();
    r.register(crate::output::window::WindowedProvider)
        .register(crate::output::window::DisplayProvider)
        .register(FfmpegProvider::new(FfmpegKind::Recording))
        .register(FfmpegProvider::new(FfmpegKind::Srt))
        .register(FfmpegProvider::new(FfmpegKind::Hls))
        .register(FfmpegProvider::new(FfmpegKind::Dash))
        .register(FfmpegProvider::new(FfmpegKind::Rtmp))
        .register(crate::ndi::sink::NdiSinkProvider)
        .register(crate::spout::sink_provider());
    #[cfg(target_os = "macos")]
    r.register(crate::syphon::sink_provider());
    r
}

/// The device managers shared by providers and other features: cameras, depth
/// sensors, and the NDI, Syphon and Spout runtimes. `--no-*` flags build a
/// manager disabled so its source type can report why it is unavailable.
pub(crate) fn source_services(config: &AppConfig) -> Services {
    let services = Services::new()
        .with(crate::camera::CameraManager::new())
        .with(crate::depth::DepthSensorManager::new())
        // Built disabled so `--no-screen-capture` never triggers the macOS TCC prompt.
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
        // Filled by the window host, which headless runs have too.
        .with(crate::output::window::Monitors::default())
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
    /// Panics if `T` was never registered. [`source_services`] registers every
    /// manager the engine uses.
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

    /// The registry and an environment over these services, borrowed separately.
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

    /// Every source type for snapshots. Rebuilt after a command or once
    /// [`TYPE_LISTING_TTL`] has passed.
    pub(crate) fn type_snapshots(
        &self,
        channels: &[(String, String)],
    ) -> Arc<Vec<ProviderTypeSnapshot>> {
        self.type_cache
            .get_or_build(|| self.providers.type_snapshots(&self.query(channels)))
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

    /// The library order: generators first, then media and devices, the
    /// stream types together, the texture-sharing types together, and the
    /// utility types last.
    #[test]
    fn the_library_lists_types_in_its_order() {
        let ids: Vec<&str> = source_providers()
            .iter()
            .map(crate::source::DeckSourceProvider::id)
            .collect();
        let at = |id: &str| ids.iter().position(|i| *i == id).expect(id);
        assert_eq!(ids[0], "Shader");
        let streams: Vec<usize> = ["Ndi", "Srt", "Hls", "Dash", "Rtmp"]
            .iter()
            .map(|id| at(id))
            .collect();
        assert!(streams.windows(2).all(|w| w[1] == w[0] + 1), "{ids:?}");
        #[cfg(target_os = "macos")]
        assert_eq!(at("Syphon"), at("Spout") + 1, "{ids:?}");
        assert_eq!(&ids[ids.len() - 3..], ["Tap", "SolidColor", "Text"]);
    }

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
            "Text",
        ] {
            assert!(ids.contains(&id), "{id} is not registered");
        }
        #[cfg(target_os = "macos")]
        assert!(ids.contains(&"Syphon"), "Syphon is not registered");
    }

    #[test]
    fn every_sink_registers_once_under_its_saved_tag() {
        let registry = output_sinks();
        let ids: Vec<&str> = registry
            .iter()
            .map(crate::output::OutputSinkProvider::id)
            .collect();
        for id in [
            "windowed",
            "display",
            "recording",
            "srt_stream",
            "hls_stream",
            "dash_stream",
            "rtmp_stream",
            "ndi_send",
            "spout_sender",
        ] {
            assert!(ids.contains(&id), "{id} is not registered");
        }
        #[cfg(target_os = "macos")]
        assert!(
            ids.contains(&"syphon_server"),
            "Syphon output is not registered"
        );
    }

    fn headless_app() -> Option<super::super::VardaApp> {
        crate::testing::headless_app()
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

        // Saving a URL does, and the next snapshot lists it.
        let entry =
            crate::source::SourceConfig::new("Hls").with("url", "https://example.invalid/a.m3u8");
        app.execute_command(crate::engine::EngineCommand::AddSourceLibraryEntry {
            entry: entry.clone(),
        });
        let after = app.build_engine_state().sources;
        assert!(!Arc::ptr_eq(&first, &after));
        let hls = after.iter().find(|t| t.type_id == "Hls").expect("HLS type");
        assert!(
            hls.library
                .entries
                .iter()
                .any(|e| e.config.str("url") == entry.str("url"))
        );
    }
}
