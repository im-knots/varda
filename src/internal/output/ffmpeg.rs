//! Recordings and streams: every sink that reads frames back and hands them to
//! an ffmpeg encoder. One provider type, registered once per kind, around the
//! shared [`FfmpegSink`] building block.

use super::{
    ControlSpec, ControlValue, Delivered, FramePath, OutputSinkInstance, OutputSinkProvider,
    ParamEffect, Presentation, SinkConfig, SinkEnv, SinkQuery, SinkStart, WidgetHint,
};
use crate::delivery::presentation::{modes_for, recording_presentation, streaming_presentation};
use crate::delivery::{FfmpegSubprocess, StreamingProtocol};
use crate::engine::value::render::{
    PresentationRequest, RecordingCodec, ResolvedPresentation, RtmpCodecContract, SrtCodec,
    StreamingCodec,
};
use crate::renderer::ReadbackFrame;
use crate::renderer::context::GpuContext;
use crate::source::{ControlError, choice_index, choice_value, expect_norm, expect_text};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Which ffmpeg delivery a provider builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfmpegKind {
    Recording,
    Srt,
    Hls,
    Dash,
    Rtmp,
}

impl FfmpegKind {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Srt => "srt_stream",
            Self::Hls => "hls_stream",
            Self::Dash => "dash_stream",
            Self::Rtmp => "rtmp_stream",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Recording => "Recording",
            Self::Srt => "SRT stream",
            Self::Hls => "HLS stream",
            Self::Dash => "DASH stream",
            Self::Rtmp => "RTMP stream",
        }
    }

    const fn icon(self) -> &'static str {
        match self {
            Self::Recording => "⏺",
            Self::Srt | Self::Rtmp => "📺",
            Self::Hls | Self::Dash => "📡",
        }
    }
}

fn labels<T: std::fmt::Display>(all: &[T]) -> Vec<String> {
    all.iter().map(ToString::to_string).collect()
}

/// Every setting is addressable as `output/<uuid>/<name>`.
fn choice(name: &str, label: &str, options: &[String]) -> ControlSpec {
    let options: Vec<&str> = options.iter().map(String::as_str).collect();
    ControlSpec::choice(name, label, &options).routed(name)
}

fn text(name: &str, label: &str) -> ControlSpec {
    ControlSpec::text(name, label).routed(name)
}

fn audio_device() -> ControlSpec {
    text("audio_device", "Audio").in_widget(WidgetHint::AudioDevice)
}

static RECORDING_PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    vec![
        text("path", "File"),
        choice("codec", "Codec", &labels(&RecordingCodec::ALL)),
        audio_device(),
    ]
});

static SRT_PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    vec![
        text("url", "URL"),
        choice("codec", "Codec", &labels(&SrtCodec::ALL)),
        audio_device(),
    ]
});

static HLS_PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    vec![
        text("name", "Name"),
        choice("codec", "Codec", &labels(&StreamingCodec::ALL)),
        ControlSpec::toggle("short_segments", "Short segments").routed("short_segments"),
        audio_device(),
    ]
});

static DASH_PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    vec![
        text("name", "Name"),
        choice("codec", "Codec", &labels(&StreamingCodec::ALL)),
        audio_device(),
    ]
});

static RTMP_PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    let contracts: Vec<String> = RtmpCodecContract::ALL
        .iter()
        .map(|c| c.label().to_string())
        .collect();
    vec![
        text("url", "URL"),
        choice("codec", "Codec", &labels(&StreamingCodec::ALL)),
        choice("codec_contract", "Signaling", &contracts),
        audio_device(),
    ]
});

/// The saved shape of every ffmpeg sink, a superset of each kind's fields.
/// Codecs are saved as their labels.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default)]
    codec: String,
    #[serde(
        default,
        alias = "low_latency",
        skip_serializing_if = "Option::is_none"
    )]
    short_segments: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codec_contract: Option<RtmpCodecContract>,
    #[serde(default)]
    audio_device: Option<String>,
}

/// What each kind sends to, with its codec parsed.
#[derive(Debug, Clone, PartialEq)]
enum Destination {
    Recording {
        path: String,
        codec: RecordingCodec,
    },
    Srt {
        url: String,
        codec: SrtCodec,
    },
    Hls {
        name: String,
        codec: StreamingCodec,
        short_segments: bool,
    },
    Dash {
        name: String,
        codec: StreamingCodec,
    },
    Rtmp {
        url: String,
        codec: StreamingCodec,
        contract: RtmpCodecContract,
    },
}

