//! Screen and window capture as a deck source.

use super::backend::{
    CaptureConfig, CropRect, DEFAULT_CAPTURE_RATE, MAX_CAPTURE_RATE, MIN_CAPTURE_RATE,
    TargetIdentity,
};
use super::{CaptureId, ScreenCaptureManager, UNBOUND_CAPTURE_ID};
use crate::source::{
    ControlError, ControlSpec, ControlStatus, ControlValue, DeckSourceInstance, DeckSourceProvider,
    Feed, LibraryEntry, LibraryNotice, LibrarySection, SourceConfig, SourceControl, SourceEnv,
    SourceFrame, SourceQuery, WidgetHint, decode_config, downcast_mut, downcast_ref, encode_config,
    expect_norm, scaling_mode_spec,
};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "ScreenCapture";

/// Capture controls. All are MIDI-learnable, OSC-addressable and
/// macro-drivable; none is a modulation target.
static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    vec![
        ControlSpec::float("rate", "Rate", MIN_CAPTURE_RATE, MAX_CAPTURE_RATE)
            .unit("fps")
            .routed("capture/rate"),
        ControlSpec::float("crop_x", "Crop X", 0.0, 1.0)
            .routed("capture/crop_x")
            .in_widget(WidgetHint::CropRect),
        ControlSpec::float("crop_y", "Crop Y", 0.0, 1.0)
            .routed("capture/crop_y")
            .in_widget(WidgetHint::CropRect),
        ControlSpec::float("crop_w", "Crop W", 0.0, 1.0)
            .routed("capture/crop_w")
            .in_widget(WidgetHint::CropRect),
        ControlSpec::float("crop_h", "Crop H", 0.0, 1.0)
            .routed("capture/crop_h")
            .in_widget(WidgetHint::CropRect),
        ControlSpec::toggle("cursor", "Show cursor").routed("capture/cursor"),
        ControlSpec::toggle("exclude_varda", "Exclude Varda").routed("capture/exclude_varda"),
        scaling_mode_spec(),
    ]
});

/// A capture target in handle-free form, so a scene survives a reboot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetConfig {
    Display { name: String },
    Window { app: String, title: String },
}

impl TargetConfig {
    pub fn label(&self) -> String {
        match self {
            Self::Display { name } => name.clone(),
            Self::Window { app, title } if title.is_empty() => app.clone(),
            Self::Window { app, title } => format!("{app} — {title}"),
        }
    }

    fn is_display(&self) -> bool {
        matches!(self, Self::Display { .. })
    }
}

impl From<&TargetIdentity> for TargetConfig {
    fn from(id: &TargetIdentity) -> Self {
        match id {
            TargetIdentity::Display { label } => Self::Display {
                name: label.clone(),
            },
            TargetIdentity::Window { app, title } => Self::Window {
                app: app.clone(),
                title: title.clone(),
            },
        }
    }
}

