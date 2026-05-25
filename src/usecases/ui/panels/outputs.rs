//! Unified output management and warp calibration.

use crate::renderer::context::OutputTarget;
use super::super::{UIData, UIActions, OutputAction};

pub(super) fn render_output_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    // New output buttons
    ui.horizontal(|ui| {
        if ui.button("+ Windowed").clicked() {
            actions.output_actions.push(OutputAction::Create);
        }
        if ui.button("+ Recording").clicked() {
            use crate::renderer::context::RecordingCodec;
            actions.output_actions.push(OutputAction::CreateHeadless {
                target: OutputTarget::Recording { path: "output.mp4".to_string(), codec: RecordingCodec::H264 },
            });
        }
        if ui.button("+ Stream").clicked() {
            actions.output_actions.push(OutputAction::CreateHeadless {
                target: OutputTarget::NdiSend { sender_name: "Varda NDI".to_string() },
            });
        }
    });

    ui.add_space(4.0);

    // List all outputs (unified)
    if data.outputs.is_empty() {
        ui.label(egui::RichText::new("No outputs").small().color(egui::Color32::GRAY));
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

                    // Preview toggle + image
                    {
                        let preview_id = egui::Id::new("output_preview_toggle").with(&output.uuid);
                        let show_preview: bool = ui.data(|d| d.get_temp(preview_id)).unwrap_or(false);
                        let toggle_label = if show_preview { "▼ Hide Preview" } else { "▶ Show Preview" };
                        if ui.small_button(egui::RichText::new(toggle_label).small()).clicked() {
                            ui.data_mut(|d| d.insert_temp(preview_id, !show_preview));
                        }
                        if show_preview {
                            if let Some(&tex_id) = data.output_preview_textures.get(&idx) {
                                let preview_width = ui.available_width().min(320.0);
                                let preview_height = preview_width * 9.0 / 16.0;
                                ui.image(egui::load::SizedTexture::new(tex_id, egui::vec2(preview_width, preview_height)));
                            } else {
                                ui.label(egui::RichText::new("No preview available").small().weak());
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
fn render_windowed_controls(ui: &mut egui::Ui, idx: usize, output: &super::super::OutputUI, data: &UIData, actions: &mut UIActions) {
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
                        idx, target: OutputTarget::Windowed,
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
                    if ui.selectable_label(output.rotation == rot, rot.label()).clicked() {
                        actions.output_actions.push(OutputAction::SetRotation {
                            idx, rotation: rot,
                        });
                    }
                }
            });
    });

    // Calibration toggle
    ui.horizontal(|ui| {
        let cal_label = if output.calibration_mode { "🔧 Done" } else { "🔧 Calibrate" };
        if ui.button(egui::RichText::new(cal_label).small()).clicked() {
            actions.output_actions.push(OutputAction::ToggleCalibration { idx });
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
                    let already_assigned = output.surface_assignments.iter().any(|a| a.surface_uuid == surface.uuid);
                    if !already_assigned {
                        if ui.selectable_label(false, &surface.name).clicked() {
                            actions.output_actions.push(OutputAction::AssignSurface {
                                output_idx: idx, surface_uuid: surface.uuid.clone(),
                            });
                        }
                    }
                }
            });
    });

    for (ai, assignment) in output.surface_assignments.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&assignment.surface_name).small());
            if ui.small_button("↺").on_hover_text("Reset warp").clicked() {
                actions.output_actions.push(OutputAction::ResetWarp { output_idx: idx, assignment_idx: ai });
            }
            if ui.small_button("x").on_hover_text("Unassign").clicked() {
                actions.output_actions.push(OutputAction::UnassignSurface { output_idx: idx, assignment_idx: ai });
            }
        });
    }

    if output.calibration_mode && !output.surface_assignments.is_empty() {
        render_warp_calibration(ui, idx, output, actions);
    }

    // Edge blending
    render_edge_blend_controls(ui, idx, output, actions);
}

