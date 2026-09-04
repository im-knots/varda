//! Unified output management and warp calibration.

use std::fmt::Write as _;

use super::super::{UIActions, UIData};
use crate::engine::EngineCommand;
use crate::renderer::context::OutputTarget;

pub(super) fn render_output_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // New output buttons
    ui.horizontal(|ui| {
        if ui.button("+ Windowed").clicked() {
            actions.commands.push(EngineCommand::CreateOutput);
        }
        if ui.button("+ Recording").clicked() {
            use crate::renderer::context::RecordingCodec;
            actions.commands.push(EngineCommand::CreateHeadlessOutput {
                target: OutputTarget::Recording {
                    path: "output.mp4".to_string(),
                    codec: RecordingCodec::H264,
                    audio_device: None,
                },
            });
        }
        if ui.button("+ Stream").clicked() {
            actions.commands.push(EngineCommand::CreateHeadlessOutput {
                target: OutputTarget::NdiSend {
                    sender_name: "Varda NDI".to_string(),
                },
            });
        }
    });

    ui.add_space(4.0);

    // List all outputs (unified)
    if data.outputs.is_empty() {
        ui.label(
            egui::RichText::new("No outputs")
                .small()
                .color(egui::Color32::GRAY),
        );
    } else {
        for (idx, output) in data.outputs.iter().enumerate() {
            egui::Frame::default()
                .inner_margin(6.0)
                .corner_radius(4.0)
                .fill(egui::Color32::from_rgb(30, 30, 45))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Status indicator
                        let status_color = if output.is_active {
                            egui::Color32::from_rgb(80, 255, 80)
                        } else {
                            egui::Color32::from_rgb(128, 128, 128)
                        };
                        ui.colored_label(status_color, "●");
                        ui.label(egui::RichText::new(&output.name).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("x").on_hover_text("Close output").clicked() {
                                actions.commands.push(EngineCommand::CloseOutput {
                                    output_uuid: output.uuid.clone(),
                                });
                            }
                        });
                    });

                    // Target label
                    ui.label(egui::RichText::new(&output.target_label).small().weak());

                    // Audio passthrough health (active outputs with audio only)
                    if let Some(audio) = &output.audio_passthrough {
                        let color = if audio.frames_dropped > 0 {
                            egui::Color32::from_rgb(255, 200, 80)
                        } else {
                            egui::Color32::from_rgb(120, 200, 255)
                        };
                        // Spliced silence is the audible part of a drop, so it
                        // is what the warning reports; the chunk count alone
                        // says nothing about how long the interruption was.
                        let mut text = format!(
                            "♪ {} — {} sent, {} dropped",
                            audio.device, audio.frames_written, audio.frames_dropped
                        );
                        if audio.silence_spliced > 0 {
                            let _ = write!(text, ", {} samples muted", audio.silence_spliced);
                        }
                        ui.label(egui::RichText::new(text).small().color(color));
                    }

                    if let Some(delivery) = &output.delivery {
                        let color = if delivery.frames_dropped > 0 {
                            egui::Color32::from_rgb(255, 200, 80)
                        } else {
                            egui::Color32::from_rgb(120, 200, 255)
                        };
                        let mut text = format!(
                            "▸ {} sent, {} dropped",
                            delivery.frames_written, delivery.frames_dropped
                        );
                        if delivery.frames_padded > 0 {
                            let _ = write!(text, ", {} padded", delivery.frames_padded);
                        }
                        ui.label(egui::RichText::new(text).small().color(color));
                    }

                    // Preview toggle + image
                    {
                        let preview_id = egui::Id::new("output_preview_toggle").with(&output.uuid);
                        let show_preview: bool =
                            ui.data(|d| d.get_temp(preview_id)).unwrap_or(false);
                        let toggle_label = if show_preview {
                            "▼ Hide Preview"
                        } else {
                            "▶ Show Preview"
                        };
                        if ui
                            .small_button(egui::RichText::new(toggle_label).small())
                            .clicked()
                        {
                            ui.data_mut(|d| d.insert_temp(preview_id, !show_preview));
                        }
                        if show_preview {
                            if let Some(&tex_id) = data.output_preview_textures.get(&idx) {
                                let width = ui.available_width().min(320.0);
                                // The output's own texture size, not the render
                                // resolution: a windowed output previews its
                                // window, which can be a different shape.
                                let size = super::utils::preview_size(
                                    egui::vec2(width, width),
                                    output.preview_width,
                                    output.preview_height,
                                );
                                ui.image(egui::load::SizedTexture::new(tex_id, size));
                            } else {
                                ui.label(
                                    egui::RichText::new("No preview available").small().weak(),
                                );
                            }
                        }
                    }

                    if output.is_windowed {
                        // Windowed output controls
                        render_windowed_controls(ui, &output.uuid, output, data, actions);
                    } else {
                        // Headless output controls (recording/SRT/NDI/Syphon)
                        render_headless_controls(ui, &output.uuid, output, data, actions);
                    }
                    render_presentation_controls(ui, &output.uuid, output, actions);
                });
            ui.add_space(4.0);
        }
    }
}

