//! Inter-application texture sharing (Syphon, Spout) as a deck source.
//!
//! Both protocols look the same from here: a directory of named servers, a
//! client per server, and a texture that follows whatever the server publishes.
//! A server may start after the deck that wants it (a restored scene opening
//! before the producer), so a deck binds by name whenever the server appears
//! and shows black until then.

use super::{
    ControlError, ControlSpec, ControlStatus, ControlValue, DeckSourceInstance, DeckSourceProvider,
    Feed, LibraryEntry, LibrarySection, ScalingMode, SourceConfig, SourceControl, SourceEnv,
    SourceFrame, SourceQuery, decode_config, downcast_mut, downcast_ref, encode_config,
    scaling_mode_spec,
};
use anyhow::{Context, Result};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| vec![scaling_mode_spec()]);

/// How often the server directory is re-read, so a producer that starts or
/// restarts after Varda is found without anyone pressing rescan.
const SCAN_INTERVAL: Duration = Duration::from_secs(1);

/// What a texture-sharing manager offers a deck.
pub trait ShareReceiver: 'static {
    fn is_available(&self) -> bool;
    /// Names of the servers currently published.
    fn server_names(&self) -> Vec<String>;
    fn discover(&mut self);
    fn start_receive(&mut self, name: &str, device: &wgpu::Device) -> Option<usize>;
    fn stop_receive(&mut self, client: usize);
    /// Follow every client's server to its current frame.
    fn update(&mut self, device: &wgpu::Device);
    fn texture_view(&self, client: usize) -> Option<&wgpu::TextureView>;
    fn client_dimensions(&self, client: usize) -> Option<(u32, u32)>;
    fn is_connected(&self, client: usize) -> bool;
}

/// Static description of one sharing protocol.
pub struct ShareProtocol {
    pub id: &'static str,
    pub label: &'static str,
    /// Why the manager is unavailable, for the library and the API.
    pub unavailable: &'static str,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    name: String,
    #[serde(default)]
    scaling_mode: ScalingMode,
}

/// A texture-sharing protocol's deck source.
pub struct ShareProvider<M: ShareReceiver> {
    protocol: ShareProtocol,
    last_scan: Instant,
    /// Servers seen at the last scan, so a deck waiting for one binds on the
    /// frame it appears.
    servers: Vec<String>,
    _manager: std::marker::PhantomData<fn() -> M>,
}

impl<M: ShareReceiver> ShareProvider<M> {
    pub fn new(protocol: ShareProtocol) -> Self {
        Self {
            protocol,
            last_scan: Instant::now()
                .checked_sub(SCAN_INTERVAL)
                .unwrap_or_else(Instant::now),
            servers: Vec::new(),
            _manager: std::marker::PhantomData,
        }
    }

    fn config_for(&self, name: &str) -> SourceConfig {
        encode_config(
            self.protocol.id,
            &Config {
                name: name.to_string(),
                scaling_mode: ScalingMode::default(),
            },
        )
    }
}

