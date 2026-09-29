//! SRT, HLS, DASH and RTMP receive as deck sources. The four source types
//! share one implementation and differ in protocol and config fields.

use super::{RtmpMode, SrtMode, StreamManager, StreamProtocol};
use crate::source::{
    ControlError, ControlSpec, ControlStatus, ControlValue, DeckSourceInstance, DeckSourceProvider,
    Feed, LibraryCreate, LibraryEntry, LibrarySection, SourceConfig, SourceControl, SourceEnv,
    SourceFrame, SourceQuery, decode_config, downcast_mut, downcast_ref, encode_config,
    scaling_mode_spec,
};
use anyhow::{Context, Result};
use std::sync::LazyLock;

static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| vec![scaling_mode_spec()]);

/// Which of the four stream protocols a provider serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Srt,
    Hls,
    Dash,
    Rtmp,
}

impl StreamKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Srt => "Srt",
            Self::Hls => "Hls",
            Self::Dash => "Dash",
            Self::Rtmp => "Rtmp",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Srt => "SRT",
            Self::Hls => "HLS",
            Self::Dash => "DASH",
            Self::Rtmp => "RTMP",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Srt | Self::Rtmp => "📺",
            Self::Hls | Self::Dash => "📡",
        }
    }

    fn modes(self) -> &'static [&'static str] {
        match self {
            Self::Srt => &["Listener", "Caller"],
            Self::Rtmp => &["Pull", "Listen"],
            Self::Hls | Self::Dash => &[],
        }
    }

    fn default_mode(self) -> Option<&'static str> {
        match self {
            Self::Srt => Some("caller"),
            Self::Rtmp => Some("pull"),
            Self::Hls | Self::Dash => None,
        }
    }

    fn default_url(self) -> &'static str {
        match self {
            Self::Srt => "srt://127.0.0.1:9001",
            Self::Hls => "https://example.com/stream.m3u8",
            Self::Dash => "https://example.com/stream.mpd",
            Self::Rtmp => "rtmp://",
        }
    }

    /// The protocol a config asks for. Modes match case-insensitively because
    /// saved scenes and API clients use both spellings.
    fn protocol(self, mode: Option<&str>) -> StreamProtocol {
        let mode = mode.map(str::to_ascii_lowercase);
        match self {
            Self::Srt => StreamProtocol::Srt {
                mode: match mode.as_deref() {
                    Some("listener") => SrtMode::Listener,
                    Some("caller") | None => SrtMode::Caller,
                    Some(other) => {
                        log::warn!("Unknown SRT mode '{other}', defaulting to Caller");
                        SrtMode::Caller
                    }
                },
            },
            Self::Hls => StreamProtocol::Hls,
            Self::Dash => StreamProtocol::Dash,
            Self::Rtmp => StreamProtocol::Rtmp {
                mode: match mode.as_deref() {
                    Some("listen") => RtmpMode::Listen,
                    _ => RtmpMode::Pull,
                },
            },
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(default)]
    scaling_mode: crate::source::ScalingMode,
}

/// One stream protocol's deck source, plus the URLs saved to its library
/// this session.
pub struct StreamProvider {
    kind: StreamKind,
    library: Vec<Config>,
}

impl StreamProvider {
    pub fn new(kind: StreamKind) -> Self {
        Self {
            kind,
            library: Vec::new(),
        }
    }

    fn config_for(&self, url: &str, mode: Option<&str>) -> SourceConfig {
        encode_config(
            self.kind.id(),
            &Config {
                url: url.to_string(),
                mode: mode.map(str::to_ascii_lowercase),
                scaling_mode: crate::source::ScalingMode::default(),
            },
        )
    }
}

fn connected(streams: &StreamManager, url: &str) -> bool {
    (0..streams.receiver_count())
        .any(|i| streams.receiver_url(i) == Some(url) && streams.is_connected(i))
}

