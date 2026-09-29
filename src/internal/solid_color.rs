//! Solid color: a flat fill, for a base layer or for testing effects.
//!
//! The smallest provider; a template for new ones.

use crate::source::{
    ControlError, ControlSpec, ControlValue, DeckSourceInstance, DeckSourceProvider, SourceConfig,
    SourceControl, SourceEnv, SourceFrame, clear_target, decode_config, encode_config,
    expect_color,
};
use anyhow::Result;
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "SolidColor";

static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    vec![
        ControlSpec::color("color", "Color")
            .routed("color")
            .modulatable(),
    ]
});

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

    fn params(&self) -> &'static [ControlSpec] {
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
        // Changing the color patches the deck instead of rebuilding it.
        serde_json::Value::Null
    }
}

/// One solid-color deck.
pub struct SolidColor {
    color: [f32; 4],
    /// `color` with this frame's modulation applied.
    shown: [f32; 4],
}

impl SolidColor {
    pub fn new(color: [f32; 4]) -> Self {
        Self {
            color,
            shown: color,
        }
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
        let [r, g, b, a] = self.shown.map(f64::from);
        clear_target(frame, wgpu::Color { r, g, b, a }, "SolidColor Clear Pass");
        Ok(())
    }

    fn patch(&mut self, config: &SourceConfig) {
        if let Ok(Config { color }) = config.decode() {
            self.color = color;
            self.shown = color;
        }
    }

    fn control(&mut self, ctx: &mut SourceControl) {
        self.shown = ctx.modulated_color("color", self.color);
    }

    fn schema(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn param(&self, name: &str) -> Option<ControlValue> {
        (name == "color").then_some(ControlValue::Color(self.color))
    }

    fn set_param(&mut self, name: &str, value: &ControlValue) -> Result<(), ControlError> {
        match name {
            "color" => {
                self.color = expect_color(name, value)?;
                self.shown = self.color;
                Ok(())
            }
            _ => Err(ControlError::Unknown(name.to_string())),
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
        deck.set_param("color", &ControlValue::Color([1.0, 0.0, 0.0, 1.0]))
            .unwrap();
        assert_eq!(
            deck.param("color"),
            Some(ControlValue::Color([1.0, 0.0, 0.0, 1.0]))
        );
        assert!(deck.set_param("color", &ControlValue::Float(0.5)).is_err());
        assert!(deck.set_param("nope", &ControlValue::Float(0.5)).is_err());
    }

    #[test]
    fn a_component_route_writes_one_channel() {
        let mut deck = SolidColor::new([0.1, 0.2, 0.3, 1.0]);
        crate::source::write_route(&mut deck, "color/g", 0.75).unwrap();
        assert_eq!(
            deck.param("color"),
            Some(ControlValue::Color([0.1, 0.75, 0.3, 1.0]))
        );
        assert_eq!(crate::source::read_route(&deck, "color/b"), Some(0.3));
        // A color is one typed value, not a fader.
        assert!(matches!(
            crate::source::write_route(&mut deck, "color", 0.5),
            Err(ControlError::Invalid(_))
        ));
        // A point suffix on a color names nothing.
        assert!(matches!(
            crate::source::write_route(&mut deck, "color/x", 0.5),
            Err(ControlError::Unknown(_))
        ));
    }

    #[test]
    fn modulation_drives_components_not_the_whole_color() {
        let deck = SolidColor::new([0.0; 4]);
        assert!(crate::source::route_is_modulatable(&deck, "color/r"));
        assert!(crate::source::route_is_modulatable(&deck, "color/a"));
        assert!(!crate::source::route_is_modulatable(&deck, "color"));
    }

    #[test]
    fn modulated_color_is_drawn_but_not_saved() {
        use crate::modulation::{ModulationEngine, ModulationSource};
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.update_free_running(
            0.25,
            &crate::modulation::AudioValues::default(),
            &crate::modulation::AnalyzerValues::default(),
        );
        engine.assign("deck/d1/color/r", &uuid, 1.0);

        let mut deck = SolidColor::new([0.0, 0.0, 0.0, 1.0]);
        let mut scratch = String::new();
        let mut ctx = SourceControl::new(
            "d1",
            &engine,
            true,
            crate::source::SourceClock::default(),
            60,
            &mut scratch,
        );
        deck.control(&mut ctx);
        assert!(deck.shown[0] > 0.0, "red is modulated: {:?}", deck.shown);
        assert_eq!(deck.shown[1..], [0.0, 0.0, 1.0]);
        assert_eq!(deck.config(), SolidColor::config_for([0.0, 0.0, 0.0, 1.0]));
    }
}
