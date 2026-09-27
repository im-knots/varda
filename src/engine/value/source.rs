//! Deck source values: the persisted description of a source, the values its
//! controls take, and the schema and state a source type publishes.
//!
//! Every deck source type is a provider (see `crate::source`). This module is
//! what crosses the engine boundary about them: commands carry a
//! [`SourceConfig`], snapshots carry [`SourceTypeSnapshot`] and
//! [`DeckSourceSnapshot`]. Nothing here names a source type.
//!
//! See /spec/deck-source-providers.md.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// A deck source as it is saved and sent: a type id plus that type's fields.
///
/// Serializes flat, as `{"type": "<id>", ...fields}`, which is the shape every
/// scene written before providers existed already has. A provider decodes the
/// fields into its own private struct with [`SourceConfig::decode`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SourceConfig {
    source_type: String,
    fields: Map<String, Value>,
}

impl SourceConfig {
    /// An empty config of `source_type`.
    pub fn new(source_type: impl Into<String>) -> Self {
        Self {
            source_type: source_type.into(),
            fields: Map::new(),
        }
    }

    /// A config of `source_type` whose fields are `value`'s serialization.
    ///
    /// # Errors
    ///
    /// Fails when `value` does not serialize to a JSON object.
    pub fn encode<T: Serialize>(
        source_type: impl Into<String>,
        value: &T,
    ) -> Result<Self, serde_json::Error> {
        match to_json(value)? {
            Value::Object(fields) => Ok(Self {
                source_type: source_type.into(),
                fields,
            }),
            other => Err(serde::ser::Error::custom(format!(
                "a source config must be an object, got {other}"
            ))),
        }
    }

    /// Decode the fields into a provider's own config struct.
    ///
    /// # Errors
    ///
    /// Fails when a field is missing or has the wrong shape.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_value(Value::Object(self.fields.clone()))
    }

    /// Builder: set `key` to `value`'s serialization.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Serialize) -> Self {
        self.set(key, value);
        self
    }

    /// Set `key` to `value`'s serialization. A value that fails to serialize
    /// (which a plain data type never does) leaves the field unset.
    pub fn set(&mut self, key: &str, value: impl Serialize) {
        if let Ok(v) = to_json(&value) {
            self.fields.insert(key.to_string(), v);
        }
    }

    pub fn source_type(&self) -> &str {
        &self.source_type
    }

    pub fn fields(&self) -> &Map<String, Value> {
        &self.fields
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    /// A string field.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }
}

impl Serialize for SourceConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.fields.len() + 1))?;
        map.serialize_entry("type", &self.source_type)?;
        for (k, v) in &self.fields {
            if k != "type" {
                map.serialize_entry(k, v)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for SourceConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = Map::deserialize(deserializer)?;
        let source_type = match fields.remove("type") {
            Some(Value::String(t)) if !t.is_empty() => t,
            Some(_) => {
                return Err(serde::de::Error::custom(
                    "`type` must be a non-empty string",
                ));
            }
            None => return Err(serde::de::Error::missing_field("type")),
        };
        Ok(Self {
            source_type,
            fields,
        })
    }
}

/// `value` as JSON, with each `f32` at its shortest round-tripping spelling.
///
/// `serde_json::to_value` widens an `f32` to `f64` first, so a saved `0.6`
/// would come back as `0.6000000238418579`. Going through text keeps
/// `scene.json` exactly as the typed structs used to write it.
fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<Value, serde_json::Error> {
    serde_json::from_str(&serde_json::to_string(value)?)
}

impl utoipa::PartialSchema for SourceConfig {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{AdditionalProperties, ObjectBuilder, Type};
        ObjectBuilder::new()
            .description(Some(
                "A deck source: `type` names the source type (see `GET /api/library/sources`), \
                 and the remaining fields are that type's config.",
            ))
            .property(
                "type",
                ObjectBuilder::new()
                    .schema_type(Type::String)
                    .description(Some("Source type id")),
            )
            .required("type")
            .additional_properties(Some(AdditionalProperties::FreeForm(true)))
            .into()
    }
}

