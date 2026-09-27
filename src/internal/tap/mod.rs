//! Program tap: Varda's own master program, or one channel's composite,
//! re-entering as a deck source one frame behind. The broadcast re-entry bus:
//! feedback loops, picture-in-picture of the live mix, cross-channel reuse.
//! See spec/program-tap.md.

use crate::source::{
    ControlError, ControlSpec, ControlStatus, ControlValue, DeckSourceInstance, DeckSourceProvider,
    Feed, FeedbackSource, LibraryEntry, LibraryNotice, LibrarySection, SourceConfig, SourceControl,
    SourceEnv, SourceFrame, SourceQuery, decode_config, encode_config, scaling_mode_spec,
};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "Tap";

static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| vec![scaling_mode_spec()]);

/// The tap point a scene records. Channels are referenced by UUID so a tap
/// survives reordering.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TapPoint {
    MasterProgram,
    Channel { uuid: String },
}

impl From<&TapPoint> for FeedbackSource {
    fn from(point: &TapPoint) -> Self {
        match point {
            TapPoint::MasterProgram => Self::MasterProgram,
            TapPoint::Channel { uuid } => Self::Channel(uuid.clone()),
        }
    }
}

impl From<&FeedbackSource> for TapPoint {
    fn from(source: &FeedbackSource) -> Self {
        match source {
            FeedbackSource::MasterProgram => Self::MasterProgram,
            FeedbackSource::Channel(uuid) => Self::Channel { uuid: uuid.clone() },
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    source: TapPoint,
    #[serde(default)]
    scaling_mode: crate::source::ScalingMode,
}

/// Display name of a tap point. An unresolvable channel falls back to its
/// UUID; whether it is genuinely missing is reported separately as `bound`.
fn point_label(source: &FeedbackSource, channels: &[(String, String)]) -> String {
    match source {
        FeedbackSource::MasterProgram => "Master Program".to_string(),
        FeedbackSource::Channel(uuid) => channels
            .iter()
            .find(|(u, _)| u == uuid)
            .map_or_else(|| format!("Channel {uuid}"), |(_, n)| n.clone()),
    }
}

/// Taps. They own no device: the mixer resolves every tap's texture before any
/// deck renders.
pub struct TapProvider;

impl DeckSourceProvider for TapProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "Taps"
    }

    fn icon(&self) -> &'static str {
        "🔁"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    /// Built from the live channel list rather than a device scan, so there is
    /// nothing to rescan. See spec/program-tap.md § UI.
    fn library(&self, query: &SourceQuery) -> LibrarySection {
        let mut entries = vec![LibraryEntry::new(
            "Master Program",
            Tap::config_for(&TapPoint::MasterProgram),
        )];
        entries.extend(query.channels.iter().map(|(uuid, name)| {
            LibraryEntry::new(
                name.clone(),
                Tap::config_for(&TapPoint::Channel { uuid: uuid.clone() }),
            )
        }));
        LibrarySection {
            entries,
            notices: vec![LibraryNotice {
                text: "Varda's own output, one frame behind.".into(),
                level: "info".into(),
                action_label: None,
                action: None,
            }],
            ..LibrarySection::default()
        }
    }

    /// A tap holds no handle and acquires nothing, so it cannot fail to build.
    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let config: Config = decode_config(config)?;
        let source = FeedbackSource::from(&config.source);
        let label = point_label(&source, env.channels);
        // A tap matches the render resolution, so the default scaling mode is an
        // exact 1:1 blit until the user changes it.
        let mut feed = Feed::new(env.gpu, "Tap Blit Pass", (env.width, env.height))?;
        feed.blit.scaling_mode = config.scaling_mode;
        Ok(Box::new(Tap {
            source,
            label,
            feed,
        }))
    }

    /// Compared by tap point, not by label: renaming a channel must patch the
    /// deck in place rather than rebuild it.
    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("source").cloned().unwrap_or_default()
    }

    fn status(&self, instance: &dyn DeckSourceInstance, query: &SourceQuery) -> ControlStatus {
        let Some(tap) = crate::source::downcast_ref::<Tap>(instance) else {
            return instance.status();
        };
        let mut status = tap.feed.status(None);
        status.bound = Some(match &tap.source {
            FeedbackSource::MasterProgram => true,
            FeedbackSource::Channel(uuid) => query.channels.iter().any(|(u, _)| u == uuid),
        });
        let (kind, channel) = match &tap.source {
            FeedbackSource::MasterProgram => ("master_program", None),
            FeedbackSource::Channel(uuid) => ("channel", Some(uuid.clone())),
        };
        status.info.insert("kind".into(), kind.into());
        if let Some(uuid) = channel {
            status.info.insert("channel_uuid".into(), uuid.into());
        }
        status.info.insert(
            "label".into(),
            point_label(&tap.source, query.channels).into(),
        );
        status
    }
}

/// One tap deck.
pub struct Tap {
    source: FeedbackSource,
    label: String,
    feed: Feed,
}

impl Tap {
    pub fn config_for(point: &TapPoint) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                source: point.clone(),
                scaling_mode: crate::source::ScalingMode::default(),
            },
        )
    }
}

impl DeckSourceInstance for Tap {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        format!("🔁 {}", self.label)
    }

    fn config(&self) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                source: TapPoint::from(&self.source),
                scaling_mode: self.feed.blit.scaling_mode,
            },
        )
    }

    /// A source that cannot be resolved renders black until the channel
    /// comes back.
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

    fn feedback_request(&self) -> Option<&FeedbackSource> {
        Some(&self.source)
    }

    /// Renaming a channel has to move the tap's label with it, and this is the
    /// only place that sees both.
    fn bind_feedback(&mut self, view: Option<wgpu::TextureView>, label: &str) {
        self.feed.bind(view, None);
        if self.label != label {
            self.label = label.to_string();
        }
    }

    /// A tap names the channel it is watching. That channel is somebody else,
    /// so only a mapping that renamed *it* moves the tap.
    fn remap_ids(&mut self, map: &HashMap<String, String>) {
        if let FeedbackSource::Channel(uuid) = &mut self.source
            && let Some(new) = map.get(uuid)
        {
            uuid.clone_from(new);
        }
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
    fn a_saved_tap_reads_back_unchanged() {
        for json in [
            serde_json::json!({"type": "Tap", "source": {"kind": "master_program"}, "scaling_mode": "Fill"}),
            serde_json::json!({"type": "Tap", "source": {"kind": "channel", "uuid": "c1"}, "scaling_mode": "Fit"}),
        ] {
            let config: SourceConfig = serde_json::from_value(json.clone()).unwrap();
            let decoded: Config = decode_config(&config).unwrap();
            let back = encode_config(SOURCE_TYPE, &decoded);
            assert_eq!(serde_json::to_value(&back).unwrap(), json);
        }
    }

    #[test]
    fn an_unresolvable_channel_labels_by_uuid() {
        let source = FeedbackSource::Channel("gone".into());
        assert_eq!(point_label(&source, &[]), "Channel gone");
        let named = [("c1".to_string(), "Drums".to_string())];
        assert_eq!(
            point_label(&FeedbackSource::Channel("c1".into()), &named),
            "Drums"
        );
    }
}
