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
                    render_presentation_controls(ui, &output.uuid, output, data, actions);
                });
            ui.add_space(4.0);
        }
    }
}

fn render_presentation_controls(
    ui: &mut egui::Ui,
    output_uuid: &str,
    output: &super::super::OutputUI,
    data: &UIData,
    actions: &mut UIActions,
) {
    use crate::engine::value::render::{
        HDR_PEAK_NITS_PRESETS, PresentationMode, PresentationRequest, PresentationTransfer,
    };

    let request = output.presentation_request;
    let requested_mode = request.mode();
    // Every mode is listed; the ones this output cannot deliver are disabled and
    // name their obstacle on hover, which is usually a codec set elsewhere on this
    // same card. Hiding them was honest but silent, and left a user who knows
    // their protocol carries HDR with no idea what to change.
    //
    // When a stored request is not deliverable, because the target or codec
    // changed under it, the delivered mode is shown as the selection and the
    // request is left alone, so it comes back the moment the output can carry it
    // again. See /spec/presentation-mode-offering.md.
    let availability = &output.mode_availability;
    let deliverable = |mode: PresentationMode| {
        availability
            .iter()
            .any(|entry| entry.mode == mode && entry.is_available())
    };
    let current_mode = if deliverable(requested_mode) {
        requested_mode
    } else {
        output.resolved_presentation.mode()
    };

    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Output:").small());
        egui::ComboBox::from_id_salt(format!("presentation_mode_{output_uuid}"))
            .selected_text(egui::RichText::new(current_mode.label()).small())
            .width(100.0)
            .show_ui(ui, |ui| {
                for entry in availability {
                    let mode = entry.mode;
                    if let Some(reason) = &entry.blocked {
                        // Disabled rather than absent: it cannot be selected into a
                        // wrong state, and it says what to change.
                        ui.add_enabled(false, egui::Button::selectable(false, mode.label()))
                            .on_disabled_hover_text(format!(
                                "{}\n\nUnavailable: {reason}",
                                mode.description()
                            ));
                    } else if ui
                        .selectable_label(current_mode == mode, mode.label())
                        .on_hover_text(mode.description())
                        .clicked()
                    {
                        actions.commands.push(EngineCommand::SetOutputPresentation {
                            output_uuid: output_uuid.to_string(),
                            request: request.with_mode(mode),
                        });
                    }
                }
            });

        let mut dither = request.dither;
        if ui
            .checkbox(&mut dither, egui::RichText::new("Dither").small())
            .changed()
        {
            actions.commands.push(EngineCommand::SetOutputPresentation {
                output_uuid: output_uuid.to_string(),
                request: PresentationRequest { dither, ..request },
            });
        }
    });

    // Peak luminance only exists for an HDR contract, so it only appears for one.
    if current_mode.is_hdr() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Peak:").small());
            egui::ComboBox::from_id_salt(format!("presentation_peak_{output_uuid}"))
                .selected_text(egui::RichText::new(format!("{} cd/m²", request.peak_nits)).small())
                .width(100.0)
                .show_ui(ui, |ui| {
                    for peak in HDR_PEAK_NITS_PRESETS {
                        if ui
                            .selectable_label(request.peak_nits == peak, format!("{peak} cd/m²"))
                            .clicked()
                        {
                            actions.commands.push(EngineCommand::SetOutputPresentation {
                                output_uuid: output_uuid.to_string(),
                                request: PresentationRequest {
                                    peak_nits: peak,
                                    ..request
                                },
                            });
                        }
                    }
                })
                .response
                .on_hover_text(
                    "Mastering target. Linear 1.0 stays at 203 cd/m² reference white; \
                     this sets where the headroom above it ends.",
                );
        });
    }

    // The tonemap is an output transform, so it belongs on the output card as
    // well as in the show-wide Tonemap panel. `None` inherits.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Tonemap:").small());
        let effective = output.tonemap_override.unwrap_or(data.tonemap_mode);
        let selected = output.tonemap_override.map_or_else(
            || format!("Show ({})", data.tonemap_mode.label()),
            |m| m.label().to_string(),
        );
        egui::ComboBox::from_id_salt(format!("output_tonemap_{output_uuid}"))
            .selected_text(egui::RichText::new(selected).small())
            .width(150.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(output.tonemap_override.is_none(), "Show default")
                    .clicked()
                {
                    actions.commands.push(EngineCommand::SetOutputTonemap {
                        output_uuid: output_uuid.to_string(),
                        tonemap: None,
                    });
                }
                for mode in crate::renderer::tonemap::TonemapMode::ALL {
                    let label = if request.transfer.is_hdr() && !mode.has_hdr_form() {
                        format!("{} (no HDR form)", mode.label())
                    } else {
                        mode.label().to_string()
                    };
                    if ui
                        .selectable_label(output.tonemap_override == Some(mode), label)
                        .clicked()
                    {
                        actions.commands.push(EngineCommand::SetOutputTonemap {
                            output_uuid: output_uuid.to_string(),
                            tonemap: Some(mode),
                        });
                    }
                }
            });
        let _ = effective;
    });

    let resolved = &output.resolved_presentation;
    let status = if resolved.transfer == PresentationTransfer::EdrLinear {
        format!(
            "Monitoring EDR · {} · linear · to {} cd/m²",
            resolved.pixel_format,
            resolved.peak_nits.unwrap_or_default()
        )
    } else if resolved.transfer.is_hdr() {
        format!(
            "Delivering {} · {} · {} · {}",
            resolved.transfer.label(),
            resolved.pixel_format,
            if resolved.transfer == PresentationTransfer::Hlg {
                "HLG/BT.2020"
            } else {
                "PQ/BT.2020"
            },
            resolved
                .peak_nits
                .map_or_else(|| "display-relative".to_string(), |p| format!("{p} cd/m²"))
        )
    } else {
        format!(
            "Delivering {} · {}",
            resolved.resolved.label(),
            resolved.pixel_format
        )
    };
    let color = if resolved.fallback_reason.is_some() {
        egui::Color32::from_rgb(255, 190, 80)
    } else {
        egui::Color32::from_rgb(120, 200, 255)
    };
    ui.label(egui::RichText::new(status).small().color(color));
    if let Some(reason) = &resolved.fallback_reason {
        // The requested contract names the fallback, so an HDR request that
        // degrades does not report itself as a bit-depth problem.
        let label = if request.transfer.is_hdr() {
            "HDR10 fallback"
        } else {
            "10-bit fallback"
        };
        ui.label(
            egui::RichText::new(format!("{label}: {reason}"))
                .small()
                .color(color),
        );
    }

    // EDR headroom is runtime health, not part of the resolved contract: it
    // climbs from 1.0 once EDR engages and moves with display brightness. It
    // answers whether the monitor can currently show the top of the range being
    // graded. See /spec/hdr-edr-display.md.
    if resolved.transfer == PresentationTransfer::EdrLinear
        && let Some(headroom) = crate::renderer::edr::primary_headroom()
    {
        let needed = resolved.peak_nits.map_or(1.0, |peak| {
            f32::from(peak) / crate::engine::value::render::HDR_REFERENCE_WHITE_NITS
        });
        let covered = headroom.covers(needed);
        ui.label(
            egui::RichText::new(format!(
                "Display headroom {:.1}x · monitoring to {needed:.1}x{}",
                headroom.current,
                if covered {
                    ""
                } else {
                    " · top of range is clipped, lower display brightness"
                }
            ))
            .small()
            .color(if covered {
                egui::Color32::from_rgb(120, 200, 255)
            } else {
                egui::Color32::from_rgb(255, 190, 80)
            }),
        );
    }

    // CTA-861.3 expects MaxCLL and MaxFALL to be measured across the programme.
    // Saying which one is in force keeps a non-standard declaration from passing
    // as a measured one (/spec/hdr-recording-output.md).
    if let Some(metadata) = resolved.hdr_metadata {
        let color = match metadata {
            crate::engine::value::render::HdrMetadataSource::MeasuredFromContent => {
                egui::Color32::from_rgb(120, 200, 255)
            }
            crate::engine::value::render::HdrMetadataSource::DeclaredFromPeak => {
                egui::Color32::from_rgb(255, 190, 80)
            }
        };
        ui.label(
            egui::RichText::new(format!("MaxCLL/MaxFALL {}", metadata.label()))
                .small()
                .color(color),
        );
    }

    // A curve fitted against an SDR target is substituted rather than rescaled on
    // an HDR output (/spec/hdr-per-output-encode.md § Output transform range).
    // The substitution is reported so a look choice never silently changes the
    // picture a delivery carries.
    let effective_tonemap = output.tonemap_override.unwrap_or(data.tonemap_mode);
    if resolved.transfer.is_hdr() && !effective_tonemap.has_hdr_form() {
        ui.label(
            egui::RichText::new(format!(
                "{} has no HDR form; this output uses Bypass",
                effective_tonemap.label()
            ))
            .small()
            .color(egui::Color32::from_rgb(255, 190, 80)),
        );
    }

    // The calibration LUT is display-referred and does not apply to an HDR
    // output (/spec/hdr-color-management.md Decision 4). Say so here rather than
    // letting the user find it by opening the file.
    if resolved.transfer.is_hdr()
        && let Some(lut) = &data.active_lut_filename
    {
        ui.label(
            egui::RichText::new(format!(
                "LUT '{lut}' is SDR-calibrated and is not applied to this HDR10 output"
            ))
            .small()
            .color(egui::Color32::from_rgb(255, 190, 80)),
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
                    // Syphon and Spout are each pushed only on their own platform,
                    // so on Linux neither push compiles and the binding is
                    // unused-mut.
                    #[cfg_attr(
                        not(any(target_os = "macos", target_os = "windows")),
                        allow(unused_mut)
                    )]
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
                                short_segments: false,
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
                    // Offered only on Windows, for the same reason Syphon is
                    // offered only on macOS: a target that can never resolve is a
                    // dead control. See /spec/spout-output.md.
                    #[cfg(target_os = "windows")]
                    protocols.push((
                        "Spout",
                        OutputTarget::SpoutSender {
                            sender_name: "Varda".to_string(),
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
            short_segments,
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
                    short_segments: *short_segments,
                    audio_device: audio_device.clone(),
                },
            );
            if !output.is_active {
                ui.horizontal(|ui| {
                    let mut ll = *short_segments;
                    if ui
                        .checkbox(
                            &mut ll,
                            egui::RichText::new("Short segments (lower latency)").small(),
                        )
                        .on_hover_text(
                            "One-second segments instead of two. Not RFC low-latency HLS: \
                             FFmpeg writes no partial segments, so expect a couple of \
                             seconds of latency rather than sub-second.",
                        )
                        .changed()
                    {
                        actions.commands.push(EngineCommand::SetOutputTarget {
                            output_uuid: output_uuid.to_string(),
                            target: OutputTarget::HlsStream {
                                name: name.clone(),
                                codec: codec.clone(),
                                short_segments: ll,
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
        OutputTarget::SpoutSender { sender_name } if !output.is_active => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Name:").small());
                let name_id = egui::Id::new(format!("spout_name_{output_uuid}"));
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
                            target: OutputTarget::SpoutSender {
                                sender_name: current_name,
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
            // Existing cases exercise the fallback line rather than the picker, so
            // they keep an "everything deliverable" shape. The picker's own tests
            // below override this.
            mode_availability: crate::engine::value::render::PresentationMode::ALL
                .into_iter()
                .map(|mode| crate::engine::value::render::ModeAvailability {
                    mode,
                    blocked: None,
                })
                .collect(),
            tonemap_override: None,
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
            ..ResolvedPresentation::default()
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

    // ── the offered set (/spec/presentation-mode-offering.md) ────────

    /// An eight-bit-only output, with every other mode blocked and explained.
    fn only_eight_bit_is_deliverable() -> Vec<crate::engine::value::render::ModeAvailability> {
        use crate::engine::value::render::{ModeAvailability, PresentationMode};
        PresentationMode::ALL
            .into_iter()
            .map(|mode| ModeAvailability {
                mode,
                blocked: (mode != PresentationMode::Sdr8)
                    .then(|| "this codec is eight-bit".to_string()),
            })
            .collect()
    }

    /// The case that motivated this change, and its correction after the second
    /// field report. An eight-bit-only output must not let you *select* HDR10,
    /// but it must still show it and say why, because the obstacle is usually a
    /// codec set elsewhere on the same card and hiding it teaches nothing.
    #[test]
    fn a_blocked_mode_is_shown_with_its_reason_but_cannot_be_selected() {
        use crate::engine::value::render::PresentationMode;
        let mut output =
            sample_output(crate::engine::value::render::ResolvedPresentation::default());
        output.mode_availability = only_eight_bit_is_deliverable();

        let mut data = UIData::test_fixture();
        data.outputs.push(output);
        let mut actions = UIActions::new();
        {
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(420.0, 640.0))
                .build_ui(|ui| {
                    render_output_section(ui, &data, &mut actions);
                });
            harness.run();
            // A ComboBox exposes its selected text as AccessKit `value`, not
            // `label`, and its list renders only while open.
            harness.get_by_value(PresentationMode::Sdr8.label()).click();
            harness.run();

            // Visible, so the user can see what they are missing...
            let blocked = harness
                .query_by_label(PresentationMode::Hdr10.label())
                .expect("a blocked mode must still be listed, not hidden");
            // ...and inert, so it cannot be selected into a state that would only
            // produce a warning.
            blocked.click();
            harness.run();
        }
        assert!(
            !actions.commands.iter().any(|command| matches!(
                command,
                EngineCommand::SetOutputPresentation { request, .. }
                    if request.mode() == PresentationMode::Hdr10
            )),
            "clicking a blocked mode must not change the output's request"
        );
    }

    /// A request the output can no longer carry is kept rather than clamped, and
    /// the picker shows what is actually being delivered instead of a mode the
    /// output is not producing.
    #[test]
    fn an_undeliverable_request_is_retained_and_the_delivered_mode_is_shown() {
        use crate::engine::value::render::{
            PresentationDepth, PresentationMode, PresentationRequest, PresentationTransfer,
            ResolvedPresentation,
        };
        let mut output = sample_output(ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr8,
            requested_transfer: PresentationTransfer::Hdr10Pq,
            transfer: PresentationTransfer::Sdr,
            fallback_reason: Some("this codec is eight-bit".into()),
            ..ResolvedPresentation::default()
        });
        // The operator asked for HDR10 before the codec changed underneath them.
        output.presentation_request =
            PresentationRequest::default().with_mode(PresentationMode::Hdr10);
        output.mode_availability = only_eight_bit_is_deliverable();

        assert_eq!(
            output.presentation_request.mode(),
            PresentationMode::Hdr10,
            "the stored request must survive so it returns when the output can carry it again"
        );

        let mut data = UIData::test_fixture();
        data.outputs.push(output);
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 640.0))
            .build_ui(|ui| {
                render_output_section(ui, &data, &mut actions);
            });
        harness.run();
        // Selected text is the delivered mode, not the undeliverable request.
        harness.get_by_value(PresentationMode::Sdr8.label());
        // And the card still explains the difference, which is what the warning is for.
        harness.get_by_label("HDR10 fallback: this codec is eight-bit");
    }

    #[test]
    fn the_status_line_states_the_hdr_contract_in_full() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationMode,
            PresentationPixelFormat, PresentationTransfer, ResolvedPresentation,
        };
        // An operator reading the card should not have to infer the transfer,
        // the container, the peak the file was mastered against, or whether the
        // mastering metadata was measured.
        let mut data = UIData::test_fixture();
        let mut output = sample_output(ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            requested_transfer: PresentationTransfer::Hdr10Pq,
            transfer: PresentationTransfer::Hdr10Pq,
            peak_nits: Some(1000),
            hdr_metadata: Some(crate::engine::value::render::HdrMetadataSource::DeclaredFromPeak),
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::Pq2020Limited,
            alpha_mode: AlphaMode::Opaque,
            dither: true,
            fallback_reason: None,
        });
        output.presentation_request = output
            .presentation_request
            .with_mode(PresentationMode::Hdr10);
        data.outputs.push(output);

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 900.0))
            .build_ui(|ui| {
                render_output_section(ui, &data, &mut actions);
            });
        harness.run();
        // A declaration must never pass as a measurement.
        assert!(
            harness
                .query_by_label_contains("declared from peak")
                .is_some(),
            "the card must say where the mastering metadata came from"
        );
        for expected in ["HDR10", "PQ/BT.2020", "1000 cd/m²"] {
            assert!(
                harness.query_by_label_contains(expected).is_some(),
                "the status line never mentions {expected}"
            );
        }
    }

    #[test]
    fn the_peak_control_only_exists_for_an_hdr_contract() {
        use crate::engine::value::render::{PresentationMode, ResolvedPresentation};
        let mut data = UIData::test_fixture();
        let mut output = sample_output(ResolvedPresentation::default());
        output.presentation_request = output
            .presentation_request
            .with_mode(PresentationMode::Sdr10);
        data.outputs.push(output);
        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 640.0))
            .build_ui(|ui| {
                render_output_section(ui, &data, &mut actions);
            });
        harness.run();
        assert!(
            harness.query_by_label("Peak:").is_none(),
            "an SDR output must not offer a peak luminance it cannot use"
        );
    }

    #[test]
    fn an_hdr_output_warns_that_the_sdr_lut_is_not_applied() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationMode,
            PresentationPixelFormat, PresentationTransfer, ResolvedPresentation,
        };
        let mut data = UIData::test_fixture();
        data.active_lut_filename = Some("club_grade.cube".to_string());
        let mut output = sample_output(ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            requested_transfer: PresentationTransfer::Hdr10Pq,
            transfer: PresentationTransfer::Hdr10Pq,
            peak_nits: Some(1000),
            hdr_metadata: None,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::Pq2020Limited,
            alpha_mode: AlphaMode::Opaque,
            dither: true,
            fallback_reason: None,
        });
        output.presentation_request = output
            .presentation_request
            .with_mode(PresentationMode::Hdr10);
        data.outputs.push(output);

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 900.0))
            .build_ui(|ui| {
                render_output_section(ui, &data, &mut actions);
            });
        harness.run();
        // The operator must not discover this by opening the file.
        assert!(
            harness.query_by_label_contains("club_grade.cube").is_some(),
            "an HDR output with a LUT loaded must say the LUT is not applied"
        );
    }

    #[test]
    fn an_hdr_fallback_is_not_reported_as_a_bit_depth_problem() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationMode,
            PresentationPixelFormat, PresentationTransfer, ResolvedPresentation,
        };
        let mut data = UIData::test_fixture();
        let mut output = sample_output(ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            requested_transfer: PresentationTransfer::Hdr10Pq,
            transfer: PresentationTransfer::Sdr,
            peak_nits: None,
            hdr_metadata: None,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::SrgbFull,
            alpha_mode: AlphaMode::Opaque,
            dither: true,
            fallback_reason: Some("H.264 cannot carry HDR10".to_string()),
        });
        output.presentation_request = output
            .presentation_request
            .with_mode(PresentationMode::Hdr10);
        data.outputs.push(output);

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 900.0))
            .build_ui(|ui| {
                render_output_section(ui, &data, &mut actions);
            });
        harness.run();
        assert!(
            harness.query_by_label_contains("HDR10 fallback").is_some(),
            "an HDR request that degrades must name HDR, not bit depth"
        );
    }

    #[test]
    fn an_hdr_output_says_when_the_chosen_curve_has_no_hdr_form() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationMode,
            PresentationPixelFormat, PresentationTransfer, ResolvedPresentation,
        };
        let mut data = UIData::test_fixture();
        data.tonemap_mode = crate::renderer::tonemap::TonemapMode::AgX;
        let mut output = sample_output(ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            requested_transfer: PresentationTransfer::Hdr10Pq,
            transfer: PresentationTransfer::Hdr10Pq,
            peak_nits: Some(1000),
            hdr_metadata: None,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::Pq2020Limited,
            alpha_mode: AlphaMode::Opaque,
            dither: true,
            fallback_reason: None,
        });
        output.presentation_request = output
            .presentation_request
            .with_mode(PresentationMode::Hdr10);
        data.outputs.push(output);

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 900.0))
            .build_ui(|ui| {
                render_output_section(ui, &data, &mut actions);
            });
        harness.run();
        assert!(
            harness.query_by_label_contains("no HDR form").is_some(),
            "a substituted curve must be reported, never silent"
        );
    }

    #[test]
    fn an_sdr_output_never_mentions_curve_substitution() {
        use crate::engine::value::render::ResolvedPresentation;
        let mut data = UIData::test_fixture();
        data.tonemap_mode = crate::renderer::tonemap::TonemapMode::AgX;
        data.outputs
            .push(sample_output(ResolvedPresentation::default()));

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 900.0))
            .build_ui(|ui| {
                render_output_section(ui, &data, &mut actions);
            });
        harness.run();
        assert!(harness.query_by_label_contains("no HDR form").is_none());
    }

    // Note: the per-output tonemap ComboBox's selected text is not exposed to the
    // accessibility tree, so it cannot be asserted on here. Two tests that tried
    // were removed rather than weakened into assertions that observe nothing. The
    // override's effect on this panel is covered below through the substitution
    // warning, which is a real label, and end to end by the engine, API, and
    // persistence tests.

    #[test]
    fn the_substitution_warning_follows_the_override_not_the_show_curve() {
        use crate::engine::value::render::{
            AlphaMode, PresentationColorProfile, PresentationDepth, PresentationMode,
            PresentationPixelFormat, PresentationTransfer, ResolvedPresentation,
        };
        use crate::renderer::tonemap::TonemapMode;
        // Show-wide curve has an HDR form; the override does not. The warning
        // must describe what this output actually runs.
        let mut data = UIData::test_fixture();
        data.tonemap_mode = TonemapMode::Bypass;
        let mut output = sample_output(ResolvedPresentation {
            requested: PresentationDepth::Sdr10,
            resolved: PresentationDepth::Sdr10,
            requested_transfer: PresentationTransfer::Hdr10Pq,
            transfer: PresentationTransfer::Hdr10Pq,
            peak_nits: Some(1000),
            hdr_metadata: None,
            pixel_format: PresentationPixelFormat::Rgb10A2,
            color_profile: PresentationColorProfile::Pq2020Limited,
            alpha_mode: AlphaMode::Opaque,
            dither: true,
            fallback_reason: None,
        });
        output.presentation_request = output
            .presentation_request
            .with_mode(PresentationMode::Hdr10);
        output.tonemap_override = Some(TonemapMode::Lottes);
        data.outputs.push(output);

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 900.0))
            .build_ui(|ui| {
                render_output_section(ui, &data, &mut actions);
            });
        harness.run();
        assert!(
            harness
                .query_by_label_contains("Lottes (AMD) has no HDR form")
                .is_some(),
            "the warning must name the override, not the show-wide curve"
        );
    }
}
