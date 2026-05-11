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
        self.buffer.as_ref().expect("ensure_buffer must be called before buffer() access")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isf::ISFInput;

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
        }
    }

    fn make_bool_input(name: &str, default: bool) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "bool".to_string(),
            default: Some(serde_json::json!(default)),
            min: None, max: None, label: None, values: None, labels: None, identity: None,
        }
    }

    fn make_color_input(name: &str) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "color".to_string(),
            default: Some(serde_json::json!([1.0, 0.0, 0.0, 1.0])),
            min: None, max: None, label: None, values: None, labels: None, identity: None,
        }
    }

    fn make_long_input(name: &str, default: i64) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "long".to_string(),
            default: Some(serde_json::json!(default)),
            min: None, max: None, label: None,
            values: Some(vec![serde_json::json!(0), serde_json::json!(1), serde_json::json!(2)]),
            labels: Some(vec!["A".into(), "B".into(), "C".into()]),
            identity: None,
        }
    }

    fn make_point2d_input(name: &str) -> ISFInput {
        ISFInput {
            name: name.to_string(),
            input_type: "point2D".to_string(),
            default: Some(serde_json::json!([0.5, 0.5])),
            min: None, max: None, label: None, values: None, labels: None, identity: None,
        }
    }

    // ── ParamValue tests ─────────────────────────────────────────────

    #[test]
    fn param_value_from_float_input() {
        let input = make_float_input("brightness", 0.75, 0.0, 1.0);
        match ParamValue::from_isf_input(&input) {
            ParamValue::Float(v) => assert!((v - 0.75).abs() < 1e-5),
            other => panic!("Expected Float, got {:?}", other),
        }
    }

    #[test]
    fn param_value_from_bool_input() {
        let input = make_bool_input("enabled", true);
        match ParamValue::from_isf_input(&input) {
            ParamValue::Bool(v) => assert!(v),
            other => panic!("Expected Bool, got {:?}", other),
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
            other => panic!("Expected Color, got {:?}", other),
        }
    }

    #[test]
    fn param_value_from_long_input() {
        let input = make_long_input("mode", 2);
        match ParamValue::from_isf_input(&input) {
            ParamValue::Long(v) => assert_eq!(v, 2),
            other => panic!("Expected Long, got {:?}", other),
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
            other => panic!("Expected Point2D, got {:?}", other),
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
                default: None, min: None, max: None, label: None,
                values: None, labels: None, identity: None,
            },
        ];
        let params = ShaderParams::from_inputs(&inputs);
        assert_eq!(params.param_order.len(), 1); // image skipped
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
        let params = ShaderParams::from_inputs(&inputs);
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

    #[test]
    fn shader_params_modulated_buffer_no_modulation() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let params = ShaderParams::from_inputs(&inputs);
        let engine = ModulationEngine::new();
        let data = params.build_modulated_buffer_data(&engine, None);
        let base = params.build_buffer_data();
        assert_eq!(data, base, "No modulation should produce identical buffer");
    }

    #[test]
    fn shader_params_modulated_buffer_with_modulation() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let params = ShaderParams::from_inputs(&inputs);
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(crate::modulation::ModulationSource::LFO {
            waveform: crate::modulation::LFOWaveform::Sine,
            frequency: 1.0, phase: 0.0, amplitude: 1.0, bipolar: true,
        });
        engine.update(0.25, &crate::modulation::AudioValues::default());
        engine.assign("brightness", &uuid, 0.5, None);

        let modulated = params.build_modulated_buffer_data(&engine, None);
        let base = params.build_buffer_data();
        // Modulated should differ from base (LFO at t=0.25 is non-zero)
        assert_ne!(modulated, base, "Modulated buffer should differ from base");
    }

    #[test]
    fn shader_params_modulated_with_prefix() {
        let inputs = vec![make_float_input("brightness", 0.5, 0.0, 1.0)];
        let params = ShaderParams::from_inputs(&inputs);
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(crate::modulation::ModulationSource::sine_lfo(1.0));
        engine.update(0.25, &crate::modulation::AudioValues::default());
        // Assign with prefix "deck0:brightness"
        engine.assign("deck0:brightness", &uuid, 0.5, None);

        let modulated = params.build_modulated_buffer_data(&engine, Some("deck0"));
        let base = params.build_buffer_data();
        assert_ne!(modulated, base, "Prefixed modulation should apply");
    }

    #[test]
    fn shader_params_std140_alignment_point2d() {
        // Point2D requires 8-byte alignment
        let inputs = vec![
            make_float_input("a", 1.0, 0.0, 1.0), // 4 bytes at offset 0
            make_point2d_input("center"),           // should align to offset 8
        ];
        let params = ShaderParams::from_inputs(&inputs);
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
            make_color_input("tint"),               // should align to offset 16
        ];
        let params = ShaderParams::from_inputs(&inputs);
        let data = params.build_buffer_data();
        assert!(data.len() >= 32);
        // tint starts at offset 16
        let r = f32::from_le_bytes([data[16], data[17], data[18], data[19]]);
        assert!((r - 1.0).abs() < 1e-5); // red = 1.0
    }
}

