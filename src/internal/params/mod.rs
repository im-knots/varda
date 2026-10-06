//! Shader parameter system for ISF user inputs

use crate::engine::value::param::{Component, ComponentKind};
use crate::isf::ISFInput;
use crate::modulation::ModulationEngine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Parameter value types matching ISF input types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum ParamValue {
    Float(f32),
    Bool(bool),
    Long(i32),
    Color([f32; 4]),
    Point2D([f32; 2]),
}

impl ParamValue {
    /// `self` converted to the variant of `like`: numbers and bools convert
    /// between each other (a float rounds to a long; above one half is true),
    /// and a color or point needs the same shape. `None` when they cannot.
    /// Saved JSON numbers carry no type, so a long reads back as a float.
    pub fn as_type_of(self, like: &ParamValue) -> Option<ParamValue> {
        let number = match self {
            ParamValue::Float(v) => Some(f64::from(v)),
            ParamValue::Long(v) => Some(f64::from(v)),
            ParamValue::Bool(v) => Some(f64::from(u8::from(v))),
            ParamValue::Color(_) | ParamValue::Point2D(_) => None,
        };
        match (like, number) {
            (ParamValue::Float(_), Some(v)) => Some(ParamValue::Float(v as f32)),
            (ParamValue::Long(_), Some(v)) => Some(ParamValue::Long(v.round() as i32)),
            (ParamValue::Bool(_), Some(v)) => Some(ParamValue::Bool(v > 0.5)),
            (ParamValue::Color(_), None) if matches!(self, ParamValue::Color(_)) => Some(self),
            (ParamValue::Point2D(_), None) if matches!(self, ParamValue::Point2D(_)) => Some(self),
            _ => None,
        }
    }

    /// Create from ISF input default value
    pub fn from_isf_input(input: &ISFInput) -> Self {
        match input.input_type.as_str() {
            "float" => {
                let val = input
                    .default
                    .as_ref()
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0) as f32;
                ParamValue::Float(val)
            }
            // An event takes no default: it is false at rest.
            "event" => ParamValue::Bool(false),
            "bool" => {
                let val = input
                    .default
                    .as_ref()
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                ParamValue::Bool(val)
            }
            "long" => {
                let val = input
                    .default
                    .as_ref()
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0) as i32;
                ParamValue::Long(val)
            }
            "color" => {
                let arr = input.default.as_ref().and_then(|v| v.as_array()).map_or(
                    [1.0, 1.0, 1.0, 1.0],
                    |arr| {
                        let mut color = [1.0f32; 4];
                        for (i, val) in arr.iter().take(4).enumerate() {
                            color[i] = val.as_f64().unwrap_or(1.0) as f32;
                        }
                        color
                    },
                );
                ParamValue::Color(arr)
            }
            "point2D" => {
                let arr =
                    input
                        .default
                        .as_ref()
                        .and_then(|v| v.as_array())
                        .map_or([0.0, 0.0], |arr| {
                            let mut point = [0.0f32; 2];
                            for (i, val) in arr.iter().take(2).enumerate() {
                                point[i] = val.as_f64().unwrap_or(0.0) as f32;
                            }
                            point
                        });
                ParamValue::Point2D(arr)
            }
            _ => ParamValue::Float(0.0), // Default fallback
        }
    }
}

/// A preset's JSON value: a number, a bool, or an array of 4 (color) or 2 (point) numbers.
fn param_value_from_json(json: &serde_json::Value) -> Option<ParamValue> {
    let numbers = |array: &[serde_json::Value]| -> Option<Vec<f32>> {
        array.iter().map(|v| v.as_f64().map(|n| n as f32)).collect()
    };
    match json {
        serde_json::Value::Number(n) => Some(ParamValue::Float(n.as_f64()? as f32)),
        serde_json::Value::Bool(b) => Some(ParamValue::Bool(*b)),
        serde_json::Value::Array(array) => match *numbers(array)?.as_slice() {
            [r, g, b, a] => Some(ParamValue::Color([r, g, b, a])),
            [x, y] => Some(ParamValue::Point2D([x, y])),
            _ => None,
        },
        _ => None,
    }
}

/// Convert a single sRGB component to linear light.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

impl ParamValue {
    /// Size in bytes (aligned to 4 bytes for GPU)
    pub fn byte_size(&self) -> usize {
        match self {
            // Bool is stored as u32
            ParamValue::Float(_) | ParamValue::Bool(_) | ParamValue::Long(_) => 4,
            ParamValue::Color(_) => 16,
            ParamValue::Point2D(_) => 8,
        }
    }

    /// Write value to byte buffer
    pub fn write_bytes(&self, buffer: &mut Vec<u8>) {
        match self {
            ParamValue::Float(v) => buffer.extend_from_slice(&v.to_le_bytes()),
            ParamValue::Bool(v) => {
                buffer.extend_from_slice(&u32::from(*v).to_le_bytes());
            }
            ParamValue::Long(v) => buffer.extend_from_slice(&v.to_le_bytes()),
            ParamValue::Color(v) => {
                // Linearize RGB from sRGB (egui color picker values);
                // preserve alpha unchanged.
                for (i, f) in v.iter().enumerate() {
                    let val = if i < 3 { srgb_to_linear(*f) } else { *f };
                    buffer.extend_from_slice(&val.to_le_bytes());
                }
            }
            ParamValue::Point2D(v) => {
                for f in v {
                    buffer.extend_from_slice(&f.to_le_bytes());
                }
            }
        }
    }
}

/// The declared range of an input, falling back to the unit interval.
fn bounds(def: &ISFInput) -> (f32, f32) {
    (def.min.unwrap_or(0.0), def.max.unwrap_or(1.0))
}

/// Uniform integer in `[lo, hi]`.
fn pick_int(rng: &mut Rng, lo: i32, hi: i32) -> i32 {
    let span = hi.saturating_sub(lo).saturating_add(1).max(1);
    let step = i32::try_from(pick_index(rng, span as usize)).unwrap_or(0);
    lo.saturating_add(step)
}

/// Uniform index in `[0, len)`. `len` of zero yields zero.
fn pick_index(rng: &mut Rng, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    ((rng.unit() * len as f32) as usize).min(len - 1)
}

/// `SplitMix64`.
///
/// Written out instead of using `rand`, whose generators make no value-stability guarantee
/// across versions, so a seed keeps reproducing the same configuration.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`, from the top 24 bits so every result is exactly
    /// representable as an `f32`.
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }

    /// Approximately standard normal, summing six uniforms. Bounded at three sigma so a mutation
    /// never throws a parameter to its limit.
    fn normal(&mut self) -> f32 {
        let sum: f32 = (0..6).map(|_| self.unit()).sum();
        (sum - 3.0) * std::f32::consts::SQRT_2
    }
}

/// Shader parameters - stores current values and GPU buffer
pub struct ShaderParams {
    /// Parameter names in order (for consistent buffer layout)
    pub param_order: Vec<String>,
    /// Current values
    pub values: HashMap<String, ParamValue>,
    /// ISF input definitions (for UI metadata: min/max/label)
    pub definitions: HashMap<String, ISFInput>,
    /// GPU buffer (created on demand)
    buffer: Option<wgpu::Buffer>,
    /// Buffer needs re-upload
    dirty: bool,
    /// Reusable scratch buffer for serialization, avoiding per-frame allocation. Capacity settles
    /// at `buffer_size()` after the first frame.
    scratch: Vec<u8>,
    /// Reusable scratch string for modulation key construction (avoids per-param allocation).
    mod_key_scratch: String,
    /// The `SPECIALIZE` inputs, in `INPUTS` order.
    specialized: Vec<String>,
    /// The `event` inputs, in `INPUTS` order.
    events: Vec<EventInput>,
    /// The inputs a preprocessor writes (`WRITES`).
    written: Vec<String>,
}

/// A momentary `event` input. Its value is true for one rendered frame after a trigger.
struct EventInput {
    name: String,
    /// The last normalized write was above one half.
    held: bool,
}

impl ShaderParams {
    /// Create from ISF inputs
    pub fn from_inputs(inputs: &[ISFInput]) -> Self {
        let mut param_order = Vec::new();
        let mut values = HashMap::new();
        let mut definitions = HashMap::new();

        for input in inputs {
            // Skip non-parameter types (image, audio, audioFFT handled separately)
            match input.input_type.as_str() {
                "float" | "bool" | "event" | "long" | "color" | "point2D" => {
                    param_order.push(input.name.clone());
                    values.insert(input.name.clone(), ParamValue::from_isf_input(input));
                    definitions.insert(input.name.clone(), input.clone());
                }
                _ => {} // Skip image, audio, audioFFT
            }
        }

        let specialized = param_order
            .iter()
            .filter(|name| definitions[*name].specialize)
            .cloned()
            .collect();
        let events = param_order
            .iter()
            .filter(|name| definitions[*name].input_type == "event")
            .map(|name| EventInput {
                name: name.clone(),
                held: false,
            })
            .collect();
        Self {
            param_order,
            values,
            definitions,
            buffer: None,
            dirty: true,
            scratch: Vec::new(),
            mod_key_scratch: String::new(),
            specialized,
            events,
            written: Vec::new(),
        }
    }

