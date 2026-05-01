//! Shader parameter system for ISF user inputs

use crate::isf::ISFInput;
use crate::modulation::ModulationEngine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Parameter value types matching ISF input types
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Float(f32),
    Bool(bool),
    Long(i32),
    Color([f32; 4]),
    Point2D([f32; 2]),
}

impl ParamValue {
    /// Create from ISF input default value
    pub fn from_isf_input(input: &ISFInput) -> Self {
        match input.input_type.as_str() {
            "float" => {
                let val = input.default.as_ref()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32;
                ParamValue::Float(val)
            }
            "bool" => {
                let val = input.default.as_ref()
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                ParamValue::Bool(val)
            }
            "long" => {
                let val = input.default.as_ref()
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
                ParamValue::Long(val)
            }
            "color" => {
                let arr = input.default.as_ref()
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        let mut color = [1.0f32; 4];
                        for (i, val) in arr.iter().take(4).enumerate() {
                            color[i] = val.as_f64().unwrap_or(1.0) as f32;
                        }
                        color
                    })
                    .unwrap_or([1.0, 1.0, 1.0, 1.0]);
                ParamValue::Color(arr)
            }
            "point2D" => {
                let arr = input.default.as_ref()
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        let mut point = [0.0f32; 2];
                        for (i, val) in arr.iter().take(2).enumerate() {
                            point[i] = val.as_f64().unwrap_or(0.0) as f32;
                        }
                        point
                    })
                    .unwrap_or([0.0, 0.0]);
                ParamValue::Point2D(arr)
            }
            _ => ParamValue::Float(0.0), // Default fallback
        }
    }

    /// Size in bytes (aligned to 4 bytes for GPU)
    pub fn byte_size(&self) -> usize {
        match self {
            ParamValue::Float(_) => 4,
            ParamValue::Bool(_) => 4,  // Stored as u32
            ParamValue::Long(_) => 4,
            ParamValue::Color(_) => 16,
            ParamValue::Point2D(_) => 8,
        }
    }

    /// Write value to byte buffer
    pub fn write_bytes(&self, buffer: &mut Vec<u8>) {
        match self {
            ParamValue::Float(v) => buffer.extend_from_slice(&v.to_le_bytes()),
            ParamValue::Bool(v) => buffer.extend_from_slice(&(if *v { 1u32 } else { 0u32 }).to_le_bytes()),
            ParamValue::Long(v) => buffer.extend_from_slice(&v.to_le_bytes()),
            ParamValue::Color(v) => {
                for f in v { buffer.extend_from_slice(&f.to_le_bytes()); }
            }
            ParamValue::Point2D(v) => {
                for f in v { buffer.extend_from_slice(&f.to_le_bytes()); }
            }
        }
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
                "float" | "bool" | "long" | "color" | "point2D" => {
                    param_order.push(input.name.clone());
                    values.insert(input.name.clone(), ParamValue::from_isf_input(input));
                    definitions.insert(input.name.clone(), input.clone());
                }
                _ => {} // Skip image, audio, audioFFT, event
            }
        }

        Self {
            param_order,
            values,
            definitions,
            buffer: None,
            dirty: true,
        }
    }

    /// Check if this has any parameters
    pub fn is_empty(&self) -> bool {
        self.param_order.is_empty()
    }

    /// Get a float value
    pub fn get_float(&self, name: &str) -> Option<f32> {
        match self.values.get(name) {
            Some(ParamValue::Float(v)) => Some(*v),
            _ => None,
        }
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

    /// Set a bool value
    pub fn set_bool(&mut self, name: &str, value: bool) {
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

    /// Set a long value
    pub fn set_long(&mut self, name: &str, value: i32) {
        if let Some(ParamValue::Long(v)) = self.values.get_mut(name) {
            *v = value;
            self.dirty = true;
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

    /// Build byte buffer for GPU upload (respects std140 alignment rules)
    pub fn build_buffer_data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.buffer_size());
        for name in &self.param_order {
            if let Some(value) = self.values.get(name) {
                // std140 alignment rules:
                // - float, bool, int: 4-byte alignment
                // - vec2: 8-byte alignment
                // - vec3, vec4: 16-byte alignment
                let alignment = match value {
                    ParamValue::Float(_) | ParamValue::Bool(_) | ParamValue::Long(_) => 4,
                    ParamValue::Point2D(_) => 8,
                    ParamValue::Color(_) => 16,
                };
                // Pad to required alignment
                while data.len() % alignment != 0 { data.push(0); }
                value.write_bytes(&mut data);
            }
        }
        // Pad to minimum 16 bytes
        while data.len() < 16 { data.push(0); }
        // Align to 16 bytes (uniform buffer requirement)
        while data.len() % 16 != 0 { data.push(0); }
        data
    }

    /// Create or get GPU buffer
    pub fn ensure_buffer(&mut self, device: &wgpu::Device) -> &wgpu::Buffer {
        if self.buffer.is_none() {
            let data = self.build_buffer_data();
            self.buffer = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Shader Params Buffer"),
                contents: &data,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            }));
            self.dirty = false;
        }
        self.buffer.as_ref().unwrap()
    }

    /// Update GPU buffer if dirty
    pub fn update_buffer(&mut self, queue: &wgpu::Queue) {
        if self.dirty {
            if let Some(buffer) = &self.buffer {
                let data = self.build_buffer_data();
                queue.write_buffer(buffer, 0, &data);
                self.dirty = false;
            }
        }
    }

    /// Get the buffer reference (panics if not created)
    pub fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }

    /// Mark as needing re-upload
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Generic set method for any parameter value
    pub fn set(&mut self, name: &str, value: ParamValue) {
        if self.values.contains_key(name) {
            self.values.insert(name.to_string(), value);
            self.dirty = true;
        }
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

    /// Build byte buffer with modulation applied
    /// This creates a temporary modulated value for GPU upload without modifying base values
    /// `param_prefix` is used to look up modulation (e.g., "deck0" to look up "deck0:paramname")
    pub fn build_modulated_buffer_data(&self, modulation: &ModulationEngine, param_prefix: Option<&str>) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.buffer_size());

        for name in &self.param_order {
            if let Some(value) = self.values.get(name) {
                // std140 alignment rules
                let alignment = match value {
                    ParamValue::Float(_) | ParamValue::Bool(_) | ParamValue::Long(_) => 4,
                    ParamValue::Point2D(_) => 8,
                    ParamValue::Color(_) => 16,
                };
                // Pad to required alignment
                while data.len() % alignment != 0 { data.push(0); }

                // Apply modulation and write
                let modulated = self.apply_modulation_to_value(name, value, modulation, param_prefix);
                modulated.write_bytes(&mut data);
            }
        }
        // Pad to minimum 16 bytes
        while data.len() < 16 { data.push(0); }
        // Align to 16 bytes (uniform buffer requirement)
        while data.len() % 16 != 0 { data.push(0); }
        data
    }

    /// Apply modulation to a parameter value
    /// `param_prefix` is used to look up modulation (e.g., "deck0" to look up "deck0:paramname")
    fn apply_modulation_to_value(&self, name: &str, value: &ParamValue, modulation: &ModulationEngine, param_prefix: Option<&str>) -> ParamValue {
        // Get min/max from definition for clamping
        let definition = self.definitions.get(name);

        // Build the full modulation key
        let mod_key = match param_prefix {
            Some(prefix) => format!("{}:{}", prefix, name),
            None => name.to_string(),
        };

        match value {
            ParamValue::Float(base) => {
                let offset = modulation.get_modulation(&mod_key);
                if offset == 0.0 {
                    return *value;
                }
                // Get range from definition
                let (min_val, max_val) = definition
                    .map(|d| {
                        let min = d.min.unwrap_or(0.0);
                        let max = d.max.unwrap_or(1.0);
                        (min, max)
                    })
                    .unwrap_or((0.0, 1.0));
                let range = max_val - min_val;
                let modulated = (base + offset * range).clamp(min_val, max_val);
                ParamValue::Float(modulated)
            }
            ParamValue::Color(base) => {
                let mut result = *base;
                for i in 0..4 {
                    let offset = modulation.get_modulation_for_component(&mod_key, Some(i));
                    if offset != 0.0 {
                        result[i] = (result[i] + offset).clamp(0.0, 1.0);
                    }
                }
                ParamValue::Color(result)
            }
            ParamValue::Point2D(base) => {
                let mut result = *base;
                for i in 0..2 {
                    let offset = modulation.get_modulation_for_component(&mod_key, Some(i));
                    if offset != 0.0 {
                        result[i] = result[i] + offset; // Point2D can be unbounded
                    }
                }
                ParamValue::Point2D(result)
            }
            // Bool and Long don't support continuous modulation
            _ => *value,
        }
    }

    /// Update GPU buffer with modulation applied
    /// `param_prefix` is used to look up modulation (e.g., "deck0" to look up "deck0:paramname")
    pub fn update_buffer_with_modulation(&mut self, queue: &wgpu::Queue, modulation: &ModulationEngine, param_prefix: Option<&str>) {
        if let Some(buffer) = &self.buffer {
            let data = self.build_modulated_buffer_data(modulation, param_prefix);
            queue.write_buffer(buffer, 0, &data);
        }
        // Note: we don't clear dirty flag here since base values may have changed
    }
}