impl utoipa::ToSchema for SourceConfig {}

/// A value a source control takes.
///
/// Numeric controls (`Float`, `Choice`, `Toggle` kinds) speak the router's
/// normalized `0..1`, so a MIDI fader, the API and the GUI all write the same
/// number to the same path. The provider maps it onto its own range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum SourceValue {
    Bool(bool),
    Float(f32),
    Color([f32; 4]),
    Point([f32; 2]),
    Text(String),
}

impl SourceValue {
    /// The value as a normalized scalar: floats as-is, `true` as 1. Colors,
    /// points and text have no scalar reading.
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Float(v) => Some(*v),
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// What kind of control a source parameter is, which decides how every
/// consumer draws and writes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceParamKind {
    /// A continuous control, written normalized. `display_min`/`display_max`
    /// and `unit` are for labels only.
    Float {
        display_min: f32,
        display_max: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    /// A number in its own units (seconds, frames), written raw rather than
    /// normalized. Not a fader: it has no router path.
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        /// Drag speed per pixel, for a UI.
        step: f32,
    },
    /// On or off. A normalized write above 0.5 is on.
    Toggle,
    /// One of `options`, written as a normalized fader bucketed into
    /// `options.len()` equal steps.
    Choice {
        options: Vec<String>,
    },
    Color,
    /// Free text, such as a URL.
    Text,
    /// A momentary action: a write above 0.5 fires it.
    Action,
}

/// A group of parameters a consumer may draw as one richer control.
///
/// The set is closed and each hint is owned by the consumer that draws it; a
/// consumer that does not know a hint falls back to the plain parameters.
/// See /spec/deck-source-providers.md Decision 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WidgetHint {
    /// A clip transport. Reads `playing`, `position`, `duration`, `speed`,
    /// `effective_speed`, `position_offset`, `in_point`, `out_point`,
    /// `frame_rate` and `loop_mode` from the source's status info.
    Transport,
    /// An orbiting camera: yaw, pitch and zoom parameters.
    Orbit,
    /// A normalized crop rectangle: x, y, w, h parameters.
    CropRect,
}

/// One control a source type declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SourceParamSpec {
    /// Name the provider knows the control by.
    pub name: String,
    /// Router path under `deck/<uuid>/`, such as `video/speed`. `None` for a
    /// control that is written only as a typed value (a URL, a color).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    pub label: String,
    #[serde(flatten)]
    pub kind: SourceParamKind,
    /// Whether modulation and automation may drive it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub modulatable: bool,
    /// The richer control this parameter is part of, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<WidgetHint>,
}

impl SourceParamSpec {
    fn new(name: &str, label: &str, kind: SourceParamKind) -> Self {
        Self {
            name: name.to_string(),
            route: None,
            label: label.to_string(),
            kind,
            modulatable: false,
            widget: None,
        }
    }

    pub fn float(name: &str, label: &str, display_min: f32, display_max: f32) -> Self {
        Self::new(
            name,
            label,
            SourceParamKind::Float {
                display_min,
                display_max,
                unit: None,
            },
        )
    }

    pub fn number(name: &str, label: &str, unit: &str, step: f32) -> Self {
        Self::new(
            name,
            label,
            SourceParamKind::Number {
                unit: Some(unit.to_string()),
                step,
            },
        )
    }

    pub fn toggle(name: &str, label: &str) -> Self {
        Self::new(name, label, SourceParamKind::Toggle)
    }

    pub fn choice(name: &str, label: &str, options: &[&str]) -> Self {
        Self::new(
            name,
            label,
            SourceParamKind::Choice {
                options: options.iter().map(|s| (*s).to_string()).collect(),
            },
        )
    }