impl From<&TargetConfig> for TargetIdentity {
    fn from(cfg: &TargetConfig) -> Self {
        match cfg {
            TargetConfig::Display { name } => Self::Display {
                label: name.clone(),
            },
            TargetConfig::Window { app, title } => Self::Window {
                app: app.clone(),
                title: title.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
struct CropConfig {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn default_rate() -> f32 {
    DEFAULT_CAPTURE_RATE
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    /// Matched by name on restore, not by platform handle: display ids and window
    /// numbers do not survive a reboot.
    target: TargetConfig,
    #[serde(default = "default_rate")]
    rate: f32,
    /// Absent means the full frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    crop: Option<CropConfig>,
    #[serde(default)]
    show_cursor: bool,
    /// `None` uses the per-target default: exclude Varda from a display capture,
    /// include it when the target is a Varda window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exclude_varda: Option<bool>,
    #[serde(default)]
    scaling_mode: crate::source::ScalingMode,
}

impl Config {
    fn capture_config(&self, scale_to: (u32, u32)) -> CaptureConfig {
        CaptureConfig {
            rate: self.rate,
            crop: self.crop.map_or_else(CropRect::default, |c| CropRect {
                x: c.x,
                y: c.y,
                w: c.w,
                h: c.h,
            }),
            show_cursor: self.show_cursor,
            exclude_varda: self
                .exclude_varda
                .unwrap_or_else(|| self.target.is_display()),
            scale_to: Some(scale_to),
        }
        .sanitized()
    }
}

#[derive(Default)]
pub struct ScreenCaptureProvider {
    /// Decks using each session, recounted over every deck each frame.
    holders: HashMap<CaptureId, u32>,
    /// Sessions a visible or cued deck shows. Only these upload.
    wanted: HashSet<CaptureId>,
}

impl ScreenCaptureProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens `config`'s target. With `keep_unbound`, a missing target gives an
    /// unbound deck instead of an error.
    fn open(
        config: &SourceConfig,
        env: &mut SourceEnv,
        keep_unbound: bool,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let config: Config = decode_config(config)?;
        let captures = env
            .services
            .get_mut::<ScreenCaptureManager>()
            .context("screen capture is not running")?;
        let identity = TargetIdentity::from(&config.target);
        let found = captures.find_target(&identity).cloned();
        if found.is_none() && !keep_unbound {
            anyhow::bail!(
                "No capture target matches '{}' — rescan and try again",
                config.target.label()
            );
        }
        // Capped to the deck size but kept in the target's shape, so the deck's
        // scaling mode still resolves the aspect. Uncapped, a 4K display moves
        // 33 MB per frame.
        let scale_to = found.as_ref().map_or((env.width, env.height), |info| {
            super::resample::fit_within(info.width, info.height, env.width, env.height)
        });
        let capture = config.capture_config(scale_to);

        let opened = match found {
            Some(info) => match captures.open(&info, capture.clone(), &env.gpu.device) {
                Ok(bound) => Some((info.label.clone(), bound)),
                Err(e) if keep_unbound => {
                    log::warn!(
                        "Capture target '{}' found but could not be opened: {e}",
                        info.label
                    );
                    None
                }
                Err(e) => return Err(anyhow::anyhow!("{e}")),
            },
            None => None,
        };
        let (label, id, size) = opened.map_or_else(
            || {
                log::warn!(
                    "Capture target '{}' not available — deck restored unbound",
                    config.target.label()
                );
                (
                    config.target.label(),
                    UNBOUND_CAPTURE_ID,
                    (env.width, env.height),
                )
            },
            |(label, (id, w, h))| (label, id, (w, h)),
        );
        let mut feed = Feed::new(env.gpu, "Screen Capture Blit Pass", size)?;
        feed.blit.scaling_mode = config.scaling_mode;
        Ok(Box::new(ScreenCapture {
            id,
            identity,
            label,
            config: capture,
            config_dirty: false,
            feed,
        }))
    }
}

impl DeckSourceProvider for ScreenCaptureProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "Screen Capture"
    }

    fn icon(&self) -> &'static str {
        "🖥"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn availability(&self, query: &SourceQuery) -> std::result::Result<(), String> {
        if query
            .services
            .get::<ScreenCaptureManager>()
            .is_some_and(ScreenCaptureManager::is_available)
        {
            Ok(())
        } else {
            Err("Screen capture is disabled in this build".into())
        }
    }

    /// Targets split into displays and windows, with Varda's own windows marked
    /// so self-capture is deliberate.
    fn library(&self, query: &SourceQuery) -> LibrarySection {
        let Some(captures) = query.services.get::<ScreenCaptureManager>() else {
            return LibrarySection::default();
        };
        let entries = captures
            .targets()
            .iter()
            .map(|t| {
                let target = TargetConfig::from(&t.identity());
                let is_display = target.is_display();
                let label = if t.is_varda {
                    format!("{} (Varda)", t.label)
                } else {
                    t.label.clone()
                };
                let mut row = LibraryEntry::new(label, ScreenCapture::config_for(target));
                row.group = Some(if is_display { "Displays" } else { "Windows" }.into());
                row.hover = Some(format!("{}×{}", t.width, t.height));
                row.highlight = t.is_varda;
                row
            })
            .collect();

        // macOS grants do not apply to the running process, so the text says so;
        // otherwise the feature looks broken after granting access.
        let notices = match captures.permission_state().as_str() {
            "granted" | "not_required" => Vec::new(),
            "denied" => vec![LibraryNotice {
                text: "Screen Recording access denied. Enable Varda under System Settings → \
                       Privacy & Security → Screen Recording, then restart Varda."
                    .into(),
                level: "error".into(),
                action_label: None,
                action: None,
            }],
            _ => vec![LibraryNotice {
                text: "Screen Recording access not granted. Varda must be restarted after \
                       granting."
                    .into(),
                level: "warning".into(),
                action_label: Some("Grant Screen Recording access".into()),
                action: Some("grant_permission".into()),
            }],
        };
        LibrarySection {
            entries,
            notices,
            rescan: true,
            ..LibrarySection::default()
        }
    }

    fn library_action(&mut self, action: &str, env: &mut SourceEnv) -> Result<()> {
        let captures = env
            .services
            .get_mut::<ScreenCaptureManager>()
            .context("screen capture is not running")?;
        match action {
            "rescan" => captures.scan_targets(),
            "grant_permission" => captures.request_permission(),
            _ => anyhow::bail!("Screen capture has no library action '{action}'"),
        }
        Ok(())
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        Self::open(config, env, false)
    }

    /// Unlike a camera, a missing capture target keeps the deck. Windows close
    /// often, and dropping the deck would lose its effect chain, opacity and
    /// mappings. The deck restores unbound and shows black.
    fn restore(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        Self::open(config, env, true)
    }

    /// The stored target, not the display label, so a window whose title changed
    /// is patched in place.
    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("target").cloned().unwrap_or_default()
    }

    fn observe(&mut self, instance: &dyn DeckSourceInstance, wanted: bool) {
        if let Some(deck) = downcast_ref::<ScreenCapture>(instance) {
            *self.holders.entry(deck.id).or_default() += 1;
            if wanted {
                self.wanted.insert(deck.id);
            }
        }
    }

    /// Stops sessions with no decks left. A deck dropped outside `release` (scene
    /// diff, undo, removed channel) never released its session. Then uploads only
    /// what is on screen, so an invisible capture deck costs nothing.
    fn tick(&mut self, env: &mut SourceEnv, _submit: &mut Vec<wgpu::CommandBuffer>) {
        if let Some(captures) = env.services.get_mut::<ScreenCaptureManager>() {
            captures.reconcile_holders(&self.holders);
            captures.update_selective(&env.gpu.device, &env.gpu.queue, &self.wanted);
        }
        self.holders.clear();
        self.wanted.clear();
    }

    fn prepare(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        let (Some(deck), Some(captures)) = (
            downcast_mut::<ScreenCapture>(instance),
            env.services.get_mut::<ScreenCaptureManager>(),
        ) else {
            return;
        };
        // Router and UI edits change the deck config; push changes to the capture
        // thread here.
        if std::mem::take(&mut deck.config_dirty) {
            captures.set_config(deck.id, deck.config.clone());
        }
        deck.feed.bind(
            captures.texture_view(deck.id).cloned(),
            captures.resolution(deck.id),
        );
    }

    fn release(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        if let (Some(deck), Some(captures)) = (
            downcast_ref::<ScreenCapture>(instance),
            env.services.get_mut::<ScreenCaptureManager>(),
        ) {
            captures.release(deck.id);
        }
    }

    fn status(&self, instance: &dyn DeckSourceInstance, query: &SourceQuery) -> ControlStatus {
        let Some(deck) = downcast_ref::<ScreenCapture>(instance) else {
            return instance.status();
        };
        let mut status = instance.status();
        status.bound = Some(deck.id != UNBOUND_CAPTURE_ID);
        status.connected = Some(
            query
                .services
                .get::<ScreenCaptureManager>()
                .is_some_and(|c| c.is_connected(deck.id)),
        );
        status
    }
}

pub struct ScreenCapture {
    id: CaptureId,
    identity: TargetIdentity,
    label: String,
    config: CaptureConfig,
    /// Set when a control changes `config`; the provider pushes it to the
    /// capture session next frame.
    config_dirty: bool,
    feed: Feed,
}

impl ScreenCapture {
    pub fn config_for(target: TargetConfig) -> SourceConfig {
        let exclude_varda = Some(target.is_display());
        encode_config(
            SOURCE_TYPE,
            &Config {
                target,
                rate: DEFAULT_CAPTURE_RATE,
                crop: None,
                show_cursor: false,
                exclude_varda,
                scaling_mode: crate::source::ScalingMode::default(),
            },
        )
    }