impl Destination {
    fn from_config(kind: FfmpegKind, config: &Config) -> Self {
        let text = |field: &Option<String>| field.clone().unwrap_or_default();
        match kind {
            FfmpegKind::Recording => Self::Recording {
                path: text(&config.path),
                codec: RecordingCodec::from_saved(&config.codec),
            },
            FfmpegKind::Srt => Self::Srt {
                url: text(&config.url),
                codec: SrtCodec::from_saved(&config.codec),
            },
            FfmpegKind::Hls => Self::Hls {
                name: text(&config.name),
                codec: StreamingCodec::from_saved(&config.codec),
                short_segments: config.short_segments.unwrap_or(false),
            },
            FfmpegKind::Dash => Self::Dash {
                name: text(&config.name),
                codec: StreamingCodec::from_saved(&config.codec),
            },
            FfmpegKind::Rtmp => Self::Rtmp {
                url: text(&config.url),
                codec: StreamingCodec::from_saved(&config.codec),
                contract: config.codec_contract.unwrap_or_default(),
            },
        }
    }

    fn to_config(&self, audio_device: Option<String>) -> Config {
        let mut config = Config {
            audio_device,
            ..Config::default()
        };
        match self {
            Self::Recording { path, codec } => {
                config.path = Some(path.clone());
                config.codec = codec.to_string();
            }
            Self::Srt { url, codec } => {
                config.url = Some(url.clone());
                config.codec = codec.to_string();
            }
            Self::Hls {
                name,
                codec,
                short_segments,
            } => {
                config.name = Some(name.clone());
                config.codec = codec.to_string();
                config.short_segments = Some(*short_segments);
            }
            Self::Dash { name, codec } => {
                config.name = Some(name.clone());
                config.codec = codec.to_string();
            }
            Self::Rtmp {
                url,
                codec,
                contract,
            } => {
                config.url = Some(url.clone());
                config.codec = codec.to_string();
                config.codec_contract = Some(*contract);
            }
        }
        config
    }

    fn resolve(&self, request: PresentationRequest) -> ResolvedPresentation {
        match self {
            Self::Recording { codec, .. } => recording_presentation(codec, request),
            Self::Srt { codec, .. } => {
                let codec = match codec {
                    SrtCodec::H264 => StreamingCodec::H264,
                    SrtCodec::H265 => StreamingCodec::H265,
                };
                streaming_presentation(StreamingProtocol::Srt, &codec, request)
            }
            Self::Hls { codec, .. } => {
                streaming_presentation(StreamingProtocol::Hls, codec, request)
            }
            Self::Dash { codec, .. } => {
                streaming_presentation(StreamingProtocol::Dash, codec, request)
            }
            Self::Rtmp {
                codec, contract, ..
            } => streaming_presentation(StreamingProtocol::Rtmp(*contract), codec, request),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Recording { path, codec } => format!("{path} ({codec})"),
            Self::Srt { url, .. } | Self::Rtmp { url, .. } => url.clone(),
            Self::Hls { name, .. } | Self::Dash { name, .. } => name.clone(),
        }
    }

    /// Why this destination cannot start yet, such as an RTMP URL with no
    /// server. Checked before ffmpeg is spawned so the user sees what to fill
    /// in.
    fn url_problem(&self) -> Option<String> {
        match self {
            Self::Recording { path, .. } if path.trim().is_empty() => {
                Some("Set the recording's file path first".into())
            }
            Self::Hls { name, .. } | Self::Dash { name, .. } if name.trim().is_empty() => {
                Some("Set the stream name first".into())
            }
            Self::Srt { url, .. } => match url_authority(url, "srt") {
                Some((host, Some(_))) if !host.is_empty() => None,
                _ => Some(format!(
                    "Set the SRT URL first: '{url}' needs a host and port, for example srt://0.0.0.0:9001"
                )),
            },
            Self::Rtmp { url, .. } => {
                match url_authority(url, "rtmp").or_else(|| url_authority(url, "rtmps")) {
                    Some((host, _)) if !host.is_empty() => None,
                    _ => Some(format!(
                        "Set the RTMP URL first: '{url}' has no server, for example rtmp://live.example.com/app/<stream key>"
                    )),
                }
            }
            _ => None,
        }
    }
}