fn render_presentation_controls(
    ui: &mut egui::Ui,
    output_uuid: &str,
    output: &super::super::OutputUI,
    actions: &mut UIActions,
) {
    use crate::engine::value::render::{PresentationDepth, PresentationRequest};

    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("SDR precision:").small());
        egui::ComboBox::from_id_salt(format!("presentation_depth_{output_uuid}"))
            .selected_text(egui::RichText::new(output.presentation_request.depth.label()).small())
            .width(100.0)
            .show_ui(ui, |ui| {
                for depth in PresentationDepth::ALL {
                    if ui
                        .selectable_label(output.presentation_request.depth == depth, depth.label())
                        .clicked()
                    {
                        actions.commands.push(EngineCommand::SetOutputPresentation {
                            output_uuid: output_uuid.to_string(),
                            request: PresentationRequest {
                                depth,
                                dither: output.presentation_request.dither,
                            },
                        });
                    }
                }
            });

        let mut dither = output.presentation_request.dither;
        if ui
            .checkbox(&mut dither, egui::RichText::new("Dither").small())
            .changed()
        {
            actions.commands.push(EngineCommand::SetOutputPresentation {
                output_uuid: output_uuid.to_string(),
                request: PresentationRequest {
                    depth: output.presentation_request.depth,
                    dither,
                },
            });
        }
    });

    let resolved = &output.resolved_presentation;
    let status = format!(
        "Delivering {} · {}",
        resolved.resolved.label(),
        resolved.pixel_format
    );
    let color = if resolved.fallback_reason.is_some() {
        egui::Color32::from_rgb(255, 190, 80)
    } else {
        egui::Color32::from_rgb(120, 200, 255)
    };
    ui.label(egui::RichText::new(status).small().color(color));
    if let Some(reason) = &resolved.fallback_reason {
        ui.label(
            egui::RichText::new(format!("10-bit fallback: {reason}"))
                .small()
                .color(color),
        );
    }
}

/// Controls specific to windowed outputs (display selector, calibration, surfaces).
fn render_windowed_controls(
    ui: &mut egui::Ui,
    output_uuid: &str,
    output: &super::super::OutputUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    // Display target selector
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Display:").small());
        egui::ComboBox::from_id_salt(format!("output_target_{output_uuid}"))
            .selected_text(egui::RichText::new(&output.target_label).small())
            .width(160.0)
            .show_ui(ui, |ui| {
                let is_windowed = matches!(output.target, OutputTarget::Windowed);
                if ui.selectable_label(is_windowed, "Windowed").clicked() {
                    actions.commands.push(EngineCommand::SetOutputTarget {
                        output_uuid: output_uuid.to_string(),
                        target: OutputTarget::Windowed,
                    });
                }
                for monitor in &data.available_monitors {
                    let label = format!("{} ({}x{})", monitor.name, monitor.width, monitor.height);
                    if ui.selectable_label(false, &label).clicked() {
                        actions.commands.push(EngineCommand::SetOutputTarget {
                            output_uuid: output_uuid.to_string(),
                            target: OutputTarget::Display {
                                name: monitor.name.clone(),
                                monitor_index: monitor.index,
                            },
                        });
                    }
                }
            });
    });

    // Rotation selector
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Rotation:").small());
        egui::ComboBox::from_id_salt(format!("output_rotation_{output_uuid}"))
            .selected_text(egui::RichText::new(output.rotation.label()).small())
            .width(80.0)
            .show_ui(ui, |ui| {
                for rot in crate::renderer::context::OutputRotation::ALL {
                    if ui
                        .selectable_label(output.rotation == rot, rot.label())
                        .clicked()
                    {
                        actions.commands.push(EngineCommand::SetOutputRotation {
                            output_uuid: output_uuid.to_string(),
                            rotation: rot,
                        });
                    }
                }
            });
    });

    // Calibration mode selector (Off / Projector test card / per-Surface cards).
    // Warp editing itself now lives in the stage editor's bottom detail bar.
    ui.horizontal(|ui| {
        use crate::renderer::context::CalibrationMode;
        ui.label(egui::RichText::new("🔧 Calibrate:").small());
        for (label, mode) in [
            ("Off", CalibrationMode::Off),
            ("Projector", CalibrationMode::Projector),
            ("Surfaces", CalibrationMode::Surfaces),
        ] {
            if ui
                .selectable_label(output.calibration_mode == mode, label)
                .clicked()
            {
                actions.commands.push(EngineCommand::SetCalibrationMode {
                    output_uuid: output_uuid.to_string(),
                    mode,
                });
            }
        }
    });

    // Surface assignments
    ui.add_space(2.0);
    ui.label(egui::RichText::new("Surfaces:").small().strong());
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(format!("assign_surf_{output_uuid}"))
            .selected_text("+ Assign Surface")
            .width(140.0)
            .show_ui(ui, |ui| {
                for surface in &data.surfaces {
                    let already_assigned = output
                        .surface_assignments
                        .iter()
                        .any(|a| a.surface_uuid == surface.uuid);
                    if !already_assigned && ui.selectable_label(false, &surface.name).clicked() {
                        actions.commands.push(EngineCommand::AssignSurfaceToOutput {
                            output_uuid: output_uuid.to_string(),
                            surface_uuid: surface.uuid.clone(),
                        });
                    }
                }
            });
    });

    for assignment in &output.surface_assignments {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&assignment.surface_name).small());
            if ui.small_button("x").on_hover_text("Unassign").clicked() {
                actions
                    .commands
                    .push(EngineCommand::UnassignSurfaceFromOutput {
                        output_uuid: output_uuid.to_string(),
                        surface_uuid: assignment.surface_uuid.clone(),
                    });
            }
        });
    }

    // Edge blending
    render_edge_blend_controls(ui, output_uuid, output, actions);
}

