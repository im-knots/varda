//! Unified output management and warp calibration.

use super::super::{OutputAction, UIActions, UIData};
use crate::renderer::context::OutputTarget;

pub(super) fn render_output_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // New output buttons
    ui.horizontal(|ui| {
        if ui.button("+ Windowed").clicked() {
            actions.output_actions.push(OutputAction::Create);
        }
        if ui.button("+ Recording").clicked() {
            use crate::renderer::context::RecordingCodec;
            actions.output_actions.push(OutputAction::CreateHeadless {
                target: OutputTarget::Recording {
                    path: "output.mp4".to_string(),
                    codec: RecordingCodec::H264,
                    audio_device: None,
                },
            });
        }
        if ui.button("+ Stream").clicked() {
            actions.output_actions.push(OutputAction::CreateHeadless {
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
                                actions.output_actions.push(OutputAction::Close { idx });
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
                        ui.label(
                            egui::RichText::new(format!(
                                "♪ {} — {} sent, {} dropped",
                                audio.device, audio.frames_written, audio.frames_dropped
                            ))
                            .small()
                            .color(color),
                        );
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
                                let preview_width = ui.available_width().min(320.0);
                                let preview_height = preview_width * 9.0 / 16.0;
                                ui.image(egui::load::SizedTexture::new(
                                    tex_id,
                                    egui::vec2(preview_width, preview_height),
                                ));
                            } else {
                                ui.label(
                                    egui::RichText::new("No preview available").small().weak(),
                                );
                            }
                        }
                    }

                    if output.is_windowed {
                        // Windowed output controls
                        render_windowed_controls(ui, idx, output, data, actions);
                    } else {
                        // Headless output controls (recording/SRT/NDI/Syphon)
                        render_headless_controls(ui, idx, output, data, actions);
                    }
                });
            ui.add_space(4.0);
        }
    }
}

/// Controls specific to windowed outputs (display selector, calibration, surfaces).
fn render_windowed_controls(
    ui: &mut egui::Ui,
    idx: usize,
    output: &super::super::OutputUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    // Display target selector
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Display:").small());
        egui::ComboBox::from_id_salt(format!("output_target_{}", idx))
            .selected_text(egui::RichText::new(&output.target_label).small())
            .width(160.0)
            .show_ui(ui, |ui| {
                let is_windowed = matches!(output.target, OutputTarget::Windowed);
                if ui.selectable_label(is_windowed, "Windowed").clicked() {
                    actions.output_actions.push(OutputAction::SetTarget {
                        idx,
                        target: OutputTarget::Windowed,
                    });
                }
                for monitor in &data.available_monitors {
                    let label = format!("{} ({}x{})", monitor.name, monitor.width, monitor.height);
                    if ui.selectable_label(false, &label).clicked() {
                        actions.output_actions.push(OutputAction::SetTarget {
                            idx,
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
        egui::ComboBox::from_id_salt(format!("output_rotation_{}", idx))
            .selected_text(egui::RichText::new(output.rotation.label()).small())
            .width(80.0)
            .show_ui(ui, |ui| {
                for rot in crate::renderer::context::OutputRotation::ALL {
                    if ui
                        .selectable_label(output.rotation == rot, rot.label())
                        .clicked()
                    {
                        actions
                            .output_actions
                            .push(OutputAction::SetRotation { idx, rotation: rot });
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
                actions
                    .output_actions
                    .push(OutputAction::SetCalibrationMode { idx, mode });
            }
        }
    });

    // Surface assignments
    ui.add_space(2.0);
    ui.label(egui::RichText::new("Surfaces:").small().strong());
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(format!("assign_surf_{}", idx))
            .selected_text("+ Assign Surface")
            .width(140.0)
            .show_ui(ui, |ui| {
                for surface in data.surfaces.iter() {
                    let already_assigned = output
                        .surface_assignments
                        .iter()
                        .any(|a| a.surface_uuid == surface.uuid);
                    if !already_assigned && ui.selectable_label(false, &surface.name).clicked() {
                        actions.output_actions.push(OutputAction::AssignSurface {
                            output_idx: idx,
                            surface_uuid: surface.uuid.clone(),
                        });
                    }
                }
            });
    });

    for (ai, assignment) in output.surface_assignments.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&assignment.surface_name).small());
            if ui.small_button("x").on_hover_text("Unassign").clicked() {
                actions.output_actions.push(OutputAction::UnassignSurface {
                    output_idx: idx,
                    assignment_idx: ai,
                });
            }
        });
    }

    // Edge blending
    render_edge_blend_controls(ui, idx, output, actions);
}