    /// The parameters of a shader's `INPUTS`, with the inputs its preprocessors write.
    pub fn from_metadata(metadata: &crate::isf::ISFMetadata) -> Self {
        let mut params = Self::from_inputs(metadata.inputs.as_deref().unwrap_or(&[]));
        params.written = metadata
            .written_inputs()
            .filter(|name| params.values.contains_key(*name))
            .map(str::to_owned)
            .collect();
        params
    }

    /// Whether a preprocessor writes `name`. Only [`Self::write_derived`] changes it.
    pub fn is_written(&self, name: &str) -> bool {
        self.written.iter().any(|w| w == name)
    }

    /// Whether `name` is an event or a written input. Neither is a setting: they are not
    /// saved, restored, explored, modulated or recorded.
    pub fn is_transient(&self, name: &str) -> bool {
        self.is_event(name) || self.is_written(name)
    }

    /// Set a written input from a preprocessor output, converted to its type and clamped to
    /// its declared range. Other inputs are left alone.
    pub fn write_derived(&mut self, name: &str, value: f32) {
        if !self.is_written(name) {
            return;
        }
        let value = match self.definitions.get(name) {
            Some(ISFInput {
                min: Some(lo),
                max: Some(hi),
                ..
            }) if lo <= hi => value.clamp(*lo, *hi),
            _ => value,
        };
        self.store(name, ParamValue::Float(value));
    }

    /// Whether `name` is an `event` input.
    pub fn is_event(&self, name: &str) -> bool {
        self.events.iter().any(|e| e.name == name)
    }

    /// A normalized write to an event: fires when the value rises past one half. A falling
    /// value sets nothing.
    pub fn press_event(&mut self, name: &str, value: f32) {
        let Some(event) = self.events.iter_mut().find(|e| e.name == name) else {
            return;
        };
        let pressed = value > 0.5;
        let fire = pressed && !event.held;
        event.held = pressed;
        if fire {
            self.set(name, ParamValue::Bool(true));
        }
    }

    /// Reset every event to false. Called once a frame has used them.
    pub fn clear_events(&mut self) {
        for event in &self.events {
            if let Some(ParamValue::Bool(v)) = self.values.get_mut(&event.name)
                && *v
            {
                *v = false;
                self.dirty = true;
            }
        }
    }

    /// The values a scene or preset saves. Events and written inputs are left out.
    pub fn saved_values(&self) -> HashMap<String, ParamValue> {
        self.values
            .iter()
            .filter(|(name, _)| !self.is_transient(name))
            .map(|(name, value)| (name.clone(), *value))
            .collect()
    }

    /// Apply saved values. Events and written inputs are skipped, so a loaded event stays
    /// false. Input presets are not applied: the saved values already hold their result.
    pub fn restore(&mut self, saved: &HashMap<String, ParamValue>) {
        for (name, value) in saved {
            if !self.is_transient(name) {
                self.write(name, *value);
            }
        }
    }