/// Controls specific to headless outputs (start/stop, duration, inline config).
fn render_headless_controls(
    ui: &mut egui::Ui,
    output_uuid: &str,
    output: &super::super::OutputUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    use crate::renderer::context::RecordingCodec;

    // Inline config for Recording outputs
    if let OutputTarget::Recording {
        ref path,
        ref codec,
        ref audio_device,
    } = output.target
        && !output.is_active
    {
        // Codec selector
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Codec:").small());
            let codec_id = egui::Id::new(format!("rec_codec_{output_uuid}"));
            egui::ComboBox::from_id_salt(codec_id)
                .selected_text(egui::RichText::new(codec.to_string()).small())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for c in &[
                        RecordingCodec::H264,
                        RecordingCodec::H265,
                        RecordingCodec::AV1,
                        RecordingCodec::ProRes,
                        RecordingCodec::ProRes4444,
                        RecordingCodec::Hap,
                        RecordingCodec::HapAlpha,
                        RecordingCodec::HapQ,
                    ] {
                        if ui.selectable_label(*codec == *c, c.to_string()).clicked() {
                            actions.commands.push(EngineCommand::SetOutputTarget {
                                output_uuid: output_uuid.to_string(),
                                target: OutputTarget::Recording {
                                    path: path.clone(),
                                    codec: c.clone(),
                                    audio_device: audio_device.clone(),
                                },
                            });
                        }
                    }
                });
        });
        // File path input
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("File:").small());
            let path_id = egui::Id::new(format!("rec_path_{output_uuid}"));
            let mut current_path: String = ui
                .data(|d| d.get_temp(path_id))
                .unwrap_or_else(|| path.clone());
            let response = ui.add(
                egui::TextEdit::singleline(&mut current_path)
                    .desired_width(160.0)
                    .font(egui::TextStyle::Small),
            );
            if response.lost_focus() || response.changed() {
                ui.data_mut(|d| d.insert_temp(path_id, current_path.clone()));
                if response.lost_focus() {
                    actions.commands.push(EngineCommand::SetOutputTarget {
                        output_uuid: output_uuid.to_string(),
                        target: OutputTarget::Recording {
                            path: current_path,
                            codec: codec.clone(),
                            audio_device: audio_device.clone(),
                        },
                    });
                }
            }
        });
    }

    // Unified stream config (SRT, HLS, DASH, RTMP, NDI, Syphon)
    let is_stream = matches!(
        output.target,
        OutputTarget::SrtStream { .. }
            | OutputTarget::HlsStream { .. }
            | OutputTarget::DashStream { .. }
            | OutputTarget::RtmpStream { .. }
            | OutputTarget::NdiSend { .. }
            | OutputTarget::SyphonServer { .. }
    );
    if is_stream {
        render_stream_config(ui, output_uuid, output, actions);
    }

    // Audio passthrough device selector (ffmpeg targets only; locked while active)
    let is_ffmpeg = matches!(
        output.target,
        OutputTarget::Recording { .. }
            | OutputTarget::SrtStream { .. }
            | OutputTarget::HlsStream { .. }
            | OutputTarget::DashStream { .. }
            | OutputTarget::RtmpStream { .. }
    );
    if is_ffmpeg && !output.is_active {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Audio:").small());
            let current = output.target.audio_device();
            let selected_text = current.unwrap_or("None (silent)");
            egui::ComboBox::from_id_salt(format!("out_audio_{output_uuid}"))
                .selected_text(egui::RichText::new(selected_text).small())
                .width(160.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(current.is_none(), "None (silent)")
                        .clicked()
                    {
                        actions.commands.push(EngineCommand::SetOutputTarget {
                            output_uuid: output_uuid.to_string(),
                            target: output.target.with_audio_device(None),
                        });
                    }
                    for dev in &data.audio.devices {
                        let selected = current == Some(dev.name.as_str());
                        if ui.selectable_label(selected, &dev.name).clicked() {
                            actions.commands.push(EngineCommand::SetOutputTarget {
                                output_uuid: output_uuid.to_string(),
                                target: output.target.with_audio_device(Some(dev.name.clone())),
                            });
                        }
                    }
                });
        });
    }

    // Rotation selector
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Rotation:").small());
        egui::ComboBox::from_id_salt(format!("headless_rotation_{output_uuid}"))
            .selected_text(egui::RichText::new(output.rotation.label()).small())
            .width(80.0)
            .show_ui(ui, |ui| {
                for rot in crate::renderer::context::OutputRotation::ALL {
                    if ui
                        .selectable_label(output.rotation == rot, rot.label())
                        .clicked()
                    {
                        actions.commands.push(EngineCommand::SetOutputRotation {
                            output_uuid: output_uuid.to_string(),
                            rotation: rot,
                        });
                    }
                }
            });
    });

    // Surface assignments
    ui.add_space(2.0);
    ui.label(egui::RichText::new("Surfaces:").small().strong());
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(format!("assign_surf_{output_uuid}"))
            .selected_text("+ Assign Surface")
            .width(140.0)
            .show_ui(ui, |ui| {
                for surface in &data.surfaces {
                    let already_assigned = output
                        .surface_assignments
                        .iter()
                        .any(|a| a.surface_uuid == surface.uuid);
                    if !already_assigned && ui.selectable_label(false, &surface.name).clicked() {
                        actions.commands.push(EngineCommand::AssignSurfaceToOutput {
                            output_uuid: output_uuid.to_string(),
                            surface_uuid: surface.uuid.clone(),
                        });
                    }
                }
            });
    });

    for assignment in &output.surface_assignments {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&assignment.surface_name).small());
            if ui.small_button("x").on_hover_text("Unassign").clicked() {
                actions
                    .commands
                    .push(EngineCommand::UnassignSurfaceFromOutput {
                        output_uuid: output_uuid.to_string(),
                        surface_uuid: assignment.surface_uuid.clone(),
                    });
            }
        });
    }

    // Start/Stop + duration
    ui.horizontal(|ui| {
        if output.is_active {
            let dur = output.active_duration.as_secs_f32();
            ui.label(
                egui::RichText::new(format!("{dur:.1}s"))
                    .monospace()
                    .color(egui::Color32::from_rgb(255, 80, 80)),
            );
            if ui.button("⏹ Stop").clicked() {
                actions.commands.push(EngineCommand::StopOutput {
                    output_uuid: output_uuid.to_string(),
                });
            }
        } else if ui.button("▶ Start").clicked() {
            actions.commands.push(EngineCommand::StartOutput {
                output_uuid: output_uuid.to_string(),
            });
        }
    });

    // Edge blending
    render_edge_blend_controls(ui, output_uuid, output, actions);
}