/// The host and port of `url` when it uses `scheme`. `None` for another
/// scheme; an empty host when the URL names none (`rtmp://`).
fn url_authority<'a>(url: &'a str, scheme: &str) -> Option<(&'a str, Option<&'a str>)> {
    let rest = url.trim().strip_prefix(scheme)?.strip_prefix("://")?;
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    Some(match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            (host, Some(port))
        }
        _ => (authority, None),
    })
}

/// A recording or stream provider.
pub struct FfmpegProvider {
    kind: FfmpegKind,
}

impl FfmpegProvider {
    pub const fn new(kind: FfmpegKind) -> Self {
        Self { kind }
    }
}

impl OutputSinkProvider for FfmpegProvider {
    fn id(&self) -> &'static str {
        self.kind.id()
    }

    fn label(&self) -> &'static str {
        self.kind.label()
    }

    fn icon(&self) -> &'static str {
        self.kind.icon()
    }

    fn params(&self) -> &'static [ControlSpec] {
        match self.kind {
            FfmpegKind::Recording => &RECORDING_PARAMS,
            FfmpegKind::Srt => &SRT_PARAMS,
            FfmpegKind::Hls => &HLS_PARAMS,
            FfmpegKind::Dash => &DASH_PARAMS,
            FfmpegKind::Rtmp => &RTMP_PARAMS,
        }
    }

    fn default_config(&self) -> SinkConfig {
        let destination = match self.kind {
            FfmpegKind::Recording => Destination::Recording {
                path: "output.mp4".into(),
                codec: RecordingCodec::H264,
            },
            FfmpegKind::Srt => Destination::Srt {
                url: "srt://0.0.0.0:9001".into(),
                codec: SrtCodec::H264,
            },
            FfmpegKind::Hls => Destination::Hls {
                name: "live".into(),
                codec: StreamingCodec::H264,
                short_segments: false,
            },
            FfmpegKind::Dash => Destination::Dash {
                name: "live".into(),
                codec: StreamingCodec::H264,
            },
            FfmpegKind::Rtmp => Destination::Rtmp {
                url: "rtmp://".into(),
                codec: StreamingCodec::H264,
                contract: RtmpCodecContract::Legacy,
            },
        };
        crate::source::encode_config(self.kind.id(), &destination.to_config(None))
    }

    fn create(
        &mut self,
        config: &SinkConfig,
        _env: &mut SinkEnv,
    ) -> Result<Box<dyn OutputSinkInstance>> {
        let config: Config = crate::source::decode_config(config)?;
        Ok(Box::new(FfmpegSink {
            kind: self.kind,
            destination: Destination::from_config(self.kind, &config),
            audio_device: config.audio_device,
            subprocess: None,
        }))
    }
}

/// A recording or stream: an ffmpeg encoder fed read-back frames.
pub struct FfmpegSink {
    kind: FfmpegKind,
    destination: Destination,
    audio_device: Option<String>,
    subprocess: Option<FfmpegSubprocess>,
}

impl FfmpegSink {
    /// A sink of `kind` with its default settings and no encoder running, for
    /// benchmarks of the delivery path.
    pub fn idle(kind: FfmpegKind) -> Self {
        let config: Config =
            crate::source::decode_config(&FfmpegProvider::new(kind).default_config())
                .unwrap_or_default();
        Self {
            kind,
            destination: Destination::from_config(kind, &config),
            audio_device: None,
            subprocess: None,
        }
    }

    fn params(&self) -> &'static [ControlSpec] {
        FfmpegProvider::new(self.kind).params()
    }
}

/// The index of `value` in `all`, as a normalized choice.
fn choice_of<T: PartialEq>(all: &[T], value: &T) -> f32 {
    let index = all.iter().position(|v| v == value).unwrap_or(0);
    choice_value(index, all.len())
}

impl OutputSinkInstance for FfmpegSink {
    fn sink_type(&self) -> &str {
        self.kind.id()
    }

    fn label(&self) -> String {
        self.destination.label()
    }

    fn config(&self) -> SinkConfig {
        crate::source::encode_config(
            self.kind.id(),
            &self.destination.to_config(self.audio_device.clone()),
        )
    }

    fn frame_path(&self) -> FramePath {
        FramePath::Readback
    }

    fn configure(
        &mut self,
        _gpu: &GpuContext,
        _query: &SinkQuery,
        request: PresentationRequest,
    ) -> Result<Presentation> {
        Ok(Presentation {
            resolved: self.destination.resolve(request),
            modes: modes_for(|r| self.destination.resolve(r)),
        })
    }

