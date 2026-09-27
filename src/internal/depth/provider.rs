//! Depth sensors as a deck source: the point cloud, reprojected into the
//! deck's 2D texture. See spec/depth-sensors.md.

use super::point_cloud::{ColorMode, PointCloudParams, PointCloudPipeline};
use super::{DepthSensorId, DepthSensorManager, backend::DepthIntrinsics};
use crate::source::{
    DeckSourceInstance, DeckSourceProvider, LibraryEntry, LibrarySection, SourceConfig, SourceEnv,
    SourceFrame, SourceParamError, SourceParamSpec, SourceQuery, SourceStatus, SourceValue,
    WidgetHint, clear_target, decode_config, downcast_mut, downcast_ref, encode_config,
    expect_norm,
};
use anyhow::{Context, Result};
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "DepthSensor";

static PARAMS: LazyLock<Vec<SourceParamSpec>> = LazyLock::new(|| {
    vec![
        SourceParamSpec::float("orbit_yaw", "Yaw", -180.0, 180.0)
            .unit("°")
            .routed("depth/orbit_yaw")
            .in_widget(WidgetHint::Orbit),
        SourceParamSpec::float("orbit_pitch", "Pitch", -90.0, 90.0)
            .unit("°")
            .routed("depth/orbit_pitch")
            .in_widget(WidgetHint::Orbit),
        SourceParamSpec::float("zoom", "Zoom", 0.1, 5.0)
            .routed("depth/zoom")
            .in_widget(WidgetHint::Orbit),
        SourceParamSpec::float("point_size", "Point Size", 0.25, 10.0).routed("depth/point_size"),
        SourceParamSpec::choice("color_mode", "Color", &["RGB", "Depth Ramp", "Solid"])
            .routed("depth/color_mode"),
        SourceParamSpec::float("depth_min", "Near", 0.0, 8000.0)
            .unit("mm")
            .routed("depth/depth_min"),
        SourceParamSpec::float("depth_max", "Far", 0.0, 8000.0)
            .unit("mm")
            .routed("depth/depth_max"),
        SourceParamSpec::float("seed", "Jitter", 0.0, 0.1).routed("depth/seed"),
        SourceParamSpec::float("drift", "Drift", 0.0, 1.0).routed("depth/drift"),
        SourceParamSpec::float("disruption", "Disruption", 0.0, 1.0).routed("depth/disruption"),
    ]
});

/// Point-cloud view params in physical units, as scenes store them. Every
/// field defaults so scenes written before a field existed still load.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ParamsConfig {
    #[serde(default)]
    orbit_yaw: f32,
    #[serde(default)]
    orbit_pitch: f32,
    #[serde(default)]
    zoom: f32,
    #[serde(default)]
    point_size: f32,
    /// 0 = Rgb, 1 = `DepthRamp`, 2 = Solid
    #[serde(default)]
    color_mode: u8,
    #[serde(default)]
    depth_min_mm: f32,
    #[serde(default)]
    depth_max_mm: f32,
    #[serde(default)]
    solid_color: [f32; 3],
    #[serde(default)]
    seed: f32,
    #[serde(default)]
    drift: f32,
    #[serde(default)]
    disruption: f32,
}

impl From<&PointCloudParams> for ParamsConfig {
    fn from(p: &PointCloudParams) -> Self {
        Self {
            orbit_yaw: p.orbit_yaw,
            orbit_pitch: p.orbit_pitch,
            zoom: p.zoom,
            point_size: p.point_size,
            color_mode: p.color_mode.as_u8(),
            depth_min_mm: p.depth_min_mm,
            depth_max_mm: p.depth_max_mm,
            solid_color: p.solid_color,
            seed: p.seed,
            drift: p.drift,
            disruption: p.disruption,
        }
    }
}

impl From<&ParamsConfig> for PointCloudParams {
    fn from(p: &ParamsConfig) -> Self {
        Self {
            orbit_yaw: p.orbit_yaw,
            orbit_pitch: p.orbit_pitch,
            zoom: p.zoom,
            point_size: p.point_size,
            color_mode: ColorMode::from_u8(p.color_mode),
            depth_min_mm: p.depth_min_mm,
            depth_max_mm: p.depth_max_mm,
            solid_color: p.solid_color,
            seed: p.seed,
            drift: p.drift,
            disruption: p.disruption,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    /// Matched by device name, which survives replugging; ids do not.
    name: String,
    /// `None` on scenes written before the params were saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    params: Option<ParamsConfig>,
}

/// Depth sensors.
pub struct DepthSensorProvider;

impl DeckSourceProvider for DepthSensorProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "Depth Sensors"
    }

