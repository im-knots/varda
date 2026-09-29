//! Cameras as a deck source. One capture session per device feeds every deck
//! that shows it.

use super::{CameraId, CameraManager};
use crate::source::{
    ControlError, ControlSpec, ControlStatus, ControlValue, DeckSourceInstance, DeckSourceProvider,
    Feed, LibraryEntry, LibrarySection, SourceConfig, SourceControl, SourceEnv, SourceFrame,
    SourceQuery, decode_config, downcast_mut, downcast_ref, encode_config, scaling_mode_spec,
};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "Camera";

static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| vec![scaling_mode_spec()]);

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    /// Devices are matched by name, which survives replugging; ids do not.
    name: String,
    #[serde(default)]
    scaling_mode: crate::source::ScalingMode,
}

#[derive(Default)]
pub struct CameraProvider {
    /// Cameras some visible or cued deck shows this frame. Only these upload.
    wanted: HashSet<CameraId>,
}

impl CameraProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DeckSourceProvider for CameraProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "Cameras"
    }

    fn icon(&self) -> &'static str {
        "📹"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn library(&self, query: &SourceQuery) -> LibrarySection {
        LibrarySection {
            entries: query
                .services
                .get::<CameraManager>()
                .map(|cameras| {
                    cameras
                        .devices()
                        .iter()
                        .map(|d| LibraryEntry::new(d.name.clone(), CameraFeed::config_for(&d.name)))
                        .collect()
                })
                .unwrap_or_default(),
            rescan: true,
            ..LibrarySection::default()
        }
    }

    fn library_action(&mut self, action: &str, env: &mut SourceEnv) -> Result<()> {
        anyhow::ensure!(
            action == "rescan",
            "Cameras have no library action '{action}'"
        );
        if let Some(cameras) = env.services.get_mut::<CameraManager>() {
            cameras.scan_devices();
        }
        Ok(())
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let config: Config = decode_config(config)?;
        let cameras = env
            .services
            .get_mut::<CameraManager>()
            .context("cameras are not available")?;
        let id = cameras
            .devices()
            .iter()
            .find(|d| d.name == config.name)
            .map(|d| d.id)
            .with_context(|| format!("Camera '{}' not found — is it connected?", config.name))?;
        let size = cameras
            .open_camera(id, &env.gpu.device)
            .with_context(|| format!("Failed to open camera '{}'", config.name))?;
        let mut feed = Feed::new(env.gpu, "Camera Blit Pass", size)?;
        feed.blit.scaling_mode = config.scaling_mode;
        Ok(Box::new(CameraFeed {
            name: config.name,
            id,
            feed,
        }))
    }

    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("name").cloned().unwrap_or_default()
    }

    fn observe(&mut self, instance: &dyn DeckSourceInstance, wanted: bool) {
        if wanted && let Some(deck) = downcast_ref::<CameraFeed>(instance) {
            self.wanted.insert(deck.id);
        }
    }

    fn tick(&mut self, env: &mut SourceEnv, _submit: &mut Vec<wgpu::CommandBuffer>) {
        if let Some(cameras) = env.services.get_mut::<CameraManager>() {
            cameras.update_selective(&env.gpu.queue, &self.wanted);
        }
        self.wanted.clear();
    }

    fn prepare(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        let (Some(deck), Some(cameras)) = (
            downcast_mut::<CameraFeed>(instance),
            env.services.get::<CameraManager>(),
        ) else {
            return;
        };
        deck.feed.bind(
            cameras.texture_view(deck.id).cloned(),
            cameras.resolution(deck.id),
        );
    }

    fn release(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        if let (Some(deck), Some(cameras)) = (
            downcast_ref::<CameraFeed>(instance),
            env.services.get_mut::<CameraManager>(),
        ) {
            cameras.release_camera(deck.id);
        }
    }

    fn status(&self, instance: &dyn DeckSourceInstance, query: &SourceQuery) -> ControlStatus {
        let Some(deck) = downcast_ref::<CameraFeed>(instance) else {
            return instance.status();
        };
        let connected = query
            .services
            .get::<CameraManager>()
            .map(|c| c.is_connected(deck.id));
        deck.feed.status(connected)
    }
}

pub struct CameraFeed {
    name: String,
    id: CameraId,
    feed: Feed,
}

impl CameraFeed {
    pub fn config_for(name: &str) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                name: name.to_string(),
                scaling_mode: crate::source::ScalingMode::default(),
            },
        )
    }
}

impl DeckSourceInstance for CameraFeed {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        format!("📹 {}", self.name)
    }

    fn config(&self) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                name: self.name.clone(),
                scaling_mode: self.feed.blit.scaling_mode,
            },
        )
    }

    fn render(&mut self, frame: &mut SourceFrame) -> Result<()> {
        self.feed.render(frame);
        Ok(())
    }

    fn patch(&mut self, config: &SourceConfig) {
        if let Ok(config) = config.decode::<Config>() {
            self.feed.blit.scaling_mode = config.scaling_mode;
        }
    }

    fn control(&mut self, ctx: &mut SourceControl) {
        self.feed.control(ctx);
    }

    fn schema(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn param(&self, name: &str) -> Option<ControlValue> {
        self.feed.param(name)
    }

    fn set_param(&mut self, name: &str, value: &ControlValue) -> Result<(), ControlError> {
        self.feed.set_param(name, value)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
