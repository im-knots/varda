//! The set of registered source types, and the operations that dispatch to
//! the right one.

use super::{
    DeckSourceInstance, DeckSourceProvider, DeckSourceSnapshot, ProviderTypeSnapshot, SourceConfig,
    SourceEnv, SourceLoader, SourceQuery, UnavailableSource,
};
use anyhow::Result;

/// Every registered source type, in the order the library lists them.
#[derive(Default)]
pub struct SourceRegistry {
    providers: Vec<Box<dyn DeckSourceProvider>>,
}

impl SourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider.
    ///
    /// # Panics
    ///
    /// Panics if a provider with the same id is already registered.
    pub fn register(&mut self, provider: impl DeckSourceProvider) -> &mut Self {
        assert!(
            self.get(provider.id()).is_none(),
            "source type '{}' registered twice",
            provider.id()
        );
        self.providers.push(Box::new(provider));
        self
    }

    pub fn get(&self, id: &str) -> Option<&dyn DeckSourceProvider> {
        self.providers
            .iter()
            .find(|p| p.id() == id)
            .map(AsRef::as_ref)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut (dyn DeckSourceProvider + 'static)> {
        self.providers
            .iter_mut()
            .find(|p| p.id() == id)
            .map(AsMut::as_mut)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn DeckSourceProvider> {
        self.providers.iter().map(AsRef::as_ref)
    }

    fn available_provider(
        &self,
        source_type: &str,
        query: &SourceQuery,
    ) -> Result<&dyn DeckSourceProvider> {
        let provider = self
            .get(source_type)
            .ok_or_else(|| anyhow::anyhow!("unknown source type '{source_type}'"))?;
        provider
            .availability(query)
            .map_err(|reason| anyhow::anyhow!("{} is unavailable: {reason}", provider.label()))?;
        Ok(provider)
    }

    /// A builder for `config` that runs off the render thread, when its type
    /// has one.
    ///
    /// # Errors
    ///
    /// Fails for an unknown or unavailable type, or a malformed config.
    pub fn loader(
        &self,
        config: &SourceConfig,
        query: &SourceQuery,
    ) -> Option<Result<SourceLoader>> {
        match self.available_provider(config.type_id(), query) {
            Ok(provider) => provider.loader(config, query),
            Err(e) => Some(Err(e)),
        }
    }

    /// Build a new source on the render thread.
    ///
    /// # Errors
    ///
    /// Fails for an unknown or unavailable type, or when the provider cannot
    /// open what the config names.
    pub fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        self.available_provider(config.type_id(), &env.query())?;
        let provider = self
            .get_mut(config.type_id())
            .ok_or_else(|| anyhow::anyhow!("unknown source type '{}'", config.type_id()))?;
        if let Some(loader) = provider.loader(config, &env.query()) {
            return loader?(env.gpu, env.width, env.height);
        }
        provider.create(config, env)
    }

    /// Rebuild a saved source. Never fails: a source that cannot run here
    /// becomes a placeholder that keeps its config, and the reason is returned
    /// for the restore report.
    pub fn restore(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> (Box<dyn DeckSourceInstance>, Option<String>) {
        let built = self
            .available_provider(config.type_id(), &env.query())
            .map(|_| ())
            .and_then(|()| {
                let provider = self
                    .get_mut(config.type_id())
                    .ok_or_else(|| anyhow::anyhow!("unknown source type '{}'", config.type_id()))?;
                if let Some(loader) = provider.loader(config, &env.query()) {
                    return loader?(env.gpu, env.width, env.height);
                }
                provider.restore(config, env)
            });
        match built {
            Ok(instance) => (instance, None),
            Err(e) => {
                let reason = format!("{e:#}");
                (
                    Box::new(UnavailableSource::new(config.clone(), reason.clone())),
                    Some(reason),
                )
            }
        }
    }

    /// Whether two configs name the same source, so a deck built from one can
    /// be patched in place to the other.
    pub fn same_source(&self, a: &SourceConfig, b: &SourceConfig) -> bool {
        if a.type_id() != b.type_id() {
            return false;
        }
        match self.get(a.type_id()) {
            Some(provider) => provider.identity(a) == provider.identity(b),
            None => a == b,
        }
    }

    /// Service devices for one frame: every provider observes its decks, ticks
    /// once, then prepares each deck. Each entry is a deck's source and whether
    /// the deck is in use this frame.
    pub fn service_frame(
        &mut self,
        decks: &mut [(&mut Box<dyn DeckSourceInstance>, bool)],
        env: &mut SourceEnv,
        submit: &mut Vec<wgpu::CommandBuffer>,
    ) {
        for (instance, wanted) in decks.iter() {
            if let Some(provider) = self.get_mut(instance.source_type()) {
                provider.observe(instance.as_ref(), *wanted);
            }
        }
        for provider in &mut self.providers {
            provider.tick(env, submit);
        }
        for (instance, _) in decks.iter_mut() {
            let source_type = instance.source_type().to_string();
            if let Some(provider) = self.get_mut(&source_type) {
                provider.prepare(instance.as_mut(), env);
            }
        }
    }

    /// Release what a removed deck held.
    pub fn release(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        let source_type = instance.source_type().to_string();
        if let Some(provider) = self.get_mut(&source_type) {
            provider.release(instance, env);
        }
    }

    /// A deck's source for a snapshot.
    pub fn deck_snapshot(
        &self,
        instance: &dyn DeckSourceInstance,
        query: &SourceQuery,
    ) -> DeckSourceSnapshot {
        let placeholder = instance.as_any().is::<UnavailableSource>();
        let mut status = match self.get(instance.source_type()) {
            Some(provider) if !placeholder => provider.status(instance, query),
            _ => instance.status(),
        };
        status.inactive.extend(
            instance
                .inactive()
                .into_iter()
                .map(|(name, reason)| (name.to_string(), reason.to_string())),
        );
        DeckSourceSnapshot {
            source_type: instance.source_type().to_string(),
            available: !placeholder,
            owns_alpha: instance.owns_alpha(),
            status,
        }
    }

    /// Every registered type, for snapshots.
    pub fn type_snapshots(&self, query: &SourceQuery) -> Vec<ProviderTypeSnapshot> {
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
                    library_group: provider.library_group(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Services, SourceFrame};

    struct Fixed(&'static str);

    struct FixedInstance(&'static str);

    impl DeckSourceInstance for FixedInstance {
        fn source_type(&self) -> &str {
            self.0
        }
        fn label(&self) -> String {
            self.0.to_string()
        }
        fn config(&self) -> SourceConfig {
            SourceConfig::new(self.0)
        }
        fn render(&mut self, _frame: &mut SourceFrame) -> Result<()> {
            Ok(())
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    impl DeckSourceProvider for Fixed {
        fn id(&self) -> &'static str {
            self.0
        }
        fn label(&self) -> &'static str {
            self.0
        }
        fn icon(&self) -> &'static str {
            "·"
        }
        fn availability(&self, _query: &SourceQuery) -> std::result::Result<(), String> {
            if self.0 == "off" {
                Err("not in this build".into())
            } else {
                Ok(())
            }
        }
        fn create(
            &mut self,
            _config: &SourceConfig,
            _env: &mut SourceEnv,
        ) -> Result<Box<dyn DeckSourceInstance>> {
            Ok(Box::new(FixedInstance(self.0)))
        }
        fn identity(&self, config: &SourceConfig) -> serde_json::Value {
            config.get("path").cloned().unwrap_or_default()
        }
    }

    fn registry() -> SourceRegistry {
        let mut r = SourceRegistry::new();
        r.register(Fixed("on")).register(Fixed("off"));
        r
    }

    #[test]
    #[should_panic(expected = "registered twice")]
    fn a_type_cannot_be_registered_twice() {
        let mut r = registry();
        r.register(Fixed("on"));
    }

    #[test]
    fn identity_decides_whether_two_configs_are_the_same_source() {
        let r = registry();
        let a = SourceConfig::new("on").with("path", "x").with("speed", 1.0);
        let b = SourceConfig::new("on").with("path", "x").with("speed", 2.0);
        let c = SourceConfig::new("on").with("path", "y");
        assert!(r.same_source(&a, &b));
        assert!(!r.same_source(&a, &c));
        assert!(!r.same_source(&a, &SourceConfig::new("off").with("path", "x")));
    }

    #[test]
    fn type_snapshots_report_availability_in_registration_order() {
        let r = registry();
        let services = Services::new();
        let shaders = crate::registry::ShaderRegistry::new();
        let query = SourceQuery {
            services: &services,
            shaders: &shaders,
            channels: &[],
        };
        let types = r.type_snapshots(&query);
        assert_eq!(types[0].type_id, "on");
        assert!(types[0].available);
        assert_eq!(
            types[1].unavailable_reason.as_deref(),
            Some("not in this build")
        );
    }
}