/// Unified stream output config with protocol dropdown (SRT, HLS, DASH, RTMP, NDI, Syphon).
fn render_stream_config(
    ui: &mut egui::Ui,
    output_uuid: &str,
    output: &super::super::OutputUI,
    actions: &mut UIActions,
) {
    use crate::renderer::context::{RtmpCodecContract, SrtCodec, StreamingCodec};

    // Determine current protocol label
    let current_proto = match &output.target {
        OutputTarget::SrtStream { .. } => "SRT",
        OutputTarget::HlsStream { .. } => "HLS",
        OutputTarget::DashStream { .. } => "DASH",
        OutputTarget::RtmpStream { .. } => "RTMP",
        OutputTarget::NdiSend { .. } => "NDI",
        OutputTarget::SyphonServer { .. } => "Syphon",
        _ => return,
    };

    // Protocol dropdown (disabled while active)
    if output.is_active {
        ui.label(
            egui::RichText::new(format!("Protocol: {current_proto}"))
                .small()
                .weak(),
        );
    } else {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Protocol:").small());
            egui::ComboBox::from_id_salt(format!("stream_proto_{output_uuid}"))
                .selected_text(egui::RichText::new(current_proto).small())
                .width(80.0)
                .show_ui(ui, |ui| {
                    // `mut` is required on macOS for the Syphon push below; on other
                    // platforms that push is compiled out, leaving the binding unused-mut.
                    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
                    let mut protocols: Vec<(&str, OutputTarget)> = vec![
                        (
                            "SRT",
                            OutputTarget::SrtStream {
                                url: "srt://0.0.0.0:9001".to_string(),
                                codec: SrtCodec::default(),
                                audio_device: None,
                            },
                        ),
                        (
                            "HLS",
                            OutputTarget::HlsStream {
                                name: "live".to_string(),
                                codec: StreamingCodec::default(),
                                low_latency: false,
                                audio_device: None,
                            },
                        ),
                        (
                            "DASH",
                            OutputTarget::DashStream {
                                name: "live".to_string(),
                                codec: StreamingCodec::default(),
                                audio_device: None,
                            },
                        ),
                        (
                            "RTMP",
                            OutputTarget::RtmpStream {
                                url: "rtmp://".to_string(),
                                codec: StreamingCodec::default(),
                                codec_contract: RtmpCodecContract::default(),
                                audio_device: None,
                            },
                        ),
                        (
                            "NDI",
                            OutputTarget::NdiSend {
                                sender_name: "Varda NDI".to_string(),
                            },
                        ),
                    ];
                    #[cfg(target_os = "macos")]
                    protocols.push((
                        "Syphon",
                        OutputTarget::SyphonServer {
                            server_name: "Varda".to_string(),
                        },
                    ));
                    for (label, default_target) in &protocols {
                        if ui
                            .selectable_label(current_proto == *label, *label)
                            .clicked()
                            && current_proto != *label
                        {
                            actions.commands.push(EngineCommand::SetOutputTarget {
                                output_uuid: output_uuid.to_string(),
                                target: default_target.clone(),
                            });
                        }
                    }
                });
        });
    }

    // Protocol-specific config
    match &output.target {
        OutputTarget::SrtStream {
            url,
            codec,
            audio_device,
        } => {
            if !output.is_active {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Codec:").small());
                    egui::ComboBox::from_id_salt(format!("srt_codec_{output_uuid}"))
                        .selected_text(egui::RichText::new(codec.to_string()).small())
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for c in &[SrtCodec::H264, SrtCodec::H265] {
                                if ui.selectable_label(*codec == *c, c.to_string()).clicked() {
                                    actions.commands.push(EngineCommand::SetOutputTarget {
                                        output_uuid: output_uuid.to_string(),
                                        target: OutputTarget::SrtStream {
                                            url: url.clone(),
                                            codec: c.clone(),
                                            audio_device: audio_device.clone(),
                                        },
                                    });
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("URL:").small());
                    let url_id = egui::Id::new(format!("srt_url_{output_uuid}"));
                    let mut current_url: String = ui
                        .data(|d| d.get_temp(url_id))
                        .unwrap_or_else(|| url.clone());
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut current_url)
                            .desired_width(180.0)
                            .font(egui::TextStyle::Small),
                    );
                    if response.lost_focus() || response.changed() {
                        ui.data_mut(|d| d.insert_temp(url_id, current_url.clone()));
                        if response.lost_focus() {
                            actions.commands.push(EngineCommand::SetOutputTarget {
                                output_uuid: output_uuid.to_string(),
                                target: OutputTarget::SrtStream {
                                    url: current_url,
                                    codec: codec.clone(),
                                    audio_device: audio_device.clone(),
                                },
                            });
                        }
                    }
                });
            }
        }
        OutputTarget::HlsStream {
            name,
            codec,
            low_latency,
            audio_device,
        } => {
            render_hls_dash_name_codec(
                ui,
                output_uuid,
                "hls",
                name,
                codec,
                output.is_active,
                actions,
                |n, c| OutputTarget::HlsStream {
                    name: n,
                    codec: c,
                    low_latency: *low_latency,
                    audio_device: audio_device.clone(),
                },
            );
            if !output.is_active {
                ui.horizontal(|ui| {
                    let mut ll = *low_latency;
                    if ui
                        .checkbox(&mut ll, egui::RichText::new("LL-HLS (Low Latency)").small())
                        .changed()
                    {
                        actions.commands.push(EngineCommand::SetOutputTarget {
                            output_uuid: output_uuid.to_string(),
                            target: OutputTarget::HlsStream {
                                name: name.clone(),
                                codec: codec.clone(),
                                low_latency: ll,
                                audio_device: audio_device.clone(),
                            },
                        });
                    }
                });
            }
            let player_url = format!("http://localhost:8080/streams/{name}/player.html");
            let manifest_url = format!("http://localhost:8080/streams/{name}/index.m3u8");
            render_copyable_url(ui, "▶", &player_url, 10.0, actions);
            render_copyable_url(ui, "🌐", &manifest_url, 9.0, actions);
        }
        OutputTarget::DashStream {
            name,
            codec,
            audio_device,
        } => {
            render_hls_dash_name_codec(
                ui,
                output_uuid,
                "dash",
                name,
                codec,
                output.is_active,
                actions,
                |n, c| OutputTarget::DashStream {
                    name: n,
                    codec: c,
                    audio_device: audio_device.clone(),
                },
            );
            let player_url = format!("http://localhost:8080/streams/{name}/player.html");
            let manifest_url = format!("http://localhost:8080/streams/{name}/manifest.mpd");
            render_copyable_url(ui, "▶", &player_url, 10.0, actions);
            render_copyable_url(ui, "🌐", &manifest_url, 9.0, actions);
        }
        OutputTarget::RtmpStream {
            url,
            codec,
            codec_contract,
            audio_device,
        } => {
            if !output.is_active {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Codec:").small());
                    egui::ComboBox::from_id_salt(format!("rtmp_codec_{output_uuid}"))
                        .selected_text(egui::RichText::new(codec.to_string()).small())
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for c in &[
                                StreamingCodec::H264,
                                StreamingCodec::H265,
                                StreamingCodec::AV1,
                            ] {
                                if ui.selectable_label(*codec == *c, c.to_string()).clicked() {
                                    actions.commands.push(EngineCommand::SetOutputTarget {
                                        output_uuid: output_uuid.to_string(),
                                        target: OutputTarget::RtmpStream {
                                            url: url.clone(),
                                            codec: c.clone(),
                                            codec_contract: *codec_contract,
                                            audio_device: audio_device.clone(),
                                        },
                                    });
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Endpoint:").small());
                    egui::ComboBox::from_id_salt(format!("rtmp_contract_{output_uuid}"))
                        .selected_text(match codec_contract {
                            RtmpCodecContract::Legacy => "Legacy RTMP",
                            RtmpCodecContract::Enhanced => "Enhanced RTMP",
                        })
                        .show_ui(ui, |ui| {
                            for (contract, label) in [
                                (RtmpCodecContract::Legacy, "Legacy RTMP"),
                                (RtmpCodecContract::Enhanced, "Enhanced RTMP"),
                            ] {
                                if ui
                                    .selectable_label(*codec_contract == contract, label)
                                    .clicked()
                                {
                                    actions.commands.push(EngineCommand::SetOutputTarget {
                                        output_uuid: output_uuid.to_string(),
                                        target: OutputTarget::RtmpStream {
                                            url: url.clone(),
                                            codec: codec.clone(),
                                            codec_contract: contract,
                                            audio_device: audio_device.clone(),
                                        },
                                    });
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("URL:").small());
                    let url_id = egui::Id::new(format!("rtmp_url_{output_uuid}"));
                    let mut current_url: String = ui
                        .data(|d| d.get_temp(url_id))
                        .unwrap_or_else(|| url.clone());
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut current_url)
                            .desired_width(180.0)
                            .font(egui::TextStyle::Small),
                    );
                    if response.lost_focus() || response.changed() {
                        ui.data_mut(|d| d.insert_temp(url_id, current_url.clone()));
                        if response.lost_focus() {
                            actions.commands.push(EngineCommand::SetOutputTarget {
                                output_uuid: output_uuid.to_string(),
                                target: OutputTarget::RtmpStream {
                                    url: current_url,
                                    codec: codec.clone(),
                                    codec_contract: *codec_contract,
                                    audio_device: audio_device.clone(),
                                },
                            });
                        }
                    }
                });
            }
        }
        OutputTarget::NdiSend { sender_name } if !output.is_active => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Name:").small());
                let name_id = egui::Id::new(format!("ndi_name_{output_uuid}"));
                let mut current_name: String = ui
                    .data(|d| d.get_temp(name_id))
                    .unwrap_or_else(|| sender_name.clone());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut current_name)
                        .desired_width(140.0)
                        .font(egui::TextStyle::Small),
                );
                if response.lost_focus() || response.changed() {
                    ui.data_mut(|d| d.insert_temp(name_id, current_name.clone()));
                    if response.lost_focus() {
                        actions.commands.push(EngineCommand::SetOutputTarget {
                            output_uuid: output_uuid.to_string(),
                            target: OutputTarget::NdiSend {
                                sender_name: current_name,
                            },
                        });
                    }
                }
            });
        }
        OutputTarget::SyphonServer { server_name } if !output.is_active => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Name:").small());
                let name_id = egui::Id::new(format!("syphon_name_{output_uuid}"));
                let mut current_name: String = ui
                    .data(|d| d.get_temp(name_id))
                    .unwrap_or_else(|| server_name.clone());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut current_name)
                        .desired_width(140.0)
                        .font(egui::TextStyle::Small),
                );
                if response.lost_focus() || response.changed() {
                    ui.data_mut(|d| d.insert_temp(name_id, current_name.clone()));
                    if response.lost_focus() {
                        actions.commands.push(EngineCommand::SetOutputTarget {
                            output_uuid: output_uuid.to_string(),
                            target: OutputTarget::SyphonServer {
                                server_name: current_name,
                            },
                        });
                    }
                }
            });
        }
        _ => {}
    }
}

