//! The set of registered sink types, and the operations that dispatch to the
//! right one.

use super::{
    OutputSinkInstance, OutputSinkProvider, ProviderTypeSnapshot, SinkConfig, SinkEnv, SinkQuery,
};
use anyhow::{Context, Result};

/// Every registered sink type, in the order the output panel lists them.
#[derive(Default)]
pub struct SinkRegistry {
    providers: Vec<Box<dyn OutputSinkProvider>>,
}

impl SinkRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider.
    ///
    /// # Panics
    ///
    /// Panics if a provider with the same id is already registered: two
    /// providers answering to one `type` tag would make every saved stage
    /// ambiguous.
    pub fn register(&mut self, provider: impl OutputSinkProvider) -> &mut Self {
        assert!(
            self.get(provider.id()).is_none(),
            "sink type '{}' registered twice",
            provider.id()
        );
        self.providers.push(Box::new(provider));
        self
    }

    pub fn get(&self, id: &str) -> Option<&dyn OutputSinkProvider> {
        self.providers
            .iter()
            .find(|p| p.id() == id)
            .map(AsRef::as_ref)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut (dyn OutputSinkProvider + 'static)> {
        self.providers
            .iter_mut()
            .find(|p| p.id() == id)
            .map(AsMut::as_mut)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn OutputSinkProvider> {
        self.providers.iter().map(AsRef::as_ref)
    }

    /// Build a sink from `config`. Fields it leaves out take the type's
    /// defaults, so `{"type": "recording"}` is a whole request.
    ///
    /// # Errors
    ///
    /// Fails for an unknown or unavailable type, or a config its provider
    /// rejects.
    pub fn create(
        &mut self,
        config: &SinkConfig,
        env: &mut SinkEnv,
    ) -> Result<Box<dyn OutputSinkInstance>> {
        let id = config.type_id().to_string();
        let query = env.query();
        let provider = self
            .get(&id)
            .with_context(|| format!("unknown output type '{id}'"))?;
        provider.availability(&query).map_err(|reason| {
            anyhow::anyhow!("{} outputs are unavailable: {reason}", provider.label())
        })?;
        let mut merged = provider.default_config();
        for (key, value) in config.fields() {
            merged.set(key, value);
        }
        self.get_mut(&id)
            .context("provider vanished")?
            .create(&merged, env)
    }

    /// Rebuild a saved output's sink. A type this build or host cannot run,
    /// or a config its provider rejects, becomes a placeholder holding the
    /// config, so the output survives a save; the reason comes back with it.
    pub fn restore(
        &mut self,
        config: &SinkConfig,
        env: &mut SinkEnv,
    ) -> (Box<dyn OutputSinkInstance>, Option<String>) {
        match self.create(config, env) {
            Ok(sink) => (sink, None),
            Err(error) => {
                let reason = format!("{error:#}");
                (
                    Box::new(super::UnavailableSink::new(config.clone(), reason.clone())),
                    Some(reason),
                )
            }
        }
    }

    /// Once per frame, before any output renders.
    pub fn tick(&mut self, env: &mut SinkEnv) {
        for provider in &mut self.providers {
            provider.tick(env);
        }
    }

    /// Every registered type, for snapshots.
    pub fn type_snapshots(&self, query: &SinkQuery) -> Vec<ProviderTypeSnapshot> {
        self.providers
            .iter()
            .map(|provider| {
                let availability = provider.availability(query);
                ProviderTypeSnapshot {
                    type_id: provider.id().to_string(),
                    label: provider.label().to_string(),
                    icon: provider.icon().to_string(),
                    available: availability.is_ok(),
                    unavailable_reason: availability.err(),
                    listed: provider.listed(query),
                    params: provider.params().to_vec(),
                    library: provider.library(query),
                    library_group: None,
                }
            })
            .collect()
    }
}