/// Controls specific to headless outputs (start/stop, duration, inline config).
fn render_headless_controls(ui: &mut egui::Ui, idx: usize, output: &super::super::OutputUI, data: &UIData, actions: &mut UIActions) {
    use crate::renderer::context::RecordingCodec;

    // Inline config for Recording outputs
    if let OutputTarget::Recording { ref path, ref codec } = output.target {
        if !output.is_active {
            // Codec selector
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Codec:").small());
                let codec_id = egui::Id::new(format!("rec_codec_{}", idx));
                egui::ComboBox::from_id_salt(codec_id)
                    .selected_text(egui::RichText::new(codec.to_string()).small())
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for c in &[RecordingCodec::H264, RecordingCodec::H265, RecordingCodec::AV1, RecordingCodec::ProRes, RecordingCodec::Hap, RecordingCodec::HapAlpha, RecordingCodec::HapQ] {
                            if ui.selectable_label(*codec == *c, c.to_string()).clicked() {
                                actions.output_actions.push(OutputAction::SetTarget {
                                    idx,
                                    target: OutputTarget::Recording { path: path.clone(), codec: c.clone() },
                                });
                            }
                        }
                    });
            });
            // File path input
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("File:").small());
                let path_id = egui::Id::new(format!("rec_path_{}", idx));
                let mut current_path: String = ui.data(|d| d.get_temp(path_id))
                    .unwrap_or_else(|| path.clone());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut current_path)
                        .desired_width(160.0)
                        .font(egui::TextStyle::Small)
                );
                if response.lost_focus() || response.changed() {
                    ui.data_mut(|d| d.insert_temp(path_id, current_path.clone()));
                    if response.lost_focus() {
                        actions.output_actions.push(OutputAction::SetTarget {
                            idx,
                            target: OutputTarget::Recording { path: current_path, codec: codec.clone() },
                        });
                    }
                }
            });
        }
    }

    // Unified stream config (SRT, HLS, DASH, NDI)
    let is_stream = matches!(output.target,
        OutputTarget::SrtStream { .. } | OutputTarget::HlsStream { .. } |
        OutputTarget::DashStream { .. } | OutputTarget::RtmpStream { .. } |
        OutputTarget::NdiSend { .. }
    );
    if is_stream {
        render_stream_config(ui, idx, output, actions);
    }

    // Rotation selector
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Rotation:").small());
        egui::ComboBox::from_id_salt(format!("headless_rotation_{}", idx))
            .selected_text(egui::RichText::new(output.rotation.label()).small())
            .width(80.0)
            .show_ui(ui, |ui| {
                for rot in crate::renderer::context::OutputRotation::ALL {
                    if ui.selectable_label(output.rotation == rot, rot.label()).clicked() {
                        actions.output_actions.push(OutputAction::SetRotation {
                            idx, rotation: rot,
                        });
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
                    let already_assigned = output.surface_assignments.iter().any(|a| a.surface_uuid == surface.uuid);
                    if !already_assigned {
                        if ui.selectable_label(false, &surface.name).clicked() {
                            actions.output_actions.push(OutputAction::AssignSurface {
                                output_idx: idx, surface_uuid: surface.uuid.clone(),
                            });
                        }
                    }
                }
            });
    });

    for (ai, assignment) in output.surface_assignments.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&assignment.surface_name).small());
            if ui.small_button("↺").on_hover_text("Reset warp").clicked() {
                actions.output_actions.push(OutputAction::ResetWarp { output_idx: idx, assignment_idx: ai });
            }
            if ui.small_button("x").on_hover_text("Unassign").clicked() {
                actions.output_actions.push(OutputAction::UnassignSurface { output_idx: idx, assignment_idx: ai });
            }
        });
    }

    // Start/Stop + duration
    ui.horizontal(|ui| {
        if output.is_active {
            let dur = output.active_duration.as_secs_f32();
            ui.label(egui::RichText::new(format!("{:.1}s", dur)).monospace().color(egui::Color32::from_rgb(255, 80, 80)));
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


/// Unified stream output config with protocol dropdown (SRT, HLS, DASH, NDI).
fn render_stream_config(ui: &mut egui::Ui, idx: usize, output: &super::super::OutputUI, actions: &mut UIActions) {
    use crate::renderer::context::{StreamingCodec, SrtCodec};

    // Determine current protocol label
    let current_proto = match &output.target {
        OutputTarget::SrtStream { .. } => "SRT",
        OutputTarget::HlsStream { .. } => "HLS",
        OutputTarget::DashStream { .. } => "DASH",
        OutputTarget::RtmpStream { .. } => "RTMP",
        OutputTarget::NdiSend { .. } => "NDI",
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
                    for (label, default_target) in &[
                        ("SRT", OutputTarget::SrtStream {
                            url: format!("srt://0.0.0.0:9001"),
                            codec: SrtCodec::default(),
                        }),
                        ("HLS", OutputTarget::HlsStream {
                            name: "live".to_string(),
                            codec: StreamingCodec::default(),
                            low_latency: false,
                        }),
                        ("DASH", OutputTarget::DashStream {
                            name: "live".to_string(),
                            codec: StreamingCodec::default(),
                        }),
                        ("RTMP", OutputTarget::RtmpStream {
                            url: "rtmp://".to_string(),
                            codec: StreamingCodec::default(),
                        }),
                        ("NDI", OutputTarget::NdiSend {
                            sender_name: "Varda NDI".to_string(),
                        }),
                    ] {
                        if ui.selectable_label(current_proto == *label, *label).clicked() && current_proto != *label {
                            actions.output_actions.push(OutputAction::SetTarget {
                                idx,
                                target: default_target.clone(),
                            });
                        }
                    }
                });
        });
    } else {
        ui.label(egui::RichText::new(format!("Protocol: {}", current_proto)).small().weak());
    }

    // Protocol-specific config
    match &output.target {
        OutputTarget::SrtStream { ref url, ref codec } => {
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
                                        target: OutputTarget::SrtStream { url: url.clone(), codec: c.clone() },
                                    });
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("URL:").small());
                    let url_id = egui::Id::new(format!("srt_url_{}", idx));
                    let mut current_url: String = ui.data(|d| d.get_temp(url_id))
                        .unwrap_or_else(|| url.clone());
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut current_url)
                            .desired_width(180.0)
                            .font(egui::TextStyle::Small)
                    );
                    if response.lost_focus() || response.changed() {
                        ui.data_mut(|d| d.insert_temp(url_id, current_url.clone()));
                        if response.lost_focus() {
                            actions.output_actions.push(OutputAction::SetTarget {
                                idx,
                                target: OutputTarget::SrtStream { url: current_url, codec: codec.clone() },
                            });
                        }
                    }
                });
            }
        }
        OutputTarget::HlsStream { ref name, ref codec, low_latency } => {
            render_hls_dash_name_codec(ui, idx, "hls", name, codec, output.is_active, actions,
                |n, c| OutputTarget::HlsStream { name: n, codec: c, low_latency: *low_latency });
            if !output.is_active {
                ui.horizontal(|ui| {
                    let mut ll = *low_latency;
                    if ui.checkbox(&mut ll, egui::RichText::new("LL-HLS (Low Latency)").small()).changed() {
                        actions.output_actions.push(OutputAction::SetTarget {
                            idx,
                            target: OutputTarget::HlsStream { name: name.clone(), codec: codec.clone(), low_latency: ll },
                        });
                    }
                });
            }
            let player_url = format!("http://localhost:8080/streams/{}/player.html", name);
            let manifest_url = format!("http://localhost:8080/streams/{}/index.m3u8", name);
            render_copyable_url(ui, "▶", &player_url, 10.0, actions);
            render_copyable_url(ui, "🌐", &manifest_url, 9.0, actions);
        }
        OutputTarget::DashStream { ref name, ref codec } => {
            render_hls_dash_name_codec(ui, idx, "dash", name, codec, output.is_active, actions,
                |n, c| OutputTarget::DashStream { name: n, codec: c });
            let player_url = format!("http://localhost:8080/streams/{}/player.html", name);
            let manifest_url = format!("http://localhost:8080/streams/{}/manifest.mpd", name);
            render_copyable_url(ui, "▶", &player_url, 10.0, actions);
            render_copyable_url(ui, "🌐", &manifest_url, 9.0, actions);
        }
        OutputTarget::RtmpStream { ref url, ref codec } => {
            if !output.is_active {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Codec:").small());
                    egui::ComboBox::from_id_salt(format!("rtmp_codec_{}", idx))
                        .selected_text(egui::RichText::new(codec.to_string()).small())
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for c in &[StreamingCodec::H264, StreamingCodec::H265, StreamingCodec::AV1] {
                                if ui.selectable_label(*codec == *c, c.to_string()).clicked() {
                                    actions.output_actions.push(OutputAction::SetTarget {
                                        idx,
                                        target: OutputTarget::RtmpStream { url: url.clone(), codec: c.clone() },
                                    });
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("URL:").small());
                    let url_id = egui::Id::new(format!("rtmp_url_{}", idx));
                    let mut current_url: String = ui.data(|d| d.get_temp(url_id))
                        .unwrap_or_else(|| url.clone());
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut current_url)
                            .desired_width(180.0)
                            .font(egui::TextStyle::Small)
                    );
                    if response.lost_focus() || response.changed() {
                        ui.data_mut(|d| d.insert_temp(url_id, current_url.clone()));
                        if response.lost_focus() {
                            actions.output_actions.push(OutputAction::SetTarget {
                                idx,
                                target: OutputTarget::RtmpStream { url: current_url, codec: codec.clone() },
                            });
                        }
                    }
                });
            }
        }
        OutputTarget::NdiSend { ref sender_name } => {
            if !output.is_active {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Name:").small());
                    let name_id = egui::Id::new(format!("ndi_name_{}", idx));
                    let mut current_name: String = ui.data(|d| d.get_temp(name_id))
                        .unwrap_or_else(|| sender_name.clone());
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut current_name)
                            .desired_width(140.0)
                            .font(egui::TextStyle::Small)
                    );
                    if response.lost_focus() || response.changed() {
                        ui.data_mut(|d| d.insert_temp(name_id, current_name.clone()));
                        if response.lost_focus() {
                            actions.output_actions.push(OutputAction::SetTarget {
                                idx,
                                target: OutputTarget::NdiSend { sender_name: current_name },
                            });
                        }
                    }
                });
            }
        }
        _ => {}
    }
}


