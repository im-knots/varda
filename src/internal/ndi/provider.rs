//! NDI receive as a deck source.

use super::NdiManager;
use crate::source::{
    ControlError, ControlSpec, ControlStatus, ControlValue, DeckSourceInstance, DeckSourceProvider,
    Feed, LibraryEntry, LibrarySection, SourceConfig, SourceControl, SourceEnv, SourceFrame,
    SourceQuery, decode_config, downcast_mut, downcast_ref, encode_config, scaling_mode_spec,
};
use anyhow::{Context, Result};
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "Ndi";

static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| vec![scaling_mode_spec()]);

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    name: String,
    #[serde(default)]
    scaling_mode: crate::source::ScalingMode,
}

/// NDI network video sources.
pub struct NdiProvider;

fn manager(services: &crate::source::Services) -> Option<&NdiManager> {
    services.get::<NdiManager>()
}

impl DeckSourceProvider for NdiProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "NDI"
    }

    fn icon(&self) -> &'static str {
        "📡"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn library_group(&self) -> Option<crate::engine::value::provider::LibraryGroup> {
        Some(crate::engine::value::provider::LibraryGroup::streams())
    }

    fn library(&self, query: &SourceQuery) -> LibrarySection {
        let Some(ndi) = manager(query.services) else {
            return LibrarySection::default();
        };
        LibrarySection {
            entries: ndi
                .discovered_sources()
                .into_iter()
                .map(|name| LibraryEntry::new(name.clone(), NdiFeed::config_for(&name)))
                .collect(),
            rescan: true,
            note: (!ndi.is_available()).then(|| "(SDK not found)".to_string()),
            ..LibrarySection::default()
        }
    }

    fn library_action(&mut self, action: &str, env: &mut SourceEnv) -> Result<()> {
        anyhow::ensure!(action == "rescan", "NDI has no library action '{action}'");
        if let Some(ndi) = env.services.get_mut::<NdiManager>() {
            ndi.discover();
        }
        Ok(())
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let config: Config = decode_config(config)?;
        let ndi = env
            .services
            .get_mut::<NdiManager>()
            .context("NDI is not running")?;
        let receiver = ndi
            .start_receive(&config.name, &env.gpu.device)
            .with_context(|| format!("NDI source '{}' not available", config.name))?;
        let size = ndi.receiver_dimensions(receiver).unwrap_or((1920, 1080));
        let mut feed = Feed::new(env.gpu, "NDI Blit Pass", size)?;
        feed.blit.scaling_mode = config.scaling_mode;
        Ok(Box::new(NdiFeed {
            name: config.name,
            receiver,
            feed,
        }))
    }

    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("name").cloned().unwrap_or_default()
    }

    fn tick(&mut self, env: &mut SourceEnv, submit: &mut Vec<wgpu::CommandBuffer>) {
        // Converting received UYVY on the GPU, submitted before decks render.
        if let Some(ndi) = env.services.get_mut::<NdiManager>()
            && let Some(conversion) = ndi.update(&env.gpu.device, &env.gpu.queue)
        {
            submit.push(conversion);
        }
    }

    fn prepare(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        let (Some(deck), Some(ndi)) = (
            downcast_mut::<NdiFeed>(instance),
            env.services.get::<NdiManager>(),
        ) else {
            return;
        };
        deck.feed.bind(
            ndi.texture_view(deck.receiver).cloned(),
            ndi.receiver_dimensions(deck.receiver),
        );
    }

    fn release(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        if let (Some(deck), Some(ndi)) = (
            downcast_ref::<NdiFeed>(instance),
            env.services.get_mut::<NdiManager>(),
        ) {
            ndi.stop_receive(deck.receiver);
        }
    }

    fn status(&self, instance: &dyn DeckSourceInstance, query: &SourceQuery) -> ControlStatus {
        let Some(deck) = downcast_ref::<NdiFeed>(instance) else {
            return instance.status();
        };
        let connected = manager(query.services).map(|ndi| ndi.is_connected(deck.receiver));
        deck.feed.status(connected)
    }
}

/// One NDI deck.
pub struct NdiFeed {
    name: String,
    receiver: usize,
    feed: Feed,
}

impl NdiFeed {
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

impl DeckSourceInstance for NdiFeed {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        format!("📡 {}", self.name)
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