    fn icon(&self) -> &'static str {
        "🛰"
    }

    fn params(&self) -> &'static [SourceParamSpec] {
        &PARAMS
    }

    fn library(&self, query: &SourceQuery) -> LibrarySection {
        LibrarySection {
            entries: query
                .services
                .get::<DepthSensorManager>()
                .map(|depth| {
                    depth
                        .devices()
                        .iter()
                        .map(|d| {
                            LibraryEntry::new(d.name.clone(), DepthSensor::config_for(&d.name))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            rescan: true,
            ..LibrarySection::default()
        }
    }

    fn library_action(&mut self, action: &str, env: &mut SourceEnv) -> Result<()> {
        anyhow::ensure!(
            action == "rescan",
            "Depth sensors have no library action '{action}'"
        );
        if let Some(depth) = env.services.get_mut::<DepthSensorManager>() {
            depth.scan_devices();
        }
        Ok(())
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let config: Config = decode_config(config)?;
        let depth = env
            .services
            .get_mut::<DepthSensorManager>()
            .context("depth sensors are not available")?;
        let device = depth
            .devices()
            .iter()
            .find(|d| d.name == config.name)
            .cloned()
            .with_context(|| {
                format!(
                    "Depth sensor '{}' not found — is it connected?",
                    config.name
                )
            })?;
        super::open_depth_sensor(depth, device.id, &env.gpu.device)
            .with_context(|| format!("Failed to open depth sensor '{}'", config.name))?;
        Ok(Box::new(DepthSensor {
            name: device.name,
            id: device.id,
            params: config
                .params
                .as_ref()
                .map_or_else(PointCloudParams::default, PointCloudParams::from),
            pipeline: None,
            frame: None,
        }))
    }

    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("name").cloned().unwrap_or_default()
    }

    /// Uploads every open sensor's frames. The manager is shared with shader
    /// decks' `depth_sensor` preprocessors, which read the same textures, so
    /// this runs whether or not any point-cloud deck exists.
    fn tick(&mut self, env: &mut SourceEnv, _submit: &mut Vec<wgpu::CommandBuffer>) {
        if let Some(depth) = env.services.get_mut::<DepthSensorManager>() {
            depth.update(&env.gpu.queue);
        }
    }

    fn prepare(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        let (Some(deck), Some(depth)) = (
            downcast_mut::<DepthSensor>(instance),
            env.services.get::<DepthSensorManager>(),
        ) else {
            return;
        };
        deck.frame = match (
            depth.depth_view(deck.id),
            depth.rgb_view(deck.id),
            depth.intrinsics(deck.id),
            depth.resolution(deck.id),
        ) {
            (Some(depth_view), Some(rgb_view), Some(intrinsics), Some(size)) => Some(SensorFrame {
                depth_view: depth_view.clone(),
                rgb_view: rgb_view.clone(),
                intrinsics,
                size,
            }),
            _ => None,
        };
    }

    fn release(&mut self, instance: &mut dyn DeckSourceInstance, env: &mut SourceEnv) {
        if let (Some(deck), Some(depth)) = (
            downcast_ref::<DepthSensor>(instance),
            env.services.get_mut::<DepthSensorManager>(),
        ) {
            depth.release(deck.id);
        }
    }

    fn status(&self, instance: &dyn DeckSourceInstance, query: &SourceQuery) -> SourceStatus {
        let mut status = instance.status();
        if let Some(deck) = downcast_ref::<DepthSensor>(instance) {
            status.connected = query
                .services
                .get::<DepthSensorManager>()
                .map(|d| d.is_connected(deck.id));
        }
        status
    }
}

/// What the sensor published this frame.
struct SensorFrame {
    /// Shared `R16Uint` depth texture.
    depth_view: wgpu::TextureView,
    rgb_view: wgpu::TextureView,
    intrinsics: DepthIntrinsics,
    size: (u32, u32),
}

/// One point-cloud deck.
pub struct DepthSensor {
    name: String,
    id: DepthSensorId,
    params: PointCloudParams,
    /// Built on first draw, for the deck's own target format.
    pipeline: Option<PointCloudPipeline>,
    frame: Option<SensorFrame>,
}

impl DepthSensor {
    pub fn config_for(name: &str) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                name: name.to_string(),
                params: None,
            },
        )
    }

    /// A deck for sensor `id`, which the caller has already opened and whose
    /// reference this deck now holds (released when the deck is removed).
    pub fn for_open_sensor(id: DepthSensorId, name: &str) -> Self {
        Self {
            name: name.to_string(),
            id,
            params: PointCloudParams::default(),
            pipeline: None,
            frame: None,
        }
    }

    /// The sensor this deck holds a reference to.
    pub fn sensor(&self) -> DepthSensorId {
        self.id
    }
}

