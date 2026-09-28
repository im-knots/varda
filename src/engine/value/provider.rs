//! The vocabulary every provider speaks: deck source providers and output
//! sink providers alike. A provider's persisted description
//! ([`ProviderConfig`]), the values its controls take ([`ControlValue`]), the
//! controls it declares ([`ControlSpec`]), their live state
//! ([`ControlStatus`]), and what its library offers ([`LibrarySection`]).
//! Nothing here names a provider type.
//!
//! See /spec/deck-source-providers.md and /spec/output-sink-providers.md
//! Decision 7.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// A provider's config as it is saved and sent: a type id plus that type's
/// fields. Decks name it `SourceConfig`, outputs `SinkConfig`.
///
/// Serializes flat, as `{"type": "<id>", ...fields}`, which is the shape every
/// scene written before providers existed already has. A provider decodes the
/// fields into its own private struct with [`ProviderConfig::decode`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderConfig {
    type_id: String,
    fields: Map<String, Value>,
}

impl ProviderConfig {
    /// An empty config of `type_id`.
    pub fn new(type_id: impl Into<String>) -> Self {
        Self {
            type_id: type_id.into(),
            fields: Map::new(),
        }
    }

    /// A config of `type_id` whose fields are `value`'s serialization.
    ///
    /// # Errors
    ///
    /// Fails when `value` does not serialize to a JSON object.
    pub fn encode<T: Serialize>(
        type_id: impl Into<String>,
        value: &T,
    ) -> Result<Self, serde_json::Error> {
        match to_json(value)? {
            Value::Object(fields) => Ok(Self {
                type_id: type_id.into(),
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

    pub fn type_id(&self) -> &str {
        &self.type_id
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

impl Serialize for ProviderConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.fields.len() + 1))?;
        map.serialize_entry("type", &self.type_id)?;
        for (k, v) in &self.fields {
            if k != "type" {
                map.serialize_entry(k, v)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ProviderConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = Map::deserialize(deserializer)?;
        let type_id = match fields.remove("type") {
            Some(Value::String(t)) if !t.is_empty() => t,
            Some(_) => {
                return Err(serde::de::Error::custom(
                    "`type` must be a non-empty string",
                ));
            }
            None => return Err(serde::de::Error::missing_field("type")),
        };
        Ok(Self { type_id, fields })
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

impl utoipa::PartialSchema for ProviderConfig {
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

impl utoipa::ToSchema for ProviderConfig {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("ProviderConfig")
    }
}

/// A value a source control takes.
///
/// Numeric controls (`Float`, `Choice`, `Toggle` kinds) speak the router's
/// normalized `0..1`, so a MIDI fader, the API and the GUI all write the same
/// number to the same path. The provider maps it onto its own range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum ControlValue {
    Bool(bool),
    Float(f32),
    Color([f32; 4]),
    Point([f32; 2]),
    Text(String),
}

impl ControlValue {
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
pub enum ControlKind {
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
    Choice { options: Vec<String> },
    /// An RGBA color. Routed, its channels are faders at `<route>/r` to
    /// `<route>/a`.
    Color,
    /// A 2D point, written as [`ControlValue::Point`]. Routed, its axes are
    /// faders at `<route>/x` and `<route>/y`, normalized against
    /// `display_min`/`display_max`.
    Point { display_min: f32, display_max: f32 },
    /// Free text, such as a URL. `multiline` text is edited as a block.
    Text {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
    },
    /// A write-only path to a file with one of `extensions`, written as
    /// [`ControlValue::Text`]. The source decides what the path means.
    File { extensions: Vec<String> },
    /// A momentary action: a write above 0.5 fires it.
    Action,
}

impl ControlKind {
    /// Whether values of this kind have components, and which.
    pub fn component_kind(&self) -> Option<super::param::ComponentKind> {
        match self {
            Self::Color => Some(super::param::ComponentKind::Color),
            Self::Point { .. } => Some(super::param::ComponentKind::Point),
            _ => None,
        }
    }
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
    /// A monitor picker over a text parameter holding the monitor's name.
    Monitor,
    /// An audio input picker over a text parameter holding the device's
    /// name, with none meaning silent.
    AudioDevice,
    /// Every control of a text deck, drawn as the GUI's dedicated text deck
    /// layout. See /spec/text-source.md § Deck controls.
    TextDeck,
    /// A font family picker over a text parameter holding the family name.
    /// The families come from the source type's library entry config
    /// (`font_families`).
    FontFamily,
}

/// One control a source type declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ControlSpec {
    /// Name the provider knows the control by.
    pub name: String,
    /// Router path under `deck/<uuid>/`, such as `video/speed`. `None` for a
    /// control that is written only as a typed value (a URL, a color).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    pub label: String,
    #[serde(flatten)]
    pub kind: ControlKind,
    /// Whether modulation and automation may drive it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub modulatable: bool,
    /// The richer control this parameter is part of, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<WidgetHint>,
}

impl ControlSpec {
    fn new(name: &str, label: &str, kind: ControlKind) -> Self {
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
            ControlKind::Float {
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
            ControlKind::Number {
                unit: Some(unit.to_string()),
                step,
            },
        )
    }

    pub fn toggle(name: &str, label: &str) -> Self {
        Self::new(name, label, ControlKind::Toggle)
    }

    pub fn choice(name: &str, label: &str, options: &[&str]) -> Self {
        Self::new(
            name,
            label,
            ControlKind::Choice {
                options: options.iter().map(|s| (*s).to_string()).collect(),
            },
        )
    }

    pub fn color(name: &str, label: &str) -> Self {
        Self::new(name, label, ControlKind::Color)
    }

    pub fn point(name: &str, label: &str, display_min: f32, display_max: f32) -> Self {
        Self::new(
            name,
            label,
            ControlKind::Point {
                display_min,
                display_max,
            },
        )
    }

    pub fn text(name: &str, label: &str) -> Self {
        Self::new(name, label, ControlKind::Text { multiline: false })
    }

    /// Text edited as a block of lines.
    pub fn text_block(name: &str, label: &str) -> Self {
        Self::new(name, label, ControlKind::Text { multiline: true })
    }

    pub fn file(name: &str, label: &str, extensions: &[&str]) -> Self {
        Self::new(
            name,
            label,
            ControlKind::File {
                extensions: extensions.iter().map(|e| (*e).to_string()).collect(),
            },
        )
    }

    pub fn action(name: &str, label: &str) -> Self {
        Self::new(name, label, ControlKind::Action)
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
        if let ControlKind::Float { unit: u, .. } = &mut self.kind {
            *u = Some(unit.to_string());
        }
        self
    }
}

/// The live state of one deck's source, published in every snapshot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ControlStatus {
    /// Current value of each declared parameter, by name.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[schema(value_type = Object)]
    pub params: std::collections::BTreeMap<String, ControlValue>,
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
    /// Controls that currently have no effect, with the reason, by name.
    /// Writes to them are still stored. See /spec/deck-source-providers.md
    /// § Inactive controls.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[schema(value_type = Object)]
    pub inactive: std::collections::BTreeMap<String, String>,
}

/// One row a library section offers.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct LibraryEntry {
    pub label: String,
    /// The deck this row creates when dropped on a channel.
    pub config: ProviderConfig,
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
    pub fn new(label: impl Into<String>, config: ProviderConfig) -> Self {
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
        fields: Vec<ControlSpec>,
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
    /// Choices a picker widget offers for a control, by control name, when
    /// they depend on the host (the installed font families).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[schema(value_type = Object)]
    pub options: std::collections::BTreeMap<String, Vec<String>>,
}

/// A registered source type, published once per snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ProviderTypeSnapshot {
    #[serde(rename = "type")]
    pub type_id: String,
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
    pub params: Vec<ControlSpec>,
    pub library: LibrarySection,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_serializes_flat_with_its_type_first() {
        let config = ProviderConfig::new("Image").with("path", "/a.png");
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(json, r#"{"type":"Image","path":"/a.png"}"#);
    }

    #[test]
    fn a_saved_tagged_config_reads_back_unchanged() {
        let json = r#"{"type":"Video","path":"/a.mov","speed":1.5,"loop_mode":"PingPong"}"#;
        let config: ProviderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.type_id(), "Video");
        assert_eq!(config.str("path"), Some("/a.mov"));
        assert_eq!(
            serde_json::to_value(&config).unwrap(),
            serde_json::from_str::<Value>(json).unwrap()
        );
    }

    #[test]
    fn a_config_without_a_type_is_refused() {
        assert!(serde_json::from_str::<ProviderConfig>(r#"{"path":"/a"}"#).is_err());
        assert!(serde_json::from_str::<ProviderConfig>(r#"{"type":""}"#).is_err());
    }

    #[test]
    fn decode_and_encode_round_trip_a_provider_struct() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Url {
            url: String,
        }
        let config = ProviderConfig::encode("Hls", &Url { url: "u".into() }).unwrap();
        assert_eq!(config.decode::<Url>().unwrap(), Url { url: "u".into() });
    }

    #[test]
    fn source_values_read_untagged() {
        let v: Vec<ControlValue> =
            serde_json::from_str(r#"[true, 0.5, [1,0,0,1], [0.1,0.2], "x"]"#).unwrap();
        assert_eq!(
            v,
            vec![
                ControlValue::Bool(true),
                ControlValue::Float(0.5),
                ControlValue::Color([1.0, 0.0, 0.0, 1.0]),
                ControlValue::Point([0.1, 0.2]),
                ControlValue::Text("x".into()),
            ]
        );
    }

    #[test]
    fn f32_fields_keep_their_short_spelling() {
        #[derive(Serialize)]
        struct Crop {
            x: f32,
        }
        let config = ProviderConfig::encode("Image", &Crop { x: 0.1 })
            .unwrap()
            .with("color", [0.6_f32, 0.9, 1.0]);
        assert_eq!(
            serde_json::to_string(&config).unwrap(),
            r#"{"type":"Image","x":0.1,"color":[0.6,0.9,1.0]}"#
        );
    }
}