/// Render a clickable URL label that copies to clipboard on click.
fn render_copyable_url(
    ui: &mut egui::Ui,
    icon: &str,
    url: &str,
    font_size: f32,
    actions: &mut UIActions,
) {
    let text = format!("{icon} {url}");
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(&text)
                .size(font_size)
                .color(egui::Color32::from_rgb(130, 160, 200)),
        )
        .sense(egui::Sense::click()),
    );
    if response.clicked() {
        ui.ctx().copy_text(url.to_string());
        actions
            .session
            .info_notifications
            .push(format!("📋 Copied to clipboard: {url}"));
    }
    response.on_hover_text("Click to copy URL");
}

/// Shared codec + name config for HLS and DASH stream outputs.
// UI render fn taking many independent egui state/handle args; no shared invariant to bundle.
#[allow(clippy::too_many_arguments)]
fn render_hls_dash_name_codec(
    ui: &mut egui::Ui,
    output_uuid: &str,
    prefix: &str,
    name: &str,
    codec: &crate::renderer::context::StreamingCodec,
    is_active: bool,
    actions: &mut UIActions,
    make_target: impl Fn(String, crate::renderer::context::StreamingCodec) -> OutputTarget,
) {
    use crate::renderer::context::StreamingCodec;
    if !is_active {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Codec:").small());
            egui::ComboBox::from_id_salt(format!("{prefix}_codec_{output_uuid}"))
                .selected_text(egui::RichText::new(codec.to_string()).small())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for c in &[
                        StreamingCodec::H264,
                        StreamingCodec::H265,
                        StreamingCodec::AV1,
                    ] {
                        if ui.selectable_label(*codec == *c, c.to_string()).clicked() {
                            actions.commands.push(EngineCommand::SetOutputTarget {
                                output_uuid: output_uuid.to_string(),
                                target: make_target(name.to_string(), c.clone()),
                            });
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Name:").small());
            let name_id = egui::Id::new(format!("{prefix}_name_{output_uuid}"));
            let mut current_name: String = ui
                .data(|d| d.get_temp(name_id))
                .unwrap_or_else(|| name.to_string());
            let response = ui.add(
                egui::TextEdit::singleline(&mut current_name)
                    .desired_width(140.0)
                    .font(egui::TextStyle::Small),
            );
            if response.lost_focus() || response.changed() {
                ui.data_mut(|d| d.insert_temp(name_id, current_name.clone()));
                if response.lost_focus() {
                    actions.commands.push(EngineCommand::SetOutputTarget {
                        output_uuid: output_uuid.to_string(),
                        target: make_target(current_name, codec.clone()),
                    });
                }
            }
        });
    }
}

