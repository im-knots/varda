//! Library panel.

use super::super::{LibraryDrag, UIActions, UIData};

pub(super) fn render_library_panel(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    ui.horizontal(|ui| {
        ui.heading("📚 Library");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("◀")
                .on_hover_text("Close library (L)")
                .clicked()
            {
                actions.toggle_library_panel = true;
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .scroll_source(egui::scroll_area::ScrollSource {
            scroll_bar: true,
            drag: false,
            mouse_wheel: true,
        })
        .show(ui, |ui| {
            // === GENERATORS ===
            let gen_header =
                egui::RichText::new(format!("🎨 Generators ({})", data.generators.len())).strong();
            egui::CollapsingHeader::new(gen_header)
                .id_salt("lib_generators")
                .default_open(false)
                .show(ui, |ui| {
                    for (name, gen_idx) in &data.generators {
                        let item_id = egui::Id::new(("lib_gen", *gen_idx));
                        let resp = ui
                            .dnd_drag_source(item_id, LibraryDrag::Generator(*gen_idx), |ui| {
                                ui.label(egui::RichText::new(format!("  ◆ {}", name)).size(12.0));
                            })
                            .response;
                        // Store generator index in temp memory so the deferred drop handler can use it
                        if ui.ctx().is_being_dragged(item_id) {
                            ui.ctx().memory_mut(|mem| {
                                mem.data
                                    .insert_temp(egui::Id::new("__lib_dnd_gen_idx"), *gen_idx);
                            });
                        }
                        // Fallback: double-click adds to first channel
                        if resp.double_clicked() {
                            actions.shader_to_add = Some((0, *gen_idx));
                        }
                        resp.on_hover_text(
                            "Drag to a channel to create a deck, or double-click to add to Ch 0",
                        );
                    }
                });

            ui.add_space(4.0);

            // === EFFECTS ===
            let fx_header =
                egui::RichText::new(format!("🔮 Effects ({})", data.filters.len())).strong();
            egui::CollapsingHeader::new(fx_header)
                .id_salt("lib_effects")
                .default_open(false)
                .show(ui, |ui| {
                    for (name, filter_idx) in &data.filters {
                        let item_id = egui::Id::new(("lib_fx", *filter_idx));
                        ui.dnd_drag_source(item_id, LibraryDrag::Effect(*filter_idx), |ui| {
                            ui.label(egui::RichText::new(format!("  ◇ {}", name)).size(12.0));
                        });
                        // Store effect filter index in temp memory for deferred drop handler
                        if ui.ctx().is_being_dragged(item_id) {
                            ui.ctx().memory_mut(|mem| {
                                mem.data
                                    .insert_temp(egui::Id::new("__lib_dnd_fx_idx"), *filter_idx);
                            });
                        }
                    }
                });

            ui.add_space(4.0);

            // === IMAGES ===
            let img_header = egui::RichText::new("🖼 Images").strong();
            egui::CollapsingHeader::new(img_header)
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Load image files as deck sources")
                            .small()
                            .weak(),
                    );
                    for ch in &data.channels {
                        if ui.button(format!("📁 Load to {}", ch.name)).clicked() {
                            actions.open_image_dialog_for_channel = Some(ch.ch_idx);
                        }
                    }
                });

            ui.add_space(4.0);

            // === VIDEO ===
            let vid_header = egui::RichText::new("🎬 Video").strong();
            egui::CollapsingHeader::new(vid_header)
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Load video files as deck sources")
                            .small()
                            .weak(),
                    );
                    for ch in &data.channels {
                        if ui.button(format!("📁 Load to {}", ch.name)).clicked() {
                            actions.open_video_dialog_for_channel = Some(ch.ch_idx);
                        }
                    }
                });

            ui.add_space(4.0);

            // === CAMERAS ===
            let cam_header =
                egui::RichText::new(format!("📹 Cameras ({})", data.cameras.len())).strong();
            egui::CollapsingHeader::new(cam_header)
                .id_salt("lib_cameras")
                .default_open(false)
                .show(ui, |ui| {
                    if ui.small_button("🔄 Rescan").clicked() {
                        actions.camera_rescan = true;
                    }
                    if data.cameras.is_empty() {
                        ui.label(egui::RichText::new("No cameras detected").small().weak());
                    }
                    for (name, cam_id) in &data.cameras {
                        let item_id = egui::Id::new(("lib_cam", *cam_id));
                        ui.dnd_drag_source(item_id, LibraryDrag::Camera(*cam_id), |ui| {
                            ui.label(egui::RichText::new(format!("  📹 {}", name)).size(12.0));
                        });
                        if ui.ctx().is_being_dragged(item_id) {
                            ui.ctx().memory_mut(|mem| {
                                mem.data
                                    .insert_temp(egui::Id::new("__lib_dnd_cam_id"), *cam_id);
                            });
                        }
                    }
                });

            ui.add_space(4.0);

            // === STREAM SOURCES (grouped) ===
            {
                let total_streams = data.ndi_sources.len()
                    + data.srt_library_configs.len()
                    + data.hls_library_configs.len()
                    + data.dash_library_configs.len()
                    + data.rtmp_library_configs.len();
                let stream_header =
                    egui::RichText::new(format!("📡 Stream Sources ({})", total_streams)).strong();
                egui::CollapsingHeader::new(stream_header)
                    .id_salt("lib_streams")
                    .default_open(false)
                    .show(ui, |ui| {
                        // — NDI —
                        let ndi_header =
                            egui::RichText::new(format!("NDI ({})", data.ndi_sources.len()))
                                .strong();
                        egui::CollapsingHeader::new(ndi_header)
                            .id_salt("lib_ndi")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if ui.small_button("🔄 Rescan").clicked() {
                                        actions.ndi_rescan = true;
                                    }
                                    if !data.ndi_available {
                                        ui.label(
                                            egui::RichText::new("(SDK not found)").small().weak(),
                                        );
                                    }
                                });
                                if data.ndi_sources.is_empty() {
                                    ui.label(
                                        egui::RichText::new("No NDI sources found").small().weak(),
                                    );
                                }
                                for (i, name) in data.ndi_sources.iter().enumerate() {
                                    let item_id = egui::Id::new(("lib_ndi", i));
                                    ui.dnd_drag_source(
                                        item_id,
                                        LibraryDrag::Ndi(name.clone()),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(format!("  📡 {}", name))
                                                    .size(12.0),
                                            );
                                        },
                                    );
                                    if ui.ctx().is_being_dragged(item_id) {
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(
                                                egui::Id::new("__lib_dnd_ndi_name"),
                                                name.clone(),
                                            );
                                        });
                                    }
                                }
                            });

                        ui.add_space(4.0);

                        // — SRT —
                        let srt_header = egui::RichText::new(format!(
                            "SRT ({})",
                            data.srt_library_configs.len()
                        ))
                        .strong();
                        egui::CollapsingHeader::new(srt_header)
                            .id_salt("lib_srt")
                            .default_open(false)
                            .show(ui, |ui| {
                                // "+ Add SRT" button with inline config
                                let adding_id = ui.id().with("srt_adding");
                                let url_id = ui.id().with("srt_url_input");
                                let mode_id = ui.id().with("srt_mode_input");
                                let is_adding: bool =
                                    ui.data(|d| d.get_temp(adding_id)).unwrap_or(false);

                                if is_adding {
                                    let mut url: String = ui
                                        .data(|d| d.get_temp(url_id))
                                        .unwrap_or_else(|| "srt://127.0.0.1:9001".to_string());
                                    let mut mode_idx: usize =
                                        ui.data(|d| d.get_temp(mode_id)).unwrap_or(1);

                                    ui.horizontal(|ui| {
                                        ui.label("URL:");
                                        ui.text_edit_singleline(&mut url);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Mode:");
                                        egui::ComboBox::from_id_salt("srt_mode_combo")
                                            .selected_text(if mode_idx == 0 {
                                                "Listener"
                                            } else {
                                                "Caller"
                                            })
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(&mut mode_idx, 0, "Listener");
                                                ui.selectable_value(&mut mode_idx, 1, "Caller");
                                            });
                                    });
                                    ui.horizontal(|ui| {
                                        if ui.small_button("✓ Add").clicked() && !url.is_empty() {
                                            let mode = if mode_idx == 0 {
                                                crate::stream::SrtMode::Listener
                                            } else {
                                                crate::stream::SrtMode::Caller
                                            };
                                            // Add to library only — user drags to channel to create deck
                                            actions.srt_library_add = Some((url.clone(), mode));
                                            ui.data_mut(|d| d.insert_temp(adding_id, false));
                                        }
                                        if ui.small_button("✕ Cancel").clicked() {
                                            ui.data_mut(|d| d.insert_temp(adding_id, false));
                                        }
                                    });

                                    ui.data_mut(|d| {
                                        d.insert_temp(url_id, url);
                                        d.insert_temp(mode_id, mode_idx);
                                    });
                                } else if ui.small_button("+ Add SRT").clicked() {
                                    ui.data_mut(|d| d.insert_temp(adding_id, true));
                                }

                                // Existing SRT configs as draggable cards
                                for (i, entry) in data.srt_library_configs.iter().enumerate() {
                                    let item_id = egui::Id::new(("lib_srt", i));
                                    let status_color = if entry.connected {
                                        egui::Color32::from_rgb(100, 220, 100)
                                    } else {
                                        egui::Color32::from_rgb(120, 120, 120)
                                    };
                                    let mode = entry.mode;
                                    let url = entry.url.clone();
                                    ui.horizontal(|ui| {
                                        ui.dnd_drag_source(
                                            item_id,
                                            LibraryDrag::Srt(url.clone(), mode),
                                            |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        egui::RichText::new("●")
                                                            .color(status_color),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(format!("📺 {}", url))
                                                            .size(12.0),
                                                    );
                                                });
                                            },
                                        );
                                        if ui
                                            .small_button("✕")
                                            .on_hover_text("Remove from library")
                                            .clicked()
                                        {
                                            actions.srt_library_remove = Some(url.clone());
                                        }
                                    });
                                    ui.label(
                                        egui::RichText::new(format!("  Mode: {}", entry.mode))
                                            .size(10.0)
                                            .weak(),
                                    );
                                    if ui.ctx().is_being_dragged(item_id) {
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(
                                                egui::Id::new("__lib_dnd_srt_config"),
                                                (url, mode),
                                            );
                                        });
                                    }
                                }
                            });

                        ui.add_space(4.0);

                        // — HLS —
                        let hls_header = egui::RichText::new(format!(
                            "HLS ({})",
                            data.hls_library_configs.len()
                        ))
                        .strong();
                        egui::CollapsingHeader::new(hls_header)
                            .id_salt("lib_hls")
                            .default_open(false)
                            .show(ui, |ui| {
                                let adding_id = ui.id().with("hls_adding");
                                let url_id = ui.id().with("hls_url_input");
                                let is_adding: bool =
                                    ui.data(|d| d.get_temp(adding_id)).unwrap_or(false);

                                if is_adding {
                                    let mut url: String =
                                        ui.data(|d| d.get_temp(url_id)).unwrap_or_else(|| {
                                            "https://example.com/stream.m3u8".to_string()
                                        });
                                    ui.horizontal(|ui| {
                                        ui.label("URL:");
                                        ui.text_edit_singleline(&mut url);
                                    });
                                    ui.horizontal(|ui| {
                                        if ui.small_button("✓ Add").clicked() && !url.is_empty() {
                                            actions.hls_library_add = Some(url.clone());
                                            ui.data_mut(|d| d.insert_temp(adding_id, false));
                                        }
                                        if ui.small_button("✕ Cancel").clicked() {
                                            ui.data_mut(|d| d.insert_temp(adding_id, false));
                                        }
                                    });
                                    ui.data_mut(|d| {
                                        d.insert_temp(url_id, url);
                                    });
                                } else if ui.small_button("+ Add HLS").clicked() {
                                    ui.data_mut(|d| d.insert_temp(adding_id, true));
                                }

                                for (i, entry) in data.hls_library_configs.iter().enumerate() {
                                    let item_id = egui::Id::new(("lib_hls", i));
                                    let status_color = if entry.connected {
                                        egui::Color32::from_rgb(100, 220, 100)
                                    } else {
                                        egui::Color32::from_rgb(180, 180, 180)
                                    };
                                    let url = entry.url.clone();
                                    ui.horizontal(|ui| {
                                        ui.dnd_drag_source(
                                            item_id,
                                            LibraryDrag::Hls(url.clone()),
                                            |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        egui::RichText::new("●")
                                                            .color(status_color),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(format!("📡 {}", url))
                                                            .size(12.0),
                                                    );
                                                });
                                            },
                                        );
                                        if ui
                                            .small_button("✕")
                                            .on_hover_text("Remove from library")
                                            .clicked()
                                        {
                                            actions.hls_library_remove = Some(url.clone());
                                        }
                                    });
                                    if ui.ctx().is_being_dragged(item_id) {
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(
                                                egui::Id::new("__lib_dnd_hls_url"),
                                                url,
                                            );
                                        });
                                    }
                                }
                            });

                        ui.add_space(4.0);

                        // — DASH —
                        let dash_header = egui::RichText::new(format!(
                            "DASH ({})",
                            data.dash_library_configs.len()
                        ))
                        .strong();
                        egui::CollapsingHeader::new(dash_header)
                            .id_salt("lib_dash")
                            .default_open(false)
                            .show(ui, |ui| {
                                let adding_id = ui.id().with("dash_adding");
                                let url_id = ui.id().with("dash_url_input");
                                let is_adding: bool =
                                    ui.data(|d| d.get_temp(adding_id)).unwrap_or(false);

                                if is_adding {
                                    let mut url: String =
                                        ui.data(|d| d.get_temp(url_id)).unwrap_or_else(|| {
                                            "https://example.com/stream.mpd".to_string()
                                        });
                                    ui.horizontal(|ui| {
                                        ui.label("URL:");
                                        ui.text_edit_singleline(&mut url);
                                    });
                                    ui.horizontal(|ui| {
                                        if ui.small_button("✓ Add").clicked() && !url.is_empty() {
                                            actions.dash_library_add = Some(url.clone());
                                            ui.data_mut(|d| d.insert_temp(adding_id, false));
                                        }
                                        if ui.small_button("✕ Cancel").clicked() {
                                            ui.data_mut(|d| d.insert_temp(adding_id, false));
                                        }
                                    });
                                    ui.data_mut(|d| {
                                        d.insert_temp(url_id, url);
                                    });
                                } else if ui.small_button("+ Add DASH").clicked() {
                                    ui.data_mut(|d| d.insert_temp(adding_id, true));
                                }

                                for (i, entry) in data.dash_library_configs.iter().enumerate() {
                                    let item_id = egui::Id::new(("lib_dash", i));
                                    let status_color = if entry.connected {
                                        egui::Color32::from_rgb(100, 220, 100)
                                    } else {
                                        egui::Color32::from_rgb(180, 180, 180)
                                    };
                                    let url = entry.url.clone();
                                    ui.horizontal(|ui| {
                                        ui.dnd_drag_source(
                                            item_id,
                                            LibraryDrag::Dash(url.clone()),
                                            |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        egui::RichText::new("●")
                                                            .color(status_color),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(format!("📡 {}", url))
                                                            .size(12.0),
                                                    );
                                                });
                                            },
                                        );
                                        if ui
                                            .small_button("✕")
                                            .on_hover_text("Remove from library")
                                            .clicked()
                                        {
                                            actions.dash_library_remove = Some(url.clone());
                                        }
                                    });
                                    if ui.ctx().is_being_dragged(item_id) {
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(
                                                egui::Id::new("__lib_dnd_dash_url"),
                                                url,
                                            );
                                        });
                                    }
                                }
                            });

                        ui.add_space(4.0);

                        // — RTMP —
                        let rtmp_header = egui::RichText::new(format!(
                            "RTMP ({})",
                            data.rtmp_library_configs.len()
                        ))
                        .strong();
                        egui::CollapsingHeader::new(rtmp_header)
                            .id_salt("lib_rtmp")
                            .default_open(false)
                            .show(ui, |ui| {
                                let adding_id = ui.id().with("rtmp_adding");
                                let url_id = ui.id().with("rtmp_url_input");
                                let mode_id = ui.id().with("rtmp_mode_input");
                                let is_adding: bool =
                                    ui.data(|d| d.get_temp(adding_id)).unwrap_or(false);

                                if is_adding {
                                    let mut mode: crate::stream::RtmpMode = ui
                                        .data(|d| d.get_temp(mode_id))
                                        .unwrap_or(crate::stream::RtmpMode::Pull);
                                    let prev_mode: crate::stream::RtmpMode = ui
                                        .data(|d| d.get_temp(ui.id().with("rtmp_prev_mode")))
                                        .unwrap_or(crate::stream::RtmpMode::Pull);
                                    let listen_port = 1935 + data.rtmp_library_configs.len();
                                    let auto_listen_url =
                                        format!("rtmp://0.0.0.0:{}/live/stream", listen_port);
                                    let mut url: String =
                                        ui.data(|d| d.get_temp(url_id)).unwrap_or_else(|| {
                                            if mode == crate::stream::RtmpMode::Listen {
                                                auto_listen_url.clone()
                                            } else {
                                                "rtmp://".to_string()
                                            }
                                        });
                                    // Auto-update URL when mode changes
                                    if mode != prev_mode {
                                        if mode == crate::stream::RtmpMode::Listen {
                                            url = auto_listen_url.clone();
                                        } else if prev_mode == crate::stream::RtmpMode::Listen {
                                            url = "rtmp://".to_string();
                                        }
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label("Mode:");
                                        egui::ComboBox::from_id_salt("rtmp_mode_combo")
                                            .selected_text(mode.to_string())
                                            .width(80.0)
                                            .show_ui(ui, |ui| {
                                                if ui
                                                    .selectable_label(
                                                        mode == crate::stream::RtmpMode::Pull,
                                                        "Pull",
                                                    )
                                                    .clicked()
                                                {
                                                    mode = crate::stream::RtmpMode::Pull;
                                                }
                                                if ui
                                                    .selectable_label(
                                                        mode == crate::stream::RtmpMode::Listen,
                                                        "Listen",
                                                    )
                                                    .clicked()
                                                {
                                                    mode = crate::stream::RtmpMode::Listen;
                                                }
                                            });
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("URL:");
                                        ui.add(
                                            egui::TextEdit::singleline(&mut url)
                                                .desired_width(200.0),
                                        );
                                    });
                                    if mode == crate::stream::RtmpMode::Listen {
                                        ui.label(
                                            egui::RichText::new(
                                                "OBS → rtmp://YOUR_IP:PORT/live/stream",
                                            )
                                            .weak()
                                            .small(),
                                        );
                                    }
                                    ui.horizontal(|ui| {
                                        if ui.small_button("✓ Add").clicked() && !url.is_empty() {
                                            actions.rtmp_library_add = Some((url.clone(), mode));
                                            ui.data_mut(|d| d.insert_temp(adding_id, false));
                                        }
                                        if ui.small_button("✕ Cancel").clicked() {
                                            ui.data_mut(|d| d.insert_temp(adding_id, false));
                                        }
                                    });
                                    ui.data_mut(|d| {
                                        d.insert_temp(url_id, url);
                                        d.insert_temp(mode_id, mode);
                                        d.insert_temp(ui.id().with("rtmp_prev_mode"), mode);
                                    });
                                } else if ui.small_button("+ Add RTMP").clicked() {
                                    ui.data_mut(|d| d.insert_temp(adding_id, true));
                                }

                                for (i, entry) in data.rtmp_library_configs.iter().enumerate() {
                                    let item_id = egui::Id::new(("lib_rtmp", i));
                                    let status_color = if entry.connected {
                                        egui::Color32::from_rgb(100, 220, 100)
                                    } else {
                                        egui::Color32::from_rgb(180, 180, 180)
                                    };
                                    let url = entry.url.clone();
                                    let mode = entry.mode;
                                    ui.horizontal(|ui| {
                                        ui.dnd_drag_source(
                                            item_id,
                                            LibraryDrag::Rtmp(url.clone(), mode),
                                            |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        egui::RichText::new("●")
                                                            .color(status_color),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(format!(
                                                            "📺 {} ({})",
                                                            url, mode
                                                        ))
                                                        .size(12.0),
                                                    );
                                                });
                                            },
                                        );
                                        if ui
                                            .small_button("✕")
                                            .on_hover_text("Remove from library")
                                            .clicked()
                                        {
                                            actions.rtmp_library_remove = Some(url.clone());
                                        }
                                    });
                                    if ui.ctx().is_being_dragged(item_id) {
                                        ui.ctx().memory_mut(|mem| {
                                            mem.data.insert_temp(
                                                egui::Id::new("__lib_dnd_rtmp_config"),
                                                (url, mode),
                                            );
                                        });
                                    }
                                }
                            });
                    }); // end Stream Sources

                ui.add_space(4.0);
            }

            // === SYPHON SERVERS ===
            if data.syphon_available {
                let syph_header = egui::RichText::new(format!(
                    "🔗 Syphon Servers ({})",
                    data.syphon_sources.len()
                ))
                .strong();
                egui::CollapsingHeader::new(syph_header)
                    .id_salt("lib_syphon")
                    .default_open(false)
                    .show(ui, |ui| {
                        if ui.small_button("🔄 Rescan").clicked() {
                            actions.syphon_rescan = true;
                        }
                        if data.syphon_sources.is_empty() {
                            ui.label(
                                egui::RichText::new("No Syphon servers found")
                                    .small()
                                    .weak(),
                            );
                        }
                        for (i, name) in data.syphon_sources.iter().enumerate() {
                            let item_id = egui::Id::new(("lib_syph", i));
                            ui.dnd_drag_source(item_id, LibraryDrag::Syphon(name.clone()), |ui| {
                                ui.label(egui::RichText::new(format!("  🔗 {}", name)).size(12.0));
                            });
                            if ui.ctx().is_being_dragged(item_id) {
                                ui.ctx().memory_mut(|mem| {
                                    mem.data.insert_temp(
                                        egui::Id::new("__lib_dnd_syph_name"),
                                        name.clone(),
                                    );
                                });
                            }
                        }
                    });

                ui.add_space(4.0);
            }

            // === DECK PRESETS ===
            if !data.deck_presets.is_empty() {
                let deck_preset_header =
                    egui::RichText::new(format!("💾 Deck Presets ({})", data.deck_presets.len()))
                        .strong();
                egui::CollapsingHeader::new(deck_preset_header)
                    .id_salt("lib_deck_presets")
                    .default_open(false)
                    .show(ui, |ui| {
                        for (idx, name) in data.deck_presets.iter().enumerate() {
                            let item_id = egui::Id::new(("lib_deck_preset", idx));
                            let resp = ui
                                .dnd_drag_source(item_id, LibraryDrag::DeckPreset(idx), |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("  ◈ {}", name)).size(12.0),
                                    );
                                })
                                .response;
                            if ui.ctx().is_being_dragged(item_id) {
                                ui.ctx().memory_mut(|mem| {
                                    mem.data.insert_temp(
                                        egui::Id::new("__lib_dnd_deck_preset_idx"),
                                        idx,
                                    );
                                });
                            }
                            if resp.double_clicked() {
                                actions.deck_preset_to_add = Some((0, idx));
                            }
                            resp.on_hover_text("Drag to a channel to load this deck preset");
                        }
                    });

                ui.add_space(4.0);
            }

            // === CHANNEL PRESETS ===
            if !data.channel_presets.is_empty() {
                let ch_preset_header = egui::RichText::new(format!(
                    "💾 Channel Presets ({})",
                    data.channel_presets.len()
                ))
                .strong();
                egui::CollapsingHeader::new(ch_preset_header)
                    .id_salt("lib_ch_presets")
                    .default_open(false)
                    .show(ui, |ui| {
                        for (idx, name) in data.channel_presets.iter().enumerate() {
                            let item_id = egui::Id::new(("lib_ch_preset", idx));
                            let resp = ui
                                .dnd_drag_source(item_id, LibraryDrag::ChannelPreset(idx), |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("  ◈ {}", name)).size(12.0),
                                    );
                                })
                                .response;
                            if ui.ctx().is_being_dragged(item_id) {
                                ui.ctx().memory_mut(|mem| {
                                    mem.data
                                        .insert_temp(egui::Id::new("__lib_dnd_ch_preset_idx"), idx);
                                });
                            }
                            if resp.double_clicked() {
                                actions.channel_preset_to_add = Some((None, idx));
                            }
                            resp.on_hover_text("Double-click to add this channel to the mixer");
                        }
                    });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_library_panel_smoke() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_library_panel(ui, &data, &mut actions);
        });
    }

    #[test]
    fn render_library_panel_smoke_empty() {
        let mut data = UIData::test_fixture();
        data.generators.clear();
        data.filters.clear();
        data.cameras.clear();
        let mut actions = UIActions::new();
        let _harness = egui_kittest::Harness::new_ui(|ui| {
            render_library_panel(ui, &data, &mut actions);
        });
    }
}