    fn prepare_start(
        &mut self,
        gpu: &GpuContext,
        request: PresentationRequest,
    ) -> Result<Option<ResolvedPresentation>> {
        let Destination::Recording { codec, .. } = &self.destination else {
            return Ok(None);
        };
        let rgba16 = gpu.device.features().contains(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        );
        FfmpegSubprocess::probe_recording_presentation(codec, request, rgba16).map(Some)
    }

    fn start(&mut self, start: SinkStart) -> Result<Option<ResolvedPresentation>> {
        if let Some(problem) = self.destination.url_problem() {
            anyhow::bail!("{problem}");
        }
        let SinkStart {
            width,
            height,
            fps,
            request,
            readback_format,
            audio,
        } = start;
        let subprocess = match &self.destination {
            Destination::Recording { path, codec } => {
                FfmpegSubprocess::spawn_recording_with_presentation(
                    path,
                    codec,
                    width,
                    height,
                    fps,
                    audio,
                    request,
                    readback_format,
                )?
            }
            Destination::Srt { url, codec } => {
                FfmpegSubprocess::spawn_srt(url, codec, request, width, height, fps, audio)?
            }
            Destination::Hls {
                name,
                codec,
                short_segments,
            } => FfmpegSubprocess::spawn_hls(
                name,
                codec,
                request,
                width,
                height,
                fps,
                *short_segments,
                audio,
            )?,
            Destination::Dash { name, codec } => {
                FfmpegSubprocess::spawn_dash(name, codec, request, width, height, fps, audio)?
            }
            Destination::Rtmp {
                url,
                codec,
                contract,
            } => FfmpegSubprocess::spawn_rtmp(
                url, codec, *contract, request, width, height, fps, audio,
            )?,
        };
        let presentation = subprocess.presentation().cloned();
        self.subprocess = Some(subprocess);
        Ok(presentation)
    }

    fn stop(&mut self) {
        if let Some(mut subprocess) = self.subprocess.take() {
            subprocess.stop();
        }
    }

    fn audio_device(&self) -> Option<&str> {
        self.audio_device.as_deref().filter(|d| !d.is_empty())
    }

    /// The encoder was opened with a fixed `-s WxH`; raw frames of another size
    /// desync the stream rather than failing, and restarting a recording would
    /// truncate the take on disk.
    fn restart_on_resize(&self) -> bool {
        true
    }

    /// A frame the encoder cannot take ends the encode. An SRT listener whose
    /// client left is started again for the next client.
    fn deliver(&mut self, frame: ReadbackFrame) -> Delivered {
        let Some(subprocess) = &mut self.subprocess else {
            return Delivered::Ok;
        };
        if subprocess.feed_readback_frame(frame) {
            return Delivered::Ok;
        }
        let reason = subprocess.failure().map_or_else(
            || "ffmpeg stopped taking frames".to_string(),
            str::to_string,
        );
        self.stop();
        if self.kind == FfmpegKind::Srt {
            Delivered::Restart
        } else {
            Delivered::Failed(format!("{}: {reason}", self.destination.label()))
        }
    }

    fn encoder_health(&self) -> Option<crate::delivery::EncoderHealth> {
        self.subprocess
            .as_ref()
            .map(|sub| crate::delivery::EncoderHealth {
                frames_written: sub.frames_written(),
                frames_dropped: sub.frames_dropped(),
                frames_padded: sub.frames_padded(),
            })
    }

    fn audio_health(&self) -> Option<crate::delivery::AudioHealth> {
        let sub = self.subprocess.as_ref()?;
        Some(crate::delivery::AudioHealth {
            frames_written: sub.audio_frames_written().unwrap_or(0),
            frames_dropped: 0,
            silence_spliced: sub.audio_silence_spliced().unwrap_or(0),
        })
    }

    fn duration(&self) -> Option<std::time::Duration> {
        self.subprocess.as_ref().map(FfmpegSubprocess::duration)
    }