/// Render edge blending controls for an output (shared by windowed and headless).
fn render_edge_blend_controls(
    ui: &mut egui::Ui,
    output_uuid: &str,
    output: &super::super::OutputUI,
    actions: &mut UIActions,
) {
    use crate::renderer::edge_blend::EdgeBlendMode;

    let collapse_id = egui::Id::new("edge_blend_section").with(output_uuid);
    egui::CollapsingHeader::new(egui::RichText::new("Edge Blending").small().strong())
        .id_salt(collapse_id)
        .default_open(false)
        .show(ui, |ui| {
            // Mode toggle: Auto / Manual
            let is_auto = output.edge_blend_mode == EdgeBlendMode::Auto;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Mode:").small());
                if ui
                    .selectable_label(!is_auto, egui::RichText::new("Manual").small())
                    .clicked()
                    && is_auto
                {
                    actions.commands.push(EngineCommand::SetEdgeBlendMode {
                        output_uuid: output_uuid.to_string(),
                        mode: EdgeBlendMode::Manual,
                    });
                }
                if ui
                    .selectable_label(is_auto, egui::RichText::new("Auto").small())
                    .clicked()
                    && !is_auto
                {
                    actions.commands.push(EngineCommand::SetEdgeBlendMode {
                        output_uuid: output_uuid.to_string(),
                        mode: EdgeBlendMode::Auto,
                    });
                }
            });

            let mut cfg = output.edge_blend;
            let mut changed = false;

            if is_auto {
                // Auto mode: show per-surface overlap zones (read-only)
                let mut any_zones = false;
                for sa in &output.surface_assignments {
                    if sa.overlap_zones.any_enabled() {
                        any_zones = true;
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{}:", sa.surface_name)).small());
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} zone(s)",
                                    sa.overlap_zones.zones.len()
                                ))
                                .small()
                                .weak(),
                            );
                        });
                        for (zi, zone) in sa.overlap_zones.zones.iter().enumerate() {
                            let dir = match (zone.ramp_x as i32, zone.ramp_y as i32) {
                                (1, 0) => "→",
                                (-1, 0) => "←",
                                (0, 1) => "↓",
                                (0, -1) => "↑",
                                (1, 1) => "↘",
                                (-1, 1) => "↙",
                                (1, -1) => "↗",
                                (-1, -1) => "↖",
                                _ => "·",
                            };
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Zone {}: UV [{:.2},{:.2}]→[{:.2},{:.2}] {} γ:{:.1}",
                                        zi + 1,
                                        zone.uv_rect[0],
                                        zone.uv_rect[1],
                                        zone.uv_rect[2],
                                        zone.uv_rect[3],
                                        dir,
                                        zone.gamma,
                                    ))
                                    .small()
                                    .weak(),
                                );
                            });
                        }
                    }
                }
                if !any_zones {
                    ui.label(
                        egui::RichText::new("No overlapping surfaces detected")
                            .small()
                            .weak(),
                    );
                }
            } else {
                // Manual mode: full per-edge controls (existing behavior)
                for (label, edge) in [
                    ("Left", &mut cfg.left),
                    ("Right", &mut cfg.right),
                    ("Top", &mut cfg.top),
                    ("Bottom", &mut cfg.bottom),
                ] {
                    ui.horizontal(|ui| {
                        if ui
                            .checkbox(&mut edge.enabled, egui::RichText::new(label).small())
                            .changed()
                        {
                            changed = true;
                        }
                        if edge.enabled {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("W:").small());
                            if ui
                                .add(
                                    egui::Slider::new(&mut edge.width, 0.01..=0.5)
                                        .step_by(0.01)
                                        .max_decimals(2),
                                )
                                .on_hover_text("Blend zone width (fraction of output)")
                                .changed()
                            {
                                changed = true;
                            }
                            ui.label(egui::RichText::new("γ:").small());
                            if ui
                                .add(
                                    egui::Slider::new(&mut edge.gamma, 0.5..=4.0)
                                        .step_by(0.1)
                                        .max_decimals(1),
                                )
                                .on_hover_text("Gamma curve exponent")
                                .changed()
                            {
                                changed = true;
                            }
                        }
                    });
                }
            }

            if changed {
                actions.commands.push(EngineCommand::SetEdgeBlend {
                    output_uuid: output_uuid.to_string(),
                    config: cfg,
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    fn sample_output(
        resolved: crate::engine::value::render::ResolvedPresentation,
    ) -> crate::usecases::ui::OutputUI {
        crate::usecases::ui::OutputUI {
            uuid: "out00001".to_string(),
            name: "Main".to_string(),
            target: OutputTarget::Windowed,
            target_label: "Windowed".to_string(),
            is_windowed: true,
            is_active: true,
            active_duration: std::time::Duration::ZERO,
            surface_assignments: vec![],
            calibration_mode: crate::renderer::context::CalibrationMode::Off,
            edge_blend_mode: crate::renderer::edge_blend::EdgeBlendMode::default(),
            edge_blend: crate::renderer::edge_blend::EdgeBlendConfig::default(),
            rotation: crate::renderer::context::OutputRotation::default(),
            presentation_request: crate::engine::value::render::PresentationRequest::default(),
            resolved_presentation: resolved,
            audio_passthrough: None,
            delivery: None,
            preview_width: 1920,
            preview_height: 1080,
        }
    }

    #[test]
    fn render_output_section_smoke() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_output_section(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_output_section_smoke_with_outputs() {
        let mut data = UIData::test_fixture();
        data.outputs.push(sample_output(
            crate::engine::value::render::ResolvedPresentation::default(),
        ));
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_output_section(ui, &data, &mut actions);
        });
    }

    #[test]
    fn dither_checkbox_emits_set_output_presentation() {
        let mut data = UIData::test_fixture();
        data.outputs.push(sample_output(
            crate::engine::value::render::ResolvedPresentation::default(),
        ));
        let mut actions = UIActions::new();
        {
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(420.0, 640.0))
                .build_ui(|ui| {
                    render_output_section(ui, &data, &mut actions);
                });
            harness.get_by_label("Dither").click();
            harness.run();
        }
        assert!(actions.commands.iter().any(|command| matches!(
            command,
            EngineCommand::SetOutputPresentation {
                output_uuid,
                request
            } if output_uuid == "out00001" && !request.dither
        )));
    }

    #[test]
    fn fallback_reason_is_visible_on_the_output_card() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationPixelFormat,
            ResolvedPresentation,
        };
        let mut data = UIData::test_fixture();
        data.outputs.push(sample_output(ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr8,
            pixel_format: PresentationPixelFormat::Bgra8,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Premultiplied,
            dither: true,
            fallback_reason: Some("Syphon interoperability is limited to BGRA8".into()),
        }));
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 640.0))
            .build_ui(|ui| {
                render_output_section(ui, &data, &mut actions);
            });
        harness.run();
        harness.get_by_label("Delivering 8-bit SDR · BGRA8");
        harness.get_by_label("10-bit fallback: Syphon interoperability is limited to BGRA8");
    }
}