/// Render a clickable URL label that copies to clipboard on click.
fn render_copyable_url(ui: &mut egui::Ui, icon: &str, url: &str, font_size: f32, actions: &mut UIActions) {
    let text = format!("{} {}", icon, url);
    let response = ui.add(
        egui::Label::new(egui::RichText::new(&text).size(font_size).color(egui::Color32::from_rgb(130, 160, 200)))
            .sense(egui::Sense::click())
    );
    if response.clicked() {
        ui.ctx().copy_text(url.to_string());
        actions.info_notifications.push(format!("📋 Copied to clipboard: {}", url));
    }
    response.on_hover_text("Click to copy URL");
}

/// Shared codec + name config for HLS and DASH stream outputs.
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
                    for c in &[StreamingCodec::H264, StreamingCodec::H265, StreamingCodec::AV1] {
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
            let mut current_name: String = ui.data(|d| d.get_temp(name_id))
                .unwrap_or_else(|| name.to_string());
            let response = ui.add(
                egui::TextEdit::singleline(&mut current_name)
                    .desired_width(140.0)
                    .font(egui::TextStyle::Small)
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
                if ui.selectable_label(!is_auto, egui::RichText::new("Manual").small()).clicked() && is_auto {
                    actions.output_actions.push(OutputAction::SetEdgeBlendMode {
                        output_idx: idx, mode: EdgeBlendMode::Manual,
                    });
                }
                if ui.selectable_label(is_auto, egui::RichText::new("Auto").small()).clicked() && !is_auto {
                    actions.output_actions.push(OutputAction::SetEdgeBlendMode {
                        output_idx: idx, mode: EdgeBlendMode::Auto,
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
                            ui.label(egui::RichText::new(
                                format!("{} zone(s)", sa.overlap_zones.zones.len())
                            ).small().weak());
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
                                ui.label(egui::RichText::new(format!(
                                    "Zone {}: UV [{:.2},{:.2}]→[{:.2},{:.2}] {} γ:{:.1}",
                                    zi + 1,
                                    zone.uv_rect[0], zone.uv_rect[1],
                                    zone.uv_rect[2], zone.uv_rect[3],
                                    dir, zone.gamma,
                                )).small().weak());
                            });
                        }
                    }
                }
                if !any_zones {
                    ui.label(egui::RichText::new("No overlapping surfaces detected").small().weak());
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
                        if ui.checkbox(&mut edge.enabled, egui::RichText::new(label).small()).changed() {
                            changed = true;
                        }
                        if edge.enabled {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("W:").small());
                            if ui.add(egui::Slider::new(&mut edge.width, 0.01..=0.5).step_by(0.01).max_decimals(2))
                                .on_hover_text("Blend zone width (fraction of output)")
                                .changed()
                            {
                                changed = true;
                            }
                            ui.label(egui::RichText::new("γ:").small());
                            if ui.add(egui::Slider::new(&mut edge.gamma, 0.5..=4.0).step_by(0.1).max_decimals(1))
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
                    output_idx: idx, config: cfg,
                });
            }
        });
}