    /// The pipeline constants of the `SPECIALIZE` inputs, by `constant_id`,
    /// from the base values: modulation never reaches them. A float rounds to
    /// the nearest integer.
    pub fn specialization_constants(&self) -> impl Iterator<Item = f64> + '_ {
        self.specialized
            .iter()
            .map(|name| match self.values.get(name) {
                Some(ParamValue::Long(v)) => f64::from(*v),
                Some(ParamValue::Bool(v)) => f64::from(u8::from(*v)),
                Some(ParamValue::Float(v)) => f64::from(*v).round(),
                _ => 0.0,
            })
    }

    /// Check if this has any parameters
    pub fn is_empty(&self) -> bool {
        self.param_order.is_empty()
    }

    /// A float or long input's base value, unmodulated.
    pub fn base_number(&self, name: &str) -> Option<f64> {
        match self.values.get(name) {
            Some(ParamValue::Float(v)) => Some(f64::from(*v)),
            Some(ParamValue::Long(v)) => Some(f64::from(*v)),
            _ => None,
        }
    }

    /// Get a float value
    pub fn get_float(&self, name: &str) -> Option<f32> {
        match self.values.get(name) {
            Some(ParamValue::Float(v)) => Some(*v),
            _ => None,
        }
    }

    /// Get a float value with modulation applied, as uploaded to the GPU this frame.
    ///
    /// Takes `&mut self` to reuse the modulation-key scratch string; this runs per frame and must
    /// not allocate.
    pub fn get_float_modulated(
        &mut self,
        name: &str,
        modulation: &ModulationEngine,
        param_prefix: Option<&str>,
    ) -> Option<f32> {
        let base = self.values.get(name)?;
        if !matches!(base, ParamValue::Float(_)) {
            return None;
        }

        self.mod_key_scratch.clear();
        if let Some(prefix) = param_prefix {
            self.mod_key_scratch.push_str(prefix);
            self.mod_key_scratch
                .push(crate::engine::value::param::PARAM_KEY_SEPARATOR);
        }
        self.mod_key_scratch.push_str(name);

        match Self::apply_modulation_to_value_with_key(
            &mut self.mod_key_scratch,
            base,
            modulation,
            self.definitions.get(name),
        ) {
            ParamValue::Float(v) => Some(v),
            _ => None,
        }
    }

    /// Get one parameter exactly as it is uploaded this frame.
    ///
    /// Frameless preprocessors use this instead of copying the whole uniform
    /// block. The returned value retains its declared type.
    pub(crate) fn get_modulated(
        &mut self,
        name: &str,
        modulation: &ModulationEngine,
        param_prefix: Option<&str>,
    ) -> Option<ParamValue> {
        let base = self.values.get(name)?;
        self.mod_key_scratch.clear();
        if let Some(prefix) = param_prefix {
            self.mod_key_scratch.push_str(prefix);
            self.mod_key_scratch
                .push(crate::engine::value::param::PARAM_KEY_SEPARATOR);
        }
        self.mod_key_scratch.push_str(name);
        Some(Self::apply_modulation_to_value_with_key(
            &mut self.mod_key_scratch,
            base,
            modulation,
            self.definitions.get(name),
        ))
    }

    /// Express a value as a 0.0–1.0 fraction of its declared range.
    ///
    /// Used by the arrangement's live override, whose re-arm ramp compares the performer's value
    /// with an envelope's normalized output. Returns `None` for parameters with no meaningful
    /// range.
    pub fn normalize(&self, name: &str, value: &ParamValue) -> Option<f32> {
        let ParamValue::Float(v) = value else {
            return None;
        };
        let definition = self.definitions.get(name)?;
        let (min, max) = (definition.min?, definition.max?);
        if (max - min).abs() < f32::EPSILON {
            return None;
        }
        Some(((v - min) / (max - min)).clamp(0.0, 1.0))
    }

    /// Set a float value
    pub fn set_float(&mut self, name: &str, value: f32) {
        if let Some(ParamValue::Float(v)) = self.values.get_mut(name) {
            *v = value;
            self.dirty = true;
        }
    }

    /// Get a bool value
    pub fn get_bool(&self, name: &str) -> Option<bool> {
        match self.values.get(name) {
            Some(ParamValue::Bool(v)) => Some(*v),
            _ => None,
        }
    }

    /// Set a bool value. An event only accepts true.
    pub fn set_bool(&mut self, name: &str, value: bool) {
        if !value && self.is_event(name) {
            return;
        }
        if let Some(ParamValue::Bool(v)) = self.values.get_mut(name) {
            *v = value;
            self.dirty = true;
        }
    }

    /// Get a color value
    pub fn get_color(&self, name: &str) -> Option<[f32; 4]> {
        match self.values.get(name) {
            Some(ParamValue::Color(v)) => Some(*v),
            _ => None,
        }
    }

    /// Set a color value
    pub fn set_color(&mut self, name: &str, value: [f32; 4]) {
        if let Some(ParamValue::Color(v)) = self.values.get_mut(name) {
            *v = value;
            self.dirty = true;
        }
    }

    /// Get a long (enum) value
    pub fn get_long(&self, name: &str) -> Option<i32> {
        match self.values.get(name) {
            Some(ParamValue::Long(v)) => Some(*v),
            _ => None,
        }
    }

    /// Set a long value. A new value applies the input's preset.
    pub fn set_long(&mut self, name: &str, value: i32) {
        let value = self
            .long_range(name)
            .map_or(value, |(lo, hi)| value.clamp(lo, hi));
        if let Some(ParamValue::Long(v)) = self.values.get_mut(name) {
            let changed = *v != value;
            *v = value;
            self.dirty = true;
            if changed {
                self.apply_preset(name, value);
            }
        }
    }

    /// Get a point2D value
    pub fn get_point2d(&self, name: &str) -> Option<[f32; 2]> {
        match self.values.get(name) {
            Some(ParamValue::Point2D(v)) => Some(*v),
            _ => None,
        }
    }

    /// Set a point2D value
    pub fn set_point2d(&mut self, name: &str, value: [f32; 2]) {
        if let Some(ParamValue::Point2D(v)) = self.values.get_mut(name) {
            *v = value;
            self.dirty = true;
        }
    }

    /// Calculate total buffer size (with std140 alignment)
    pub fn buffer_size(&self) -> usize {
        let mut size = 0usize;
        for name in &self.param_order {
            if let Some(value) = self.values.get(name) {
                // std140 alignment rules
                let alignment = match value {
                    ParamValue::Float(_) | ParamValue::Bool(_) | ParamValue::Long(_) => 4,
                    ParamValue::Point2D(_) => 8,
                    ParamValue::Color(_) => 16,
                };
                // Align to required alignment
                size = (size + alignment - 1) & !(alignment - 1);
                size += value.byte_size();
            }
        }
        // Minimum 16 bytes for wgpu, align to 16
        (size.max(16) + 15) & !15
    }

    /// Serialize parameter values into the reusable scratch buffer (std140 layout).
    /// Returns a slice valid until the next `build_*` or mutable call. Allocation-free after the
    /// first call.
    pub fn build_buffer_data(&mut self) -> &[u8] {
        self.scratch.clear();
        self.scratch.reserve(self.buffer_size());
        for name in &self.param_order {
            if let Some(value) = self.values.get(name) {
                let alignment = match value {
                    ParamValue::Float(_) | ParamValue::Bool(_) | ParamValue::Long(_) => 4,
                    ParamValue::Point2D(_) => 8,
                    ParamValue::Color(_) => 16,
                };
                while !self.scratch.len().is_multiple_of(alignment) {
                    self.scratch.push(0);
                }
                value.write_bytes(&mut self.scratch);
            }
        }
        while self.scratch.len() < 16 {
            self.scratch.push(0);
        }
        while !self.scratch.len().is_multiple_of(16) {
            self.scratch.push(0);
        }
        &self.scratch
    }

    /// Create or get GPU buffer
    ///
    /// # Panics
    ///
    /// Panics only if the buffer slot is still empty after this call populated
    /// it, which cannot happen.
    pub fn ensure_buffer(&mut self, device: &wgpu::Device) -> &wgpu::Buffer {
        if self.buffer.is_none() {
            let data = self.build_buffer_data().to_vec();
            self.buffer = Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Shader Params Buffer"),
                    contents: &data,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                }),
            );
            self.dirty = false;
        }
        self.buffer
            .as_ref()
            .expect("ensure_buffer must be called before buffer() access")
    }

    /// Update GPU buffer if dirty
    pub fn update_buffer(&mut self, queue: &wgpu::Queue) {
        if self.dirty {
            // Reborrow: build into scratch, then write to GPU buffer.
            self.build_buffer_data();
            if let Some(buffer) = &self.buffer {
                queue.write_buffer(buffer, 0, &self.scratch);
            }
            self.dirty = false;
        }
    }

    /// The GPU buffer, if created.
    pub fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }

    /// Read-only access to the last serialized scratch data.
    /// Valid after a `build_buffer_data` or `build_modulated_buffer_data` call.
    pub fn scratch(&self) -> &[u8] {
        &self.scratch
    }

    /// Mark as needing re-upload
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Generic set method for any parameter value. An event only accepts `Bool(true)`, which
    /// fires it. A `long` input that changes value applies its preset.
    pub fn set(&mut self, name: &str, value: ParamValue) {
        let before = self.get_long(name);
        self.write(name, value);
        if let Some(after) = self.get_long(name)
            && before != Some(after)
        {
            self.apply_preset(name, after);
        }
    }

    /// `set` without input presets. A written input ignores it.
    fn write(&mut self, name: &str, value: ParamValue) {
        if self.is_written(name)
            || (self.is_event(name) && !matches!(value, ParamValue::Bool(true)))
        {
            return;
        }
        self.store(name, value);
    }

    /// Store `value` as the declared type of `name`, a `long` clamped to its range.
    fn store(&mut self, name: &str, value: ParamValue) {
        let range = self.long_range(name);
        let Some(current) = self.values.get_mut(name) else {
            return;
        };
        if let Some(typed) = value.as_type_of(current) {
            *current = match (typed, range) {
                (ParamValue::Long(v), Some((lo, hi))) => ParamValue::Long(v.clamp(lo, hi)),
                (typed, _) => typed,
            };
            self.dirty = true;
        }
    }

    /// The declared `MIN` and `MAX` of a `long` input, when it has both and
    /// they are in order.
    fn long_range(&self, name: &str) -> Option<(i32, i32)> {
        let def = self.definitions.get(name)?;
        if def.input_type != "long" {
            return None;
        }
        let (lo, hi) = (def.min? as i32, def.max? as i32);
        (lo <= hi).then_some((lo, hi))
    }

    /// Write the `PRESETS` entry for `value` of input `name` into the other inputs. Names that
    /// are not inputs and values of the wrong shape are skipped.
    fn apply_preset(&mut self, name: &str, value: i32) {
        let Some(preset) = self.definitions.get(name).and_then(|d| d.preset(value)) else {
            return;
        };
        let writes: Vec<(String, ParamValue)> = preset
            .iter()
            .filter(|(target, _)| target.as_str() != name)
            .filter_map(|(target, json)| Some((target.clone(), param_value_from_json(json)?)))
            .collect();
        for (target, value) in writes {
            self.write(&target, value);
        }
    }

    /// Whether the parameter `name` is a color or a point.
    pub fn component_kind(&self, name: &str) -> Option<ComponentKind> {
        match self.values.get(name)? {
            ParamValue::Color(_) => Some(ComponentKind::Color),
            ParamValue::Point2D(_) => Some(ComponentKind::Point),
            _ => None,
        }
    }

    /// One component of a color or point, as a fader reads it: a color
    /// channel as is, a point axis as its raw value (point2D inputs declare
    /// no scalar range).
    pub fn component(&self, name: &str, component: Component) -> Option<f32> {
        match self.values.get(name)? {
            ParamValue::Color(c) => component.read(c),
            ParamValue::Point2D(p) => component.read(p),
            _ => None,
        }
    }

    /// Replace one component of a color or point. A color channel is clamped
    /// to 0.0–1.0. False when `name` is not that kind of parameter.
    pub fn set_component(&mut self, name: &str, component: Component, value: f32) -> bool {
        let written = match self.values.get_mut(name) {
            Some(ParamValue::Color(c)) => component.write(c, value.clamp(0.0, 1.0)),
            Some(ParamValue::Point2D(p)) => component.write(p, value),
            _ => false,
        };
        self.dirty |= written;
        written
    }

    /// Reset all parameters to their default values from ISF definitions
    pub fn reset_to_defaults(&mut self) {
        for name in &self.param_order {
            if let Some(definition) = self.definitions.get(name) {
                let default_value = ParamValue::from_isf_input(definition);
                self.values.insert(name.clone(), default_value);
            }
        }
        self.dirty = true;
    }

    /// Whether a parameter belongs to the group this operation is scoped to.
    /// `None` scope means every parameter.
    fn in_scope(&self, name: &str, scope: Option<&str>) -> bool {
        match scope {
            None => true,
            Some(group) => {
                self.definitions.get(name).and_then(|d| d.group.as_deref()) == Some(group)
            }
        }
    }

    /// Whether exploration may touch this parameter.
    ///
    /// A parameter with no declared range has nothing to draw from. Colors are excluded because
    /// random colors look muddy and palettes are deliberate choices. Events and written inputs
    /// are not settings.
    fn is_explorable(&self, name: &str) -> bool {
        let Some(def) = self.definitions.get(name) else {
            return false;
        };
        if self.is_transient(name) {
            return false;
        }
        let bounded = def.min.is_some() && def.max.is_some();
        match self.values.get(name) {
            Some(ParamValue::Bool(_)) => true,
            Some(ParamValue::Long(_)) => bounded || !def.choices().is_empty(),
            Some(ParamValue::Float(_) | ParamValue::Point2D(_)) => bounded,
            Some(ParamValue::Color(_)) | None => false,
        }
    }

    /// Draw every in-scope parameter afresh from its declared range.
    ///
    /// `seed` makes the result reproducible, so a discarded configuration can be recovered.
    pub fn randomize(&mut self, scope: Option<&str>, seed: u64) {
        self.explore(scope, seed, |rng, def, current| match current {
            ParamValue::Float(_) => {
                let (lo, hi) = bounds(def);
                ParamValue::Float(lo + rng.unit() * (hi - lo))
            }
            ParamValue::Bool(_) => ParamValue::Bool(rng.unit() < 0.5),
            ParamValue::Long(_) => {
                let choices = def.choices();
                if choices.is_empty() {
                    let (lo, hi) = bounds(def);
                    ParamValue::Long(pick_int(rng, lo as i32, hi as i32))
                } else {
                    ParamValue::Long(choices[pick_index(rng, choices.len())].0)
                }
            }
            ParamValue::Point2D(_) => {
                let (lo, hi) = bounds(def);
                ParamValue::Point2D([lo + rng.unit() * (hi - lo), lo + rng.unit() * (hi - lo)])
            }
            other @ ParamValue::Color(_) => other,
        });
    }

    /// Nudge every in-scope parameter by a fraction of its range.
    ///
    /// Takes small steps from a configuration that already works. `amount` is a fraction of each
    /// parameter's declared range; results are clamped to that range.
    pub fn mutate(&mut self, scope: Option<&str>, amount: f32, seed: u64) {
        let amount = amount.clamp(0.0, 1.0);
        self.explore(scope, seed, |rng, def, current| match current {
            ParamValue::Float(v) => {
                let (lo, hi) = bounds(def);
                ParamValue::Float((v + amount * (hi - lo) * rng.normal()).clamp(lo, hi))
            }
            ParamValue::Bool(v) => ParamValue::Bool(if rng.unit() < amount { !v } else { v }),
            ParamValue::Long(v) => {
                if rng.unit() >= amount {
                    return ParamValue::Long(v);
                }
                // Step to a neighbor instead of jumping anywhere, so a mutation stays a small move.
                let down = rng.unit() < 0.5;
                let choices = def.choices();
                if choices.is_empty() {
                    let (lo, hi) = bounds(def);
                    let step = if down { -1 } else { 1 };
                    ParamValue::Long(v.saturating_add(step).clamp(lo as i32, hi as i32))
                } else {
                    let at = choices.iter().position(|c| c.0 == v).unwrap_or(0);
                    let next = if down {
                        at.saturating_sub(1)
                    } else {
                        (at + 1).min(choices.len() - 1)
                    };
                    ParamValue::Long(choices[next].0)
                }
            }
            ParamValue::Point2D(p) => {
                let (lo, hi) = bounds(def);
                let span = amount * (hi - lo);
                ParamValue::Point2D([
                    (p[0] + span * rng.normal()).clamp(lo, hi),
                    (p[1] + span * rng.normal()).clamp(lo, hi),
                ])
            }
            other @ ParamValue::Color(_) => other,
        });
    }

    /// Walk the parameters in buffer order, applying `draw` to each one that is in
    /// scope and explorable.
    ///
    /// Order matters: the generator is consumed in `param_order`, so the same seed
    /// and the same shader reproduce the same values.
    fn explore<F>(&mut self, scope: Option<&str>, seed: u64, draw: F)
    where
        F: Fn(&mut Rng, &ISFInput, ParamValue) -> ParamValue,
    {
        let mut rng = Rng::new(seed);
        let order = std::mem::take(&mut self.param_order);
        for name in &order {
            if !self.in_scope(name, scope) || !self.is_explorable(name) {
                continue;
            }
            let Some(def) = self.definitions.get(name) else {
                continue;
            };
            let Some(current) = self.values.get(name).copied() else {
                continue;
            };
            let next = draw(&mut rng, def, current);
            self.values.insert(name.clone(), next);
        }
        self.param_order = order;
        self.dirty = true;
    }

    /// Serialize parameter values with modulation applied into the reusable scratch buffer.
    /// Returns a slice valid until the next `build_*` or mutable call.
    pub fn build_modulated_buffer_data(
        &mut self,
        modulation: &ModulationEngine,
        param_prefix: Option<&str>,
    ) -> &[u8] {
        self.scratch.clear();
        self.scratch.reserve(self.buffer_size());

        for idx in 0..self.param_order.len() {
            let name = &self.param_order[idx];
            if let Some(value) = self.values.get(name) {
                let alignment = match value {
                    ParamValue::Float(_) | ParamValue::Bool(_) | ParamValue::Long(_) => 4,
                    ParamValue::Point2D(_) => 8,
                    ParamValue::Color(_) => 16,
                };
                while !self.scratch.len().is_multiple_of(alignment) {
                    self.scratch.push(0);
                }

                // Build modulation key in reusable scratch string (zero alloc after first frame)
                self.mod_key_scratch.clear();
                if let Some(prefix) = param_prefix {
                    self.mod_key_scratch.push_str(prefix);
                    self.mod_key_scratch
                        .push(crate::engine::value::param::PARAM_KEY_SEPARATOR);
                }
                self.mod_key_scratch.push_str(name);

                let modulated = Self::apply_modulation_to_value_with_key(
                    &mut self.mod_key_scratch,
                    value,
                    modulation,
                    self.definitions.get(name.as_str()),
                );
                modulated.write_bytes(&mut self.scratch);
            }
        }
        while self.scratch.len() < 16 {
            self.scratch.push(0);
        }
        while !self.scratch.len().is_multiple_of(16) {
            self.scratch.push(0);
        }
        &self.scratch
    }

    /// Apply modulation to a parameter value using a pre-built modulation key.
    /// Color and point components resolve `<key>/r` and the like; `mod_key`
    /// is extended for each and restored before returning.
    fn apply_modulation_to_value_with_key(
        mod_key: &mut String,
        value: &ParamValue,
        modulation: &ModulationEngine,
        definition: Option<&ISFInput>,
    ) -> ParamValue {
        match value {
            ParamValue::Float(base) => {
                let resolved = modulation.resolve(mod_key);
                if resolved.additive == 0.0 && resolved.absolute.is_none() {
                    return *value;
                }
                let (min_val, max_val) = definition.map_or((0.0, 1.0), |d| {
                    let min = d.min.unwrap_or(0.0);
                    let max = d.max.unwrap_or(1.0);
                    (min, max)
                });
                let range = max_val - min_val;
                // An absolute source replaces the base before additive sources are summed, so an
                // automated curve produces the value it was drawn at.
                let effective_base = resolved.absolute.map_or(*base, |v| min_val + v * range);
                let modulated =
                    (effective_base + resolved.additive * range).clamp(min_val, max_val);
                ParamValue::Float(modulated)
            }
            ParamValue::Color(base) => {
                let mut result = *base;
                Self::modulate_components(mod_key, ComponentKind::Color, &mut result, modulation);
                for comp in &mut result {
                    *comp = comp.clamp(0.0, 1.0);
                }
                ParamValue::Color(result)
            }
            ParamValue::Point2D(base) => {
                let mut result = *base;
                Self::modulate_components(mod_key, ComponentKind::Point, &mut result, modulation);
                ParamValue::Point2D(result)
            }
            _ => *value,
        }
    }

    /// Apply each component's modulation, resolved at `<key>/<suffix>`.
    fn modulate_components(
        mod_key: &mut String,
        kind: ComponentKind,
        values: &mut [f32],
        modulation: &ModulationEngine,
    ) {
        let base_len = mod_key.len();
        for &component in kind.components() {
            component.push_suffix(mod_key);
            let resolved = modulation.resolve(mod_key);
            mod_key.truncate(base_len);
            let comp = &mut values[component.index()];
            if let Some(absolute) = resolved.absolute {
                *comp = absolute;
            }
            *comp += resolved.additive;
        }
    }

    /// Update GPU buffer with modulation applied
    /// `param_prefix` is used to look up modulation (`deck/<u>/param` looks up `deck/<u>/param/<name>`)
    pub fn update_buffer_with_modulation(
        &mut self,
        queue: &wgpu::Queue,
        modulation: &ModulationEngine,
        param_prefix: Option<&str>,
    ) {
        // Build into scratch first, then write to GPU buffer.
        self.build_modulated_buffer_data(modulation, param_prefix);
        if let Some(buffer) = &self.buffer {
            queue.write_buffer(buffer, 0, &self.scratch);
        }
        // The dirty flag stays set: base values may have changed.
    }
}

