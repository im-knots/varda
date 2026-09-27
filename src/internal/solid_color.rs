//! Solid color: a flat fill, for a base layer or for testing effects.
//!
//! The smallest possible provider, and the one to copy when adding a new one.
//! See /spec/deck-source-providers.md.

use crate::source::{
    DeckSourceInstance, DeckSourceProvider, SourceConfig, SourceEnv, SourceFrame, SourceParamError,
    SourceParamSpec, SourceValue, clear_target, decode_config, encode_config, expect_color,
};
use anyhow::Result;
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "SolidColor";

static PARAMS: LazyLock<Vec<SourceParamSpec>> =
    LazyLock::new(|| vec![SourceParamSpec::color("color", "Color")]);

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    color: [f32; 4],
}

/// The solid color source type.
pub struct SolidColorProvider;

impl DeckSourceProvider for SolidColorProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "Solid Color"
    }

    fn icon(&self) -> &'static str {
        "■"
    }

    fn params(&self) -> &'static [SourceParamSpec] {
        &PARAMS
    }

    fn listed(&self, _query: &crate::source::SourceQuery) -> bool {
        false
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        _env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let Config { color } = decode_config(config)?;
        Ok(Box::new(SolidColor::new(color)))
    }

    fn identity(&self, _config: &SourceConfig) -> serde_json::Value {
        // Any color is the same source; a new color is a patch, not a rebuild.
        serde_json::Value::Null
    }
}

/// One solid-color deck.
pub struct SolidColor {
    color: [f32; 4],
}

impl SolidColor {
    pub fn new(color: [f32; 4]) -> Self {
        Self { color }
    }

    /// A config for a deck of `color`.
    pub fn config_for(color: [f32; 4]) -> SourceConfig {
        encode_config(SOURCE_TYPE, &Config { color })
    }
}

impl DeckSourceInstance for SolidColor {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        format!(
            "#{:02X}{:02X}{:02X}",
            (self.color[0] * 255.0) as u8,
            (self.color[1] * 255.0) as u8,
            (self.color[2] * 255.0) as u8,
        )
    }

    fn config(&self) -> SourceConfig {
        Self::config_for(self.color)
    }

    fn render(&mut self, frame: &mut SourceFrame) -> Result<()> {
        let [r, g, b, a] = self.color.map(f64::from);
        clear_target(frame, wgpu::Color { r, g, b, a }, "SolidColor Clear Pass");
        Ok(())
    }

    fn patch(&mut self, config: &SourceConfig) {
        if let Ok(Config { color }) = config.decode() {
            self.color = color;
        }
    }

    fn schema(&self) -> &'static [SourceParamSpec] {
        &PARAMS
    }

    fn param(&self, name: &str) -> Option<SourceValue> {
        (name == "color").then_some(SourceValue::Color(self.color))
    }

    fn set_param(&mut self, name: &str, value: &SourceValue) -> Result<(), SourceParamError> {
        match name {
            "color" => {
                self.color = expect_color(name, value)?;
                Ok(())
            }
            _ => Err(SourceParamError::Unknown(name.to_string())),
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
    fn config_round_trips_the_saved_shape() {
        let config: SourceConfig =
            serde_json::from_str(r#"{"type":"SolidColor","color":[1.0,0.5,0.0,1.0]}"#).unwrap();
        let Config { color } = decode_config(&config).unwrap();
        let deck = SolidColor::new(color);
        assert_eq!(deck.config(), config);
        assert_eq!(deck.label(), "#FF7F00");
    }

    #[test]
    fn the_color_is_a_typed_control() {
        let mut deck = SolidColor::new([0.0; 4]);
        deck.set_param("color", &SourceValue::Color([1.0, 0.0, 0.0, 1.0]))
            .unwrap();
        assert_eq!(
            deck.param("color"),
            Some(SourceValue::Color([1.0, 0.0, 0.0, 1.0]))
        );
        assert!(deck.set_param("color", &SourceValue::Float(0.5)).is_err());
        assert!(deck.set_param("nope", &SourceValue::Float(0.5)).is_err());
    }
}