impl DeckSourceProvider for StreamProvider {
    fn id(&self) -> &'static str {
        self.kind.id()
    }

    fn label(&self) -> &'static str {
        self.kind.label()
    }

    fn icon(&self) -> &'static str {
        self.kind.glyph()
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn library(&self, query: &SourceQuery) -> LibrarySection {
        let streams = query.services.get::<StreamManager>();
        let entries = self
            .library
            .iter()
            .map(|entry| {
                let mode = entry.mode.as_deref().map(|m| {
                    let mut c = m.chars();
                    c.next()
                        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                        .unwrap_or_default()
                });
                let label = match (self.kind, &mode) {
                    (StreamKind::Rtmp, Some(mode)) => {
                        format!("{} {} ({mode})", self.kind.glyph(), entry.url)
                    }
                    _ => format!("{} {}", self.kind.glyph(), entry.url),
                };
                let mut row =
                    LibraryEntry::new(label, self.config_for(&entry.url, entry.mode.as_deref()));
                row.removable = true;
                row.hover = Some(entry.url.clone());
                row.connected = Some(streams.is_some_and(|s| connected(s, &entry.url)));
                if self.kind == StreamKind::Srt {
                    row.detail = mode.map(|m| format!("Mode: {m}"));
                }
                row
            })
            .collect();

        let mut fields = Vec::new();
        let mut defaults = serde_json::Map::new();
        if !self.kind.modes().is_empty() {
            fields.push(ControlSpec::choice("mode", "Mode", self.kind.modes()));
            defaults.insert(
                "mode".into(),
                self.kind.default_mode().unwrap_or_default().into(),
            );
        }
        fields.push(ControlSpec::text("url", "URL"));
        defaults.insert("url".into(), self.kind.default_url().into());

        LibrarySection {
            entries,
            create: Some(LibraryCreate::Entry {
                fields,
                defaults,
                label: format!("+ Add {}", self.kind.label()),
                hint: (self.kind == StreamKind::Rtmp)
                    .then(|| "Listen mode: OBS → rtmp://YOUR_IP:PORT/live/stream".to_string()),
            }),
            ..LibrarySection::default()
        }
    }

    fn add_library_entry(&mut self, entry: SourceConfig) -> Result<()> {
        let entry: Config = decode_config(&entry)?;
        anyhow::ensure!(!entry.url.trim().is_empty(), "A stream needs a URL");
        if !self.library.iter().any(|e| e.url == entry.url) {
            log::info!(
                "Added {} source to library: {}",
                self.kind.label(),
                entry.url
            );
            self.library.push(Config {
                mode: entry
                    .mode
                    .or_else(|| self.kind.default_mode().map(str::to_string))
                    .map(|m| m.to_ascii_lowercase()),
                ..entry
            });
        }
        Ok(())
    }

    fn remove_library_entry(&mut self, entry: &SourceConfig) -> Result<()> {
        let url = entry.str("url").context("A stream entry needs a URL")?;
        self.library.retain(|e| e.url != url);
        Ok(())
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let config: Config = decode_config(config)?;
        let protocol = self.kind.protocol(config.mode.as_deref());
        let streams = env
            .services
            .get_mut::<StreamManager>()
            .context("stream receive is not running")?;
        let receiver = streams
            .start_receive(&config.url, protocol, &env.gpu.device)
            .with_context(|| {
                format!(
                    "{} source '{}' not available",
                    self.kind.label(),
                    config.url
                )
            })?;
        let size = streams
            .receiver_dimensions(receiver)
            .unwrap_or((1920, 1080));
        let mut feed = Feed::new(env.gpu, "Stream Blit Pass", size)?;
        feed.blit.scaling_mode = config.scaling_mode;
        Ok(Box::new(StreamFeed {
            kind: self.kind,
            url: config.url,
            mode: config
                .mode
                .or_else(|| self.kind.default_mode().map(str::to_string)),
            receiver,
            feed,
        }))
    }

    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("url").cloned().unwrap_or_default()
    }

    fn tick(&mut self, env: &mut SourceEnv, _submit: &mut Vec<wgpu::CommandBuffer>) {
        // Every protocol shares one manager, so only the first provider uploads.
        if self.kind == StreamKind::Srt
            && let Some(streams) = env.services.get::<StreamManager>()
        {
            streams.update(&env.gpu.queue);
        }
    }

    fn prepare(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        let (Some(deck), Some(streams)) = (
            downcast_mut::<StreamFeed>(instance),
            env.services.get::<StreamManager>(),
        ) else {
            return;
        };
        deck.feed.bind(
            streams.texture_view(deck.receiver).cloned(),
            streams.receiver_dimensions(deck.receiver),
        );
    }

    fn release(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        if let (Some(deck), Some(streams)) = (
            downcast_ref::<StreamFeed>(instance),
            env.services.get_mut::<StreamManager>(),
        ) {
            streams.stop_receive(deck.receiver);
        }
    }

    fn status(&self, instance: &dyn DeckSourceInstance, query: &SourceQuery) -> ControlStatus {
        let Some(deck) = downcast_ref::<StreamFeed>(instance) else {
            return instance.status();
        };
        let connected = query
            .services
            .get::<StreamManager>()
            .map(|s| s.is_connected(deck.receiver));
        deck.feed.status(connected)
    }
}

pub struct StreamFeed {
    kind: StreamKind,
    url: String,
    mode: Option<String>,
    receiver: usize,
    feed: Feed,
}

impl DeckSourceInstance for StreamFeed {
    fn source_type(&self) -> &str {
        self.kind.id()
    }

    fn label(&self) -> String {
        format!("{} {}", self.kind.glyph(), self.url)
    }

    fn config(&self) -> SourceConfig {
        encode_config(
            self.kind.id(),
            &Config {
                url: self.url.clone(),
                mode: self.mode.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_read_in_either_case() {
        assert_eq!(
            StreamKind::Srt.protocol(Some("Listener")),
            StreamProtocol::Srt {
                mode: SrtMode::Listener
            }
        );
        assert_eq!(
            StreamKind::Rtmp.protocol(Some("Listen")),
            StreamProtocol::Rtmp {
                mode: RtmpMode::Listen
            }
        );
        assert_eq!(
            StreamKind::Rtmp.protocol(None),
            StreamProtocol::Rtmp {
                mode: RtmpMode::Pull
            }
        );
    }

    #[test]
    fn library_entries_are_deduplicated_by_url_and_removable() {
        let mut p = StreamProvider::new(StreamKind::Hls);
        let entry = SourceConfig::new("Hls").with("url", "https://x/a.m3u8");
        p.add_library_entry(entry.clone()).unwrap();
        p.add_library_entry(entry.clone()).unwrap();
        assert_eq!(p.library.len(), 1);
        p.remove_library_entry(&entry).unwrap();
        assert!(p.library.is_empty());
        assert!(
            p.add_library_entry(SourceConfig::new("Hls").with("url", " "))
                .is_err()
        );
    }

    #[test]
    fn an_srt_entry_keeps_its_mode_lowercase() {
        let mut p = StreamProvider::new(StreamKind::Srt);
        p.add_library_entry(
            SourceConfig::new("Srt")
                .with("url", "srt://h:1")
                .with("mode", "Listener"),
        )
        .unwrap();
        assert_eq!(p.library[0].mode.as_deref(), Some("listener"));
    }
}