/// Render the warp calibration mini-canvas for an output.
/// Shows surface assignments as quads with draggable corner handles.
pub(super) fn render_warp_calibration(
    ui: &mut egui::Ui,
    output_idx: usize,
    output: &super::super::OutputUI,
    actions: &mut UIActions,
) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("⊞ Drag corners to warp").small().color(egui::Color32::YELLOW));

    let canvas_width = ui.available_width() - 4.0;
    let canvas_height = canvas_width * 0.5625; // 16:9
    let (canvas_rect, canvas_response) = ui.allocate_exact_size(
        egui::vec2(canvas_width, canvas_height),
        egui::Sense::click_and_drag(),
    );

    let painter = ui.painter_at(canvas_rect);

    // Dark background
    painter.rect_filled(canvas_rect, 2.0, egui::Color32::from_rgb(15, 15, 25));

    // Convert normalized [0..1] to canvas pixel position
    let to_screen = |nx: f32, ny: f32| -> egui::Pos2 {
        egui::pos2(
            canvas_rect.left() + nx * canvas_width,
            canvas_rect.top() + ny * canvas_height,
        )
    };
    let from_screen = |pos: egui::Pos2| -> [f32; 2] {
        [
            ((pos.x - canvas_rect.left()) / canvas_width).clamp(0.0, 1.0),
            ((pos.y - canvas_rect.top()) / canvas_height).clamp(0.0, 1.0),
        ]
    };

    let corner_colors = [
        egui::Color32::from_rgb(255, 100, 100), // TL - red
        egui::Color32::from_rgb(100, 255, 100), // TR - green
        egui::Color32::from_rgb(100, 100, 255), // BR - blue
        egui::Color32::from_rgb(255, 255, 100), // BL - yellow
    ];
    let corner_labels = ["TL", "TR", "BR", "BL"];

    // State for dragging — store as Option<(usize, usize)> consistently
    let state_id = ui.id().with("warp_cal").with(output_idx);
    let mut dragging: Option<(usize, usize)> = ui.memory(|mem| {
        mem.data.get_temp::<Option<(usize, usize)>>(state_id).flatten()
    });

    // Draw each assigned surface's warp quad and handles (corner-pin only)
    for (ai, assignment) in output.surface_assignments.iter().enumerate() {
        let corners = match &assignment.warp_mode {
            crate::renderer::warp::WarpMode::CornerPin { corners } => corners,
            crate::renderer::warp::WarpMode::Mesh(_) => continue, // Mesh warp not editable via corner handles
        };

        // Draw the warp quad outline
        let screen_corners: Vec<egui::Pos2> = corners.iter()
            .map(|c| to_screen(c[0], c[1]))
            .collect();
        for i in 0..4 {
            let j = (i + 1) % 4;
            painter.line_segment(
                [screen_corners[i], screen_corners[j]],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(200, 200, 200)),
            );
        }

        // Draw surface name at center
        let cx = corners.iter().map(|c| c[0]).sum::<f32>() / 4.0;
        let cy = corners.iter().map(|c| c[1]).sum::<f32>() / 4.0;
        painter.text(
            to_screen(cx, cy),
            egui::Align2::CENTER_CENTER,
            &assignment.surface_name,
            egui::FontId::proportional(11.0),
            egui::Color32::WHITE,
        );

        // Draw corner handles
        let handle_radius = 8.0;
        for (ci, corner) in corners.iter().enumerate() {
            let pos = to_screen(corner[0], corner[1]);
            let is_dragging = dragging == Some((ai, ci));
            let r = if is_dragging { handle_radius + 2.0 } else { handle_radius };
            painter.circle_filled(pos, r, corner_colors[ci]);
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                corner_labels[ci],
                egui::FontId::proportional(9.0),
                egui::Color32::BLACK,
            );
        }
    }

    // Handle dragging (corner-pin only)
    if canvas_response.drag_started() {
        if let Some(pos) = canvas_response.interact_pointer_pos() {
            // Find nearest corner
            let mut best: Option<(usize, usize, f32)> = None;
            for (ai, assignment) in output.surface_assignments.iter().enumerate() {
                let corners = match &assignment.warp_mode {
                    crate::renderer::warp::WarpMode::CornerPin { corners } => corners,
                    crate::renderer::warp::WarpMode::Mesh(_) => continue,
                };
                for (ci, corner) in corners.iter().enumerate() {
                    let screen_pos = to_screen(corner[0], corner[1]);
                    let dist = pos.distance(screen_pos);
                    if dist < 20.0 {
                        if best.map_or(true, |(_, _, d)| dist < d) {
                            best = Some((ai, ci, dist));
                        }
                    }
                }
            }
            if let Some((ai, ci, _)) = best {
                dragging = Some((ai, ci));
            }
        }
    }

    if let Some((ai, ci)) = dragging {
        if canvas_response.dragged() {
            if let Some(pos) = canvas_response.interact_pointer_pos() {
                let new_pos = from_screen(pos);
                actions.output_actions.push(OutputAction::SetWarpCorner {
                    output_idx,
                    assignment_idx: ai,
                    corner_idx: ci,
                    position: new_pos,
                });
            }
        }
    }

    if canvas_response.drag_stopped() {
        dragging = None;
    }

    ui.memory_mut(|mem| mem.data.insert_temp(state_id, dragging));
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
            calibration_mode: false,
            edge_blend_mode: crate::renderer::edge_blend::EdgeBlendMode::default(),
            edge_blend: crate::renderer::edge_blend::EdgeBlendConfig::default(),
            rotation: crate::renderer::context::OutputRotation::default(),
        });
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_output_section(ui, &data, &mut actions);
        });
    }
}