// ── Normalized values ───────────────────────────────────────────────────

/// Clamp a normalized value to 0.0–1.0, treating non-finite input as 0.0.
pub(crate) fn clamp_norm(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Map a normalized 0.0–1.0 value to a discrete variant index via fader bucketing.
/// Splits the range into `n` equal segments: `index = min(floor(value * n), n - 1)`.
pub(crate) fn bucket_index(value: f32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    ((clamp_norm(value) * n as f32).floor() as usize).min(n - 1)
}

/// The normalized value at the center of bucket `index` of `n`. Maps back to the same bucket
/// under [`bucket_index`]; the center keeps rounding at either edge out of a neighbor.
pub(crate) fn bucket_center(index: usize, n: usize) -> f32 {
    if n == 0 {
        return 0.0;
    }
    (index.min(n - 1) as f32 + 0.5) / n as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_index_splits_range_evenly() {
        assert_eq!(bucket_index(0.0, 4), 0);
        assert_eq!(bucket_index(0.1, 4), 0);
        assert_eq!(bucket_index(0.25, 4), 1);
        assert_eq!(bucket_index(0.4, 4), 1);
        assert_eq!(bucket_index(0.5, 4), 2);
        assert_eq!(bucket_index(0.74, 4), 2);
        assert_eq!(bucket_index(0.75, 4), 3);
        assert_eq!(bucket_index(1.0, 4), 3);
    }

    #[test]
    fn bucket_index_clamps_out_of_range() {
        assert_eq!(bucket_index(-1.0, 4), 0);
        assert_eq!(bucket_index(2.0, 4), 3);
        assert_eq!(bucket_index(f32::NAN, 4), 0);
        assert_eq!(bucket_index(0.5, 0), 0);
    }
    use crate::isf::ISFInput;
    use crate::modulation::DEFAULT_ASSIGNMENT_AMOUNT;

    fn make_float_input(name: &str, default: f64, min: f32, max: f32) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "float".to_string(),
            default: Some(serde_json::json!(default)),
            min: Some(min),
            max: Some(max),
            label: Some(name.to_string()),
            values: None,
            labels: None,
            identity: None,
            group: None,
            specialize: false,
            presets: None,
        }
    }

    fn make_bool_input(name: &str, default: bool) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "bool".to_string(),
            default: Some(serde_json::json!(default)),
            min: None,
            max: None,
            label: None,
            values: None,
            labels: None,
            identity: None,
            group: None,
            specialize: false,
            presets: None,
        }
    }

    fn make_event_input(name: &str) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "event".to_string(),
            default: None,
            min: None,
            max: None,
            label: None,
            values: None,
            labels: None,
            identity: None,
            group: None,
            specialize: false,
            presets: None,
        }
    }

    fn make_color_input(name: &str) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "color".to_string(),
            default: Some(serde_json::json!([1.0, 0.0, 0.0, 1.0])),
            min: None,
            max: None,
            label: None,
            values: None,
            labels: None,
            identity: None,
            group: None,
            specialize: false,
            presets: None,
        }
    }

    fn make_long_input(name: &str, default: i64) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "long".to_string(),
            default: Some(serde_json::json!(default)),
            min: None,
            max: None,
            label: None,
            values: Some(vec![
                serde_json::json!(0),
                serde_json::json!(1),
                serde_json::json!(2),
            ]),
            labels: Some(vec!["A".into(), "B".into(), "C".into()]),
            identity: None,
            group: None,
            specialize: false,
            presets: None,
        }
    }

    fn make_point2d_input(name: &str) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "point2D".to_string(),
            default: Some(serde_json::json!([0.5, 0.5])),
            min: None,
            max: None,
            label: None,
            values: None,
            labels: None,
            identity: None,
            group: None,
            specialize: false,
            presets: None,
        }
    }

    // ── ParamValue tests ─────────────────────────────────────────────

    #[test]
    fn param_value_from_float_input() {
        let input = make_float_input("brightness", 0.75, 0.0, 1.0);
        match ParamValue::from_isf_input(&input) {
            ParamValue::Float(v) => assert!((v - 0.75).abs() < 1e-5),
            other => panic!("Expected Float, got {other:?}"),
        }
    }

    #[test]
    fn param_value_from_bool_input() {
        let input = make_bool_input("enabled", true);
        match ParamValue::from_isf_input(&input) {
            ParamValue::Bool(v) => assert!(v),
            other => panic!("Expected Bool, got {other:?}"),
        }
    }

    #[test]
    fn param_value_from_color_input() {
        let input = make_color_input("tint");
        match ParamValue::from_isf_input(&input) {
            ParamValue::Color(c) => {
                assert!((c[0] - 1.0).abs() < 1e-5);
                assert!((c[1] - 0.0).abs() < 1e-5);
                assert!((c[2] - 0.0).abs() < 1e-5);
                assert!((c[3] - 1.0).abs() < 1e-5);
            }
            other => panic!("Expected Color, got {other:?}"),
        }
    }

    #[test]
    fn param_value_from_long_input() {
        let input = make_long_input("mode", 2);
        match ParamValue::from_isf_input(&input) {
            ParamValue::Long(v) => assert_eq!(v, 2),
            other => panic!("Expected Long, got {other:?}"),
        }
    }

    #[test]
    fn param_value_from_point2d_input() {
        let input = make_point2d_input("center");
        match ParamValue::from_isf_input(&input) {
            ParamValue::Point2D(p) => {
                assert!((p[0] - 0.5).abs() < 1e-5);
                assert!((p[1] - 0.5).abs() < 1e-5);
            }
            other => panic!("Expected Point2D, got {other:?}"),
        }
    }

    #[test]
    fn param_value_byte_sizes() {
        assert_eq!(ParamValue::Float(0.0).byte_size(), 4);
        assert_eq!(ParamValue::Bool(true).byte_size(), 4);
        assert_eq!(ParamValue::Long(0).byte_size(), 4);
        assert_eq!(ParamValue::Color([0.0; 4]).byte_size(), 16);
        assert_eq!(ParamValue::Point2D([0.0; 2]).byte_size(), 8);
    }

    #[test]
    fn param_value_write_bytes_float() {
        let mut buf = Vec::new();
        ParamValue::Float(1.0).write_bytes(&mut buf);
        assert_eq!(buf.len(), 4);
        assert_eq!(f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), 1.0);
    }

    #[test]
    fn param_value_write_bytes_bool() {
        let mut buf = Vec::new();
        ParamValue::Bool(true).write_bytes(&mut buf);
        let val = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(val, 1);

        let mut buf2 = Vec::new();
        ParamValue::Bool(false).write_bytes(&mut buf2);
        let val2 = u32::from_le_bytes([buf2[0], buf2[1], buf2[2], buf2[3]]);
        assert_eq!(val2, 0);
    }

    #[test]
    fn param_value_write_bytes_color() {
        let mut buf = Vec::new();
        ParamValue::Color([1.0, 0.5, 0.25, 0.0]).write_bytes(&mut buf);
        assert_eq!(buf.len(), 16);
    }

    // ── ShaderParams tests ───────────────────────────────────────────

    #[test]
    fn shader_params_from_inputs() {
        let inputs = vec![
            make_float_input("brightness", 0.5, 0.0, 1.0),
            make_bool_input("invert", false),
        ];
        let params = ShaderParams::from_inputs(&inputs);
        assert_eq!(params.param_order.len(), 2);
        assert!(!params.is_empty());
    }

    #[test]
    fn shader_params_skips_image_inputs() {
        let inputs = vec![
            make_float_input("brightness", 0.5, 0.0, 1.0),
            ISFInput {
                name: "inputImage".to_string(),
                input_type: "image".to_string(),
                default: None,
                min: None,
                max: None,
                label: None,
                values: None,
                labels: None,
                identity: None,
                group: None,
                specialize: false,
                presets: None,
            },
        ];
        let params = ShaderParams::from_inputs(&inputs);
        assert_eq!(params.param_order.len(), 1); // image skipped
    }

    fn typed_input(name: &str, input_type: &str, default: serde_json::Value) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: input_type.to_string(),
            default: Some(default),
            min: None,
            max: None,
            label: None,
            values: None,
            labels: None,
            identity: None,
            group: None,
            specialize: false,
            presets: None,
        }
    }

    fn look_params() -> ShaderParams {
        let inputs: Vec<ISFInput> = serde_json::from_value(serde_json::json!([
            {"NAME": "look", "TYPE": "long", "DEFAULT": 1, "VALUES": [1, 2],
             "PRESETS": [
                {"gain": 0.5, "steps": 4.0, "tint": [0.1, 0.2, 0.3, 1.0], "at": [0.5, 0.25]},
                {"gain": 2.0, "steps": 8.0, "tint": [0.9, 0.8, 0.7, 1.0], "at": [1.0, 1.0],
                 "missing": 1.0, "label": [0.0, 0.0]}
             ]},
            {"NAME": "gain", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 4.0},
            {"NAME": "steps", "TYPE": "long", "DEFAULT": 4},
            {"NAME": "tint", "TYPE": "color", "DEFAULT": [0.1, 0.2, 0.3, 1.0]},
            {"NAME": "at", "TYPE": "point2D", "DEFAULT": [0.5, 0.25]},
            {"NAME": "label", "TYPE": "float", "DEFAULT": 0.75}
        ]))
        .unwrap();
        ShaderParams::from_inputs(&inputs)
    }

    #[test]
    fn choosing_a_value_applies_its_preset() {
        let mut params = look_params();
        params.set("look", ParamValue::Long(2));
        assert_eq!(params.get_float("gain"), Some(2.0));
        assert_eq!(
            params.get_long("steps"),
            Some(8),
            "a number lands as the declared long"
        );
        assert_eq!(params.get_color("tint"), Some([0.9, 0.8, 0.7, 1.0]));
        assert_eq!(params.get_point2d("at"), Some([1.0, 1.0]));
        assert_eq!(
            params.get_float("label"),
            Some(0.75),
            "a wrong shape is skipped"
        );
        assert!(!params.values.contains_key("missing"));

        params.set_long("look", 1);
        assert_eq!(params.get_float("gain"), Some(0.5));
        assert_eq!(params.get_color("tint"), Some([0.1, 0.2, 0.3, 1.0]));
    }

    #[test]
    fn a_number_selecting_the_value_applies_its_preset() {
        let mut params = look_params();
        params.set("look", ParamValue::Float(2.0));
        assert_eq!(params.get_float("gain"), Some(2.0));
    }

    #[test]
    fn the_same_value_again_keeps_slider_changes() {
        let mut params = look_params();
        params.set("look", ParamValue::Long(2));
        params.set("gain", ParamValue::Float(3.0));
        params.set("look", ParamValue::Long(2));
        params.set_long("look", 2);
        assert_eq!(params.get_float("gain"), Some(3.0));
    }

    #[test]
    fn restore_applies_no_preset() {
        let mut params = look_params();
        let saved = HashMap::from([
            ("look".to_string(), ParamValue::Float(2.0)),
            ("gain".to_string(), ParamValue::Float(3.0)),
        ]);
        params.restore(&saved);
        assert_eq!(params.get_long("look"), Some(2));
        assert_eq!(params.get_float("gain"), Some(3.0));
        assert_eq!(params.get_long("steps"), Some(4));
    }

    /// A `long` with `MIN` and `MAX` stays in that range on every write,
    /// scene loads included.
    #[test]
    fn a_ranged_long_stays_in_its_range() {
        let mut input = typed_input("count", "long", serde_json::json!(1));
        input.min = Some(1.0);
        input.max = Some(16.0);
        let mut params = ShaderParams::from_inputs(&[input]);
        let saved: HashMap<String, ParamValue> =
            serde_json::from_str(r#"{"count": 0.0}"#).expect("saved values");
        params.restore(&saved);
        assert!(matches!(params.values["count"], ParamValue::Long(1)));
        params.set("count", ParamValue::Long(40));
        assert!(matches!(params.values["count"], ParamValue::Long(16)));
        params.set_long("count", 5);
        assert!(matches!(params.values["count"], ParamValue::Long(5)));
    }

    /// Saved JSON numbers carry no type: `3` reads back as a float. Values
    /// keep the type the shader declares, so a `long` stays a dropdown and
    /// a `bool` a checkbox after a scene loads.
    #[test]
    fn restored_values_keep_their_declared_type() {
        let mut params = ShaderParams::from_inputs(&[
            typed_input("pick", "long", serde_json::json!(0)),
            typed_input("on", "bool", serde_json::json!(false)),
            typed_input("amount", "float", serde_json::json!(0.5)),
        ]);
        let saved: HashMap<String, ParamValue> =
            serde_json::from_str(r#"{"pick": 3, "on": 1.0, "amount": 2}"#).expect("saved values");
        params.restore(&saved);
        assert!(matches!(params.values["pick"], ParamValue::Long(3)));
        assert!(matches!(params.values["on"], ParamValue::Bool(true)));
        assert!(matches!(params.values["amount"], ParamValue::Float(v) if (v - 2.0).abs() < 1e-6));
        // A value of another shape is ignored.
        params.set("pick", ParamValue::Color([1.0; 4]));
        assert!(matches!(params.values["pick"], ParamValue::Long(3)));
    }

    fn event_params() -> ShaderParams {
        ShaderParams::from_inputs(&[
            make_float_input("a", 0.5, 0.0, 1.0),
            make_event_input("reset"),
            make_float_input("b", 0.25, 0.0, 1.0),
        ])
    }

    #[test]
    fn an_event_input_is_a_bool_param_false_at_rest() {
        let mut params = event_params();
        assert_eq!(params.param_order, ["a", "reset", "b"]);
        assert!(matches!(params.values["reset"], ParamValue::Bool(false)));
        assert_eq!(params.definitions["reset"].input_type, "event");
        assert!(params.is_event("reset"));
        assert!(!params.is_event("a"));
        let data = params.build_buffer_data();
        assert_eq!(u32::from_le_bytes([data[4], data[5], data[6], data[7]]), 0);
        let b = f32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        assert!((b - 0.25).abs() < 1e-6, "laid out as a bool");
    }

    #[test]
    fn setting_an_event_true_fires_it_and_false_does_nothing() {
        let mut params = event_params();
        params.set("reset", ParamValue::Bool(true));
        assert_eq!(params.get_bool("reset"), Some(true));
        params.set("reset", ParamValue::Bool(false));
        params.set_bool("reset", false);
        assert_eq!(params.get_bool("reset"), Some(true), "false does nothing");
        params.clear_events();
        assert_eq!(params.get_bool("reset"), Some(false));
        params.set("reset", ParamValue::Float(1.0));
        assert!(matches!(params.values["reset"], ParamValue::Bool(false)));
    }

    #[test]
    fn a_pressed_event_fires_when_the_value_rises_past_one_half() {
        let mut params = event_params();
        params.press_event("reset", 1.0);
        assert_eq!(params.get_bool("reset"), Some(true), "note on");
        params.clear_events();
        params.press_event("reset", 0.8);
        assert_eq!(params.get_bool("reset"), Some(false), "still held");
        params.press_event("reset", 0.0);
        assert_eq!(
            params.get_bool("reset"),
            Some(false),
            "release sets nothing"
        );
        params.press_event("reset", 0.6);
        assert_eq!(params.get_bool("reset"), Some(true), "pressed again");
    }

    #[test]
    fn events_are_not_saved_restored_or_explored() {
        let mut params = event_params();
        params.set("reset", ParamValue::Bool(true));
        let saved = params.saved_values();
        assert!(!saved.contains_key("reset"));
        assert!(saved.contains_key("a"));

        params.clear_events();
        let mut stale = saved.clone();
        stale.insert("reset".into(), ParamValue::Bool(true));
        params.restore(&stale);
        assert_eq!(params.get_bool("reset"), Some(false));

        for seed in 0..8 {
            params.randomize(None, seed);
            params.mutate(None, 1.0, seed);
            assert_eq!(params.get_bool("reset"), Some(false));
        }
    }

    fn written_params() -> ShaderParams {
        let meta: crate::isf::ISFMetadata = serde_json::from_str(
            r#"{"INPUTS": [
                {"NAME": "a", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
                {"NAME": "track", "TYPE": "long", "DEFAULT": 1, "MIN": 0, "MAX": 3,
                 "SPECIALIZE": true},
                {"NAME": "look", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1],
                 "PRESETS": [{"track": 1}, {"track": 0}]}
            ], "PREPROCESSORS": [{"NAME": "f", "TYPE": "flight", "WRITES": ["track"]}]}"#,
        )
        .unwrap();
        ShaderParams::from_metadata(&meta)
    }

    #[test]
    fn a_written_input_takes_only_derived_writes() {
        let mut params = written_params();
        assert!(params.is_written("track") && !params.is_written("a"));
        assert!(params.is_transient("track") && !params.is_transient("a"));
        params.set("track", ParamValue::Long(0));
        params.set("look", ParamValue::Long(1));
        assert_eq!(
            params.get_long("track"),
            Some(1),
            "user and preset writes do nothing"
        );

        params.write_derived("track", 2.4);
        assert_eq!(params.get_long("track"), Some(2), "rounded to the long");
        assert_eq!(params.specialization_constants().collect::<Vec<_>>(), [2.0]);
        params.write_derived("track", 9.0);
        assert_eq!(params.get_long("track"), Some(3), "clamped to MAX");
        params.write_derived("a", 0.9);
        assert_eq!(
            params.get_float("a"),
            Some(0.5),
            "only written inputs take it"
        );
    }

    #[test]
    fn written_inputs_are_not_saved_restored_or_explored() {
        let mut params = written_params();
        params.write_derived("track", 0.0);
        let saved = params.saved_values();
        assert!(!saved.contains_key("track"));
        assert!(saved.contains_key("a"));

        let mut stale = saved.clone();
        stale.insert("track".into(), ParamValue::Long(3));
        params.restore(&stale);
        assert_eq!(params.get_long("track"), Some(0));

        for seed in 0..8 {
            params.randomize(None, seed);
            params.mutate(None, 1.0, seed);
            assert_eq!(params.get_long("track"), Some(0));
        }
    }

    #[test]
    fn shader_params_get_set_float() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        assert!((params.get_float("brightness").unwrap() - 0.5).abs() < 1e-5);
        params.set_float("brightness", 0.8);
        assert!((params.get_float("brightness").unwrap() - 0.8).abs() < 1e-5);
    }

    #[test]
    fn shader_params_get_set_bool() {
        let inputs = vec![make_bool_input("invert", false)];
        let mut params = ShaderParams::from_inputs(&inputs);
        assert_eq!(params.get_bool("invert"), Some(false));
        params.set_bool("invert", true);
        assert_eq!(params.get_bool("invert"), Some(true));
    }

    #[test]
    fn shader_params_get_set_color() {
        let inputs = vec![make_color_input("tint")];
        let mut params = ShaderParams::from_inputs(&inputs);
        let c = params.get_color("tint").unwrap();
        assert!((c[0] - 1.0).abs() < 1e-5);
        params.set_color("tint", [0.0, 1.0, 0.0, 1.0]);
        let c2 = params.get_color("tint").unwrap();
        assert!((c2[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn shader_params_get_set_long() {
        let inputs = vec![make_long_input("mode", 0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        assert_eq!(params.get_long("mode"), Some(0));
        params.set_long("mode", 2);
        assert_eq!(params.get_long("mode"), Some(2));
    }

    #[test]
    fn shader_params_get_set_point2d() {
        let inputs = vec![make_point2d_input("center")];
        let mut params = ShaderParams::from_inputs(&inputs);
        let p = params.get_point2d("center").unwrap();
        assert!((p[0] - 0.5).abs() < 1e-5);
        params.set_point2d("center", [0.1, 0.9]);
        let p2 = params.get_point2d("center").unwrap();
        assert!((p2[0] - 0.1).abs() < 1e-5);
    }

    #[test]
    fn shader_params_generic_set() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        params.set("brightness", ParamValue::Float(0.9));
        assert!((params.get_float("brightness").unwrap() - 0.9).abs() < 1e-5);
    }

    #[test]
    fn shader_params_set_nonexistent_noop() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        params.set("nonexistent", ParamValue::Float(1.0)); // should not crash
        assert!(params.get_float("nonexistent").is_none());
    }

    #[test]
    fn shader_params_buffer_size_min_16() {
        let params = ShaderParams::from_inputs(&[]);
        assert!(params.buffer_size() >= 16);
    }

    #[test]
    fn shader_params_buffer_size_aligned_to_16() {
        let inputs = vec![make_float_input("a", 0.0, 0.0, 1.0)];
        let params = ShaderParams::from_inputs(&inputs);
        assert_eq!(params.buffer_size() % 16, 0);
    }

    #[test]
    fn shader_params_build_buffer_data() {
        let inputs = vec![
            make_float_input("brightness", 0.5, 0.0, 1.0),
            make_bool_input("invert", true),
        ];
        let mut params = ShaderParams::from_inputs(&inputs);
        let data = params.build_buffer_data();
        assert!(data.len() >= 16);
        assert_eq!(data.len() % 16, 0);
        // First 4 bytes should be 0.5f32
        let val = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        assert!((val - 0.5).abs() < 1e-5);
    }

    #[test]
    fn shader_params_reset_to_defaults() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        params.set_float("brightness", 0.9);
        params.reset_to_defaults();
        assert!((params.get_float("brightness").unwrap() - 0.5).abs() < 1e-5);
    }

    #[test]
    fn shader_params_empty() {
        let params = ShaderParams::from_inputs(&[]);
        assert!(params.is_empty());
    }

    // ── Parameter exploration ────────────────────────────────────────

    fn grouped_float(name: &str, group: &str, default: f64, min: f32, max: f32) -> ISFInput {
        let mut input = make_float_input(name, default, min, max);
        input.group = Some(group.to_string());
        input
    }

    fn explorable_params() -> ShaderParams {
        ShaderParams::from_inputs(&[
            grouped_float("fold", "Formula", 2.0, 1.0, 4.0),
            grouped_float("iters", "Formula", 8.0, 1.0, 16.0),
            grouped_float("fog", "Grade", 0.3, 0.0, 1.0),
        ])
    }

    #[test]
    fn randomize_is_reproducible_from_its_seed() {
        let mut a = explorable_params();
        let mut b = explorable_params();
        a.randomize(None, 42);
        b.randomize(None, 42);
        for name in ["fold", "iters", "fog"] {
            assert!(
                (a.get_float(name).unwrap() - b.get_float(name).unwrap()).abs() < 1e-6,
                "{name} must reproduce, or a performer cannot return to a seed"
            );
        }
    }

    #[test]
    fn different_seeds_give_different_configurations() {
        let mut a = explorable_params();
        let mut b = explorable_params();
        a.randomize(None, 1);
        b.randomize(None, 2);
        assert!(
            (a.get_float("fold").unwrap() - b.get_float("fold").unwrap()).abs() > 1e-6,
            "two seeds landing on the same value would make hunting pointless"
        );
    }

    #[test]
    fn randomize_stays_inside_the_declared_range() {
        for seed in 0..64 {
            let mut params = explorable_params();
            params.randomize(None, seed);
            let fold = params.get_float("fold").unwrap();
            assert!(
                (1.0..=4.0).contains(&fold),
                "seed {seed} produced {fold}, outside the declared MIN/MAX"
            );
        }
    }

    #[test]
    fn scope_leaves_other_groups_untouched() {
        let mut params = explorable_params();
        params.randomize(Some("Formula"), 7);
        assert!(
            (params.get_float("fog").unwrap() - 0.3).abs() < 1e-6,
            "a scoped randomize must not reach outside its group, which is the \
             whole point of being able to hunt the formula while the grade holds"
        );
        assert!((params.get_float("fold").unwrap() - 2.0).abs() > 1e-6);
    }

    #[test]
    fn mutate_clamps_at_the_bounds() {
        let mut params = explorable_params();
        params.set_float("fold", 4.0);
        for seed in 0..64 {
            let mut probe =
                ShaderParams::from_inputs(&[grouped_float("fold", "Formula", 4.0, 1.0, 4.0)]);
            probe.mutate(None, 1.0, seed);
            let v = probe.get_float("fold").unwrap();
            assert!(
                (1.0..=4.0).contains(&v),
                "seed {seed} mutated a parameter sitting on its maximum to {v}"
            );
        }
    }

    #[test]
    fn mutate_of_zero_amount_changes_nothing() {
        let mut params = explorable_params();
        params.mutate(None, 0.0, 9);
        assert!((params.get_float("fold").unwrap() - 2.0).abs() < 1e-6);
        assert!((params.get_float("fog").unwrap() - 0.3).abs() < 1e-6);
    }

    #[test]
    fn mutate_stays_nearer_than_randomize() {
        // A nudge from a working configuration versus a fresh draw.
        let mut nudged = explorable_params();
        let mut redrawn = explorable_params();
        nudged.mutate(None, 0.05, 3);
        redrawn.randomize(None, 3);
        let near = (nudged.get_float("iters").unwrap() - 8.0).abs();
        let far = (redrawn.get_float("iters").unwrap() - 8.0).abs();
        assert!(
            near < far,
            "a 5% mutation moved {near} while a randomize moved {far}"
        );
    }

    #[test]
    fn colors_and_unbounded_params_are_left_alone() {
        let mut params = ShaderParams::from_inputs(&[
            make_color_input("tint"),
            make_bool_input("invert", false),
            // No MIN or MAX, so there is no range to draw from.
            ISFInput {
                name: "gain".to_string(),
                input_type: "float".to_string(),
                default: Some(serde_json::json!(0.5)),
                min: None,
                max: None,
                label: None,
                values: None,
                labels: None,
                identity: None,
                group: None,
                specialize: false,
                presets: None,
            },
        ]);
        params.randomize(None, 5);
        assert!(
            (params.get_float("gain").unwrap() - 0.5).abs() < 1e-6,
            "a parameter with no declared range has no distribution to draw from"
        );
        match params.values.get("tint") {
            Some(ParamValue::Color(c)) => assert!(
                (c[0] - 1.0).abs() < 1e-6 && (c[1] - 0.0).abs() < 1e-6,
                "colour is deliberate, so exploration leaves the palette alone"
            ),
            other => panic!("expected Color, got {other:?}"),
        }
    }

    #[test]
    fn randomize_picks_only_declared_enum_values() {
        let inputs = vec![make_long_input("mode", 0)];
        for seed in 0..32 {
            let mut params = ShaderParams::from_inputs(&inputs);
            params.randomize(None, seed);
            let v = params.get_long("mode").unwrap();
            assert!(
                (0..=2).contains(&v),
                "seed {seed} chose {v}, which is not one of the declared VALUES"
            );
        }
    }

    #[test]
    fn mutate_steps_an_enum_to_a_neighbour() {
        let inputs = vec![make_long_input("mode", 1)];
        for seed in 0..32 {
            let mut params = ShaderParams::from_inputs(&inputs);
            params.mutate(None, 1.0, seed);
            let v = params.get_long("mode").unwrap();
            assert!(
                (0..=2).contains(&v) && (v - 1).abs() <= 1,
                "seed {seed} jumped from 1 to {v} instead of stepping"
            );
        }
    }

    #[test]
    fn shader_params_modulated_buffer_no_modulation() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        let engine = ModulationEngine::new();
        let data = params.build_modulated_buffer_data(&engine, None).to_vec();
        let base = params.build_buffer_data().to_vec();
        assert_eq!(data, base, "No modulation should produce identical buffer");
    }

    #[test]
    fn shader_params_modulated_buffer_with_modulation() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(crate::modulation::ModulationSource::LFO {
            waveform: crate::modulation::LFOWaveform::Sine,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        });
        engine.update_free_running(
            0.25,
            &crate::modulation::AudioValues::default(),
            &crate::modulation::AnalyzerValues::default(),
        );
        engine.assign("brightness", &uuid, 0.5);

        let modulated = params.build_modulated_buffer_data(&engine, None).to_vec();
        let base = params.build_buffer_data().to_vec();
        // Modulated should differ from base (LFO at t=0.25 is non-zero)
        assert_ne!(modulated, base, "Modulated buffer should differ from base");
    }

    #[test]
    fn get_float_modulated_matches_uploaded_value() {
        let inputs = vec![make_float_input("speed", 1.0, 0.0, 5.0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(pinned_source(1.0));
        engine.assign("deck0/speed", &uuid, 0.5);
        engine.update_free_running(
            0.0,
            &crate::modulation::AudioValues::default(),
            &crate::modulation::AnalyzerValues::default(),
        );

        let value = params
            .get_float_modulated("speed", &engine, Some("deck0"))
            .expect("speed is a float param");
        assert!((value - 3.5).abs() < 1e-5, "got {value}");

        let uploaded = params.build_modulated_buffer_data(&engine, Some("deck0"))[..4]
            .try_into()
            .map(f32::from_le_bytes)
            .expect("first param serialises as 4 bytes");
        assert!((uploaded - value).abs() < 1e-6);
    }

    /// A modulation source pinned to a constant output. A unipolar step sequencer at rate 0 stays
    /// on step 0. Unipolar because bipolar sources carry a 0.5 range-scale weight that would
    /// confound the asserted depth.
    fn pinned_source(value: f32) -> crate::modulation::ModulationSource {
        crate::modulation::ModulationSource::StepSequencer {
            steps: vec![value; 2],
            rate: 0.0,
            interpolation: crate::modulation::StepInterpolation::None,
            bipolar: false,
        }
    }

    /// A sweep must reach the far end of the fader.
    ///
    /// `AudioReactMode::Increase` ramps 0 → 1 and wraps, so with the fader at its minimum the
    /// parameter must climb to the maximum before resetting. Half-depth assignments would stop the
    /// sweep at the midpoint. Depth belongs on the source (LFO `amplitude`, audio `gain`), not on
    /// the assignment.
    #[test]
    fn full_depth_assignment_lets_a_sweep_span_the_whole_range() {
        let inputs = vec![make_float_input("speed", 0.0, 0.0, 5.0)];
        let at = |amount: f32, ramp: f32| -> f32 {
            let mut params = ShaderParams::from_inputs(&inputs);
            let mut engine = ModulationEngine::new();
            let uuid = engine.add_source(pinned_source(ramp));
            engine.assign("deck0/speed", &uuid, amount);
            engine.update_free_running(
                0.0,
                &crate::modulation::AudioValues::default(),
                &crate::modulation::AnalyzerValues::default(),
            );
            params
                .get_float_modulated("speed", &engine, Some("deck0"))
                .expect("speed is a float param")
        };

        // Top of the ramp must land on the top of the fader, not partway up.
        let top = at(DEFAULT_ASSIGNMENT_AMOUNT, 1.0);
        assert!(
            (top - 5.0).abs() < 1e-5,
            "a full sweep should reach the parameter maximum of 5.0, got {top}"
        );
        // And the ramp must map linearly onto the range on the way there.
        let middle = at(DEFAULT_ASSIGNMENT_AMOUNT, 0.5);
        assert!(
            (middle - 2.5).abs() < 1e-5,
            "half a sweep should sit at the midpoint 2.5, got {middle}"
        );

        // Half depth stalls a full sweep at the midpoint.
        let halved = at(0.5, 1.0);
        assert!(
            (halved - 2.5).abs() < 1e-5,
            "half depth should cap a full sweep at 2.5, got {halved}"
        );
    }

    /// Sample one full LFO cycle over a 0..1 parameter, returning the fraction spent pinned at
    /// either end and the extremes reached. The extremes catch a source collapsed to a constant,
    /// which would never clamp either.
    fn sweep_over_one_cycle(bipolar: bool, base: f64) -> (f32, f32, f32) {
        const SAMPLES: usize = 720;
        let inputs = vec![make_float_input("speed", base, 0.0, 1.0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(crate::modulation::ModulationSource::LFO {
            waveform: crate::modulation::LFOWaveform::Sine,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar,
        });
        engine.assign("deck0/speed", &uuid, DEFAULT_ASSIGNMENT_AMOUNT);

        let mut pinned = 0;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 0..SAMPLES {
            // frequency = 1 Hz, so t over 0..1 is exactly one cycle.
            let t = i as f32 / SAMPLES as f32;
            engine.update_free_running(
                t,
                &crate::modulation::AudioValues::default(),
                &crate::modulation::AnalyzerValues::default(),
            );
            let v = params
                .get_float_modulated("speed", &engine, Some("deck0"))
                .expect("speed is a float param");
            if v <= 1e-4 || v >= 1.0 - 1e-4 {
                pinned += 1;
            }
            lo = lo.min(v);
            hi = hi.max(v);
        }
        (pinned as f32 / SAMPLES as f32, lo, hi)
    }

    /// A bipolar LFO must sweep the fader instead of sitting at each end.
    ///
    /// A bipolar source outputs -1..1 and its offset is scaled by the whole range, so without the
    /// 0.5 weight its swing would be twice the fader and clamp at both ends. Uses a centered base
    /// at full amplitude and depth, the UI's default for "Bipolar".
    #[test]
    fn a_bipolar_lfo_sweeps_the_fader_instead_of_pinning_at_both_ends() {
        let (pinned, lo, hi) = sweep_over_one_cycle(true, 0.5);
        assert!(
            pinned < 0.05,
            "a centred bipolar LFO sits against an end for {:.0}% of its cycle; \
             it should traverse the fader continuously",
            pinned * 100.0
        );
        // Still a full-depth sweep: it must reach both ends, just not sit there.
        assert!(
            lo < 0.01 && hi > 0.99,
            "a full-amplitude bipolar LFO should span the whole fader, got {lo}..{hi}"
        );
    }

    /// The unipolar counterpart, as a control: based at the minimum, a 0..1
    /// source scaled by the range lands exactly on the fader and never clamps.
    #[test]
    fn a_unipolar_lfo_from_the_bottom_of_the_range_never_clamps() {
        let (pinned, lo, hi) = sweep_over_one_cycle(false, 0.0);
        assert!(
            pinned < 0.05,
            "a unipolar LFO based at the minimum should fit the fader exactly, \
             but sits against an end for {:.0}% of its cycle",
            pinned * 100.0
        );
        assert!(
            lo < 0.01 && hi > 0.99,
            "a full-amplitude unipolar LFO should span the whole fader, got {lo}..{hi}"
        );
    }

    /// End-to-end version of the above through a real audio sweep source.
    ///
    /// With the fader at its minimum, `Increase` must climb to the maximum before wrapping and
    /// `Decrease` must start at the maximum and walk down.
    #[test]
    fn audio_sweep_modes_traverse_the_whole_fader_from_the_bottom() {
        use crate::modulation::{AudioReactMode, AudioSourceValues, ModulationSource};

        // Flat full-scale FFT → energy_in_range reads 1.0 on every frame.
        let loud = || {
            let mut audio = crate::modulation::AudioValues::default();
            audio.sources.insert(
                0,
                AudioSourceValues {
                    fft: vec![1.0; 256].into(),
                    level: 1.0,
                    sample_rate: 48000.0,
                },
            );
            audio
        };

        let inputs = vec![make_float_input("speed", 0.0, 0.0, 5.0)];
        let excursion = |mode: AudioReactMode| -> (f32, f32) {
            let mut params = ShaderParams::from_inputs(&inputs);
            let mut engine = ModulationEngine::new();
            let uuid = engine.add_source(ModulationSource::AudioBand {
                source_id: None,
                freq_low: 20.0,
                freq_high: 20000.0,
                gain: 1.0,
                smoothing: 0.0,
                mode,
                noise_gate: 0.0,
            });
            engine.assign("deck0/speed", &uuid, DEFAULT_ASSIGNMENT_AMOUNT);

            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            // Long enough for the ramp to cross the full 0..1 span at the
            // default step of raw * dt * 4.
            for frame in 0..200 {
                engine.update_free_running(
                    frame as f32 * 0.016,
                    &loud(),
                    &crate::modulation::AnalyzerValues::default(),
                );
                let v = params
                    .get_float_modulated("speed", &engine, Some("deck0"))
                    .expect("speed is a float param");
                lo = lo.min(v);
                hi = hi.max(v);
            }
            (lo, hi)
        };

        for mode in [AudioReactMode::Increase, AudioReactMode::Decrease] {
            let (lo, hi) = excursion(mode);
            assert!(
                lo < 0.1,
                "{mode:?} should reach the bottom of the 0..5 fader, got {lo}"
            );
            assert!(
                hi > 4.9,
                "{mode:?} should reach the top of the 0..5 fader, got {hi} \
                 — the sweep is being truncated before it fills the slider"
            );
        }
    }

    #[test]
    fn get_float_modulated_returns_none_for_non_float() {
        let inputs = vec![make_bool_input("invert", true)];
        let mut params = ShaderParams::from_inputs(&inputs);
        let engine = ModulationEngine::new();
        assert!(
            params
                .get_float_modulated("invert", &engine, Some("deck0"))
                .is_none()
        );
        assert!(
            params
                .get_float_modulated("missing", &engine, Some("deck0"))
                .is_none()
        );
    }

    #[test]
    fn shader_params_modulated_with_prefix() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let mut params = ShaderParams::from_inputs(&inputs);
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(crate::modulation::ModulationSource::sine_lfo(1.0));
        engine.update_free_running(
            0.25,
            &crate::modulation::AudioValues::default(),
            &crate::modulation::AnalyzerValues::default(),
        );
        // Assign with prefix "deck0/brightness"
        engine.assign("deck0/brightness", &uuid, 0.5);

        let modulated = params
            .build_modulated_buffer_data(&engine, Some("deck0"))
            .to_vec();
        let base = params.build_buffer_data().to_vec();
        assert_ne!(modulated, base, "Prefixed modulation should apply");
    }

    #[test]
    fn shader_params_std140_alignment_point2d() {
        // Point2D requires 8-byte alignment
        let inputs = vec![
            make_float_input("a", 1.0, 0.0, 1.0), // 4 bytes at offset 0
            make_point2d_input("center"),         // should align to offset 8
        ];
        let mut params = ShaderParams::from_inputs(&inputs);
        let data = params.build_buffer_data();
        // offset 0..4: float a
        // offset 4..8: padding (align to 8 for vec2)
        // offset 8..16: point2D center
        assert!(data.len() >= 16);
        let p0 = f32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let p1 = f32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        assert!((p0 - 0.5).abs() < 1e-5);
        assert!((p1 - 0.5).abs() < 1e-5);
    }

    #[test]
    fn shader_params_std140_alignment_color() {
        // Color requires 16-byte alignment
        let inputs = vec![
            make_float_input("a", 1.0, 0.0, 1.0), // 4 bytes at offset 0
            make_color_input("tint"),             // should align to offset 16
        ];
        let mut params = ShaderParams::from_inputs(&inputs);
        let data = params.build_buffer_data();
        assert!(data.len() >= 32);
        // tint starts at offset 16
        let r = f32::from_le_bytes([data[16], data[17], data[18], data[19]]);
        assert!((r - 1.0).abs() < 1e-5); // red = 1.0
    }
}
