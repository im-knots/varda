//! HTML pages (rendered by Servo) as a deck source.

use super::HtmlManager;
use crate::source::{
    ControlError, ControlSpec, ControlStatus, ControlValue, DeckSourceInstance, DeckSourceProvider,
    Feed, LibraryCreate, LibraryEntry, LibrarySection, SourceConfig, SourceControl, SourceEnv,
    SourceFrame, SourceQuery, decode_config, downcast_mut, downcast_ref, encode_config,
    scaling_mode_spec,
};
use anyhow::{Context, Result};
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "Html";

static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    vec![
        scaling_mode_spec(),
        ControlSpec::action("reload", "Reload").routed("html/reload"),
        ControlSpec::action("interactive", "Interactive").routed("html/interactive"),
    ]
});

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    url: String,
    #[serde(default)]
    scaling_mode: crate::source::ScalingMode,
}

/// HTML sources, plus the URLs saved to the library this session.
#[derive(Default)]
pub struct HtmlProvider {
    library: Vec<String>,
}

impl HtmlProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DeckSourceProvider for HtmlProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "HTML Sources"
    }

    fn icon(&self) -> &'static str {
        "🌐"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn availability(&self, query: &SourceQuery) -> std::result::Result<(), String> {
        if query
            .services
            .get::<HtmlManager>()
            .is_some_and(HtmlManager::is_available)
        {
            Ok(())
        } else {
            Err("HTML rendering is not available in this build".into())
        }
    }

    fn library(&self, query: &SourceQuery) -> LibrarySection {
        let html = query.services.get::<HtmlManager>();
        let active = |url: &str| {
            html.is_some_and(|m| (0..m.instance_count()).any(|i| m.instance_url(i) == Some(url)))
        };
        let mut defaults = serde_json::Map::new();
        defaults.insert("url".into(), "https://example.com/visuals.html".into());
        LibrarySection {
            entries: self
                .library
                .iter()
                .map(|url| {
                    let mut row = LibraryEntry::new(format!("🌐 {url}"), HtmlFeed::config_for(url));
                    row.removable = true;
                    row.hover = Some(url.clone());
                    row.connected = Some(active(url));
                    row
                })
                .collect(),
            create: Some(LibraryCreate::Entry {
                fields: vec![ControlSpec::text("url", "URL")],
                defaults,
                label: "+ Add HTML".into(),
                hint: None,
            }),
            ..LibrarySection::default()
        }
    }

    fn add_library_entry(&mut self, entry: SourceConfig) -> Result<()> {
        let url = entry
            .str("url")
            .filter(|u| !u.trim().is_empty())
            .context("An HTML source needs a URL")?;
        if !self.library.iter().any(|u| u == url) {
            log::info!("Added HTML source to library: {url}");
            self.library.push(url.to_string());
        }
        Ok(())
    }

    fn remove_library_entry(&mut self, entry: &SourceConfig) -> Result<()> {
        let url = entry.str("url").context("An HTML entry needs a URL")?;
        self.library.retain(|u| u != url);
        Ok(())
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let config: Config = decode_config(config)?;
        let html = env
            .services
            .get_mut::<HtmlManager>()
            .context("HTML rendering is not running")?;
        let instance = html
            .start_render(&config.url, env.width, env.height, &env.gpu.device)
            .with_context(|| format!("HTML source '{}' not available", config.url))?;
        let size = html.instance_dimensions(instance).unwrap_or((1920, 1080));
        let mut feed = Feed::new(env.gpu, "HTML Blit Pass", size)?;
        feed.blit.scaling_mode = config.scaling_mode;
        Ok(Box::new(HtmlFeed {
            url: config.url,
            instance,
            feed,
            reload_requested: false,
            interactive_requested: false,
        }))
    }

    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("url").cloned().unwrap_or_default()
    }

    fn tick(&mut self, env: &mut SourceEnv, _submit: &mut Vec<wgpu::CommandBuffer>) {
        if let Some(html) = env.services.get_mut::<HtmlManager>() {
            html.update(&env.gpu.device, &env.gpu.queue);
        }
    }

    fn prepare(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        let (Some(deck), Some(html)) = (
            downcast_mut::<HtmlFeed>(instance),
            env.services.get_mut::<HtmlManager>(),
        ) else {
            return;
        };
        if std::mem::take(&mut deck.reload_requested) {
            html.reload(deck.instance);
        }
        deck.feed.bind(
            html.texture_view(deck.instance).cloned(),
            html.instance_dimensions(deck.instance),
        );
    }

    fn status(&self, instance: &dyn DeckSourceInstance, _query: &SourceQuery) -> ControlStatus {
        match downcast_ref::<HtmlFeed>(instance) {
            Some(deck) => deck.feed.status(None),
            None => instance.status(),
        }
    }
}

pub struct HtmlFeed {
    url: String,
    instance: usize,
    feed: Feed,
    /// Set by the reload action. The provider reloads the page next frame,
    /// since only it holds the manager.
    reload_requested: bool,
    /// Set by the interactive action. The app opens or closes the window,
    /// since only it can create one. See [`take_interactive_request`].
    interactive_requested: bool,
}

impl HtmlFeed {
    pub fn config_for(url: &str) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                url: url.to_string(),
                scaling_mode: crate::source::ScalingMode::default(),
            },
        )
    }

    pub fn instance(&self) -> usize {
        self.instance
    }
}

/// The HTML instance a deck shows, if the deck is an HTML deck.
pub fn html_instance(source: &dyn DeckSourceInstance) -> Option<usize> {
    downcast_ref::<HtmlFeed>(source).map(HtmlFeed::instance)
}

/// Takes a pending request to toggle this deck's interactive window,
/// returning the HTML instance to show.
pub fn take_interactive_request(source: &mut dyn DeckSourceInstance) -> Option<usize> {
    let deck = downcast_mut::<HtmlFeed>(source)?;
    std::mem::take(&mut deck.interactive_requested).then_some(deck.instance)
}

impl DeckSourceInstance for HtmlFeed {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        format!("🌐 {}", self.url)
    }

    fn config(&self) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                url: self.url.clone(),
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

    fn trigger(&mut self, action: &str) -> Result<(), ControlError> {
        match action {
            "reload" => self.reload_requested = true,
            "interactive" => self.interactive_requested = true,
            _ => return Err(ControlError::Unknown(action.to_string())),
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