/// Controls specific to headless outputs (start/stop, duration, inline config).
fn render_headless_controls(
    ui: &mut egui::Ui,
    idx: usize,
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
    {
        if !output.is_active {
            // Codec selector
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Codec:").small());
                let codec_id = egui::Id::new(format!("rec_codec_{}", idx));
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
                                actions.output_actions.push(OutputAction::SetTarget {
                                    idx,
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
                let path_id = egui::Id::new(format!("rec_path_{}", idx));
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
                        actions.output_actions.push(OutputAction::SetTarget {
                            idx,
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
        render_stream_config(ui, idx, output, actions);
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
            egui::ComboBox::from_id_salt(format!("out_audio_{}", idx))
                .selected_text(egui::RichText::new(selected_text).small())
                .width(160.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(current.is_none(), "None (silent)")
                        .clicked()
                    {
                        actions.output_actions.push(OutputAction::SetTarget {
                            idx,
                            target: output.target.with_audio_device(None),
                        });
                    }
                    for dev in &data.audio.devices {
                        let selected = current == Some(dev.name.as_str());
                        if ui.selectable_label(selected, &dev.name).clicked() {
                            actions.output_actions.push(OutputAction::SetTarget {
                                idx,
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
        egui::ComboBox::from_id_salt(format!("headless_rotation_{}", idx))
            .selected_text(egui::RichText::new(output.rotation.label()).small())
            .width(80.0)
            .show_ui(ui, |ui| {
                for rot in crate::renderer::context::OutputRotation::ALL {
                    if ui
                        .selectable_label(output.rotation == rot, rot.label())
                        .clicked()
                    {
                        actions
                            .output_actions
                            .push(OutputAction::SetRotation { idx, rotation: rot });
                    }
                }
            });
    });

    // Surface assignments
    ui.add_space(2.0);
    ui.label(egui::RichText::new("Surfaces:").small().strong());
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(format!("assign_surf_{}", idx))
            .selected_text("+ Assign Surface")
            .width(140.0)
            .show_ui(ui, |ui| {
                for surface in data.surfaces.iter() {
                    let already_assigned = output
                        .surface_assignments
                        .iter()
                        .any(|a| a.surface_uuid == surface.uuid);
                    if !already_assigned && ui.selectable_label(false, &surface.name).clicked() {
                        actions.output_actions.push(OutputAction::AssignSurface {
                            output_idx: idx,
                            surface_uuid: surface.uuid.clone(),
                        });
                    }
                }
            });
    });

    for (ai, assignment) in output.surface_assignments.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&assignment.surface_name).small());
            if ui.small_button("x").on_hover_text("Unassign").clicked() {
                actions.output_actions.push(OutputAction::UnassignSurface {
                    output_idx: idx,
                    assignment_idx: ai,
                });
            }
        });
    }

    // Start/Stop + duration
    ui.horizontal(|ui| {
        if output.is_active {
            let dur = output.active_duration.as_secs_f32();
            ui.label(
                egui::RichText::new(format!("{:.1}s", dur))
                    .monospace()
                    .color(egui::Color32::from_rgb(255, 80, 80)),
            );
            if ui.button("⏹ Stop").clicked() {
                actions.output_actions.push(OutputAction::Stop { idx });
            }
        } else if ui.button("▶ Start").clicked() {
            actions.output_actions.push(OutputAction::Start { idx });
        }
    });

    // Edge blending
    render_edge_blend_controls(ui, idx, output, actions);
}

/// Unified stream output config with protocol dropdown (SRT, HLS, DASH, RTMP, NDI, Syphon).
fn render_stream_config(
    ui: &mut egui::Ui,
    idx: usize,
    output: &super::super::OutputUI,
    actions: &mut UIActions,
) {
    use crate::renderer::context::{SrtCodec, StreamingCodec};

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
    if !output.is_active {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Protocol:").small());
            egui::ComboBox::from_id_salt(format!("stream_proto_{}", idx))
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
                            actions.output_actions.push(OutputAction::SetTarget {
                                idx,
                                target: default_target.clone(),
                            });
                        }
                    }
                });
        });
    } else {
        ui.label(
            egui::RichText::new(format!("Protocol: {}", current_proto))
                .small()
                .weak(),
        );
    }

    // Protocol-specific config
    match &output.target {
        OutputTarget::SrtStream {
            ref url,
            ref codec,
            ref audio_device,
        } => {
            if !output.is_active {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Codec:").small());
                    egui::ComboBox::from_id_salt(format!("srt_codec_{}", idx))
                        .selected_text(egui::RichText::new(codec.to_string()).small())
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for c in &[SrtCodec::H264, SrtCodec::H265] {
                                if ui.selectable_label(*codec == *c, c.to_string()).clicked() {
                                    actions.output_actions.push(OutputAction::SetTarget {
                                        idx,
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
                    let url_id = egui::Id::new(format!("srt_url_{}", idx));
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
                            actions.output_actions.push(OutputAction::SetTarget {
                                idx,
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
            ref name,
            ref codec,
            low_latency,
            ref audio_device,
        } => {
            render_hls_dash_name_codec(
                ui,
                idx,
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
                        actions.output_actions.push(OutputAction::SetTarget {
                            idx,
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
            let player_url = format!("http://localhost:8080/streams/{}/player.html", name);
            let manifest_url = format!("http://localhost:8080/streams/{}/index.m3u8", name);
            render_copyable_url(ui, "▶", &player_url, 10.0, actions);
            render_copyable_url(ui, "🌐", &manifest_url, 9.0, actions);
        }
        OutputTarget::DashStream {
            ref name,
            ref codec,
            ref audio_device,
        } => {
            render_hls_dash_name_codec(
                ui,
                idx,
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
            let player_url = format!("http://localhost:8080/streams/{}/player.html", name);
            let manifest_url = format!("http://localhost:8080/streams/{}/manifest.mpd", name);
            render_copyable_url(ui, "▶", &player_url, 10.0, actions);
            render_copyable_url(ui, "🌐", &manifest_url, 9.0, actions);
        }
        OutputTarget::RtmpStream {
            ref url,
            ref codec,
            ref audio_device,
        } => {
            if !output.is_active {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Codec:").small());
                    egui::ComboBox::from_id_salt(format!("rtmp_codec_{}", idx))
                        .selected_text(egui::RichText::new(codec.to_string()).small())
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for c in &[
                                StreamingCodec::H264,
                                StreamingCodec::H265,
                                StreamingCodec::AV1,
                            ] {
                                if ui.selectable_label(*codec == *c, c.to_string()).clicked() {
                                    actions.output_actions.push(OutputAction::SetTarget {
                                        idx,
                                        target: OutputTarget::RtmpStream {
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
                    let url_id = egui::Id::new(format!("rtmp_url_{}", idx));
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
                            actions.output_actions.push(OutputAction::SetTarget {
                                idx,
                                target: OutputTarget::RtmpStream {
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
        OutputTarget::NdiSend { ref sender_name } if !output.is_active => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Name:").small());
                let name_id = egui::Id::new(format!("ndi_name_{}", idx));
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
                        actions.output_actions.push(OutputAction::SetTarget {
                            idx,
                            target: OutputTarget::NdiSend {
                                sender_name: current_name,
                            },
                        });
                    }
                }
            });
        }
        OutputTarget::SyphonServer { ref server_name } if !output.is_active => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Name:").small());
                let name_id = egui::Id::new(format!("syphon_name_{}", idx));
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
                        actions.output_actions.push(OutputAction::SetTarget {
                            idx,
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
    let text = format!("{} {}", icon, url);
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
            .info_notifications
            .push(format!("📋 Copied to clipboard: {}", url));
    }
    response.on_hover_text("Click to copy URL");
}

/// Shared codec + name config for HLS and DASH stream outputs.
// UI render fn taking many independent egui state/handle args; no shared invariant to bundle.
#[allow(clippy::too_many_arguments)]
fn render_hls_dash_name_codec(
    ui: &mut egui::Ui,
    idx: usize,
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
            egui::ComboBox::from_id_salt(format!("{}_codec_{}", prefix, idx))
                .selected_text(egui::RichText::new(codec.to_string()).small())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for c in &[
                        StreamingCodec::H264,
                        StreamingCodec::H265,
                        StreamingCodec::AV1,
                    ] {
                        if ui.selectable_label(*codec == *c, c.to_string()).clicked() {
                            actions.output_actions.push(OutputAction::SetTarget {
                                idx,
                                target: make_target(name.to_string(), c.clone()),
                            });
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Name:").small());
            let name_id = egui::Id::new(format!("{}_name_{}", prefix, idx));
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
                    actions.output_actions.push(OutputAction::SetTarget {
                        idx,
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
    idx: usize,
    output: &super::super::OutputUI,
    actions: &mut UIActions,
) {
    use crate::renderer::edge_blend::EdgeBlendMode;

    let collapse_id = egui::Id::new("edge_blend_section").with(idx);
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
                    actions.output_actions.push(OutputAction::SetEdgeBlendMode {
                        output_idx: idx,
                        mode: EdgeBlendMode::Manual,
                    });
                }
                if ui
                    .selectable_label(is_auto, egui::RichText::new("Auto").small())
                    .clicked()
                    && !is_auto
                {
                    actions.output_actions.push(OutputAction::SetEdgeBlendMode {
                        output_idx: idx,
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
                actions.output_actions.push(OutputAction::SetEdgeBlend {
                    output_idx: idx,
                    config: cfg,
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        data.outputs.push(crate::usecases::ui::OutputUI {
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
            audio_passthrough: None,
        });
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_output_section(ui, &data, &mut actions);
        });
    }
}