impl DeckSourceInstance for DepthSensor {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        format!("🛰 {}", self.name)
    }

    fn config(&self) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                name: self.name.clone(),
                params: Some(ParamsConfig::from(&self.params)),
            },
        )
    }

    /// Deproject the depth texels to camera-space points and splat them into
    /// the deck: a contained reprojection, not a 3D engine. Black until the
    /// first frame and intrinsics arrive.
    fn render(&mut self, frame: &mut SourceFrame) -> Result<()> {
        let Some(sensor) = &self.frame else {
            clear_target(frame, wgpu::Color::BLACK, "Point Cloud (no frame)");
            return Ok(());
        };
        let pipeline = self.pipeline.get_or_insert_with(|| {
            PointCloudPipeline::new(&frame.gpu.device, frame.gpu.compositing_format)
        });
        let (sw, sh) = sensor.size;
        pipeline.update_uniform(
            &frame.gpu.queue,
            sensor.intrinsics,
            sw,
            sh,
            frame.width,
            frame.height,
            frame.time,
            &self.params,
        );
        pipeline.render(
            &frame.gpu.device,
            &sensor.depth_view,
            &sensor.rgb_view,
            frame.target,
            sw * sh,
            frame.cmd_buffers,
        );
        Ok(())
    }

    fn patch(&mut self, config: &SourceConfig) {
        if let Ok(Config {
            params: Some(params),
            ..
        }) = config.decode::<Config>()
        {
            self.params = PointCloudParams::from(&params);
        }
    }

    fn schema(&self) -> &'static [SourceParamSpec] {
        &PARAMS
    }

    fn param(&self, name: &str) -> Option<SourceValue> {
        self.params.normalized_param(name).map(SourceValue::Float)
    }

    /// Continuous params map linearly onto their range; `color_mode` buckets
    /// into three modes.
    fn set_param(&mut self, name: &str, value: &SourceValue) -> Result<(), SourceParamError> {
        let v = expect_norm(name, value)?;
        if self.params.set_normalized_param(name, v) {
            Ok(())
        } else {
            Err(SourceParamError::Unknown(name.to_string()))
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
    fn a_saved_depth_config_reads_back_unchanged() {
        let json = serde_json::json!({
            "type": "DepthSensor",
            "name": "Kinect (#0)",
            "params": {
                "orbit_yaw": 0.5, "orbit_pitch": 0.0, "zoom": 1.0, "point_size": 2.0,
                "color_mode": 1, "depth_min_mm": 400.0, "depth_max_mm": 4000.0,
                "solid_color": [0.6, 0.9, 1.0], "seed": 0.0, "drift": 0.0, "disruption": 0.0
            }
        });
        let config: SourceConfig = serde_json::from_value(json.clone()).unwrap();
        let decoded: Config = decode_config(&config).unwrap();
        assert_eq!(
            serde_json::to_value(encode_config(SOURCE_TYPE, &decoded)).unwrap(),
            json
        );
    }

    #[test]
    fn a_legacy_config_without_params_uses_defaults() {
        let config: SourceConfig =
            serde_json::from_str(r#"{"type":"DepthSensor","name":"K"}"#).unwrap();
        let decoded: Config = decode_config(&config).unwrap();
        assert!(decoded.params.is_none());
    }
}