    fn rate_norm(&self) -> f32 {
        ((self.config.rate - MIN_CAPTURE_RATE) / (MAX_CAPTURE_RATE - MIN_CAPTURE_RATE))
            .clamp(0.0, 1.0)
    }
}

impl DeckSourceInstance for ScreenCapture {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        format!("🖥 {}", self.label)
    }

    fn config(&self) -> SourceConfig {
        let crop = self.config.crop;
        encode_config(
            SOURCE_TYPE,
            &Config {
                target: TargetConfig::from(&self.identity),
                rate: self.config.rate,
                crop: (!crop.is_full_frame()).then_some(CropConfig {
                    x: crop.x,
                    y: crop.y,
                    w: crop.w,
                    h: crop.h,
                }),
                show_cursor: self.config.show_cursor,
                exclude_varda: Some(self.config.exclude_varda),
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
            let scale_to = self.config.scale_to.unwrap_or((1920, 1080));
            self.config = config.capture_config(scale_to);
            self.config_dirty = true;
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
        let c = &self.config;
        Some(match name {
            "rate" => ControlValue::Float(self.rate_norm()),
            "crop_x" => ControlValue::Float(c.crop.x),
            "crop_y" => ControlValue::Float(c.crop.y),
            "crop_w" => ControlValue::Float(c.crop.w),
            "crop_h" => ControlValue::Float(c.crop.h),
            "cursor" => ControlValue::Bool(c.show_cursor),
            "exclude_varda" => ControlValue::Bool(c.exclude_varda),
            _ => return self.feed.param(name),
        })
    }

    fn set_param(&mut self, name: &str, value: &ControlValue) -> Result<(), ControlError> {
        let c = &mut self.config;
        match name {
            "rate" => {
                let v = expect_norm(name, value)?;
                c.rate = MIN_CAPTURE_RATE + v * (MAX_CAPTURE_RATE - MIN_CAPTURE_RATE);
            }
            "crop_x" => c.crop.x = expect_norm(name, value)?,
            "crop_y" => c.crop.y = expect_norm(name, value)?,
            "crop_w" => c.crop.w = expect_norm(name, value)?,
            "crop_h" => c.crop.h = expect_norm(name, value)?,
            // Bucketed so a MIDI fader can drive it like every other toggle.
            "cursor" => c.show_cursor = expect_norm(name, value)? > 0.5,
            "exclude_varda" => c.exclude_varda = expect_norm(name, value)? > 0.5,
            _ => return self.feed.set_param(name, value),
        }
        *c = c.clone().sanitized();
        self.config_dirty = true;
        Ok(())
    }

    fn status(&self) -> ControlStatus {
        let mut status = crate::source::status_from_params(self);
        status
            .display
            .insert("rate".into(), format!("{:.0} fps", self.config.rate));
        status.info.insert(
            "target_label".into(),
            TargetConfig::from(&self.identity).label().into(),
        );
        status.info.insert(
            "is_display".into(),
            matches!(self.identity, TargetIdentity::Display { .. }).into(),
        );
        status
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
    fn a_saved_capture_config_reads_back_unchanged() {
        let json = serde_json::json!({
            "type": "ScreenCapture",
            "target": {"kind": "window", "app": "Safari", "title": "Docs"},
            "rate": 24.0,
            "crop": {"x": 0.1, "y": 0.0, "w": 0.5, "h": 1.0},
            "show_cursor": true,
            "exclude_varda": false,
            "scaling_mode": "Fit"
        });
        let config: SourceConfig = serde_json::from_value(json.clone()).unwrap();
        let decoded: Config = decode_config(&config).unwrap();
        assert_eq!(decoded.target.label(), "Safari — Docs");
        let back = encode_config(SOURCE_TYPE, &decoded);
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
    }

    #[test]
    fn exclude_varda_defaults_by_target_kind() {
        let display: Config = serde_json::from_value(serde_json::json!({
            "target": {"kind": "display", "name": "Built-in"}
        }))
        .unwrap();
        assert!(display.capture_config((64, 64)).exclude_varda);
        let window: Config = serde_json::from_value(serde_json::json!({
            "target": {"kind": "window", "app": "Varda", "title": ""}
        }))
        .unwrap();
        assert!(!window.capture_config((64, 64)).exclude_varda);
    }
}