    fn schema(&self) -> &'static [ControlSpec] {
        self.params()
    }

    /// Settings, plus for HLS and DASH the links a viewer opens: Varda serves
    /// segments itself, so they are known without the stream running.
    fn status(&self) -> crate::output::ControlStatus {
        let mut status = crate::output::ControlStatus::default();
        for spec in self.schema() {
            if let Some(value) = self.param(&spec.name) {
                status.params.insert(spec.name.clone(), value);
            }
        }
        let manifest = match &self.destination {
            Destination::Hls { name, .. } => Some((name, "index.m3u8")),
            Destination::Dash { name, .. } => Some((name, "manifest.mpd")),
            _ => None,
        };
        if let (Destination::Srt { url, .. }, Some(subprocess)) =
            (&self.destination, &self.subprocess)
            && subprocess.waiting_for_client()
        {
            status.info.insert(
                "note".into(),
                serde_json::json!(format!("Waiting for a client on {url}")),
            );
        }
        if let Some((name, manifest)) = manifest {
            status.info.insert(
                "links".into(),
                serde_json::json!([
                    {"icon": "▶", "url": format!("http://localhost:8080/streams/{name}/player.html")},
                    {"icon": "🌐", "url": format!("http://localhost:8080/streams/{name}/{manifest}")},
                ]),
            );
        }
        status
    }

    fn param(&self, name: &str) -> Option<ControlValue> {
        let text = |s: &str| Some(ControlValue::Text(s.to_string()));
        match (name, &self.destination) {
            ("path", Destination::Recording { path, .. }) => text(path),
            ("url", Destination::Srt { url, .. } | Destination::Rtmp { url, .. }) => text(url),
            ("name", Destination::Hls { name, .. } | Destination::Dash { name, .. }) => text(name),
            ("codec", Destination::Recording { codec, .. }) => {
                Some(ControlValue::Float(choice_of(&RecordingCodec::ALL, codec)))
            }
            ("codec", Destination::Srt { codec, .. }) => {
                Some(ControlValue::Float(choice_of(&SrtCodec::ALL, codec)))
            }
            (
                "codec",
                Destination::Hls { codec, .. }
                | Destination::Dash { codec, .. }
                | Destination::Rtmp { codec, .. },
            ) => Some(ControlValue::Float(choice_of(&StreamingCodec::ALL, codec))),
            ("short_segments", Destination::Hls { short_segments, .. }) => {
                Some(ControlValue::Bool(*short_segments))
            }
            ("codec_contract", Destination::Rtmp { contract, .. }) => Some(ControlValue::Float(
                choice_of(&RtmpCodecContract::ALL, contract),
            )),
            ("audio_device", _) => text(self.audio_device.as_deref().unwrap_or_default()),
            _ => None,
        }
    }

    /// Every setting changes what the encoder is opened with, so a write
    /// rebuilds the sink; a running one is stopped first.
    fn set_param(
        &mut self,
        name: &str,
        value: &ControlValue,
    ) -> std::result::Result<ParamEffect, ControlError> {
        let pick = |n: usize| expect_norm(name, value).map(|v| choice_index(v, n));
        match (name, &mut self.destination) {
            ("path", Destination::Recording { path, .. })
            | ("url", Destination::Srt { url: path, .. } | Destination::Rtmp { url: path, .. })
            | (
                "name",
                Destination::Hls { name: path, .. } | Destination::Dash { name: path, .. },
            ) => {
                *path = expect_text(name, value)?.to_string();
            }
            ("codec", Destination::Recording { codec, .. }) => {
                *codec = RecordingCodec::ALL[pick(RecordingCodec::ALL.len())?].clone();
            }
            ("codec", Destination::Srt { codec, .. }) => {
                *codec = SrtCodec::ALL[pick(SrtCodec::ALL.len())?].clone();
            }
            (
                "codec",
                Destination::Hls { codec, .. }
                | Destination::Dash { codec, .. }
                | Destination::Rtmp { codec, .. },
            ) => {
                *codec = StreamingCodec::ALL[pick(StreamingCodec::ALL.len())?].clone();
            }
            ("short_segments", Destination::Hls { short_segments, .. }) => {
                *short_segments = expect_norm(name, value)? > 0.5;
            }
            ("codec_contract", Destination::Rtmp { contract, .. }) => {
                *contract = RtmpCodecContract::ALL[pick(RtmpCodecContract::ALL.len())?];
            }
            ("audio_device", _) => {
                let device = expect_text(name, value)?;
                self.audio_device = (!device.is_empty()).then(|| device.to_string());
            }
            _ => return Err(ControlError::Unknown(name.to_string())),
        }
        Ok(ParamEffect::Rebuild)
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
    #[test]
    fn a_stream_without_a_server_is_refused_before_ffmpeg_starts() {
        let rtmp = |url: &str| Destination::Rtmp {
            url: url.into(),
            codec: StreamingCodec::H264,
            contract: RtmpCodecContract::Legacy,
        };
        let srt = |url: &str| Destination::Srt {
            url: url.into(),
            codec: SrtCodec::H264,
        };
        assert!(
            rtmp("rtmp://").url_problem().is_some(),
            "the default has no server"
        );
        assert!(rtmp("").url_problem().is_some());
        assert!(rtmp("rtmp:///app/key").url_problem().is_some());
        assert!(
            rtmp("rtmp://live.twitch.tv/app/key")
                .url_problem()
                .is_none()
        );
        assert!(
            rtmp("rtmps://a.rtmp.youtube.com:443/live2/key")
                .url_problem()
                .is_none()
        );
        assert!(
            srt("srt://0.0.0.0:9001").url_problem().is_none(),
            "the default is complete"
        );
        assert!(srt("srt://h:9000?mode=caller").url_problem().is_none());
        assert!(
            srt("srt://0.0.0.0").url_problem().is_some(),
            "SRT needs a port"
        );
        assert!(srt("srt://:9001").url_problem().is_some());
    }

    /// The default RTMP output cannot start, and says what to fill in.
    #[test]
    fn starting_the_default_rtmp_output_names_the_missing_server() {
        let provider = FfmpegProvider::new(FfmpegKind::Rtmp);
        let config: Config = crate::source::decode_config(&provider.default_config()).unwrap();
        let destination = Destination::from_config(FfmpegKind::Rtmp, &config);
        let problem = destination.url_problem().expect("rtmp:// is refused");
        assert!(problem.contains("RTMP URL"), "{problem}");
    }

    use super::*;

    /// Saved ffmpeg targets read back unchanged, codec labels included.
    #[test]
    fn saved_stage_targets_read_back_unchanged() {
        let saved = [
            (
                FfmpegKind::Recording,
                serde_json::json!({"type": "recording", "path": "/take.mov", "codec": "ProRes 4444", "audio_device": null}),
            ),
            (
                FfmpegKind::Srt,
                serde_json::json!({"type": "srt_stream", "url": "srt://h:9000", "codec": "H.265 (HEVC)", "audio_device": "Mic"}),
            ),
            (
                FfmpegKind::Hls,
                serde_json::json!({"type": "hls_stream", "name": "show", "codec": "AV1", "short_segments": true, "audio_device": null}),
            ),
            (
                FfmpegKind::Dash,
                serde_json::json!({"type": "dash_stream", "name": "show", "codec": "H.264", "audio_device": null}),
            ),
            (
                FfmpegKind::Rtmp,
                serde_json::json!({"type": "rtmp_stream", "url": "rtmp://h/live", "codec": "H.264", "codec_contract": "enhanced", "audio_device": null}),
            ),
        ];
        for (kind, json) in saved {
            let config: SinkConfig = serde_json::from_value(json.clone()).unwrap();
            let decoded: Config = crate::source::decode_config(&config).unwrap();
            let destination = Destination::from_config(kind, &decoded);
            let back = crate::source::encode_config(
                kind.id(),
                &destination.to_config(decoded.audio_device.clone()),
            );
            assert_eq!(serde_json::to_value(back).unwrap(), json, "{kind:?}");
        }
    }

    /// Older stages named the HLS short-segment flag `low_latency`.
    #[test]
    fn the_old_low_latency_name_still_reads() {
        let config: SinkConfig = serde_json::from_value(
            serde_json::json!({"type": "hls_stream", "name": "show", "codec": "H.264", "low_latency": true}),
        )
        .unwrap();
        let decoded: Config = crate::source::decode_config(&config).unwrap();
        assert_eq!(
            Destination::from_config(FfmpegKind::Hls, &decoded),
            Destination::Hls {
                name: "show".into(),
                codec: StreamingCodec::H264,
                short_segments: true
            }
        );
    }

    #[test]
    fn a_codec_setting_round_trips_through_its_choice() {
        let mut sink = FfmpegSink {
            kind: FfmpegKind::Recording,
            destination: Destination::Recording {
                path: "a.mov".into(),
                codec: RecordingCodec::H264,
            },
            audio_device: None,
            subprocess: None,
        };
        let hevc = ControlValue::Float(choice_of(&RecordingCodec::ALL, &RecordingCodec::H265));
        assert_eq!(sink.set_param("codec", &hevc), Ok(ParamEffect::Rebuild));
        assert_eq!(sink.param("codec"), Some(hevc));
        assert!(
            sink.set_param("url", &ControlValue::Text("x".into()))
                .is_err()
        );
    }
}