    pub fn color(name: &str, label: &str) -> Self {
        Self::new(name, label, SourceParamKind::Color)
    }

    pub fn text(name: &str, label: &str) -> Self {
        Self::new(name, label, SourceParamKind::Text)
    }

    pub fn action(name: &str, label: &str) -> Self {
        Self::new(name, label, SourceParamKind::Action)
    }

    /// Builder: expose the control on the router at `deck/<uuid>/<route>`.
    #[must_use]
    pub fn routed(mut self, route: &str) -> Self {
        self.route = Some(route.to_string());
        self
    }

    /// Builder: let modulation and automation drive it.
    #[must_use]
    pub fn modulatable(mut self) -> Self {
        self.modulatable = true;
        self
    }

    /// Builder: draw it as part of `widget`.
    #[must_use]
    pub fn in_widget(mut self, widget: WidgetHint) -> Self {
        self.widget = Some(widget);
        self
    }

    /// Builder: a unit label for a float control.
    #[must_use]
    pub fn unit(mut self, unit: &str) -> Self {
        if let SourceParamKind::Float { unit: u, .. } = &mut self.kind {
            *u = Some(unit.to_string());
        }
        self
    }
}

/// The live state of one deck's source, published in every snapshot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, utoipa::ToSchema)]
pub struct SourceStatus {
    /// Current value of each declared parameter, by name.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[schema(value_type = Object)]
    pub params: std::collections::BTreeMap<String, SourceValue>,
    /// Human-readable rendering of a parameter's value (`"30 fps"`), by name.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[schema(value_type = Object)]
    pub display: std::collections::BTreeMap<String, String>,
    /// Read-only state a widget hint or a client may want (a clip's position,
    /// the name of the device it is bound to).
    #[serde(skip_serializing_if = "Map::is_empty")]
    #[schema(value_type = Object)]
    pub info: Map<String, Value>,
    /// Whether the source is attached to what it reads (a window, a
    /// channel). `None` when the source has nothing to bind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound: Option<bool>,
    /// Whether frames are arriving. `None` when that is not meaningful.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected: Option<bool>,
}

/// A deck's source in a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct DeckSourceSnapshot {
    /// Source type id; its schema is in [`SourceTypeSnapshot`].
    #[serde(rename = "type")]
    pub source_type: String,
    /// False when this build cannot run the type, and the deck is a
    /// placeholder holding its config. See /spec/deck-source-providers.md
    /// Decision 4.
    pub available: bool,
    pub status: SourceStatus,
}

/// One row a library section offers.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct LibraryEntry {
    pub label: String,
    /// The deck this row creates when dropped on a channel.
    pub config: SourceConfig,
    /// Sub-heading inside the section (`Displays`, `Windows`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// A second line of detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Hover text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover: Option<String>,
    /// Live connection state, for entries that have one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected: Option<bool>,
    /// The user added this entry and may remove it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removable: bool,
    /// Draw the entry highlighted (Varda's own windows).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub highlight: bool,
}

impl LibraryEntry {
    pub fn new(label: impl Into<String>, config: SourceConfig) -> Self {
        Self {
            label: label.into(),
            config,
            group: None,
            detail: None,
            hover: None,
            connected: None,
            removable: false,
            highlight: false,
        }
    }
}

/// A notice a library section shows above its entries, with an optional
/// action the user can fire.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct LibraryNotice {
    pub text: String,
    /// `info`, `warning` or `error`.
    pub level: String,
    /// Label of the button that fires `action`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_label: Option<String>,
    /// Library action id sent back with `SourceLibraryAction`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// How a user creates a deck of this type that no entry lists.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibraryCreate {
    /// Pick a file; the chosen path is written into `field`.
    File {
        field: String,
        extensions: Vec<String>,
        label: String,
    },
    /// Fill a form; the values become a library entry the user then drags.
    Entry {
        fields: Vec<SourceParamSpec>,
        /// Initial form values, by field name.
        #[schema(value_type = Object)]
        defaults: Map<String, Value>,
        label: String,
        /// A line of guidance under the form.
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
    },
}