impl<M: ShareReceiver> DeckSourceProvider for ShareProvider<M> {
    fn id(&self) -> &'static str {
        self.protocol.id
    }

    fn label(&self) -> &'static str {
        self.protocol.label
    }

    fn icon(&self) -> &'static str {
        "🔗"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn availability(&self, query: &SourceQuery) -> std::result::Result<(), String> {
        if query.services.get::<M>().is_some_and(M::is_available) {
            Ok(())
        } else {
            Err(self.protocol.unavailable.to_string())
        }
    }

    fn listed(&self, query: &SourceQuery) -> bool {
        self.availability(query).is_ok()
    }

    fn library(&self, query: &SourceQuery) -> LibrarySection {
        LibrarySection {
            entries: query
                .services
                .get::<M>()
                .map(M::server_names)
                .unwrap_or_default()
                .into_iter()
                .map(|name| LibraryEntry::new(name.clone(), self.config_for(&name)))
                .collect(),
            rescan: true,
            ..LibrarySection::default()
        }
    }

    fn library_action(&mut self, action: &str, env: &mut SourceEnv) -> Result<()> {
        anyhow::ensure!(
            action == "rescan",
            "{} has no library action '{action}'",
            self.protocol.label
        );
        if let Some(manager) = env.services.get_mut::<M>() {
            manager.discover();
            self.servers = manager.server_names();
        }
        Ok(())
    }

    /// Never fails for a missing server: the deck waits for it.
    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let config: Config = decode_config(config)?;
        let manager = env
            .services
            .get_mut::<M>()
            .with_context(|| format!("{} is not running", self.protocol.label))?;
        let client = if manager.server_names().contains(&config.name) {
            manager.start_receive(&config.name, &env.gpu.device)
        } else {
            log::info!(
                "{} deck '{}' waiting for its server to appear",
                self.protocol.label,
                config.name
            );
            None
        };
        let size = client
            .and_then(|c| manager.client_dimensions(c))
            .unwrap_or((1920, 1080));
        let mut feed = Feed::new(env.gpu, "Share Blit Pass", size)?;
        feed.blit.scaling_mode = config.scaling_mode;
        Ok(Box::new(ShareFeed {
            source_type: self.protocol.id,
            name: config.name,
            client,
            feed,
        }))
    }

    fn one_per_channel(&self) -> bool {
        true
    }

    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("name").cloned().unwrap_or_default()
    }

    fn tick(&mut self, env: &mut SourceEnv, _submit: &mut Vec<wgpu::CommandBuffer>) {
        let Some(manager) = env.services.get_mut::<M>() else {
            return;
        };
        if !manager.is_available() {
            return;
        }
        if self.last_scan.elapsed() >= SCAN_INTERVAL {
            self.last_scan = Instant::now();
            manager.discover();
            self.servers = manager.server_names();
        }
        manager.update(&env.gpu.device);
    }

    fn prepare(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        let (Some(deck), Some(manager)) = (
            downcast_mut::<ShareFeed>(instance),
            env.services.get_mut::<M>(),
        ) else {
            return;
        };
        if deck.client.is_none() && self.servers.contains(&deck.name) {
            deck.client = manager.start_receive(&deck.name, &env.gpu.device);
            if deck.client.is_some() {
                log::info!("{} deck '{}' bound", self.protocol.label, deck.name);
            }
        }
        match deck.client {
            Some(client) => deck.feed.bind(
                manager.texture_view(client).cloned(),
                manager.client_dimensions(client),
            ),
            None => deck.feed.bind(None, None),
        }
    }

    fn release(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        if let (Some(deck), Some(manager)) = (
            downcast_ref::<ShareFeed>(instance),
            env.services.get_mut::<M>(),
        ) && let Some(client) = deck.client
        {
            manager.stop_receive(client);
        }
    }

    fn status(&self, instance: &dyn DeckSourceInstance, query: &SourceQuery) -> ControlStatus {
        let Some(deck) = downcast_ref::<ShareFeed>(instance) else {
            return instance.status();
        };
        let connected = deck
            .client
            .and_then(|c| query.services.get::<M>().map(|m| m.is_connected(c)));
        let mut status = deck.feed.status(Some(connected.unwrap_or(false)));
        status.bound = Some(deck.client.is_some());
        status
    }
}

/// One Syphon or Spout deck.
pub struct ShareFeed {
    source_type: &'static str,
    name: String,
    /// `None` while waiting for the server to appear.
    client: Option<usize>,
    feed: Feed,
}

impl DeckSourceInstance for ShareFeed {
    fn source_type(&self) -> &str {
        self.source_type
    }

    fn label(&self) -> String {
        format!("🔗 {}", self.name)
    }

    fn config(&self) -> SourceConfig {
        encode_config(
            self.source_type,
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