/// What a source type offers for creating decks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, utoipa::ToSchema)]
pub struct LibrarySection {
    pub entries: Vec<LibraryEntry>,
    pub notices: Vec<LibraryNotice>,
    /// Whether a rescan action is offered.
    pub rescan: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create: Option<LibraryCreate>,
    /// A short note next to the heading, such as `(SDK not found)`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A registered source type, published once per snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct SourceTypeSnapshot {
    #[serde(rename = "type")]
    pub source_type: String,
    pub label: String,
    pub icon: String,
    /// Whether this build and host can create it.
    pub available: bool,
    /// Why not, when unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    /// Whether the library panel lists it at all. False for types that exist
    /// only to be restored or created through the API.
    pub listed: bool,
    pub params: Vec<SourceParamSpec>,
    pub library: LibrarySection,
}

/// Scaling mode for sources that are drawn onto the deck from a texture of
/// their own size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum ScalingMode {
    /// Scale to fill the entire target, cropping edges if aspect ratio differs
    #[default]
    Fill,
    /// Scale to fit within the target, letterboxing if aspect ratio differs
    Fit,
    /// Stretch to exactly match target dimensions (may distort)
    Stretch,
    /// No scaling, center at native resolution
    Center,
}

impl ScalingMode {
    pub const ALL: [Self; 4] = [Self::Fill, Self::Fit, Self::Stretch, Self::Center];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Fit => "Fit",
            Self::Stretch => "Stretch",
            Self::Center => "Center",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_serializes_flat_with_its_type_first() {
        let config = SourceConfig::new("Image").with("path", "/a.png");
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(json, r#"{"type":"Image","path":"/a.png"}"#);
    }

    #[test]
    fn a_saved_tagged_config_reads_back_unchanged() {
        let json = r#"{"type":"Video","path":"/a.mov","speed":1.5,"loop_mode":"PingPong"}"#;
        let config: SourceConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.source_type(), "Video");
        assert_eq!(config.str("path"), Some("/a.mov"));
        assert_eq!(
            serde_json::to_value(&config).unwrap(),
            serde_json::from_str::<Value>(json).unwrap()
        );
    }

    #[test]
    fn a_config_without_a_type_is_refused() {
        assert!(serde_json::from_str::<SourceConfig>(r#"{"path":"/a"}"#).is_err());
        assert!(serde_json::from_str::<SourceConfig>(r#"{"type":""}"#).is_err());
    }

    #[test]
    fn decode_and_encode_round_trip_a_provider_struct() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Url {
            url: String,
        }
        let config = SourceConfig::encode("Hls", &Url { url: "u".into() }).unwrap();
        assert_eq!(config.decode::<Url>().unwrap(), Url { url: "u".into() });
    }

    #[test]
    fn source_values_read_untagged() {
        let v: Vec<SourceValue> =
            serde_json::from_str(r#"[true, 0.5, [1,0,0,1], [0.1,0.2], "x"]"#).unwrap();
        assert_eq!(
            v,
            vec![
                SourceValue::Bool(true),
                SourceValue::Float(0.5),
                SourceValue::Color([1.0, 0.0, 0.0, 1.0]),
                SourceValue::Point([0.1, 0.2]),
                SourceValue::Text("x".into()),
            ]
        );
    }

    #[test]
    fn f32_fields_keep_their_short_spelling() {
        #[derive(Serialize)]
        struct Crop {
            x: f32,
        }
        let config = SourceConfig::encode("Image", &Crop { x: 0.1 })
            .unwrap()
            .with("color", [0.6_f32, 0.9, 1.0]);
        assert_eq!(
            serde_json::to_string(&config).unwrap(),
            r#"{"type":"Image","x":0.1,"color":[0.6,0.9,1.0]}"#
        );
    }
}
