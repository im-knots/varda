//! UI panel rendering
//!
//! Each sub-module renders a specific panel or UI section.
//! The `render_ui` function orchestrates the top-level layout.

mod bottom_bar;
mod deck_detail;
mod dnd;
mod effects;
mod library;
mod macros;
mod midi;
mod mixer;
mod modulation;
mod notifications_overlay;
mod outputs;
mod popovers;
mod right_panel;
mod sequence;
mod stage;
pub(crate) mod utils;

use super::{UIActions, UIData};
use crate::engine::EngineCommand;
use bottom_bar::render_bottom_panel;
use dnd::{handle_effect_dnd, handle_library_dnd, handle_sequence_step_dnd};
use library::render_library_panel;
use mixer::render_central_panel;
use notifications_overlay::render_notifications;
use popovers::{
    handle_midi_learn_popup, render_clock_popover, render_fps_popover, render_gpu_popover,
    render_resolution_popover, render_target_fps_popover, render_tonemap_popover,
    tonemap_short_name,
};
use right_panel::render_right_panel;

/// Top-level UI rendering entry point. Orchestrates all panels.
pub fn render_ui(ui: &mut egui::Ui, data: &UIData) -> UIActions {
    let mut actions = UIActions::new();

    // Disable all egui animations — instant panel/widget transitions
    ui.global_style_mut(|style| {
        style.animation_time = 0.0;
    });

    // === LEFT PANEL: Library (collapsible) ===
    if data.library_panel_open {
        egui::Panel::left("library_panel")
            .min_size(180.0)
            .default_size(220.0)
            .resizable(true)
            .show_inside(ui, |ui| {
                render_library_panel(ui, data, &mut actions);
            });
    } else {
        egui::Panel::left("library_collapsed")
            .exact_size(36.0)
            .resizable(false)
            .show_inside(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    if ui
                        .small_button("▶")
                        .on_hover_text("Open library (L)")
                        .clicked()
                    {
                        actions.session.toggle_library_panel = true;
                    }
                });
            });
    }

    // === RIGHT PANEL: Main Output + Master Effects (collapsible) ===
    if data.right_panel_open {
        egui::Panel::right("master_panel")
            .min_size(280.0)
            .default_size(320.0)
            .resizable(true)
            .show_inside(ui, |ui| {
                render_right_panel(ui, data, &mut actions);
            });
    } else {
        egui::Panel::right("master_collapsed")
            .exact_size(36.0)
            .resizable(false)
            .show_inside(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    if ui
                        .small_button("«")
                        .on_hover_text("Open master panel")
                        .clicked()
                    {
                        actions.session.toggle_right_panel = true;
                    }
                });
            });
    }

    // === BOTTOM PANEL: Audio, Modulation, Shader Browser ===
    egui::Panel::bottom("bottom_panel")
        .min_size(80.0)
        .max_size(400.0)
        .default_size(180.0)
        .resizable(true)
        .show_separator_line(true)
        .show_inside(ui, |ui| {
            ui.set_min_height(ui.max_rect().height());
            render_bottom_panel(ui, data, &mut actions);
        });

    // === TOP BAR: Save button + FPS/BPM status ===
    egui::Panel::top("top_bar")
        .exact_size(28.0)
        .show_inside(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let any_learn = data.midi_learn_active || data.keyboard_learn_active;
                // Undo / Redo / Save — in learn mode: show glow + select target on click.
                // Outside learn mode: normal action on click.
                {
                    let undo_enabled = if any_learn { true } else { data.can_undo };
                    let undo_resp = ui
                        .add_enabled(undo_enabled, egui::Button::new("↩ Undo"))
                        .on_hover_text("Undo (⌘Z)");
                    if any_learn {
                        if data.midi_learn_active {
                            let is_target =
                                data.midi_learn_target.as_deref() == Some("action/undo");
                            if is_target {
                                super::widgets::draw_midi_learn_selected(ui, undo_resp.rect);
                            } else {
                                super::widgets::draw_midi_learn_glow(ui, undo_resp.rect);
                            }
                            if undo_resp.clicked() {
                                actions.session.midi_learn_select = Some("action/undo".to_string());
                            }
                        } else {
                            let is_target = data.keyboard_learn_target.as_deref() == Some("Undo");
                            if is_target {
                                super::widgets::draw_keyboard_learn_selected(ui, undo_resp.rect);
                            } else {
                                super::widgets::draw_keyboard_learn_glow(ui, undo_resp.rect);
                            }
                            if undo_resp.clicked() {
                                actions.session.keyboard_learn_select = Some(
                                    crate::keymap::KeyTarget::Action(crate::keymap::ActionId::Undo),
                                );
                            }
                        }
                    } else if undo_resp.clicked() {
                        actions.session.undo_requested = true;
                    }
                }
                {
                    let redo_enabled = if any_learn { true } else { data.can_redo };
                    let redo_resp = ui
                        .add_enabled(redo_enabled, egui::Button::new("↪ Redo"))
                        .on_hover_text("Redo (⌘⇧Z)");
                    if any_learn {
                        if data.midi_learn_active {
                            let is_target =
                                data.midi_learn_target.as_deref() == Some("action/redo");
                            if is_target {
                                super::widgets::draw_midi_learn_selected(ui, redo_resp.rect);
                            } else {
                                super::widgets::draw_midi_learn_glow(ui, redo_resp.rect);
                            }
                            if redo_resp.clicked() {
                                actions.session.midi_learn_select = Some("action/redo".to_string());
                            }
                        } else {
                            let is_target = data.keyboard_learn_target.as_deref() == Some("Redo");
                            if is_target {
                                super::widgets::draw_keyboard_learn_selected(ui, redo_resp.rect);
                            } else {
                                super::widgets::draw_keyboard_learn_glow(ui, redo_resp.rect);
                            }
                            if redo_resp.clicked() {
                                actions.session.keyboard_learn_select = Some(
                                    crate::keymap::KeyTarget::Action(crate::keymap::ActionId::Redo),
                                );
                            }
                        }
                    } else if redo_resp.clicked() {
                        actions.session.redo_requested = true;
                    }
                }
                {
                    let save_resp = ui.button("💾 Save").on_hover_text("Save workspace (⌘S)");
                    if any_learn {
                        if data.midi_learn_active {
                            let is_target =
                                data.midi_learn_target.as_deref() == Some("action/save");
                            if is_target {
                                super::widgets::draw_midi_learn_selected(ui, save_resp.rect);
                            } else {
                                super::widgets::draw_midi_learn_glow(ui, save_resp.rect);
                            }
                            if save_resp.clicked() {
                                actions.session.midi_learn_select = Some("action/save".to_string());
                            }
                        } else {
                            let is_target = data.keyboard_learn_target.as_deref() == Some("Save");
                            if is_target {
                                super::widgets::draw_keyboard_learn_selected(ui, save_resp.rect);
                            } else {
                                super::widgets::draw_keyboard_learn_glow(ui, save_resp.rect);
                            }
                            if save_resp.clicked() {
                                actions.session.keyboard_learn_select = Some(
                                    crate::keymap::KeyTarget::Action(crate::keymap::ActionId::Save),
                                );
                            }
                        }
                    } else if save_resp.clicked() {
                        actions.session.save_requested = true;
                    }
                }

                // Learn mode indicators
                if data.midi_learn_active {
                    let text = egui::RichText::new("🎹 MIDI LEARN")
                        .color(egui::Color32::from_rgb(180, 100, 255))
                        .strong();
                    if ui
                        .button(text)
                        .on_hover_text("Click to exit MIDI learn mode")
                        .clicked()
                    {
                        actions.session.midi_learn_toggle = true;
                    }
                }
                if data.keyboard_learn_active {
                    let text = egui::RichText::new("⌨ KB LEARN")
                        .color(egui::Color32::from_rgb(255, 165, 0))
                        .strong();
                    if ui
                        .button(text)
                        .on_hover_text("Click to exit keyboard learn mode")
                        .clicked()
                    {
                        actions.session.keyboard_learn_toggle = true;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // BPM from unified clock (MIDI > OSC > Audio > --)
                    let bpm_text = if let Some(bpm) = data.clock_bpm {
                        format!("{bpm:.0} BPM")
                    } else {
                        "-- BPM".to_string()
                    };
                    if let Some(dev) = &data.clock_device_name {
                        ui.label(egui::RichText::new(format!("({dev})")).weak().small());
                    }
                    // Clickable BPM label → opens clock source popover
                    let bpm_response = ui
                        .add(
                            egui::Label::new(egui::RichText::new(&bpm_text).monospace())
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Click to select clock source");
                    egui::Popup::from_toggle_button_response(&bpm_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_clock_popover(ui, data, &mut actions);
                        });

                    ui.separator();

                    let fps = data.fps;
                    let fps_color = if fps > 55.0 {
                        egui::Color32::from_rgb(100, 220, 100)
                    } else if fps > 30.0 {
                        egui::Color32::from_rgb(220, 200, 60)
                    } else {
                        egui::Color32::from_rgb(220, 60, 60)
                    };
                    let fps_response = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(format!("{fps:.0} FPS"))
                                    .color(fps_color)
                                    .monospace(),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Click for render timing details");
                    egui::Popup::from_toggle_button_response(&fps_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_fps_popover(ui, data);
                        });

                    ui.separator();

                    // GPU utilization % from GPU timestamp data
                    let gpu_load_pct = data.gpu_utilization;
                    let gpu_color = if gpu_load_pct < 50.0 {
                        egui::Color32::from_rgb(100, 220, 100)
                    } else if gpu_load_pct < 80.0 {
                        egui::Color32::from_rgb(220, 200, 60)
                    } else {
                        egui::Color32::from_rgb(220, 60, 60)
                    };
                    let gpu_response = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(format!("🖥 {gpu_load_pct:.0}%"))
                                    .color(gpu_color)
                                    .monospace(),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("GPU utilization — click for details");
                    egui::Popup::from_toggle_button_response(&gpu_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_gpu_popover(ui, data);
                        });

                    ui.separator();

                    // CPU & RAM usage
                    let cpu_color = if data.cpu_usage < 50.0 {
                        egui::Color32::from_rgb(100, 220, 100)
                    } else if data.cpu_usage < 80.0 {
                        egui::Color32::from_rgb(220, 200, 60)
                    } else {
                        egui::Color32::from_rgb(220, 60, 60)
                    };
                    ui.label(
                        egui::RichText::new(format!("CPU {:.0}%", data.cpu_usage))
                            .color(cpu_color)
                            .monospace()
                            .small(),
                    );

                    let ram_gb = data.ram_used as f64 / (1024.0 * 1024.0 * 1024.0);
                    let ram_total_gb = data.ram_total as f64 / (1024.0 * 1024.0 * 1024.0);
                    let ram_pct = if data.ram_total > 0 {
                        data.ram_used as f32 / data.ram_total as f32 * 100.0
                    } else {
                        0.0
                    };
                    let ram_color = if ram_pct < 50.0 {
                        egui::Color32::from_rgb(100, 220, 100)
                    } else if ram_pct < 80.0 {
                        egui::Color32::from_rgb(220, 200, 60)
                    } else {
                        egui::Color32::from_rgb(220, 60, 60)
                    };
                    ui.label(
                        egui::RichText::new(format!("RAM {ram_gb:.1}/{ram_total_gb:.0}G"))
                            .color(ram_color)
                            .monospace()
                            .small(),
                    );

                    ui.separator();

                    // Resolution selector
                    let res_label = format!("📐 {}×{}", data.render_width, data.render_height);
                    let res_response = ui
                        .add(
                            egui::Label::new(egui::RichText::new(&res_label).monospace())
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Click to change render resolution");
                    egui::Popup::from_toggle_button_response(&res_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_resolution_popover(ui, data, &mut actions);
                        });

                    ui.separator();

                    // FPS target selector
                    let fps_target_label = if data.target_fps == 0 {
                        "🎯 Uncapped".to_string()
                    } else {
                        format!("🎯 {}fps", data.target_fps)
                    };
                    let fps_target_response = ui
                        .add(
                            egui::Label::new(egui::RichText::new(&fps_target_label).monospace())
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Click to change target FPS");
                    egui::Popup::from_toggle_button_response(&fps_target_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_target_fps_popover(ui, data, &mut actions);
                        });

                    ui.separator();

                    // Tonemap mode selector
                    let tonemap_label = tonemap_short_name(data.tonemap_mode);
                    let tonemap_response = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(tonemap_label).monospace().small(),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Tonemap mode — click to change");
                    egui::Popup::from_toggle_button_response(&tonemap_response)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            render_tonemap_popover(ui, data, &mut actions);
                        });
                });
            });
        });

    // === CENTRAL AREA: Decks as columns; macro controls live in the center
    // column (see mixer.rs) and their config shows in the bottom bar. ===
    egui::CentralPanel::default().show_inside(ui, |ui| {
        render_central_panel(ui, data, &mut actions);
    });

    // === LIBRARY DnD: deferred drop handler ===
    handle_library_dnd(ui, data, &mut actions);

    // === EFFECT REORDER DnD: deferred drop handler ===
    handle_effect_dnd(ui, data, &mut actions);

    // === SEQUENCE STEP REORDER DnD: deferred drop handler ===
    handle_sequence_step_dnd(ui, data, &mut actions);

    // === NOTIFICATION OVERLAY ===
    render_notifications(ui, &data.notifications, &mut actions);

    // === GLOBAL RIGHT-CLICK: Toggle MIDI Learn Mode ===
    handle_midi_learn_popup(ui, data, &mut actions);

    // === KEYBOARD SHORTCUTS (data-driven via keymap) ===
    {
        use crate::keymap::{collect_pressed_keys, ActionId, KeyCombo, KeyTarget};
        let pressed = collect_pressed_keys(ui);

        if data.keyboard_learn_active {
            // In learn mode: intercept key presses for binding, don't dispatch normally
            if let Some((key, mods)) = pressed.first() {
                let combo = KeyCombo::from_egui(*key, mods);
                actions.session.keyboard_learn_bind = Some(combo);
            }
        } else {
            // Normal dispatch: look up each pressed key in the keymap
            for (key, mods) in &pressed {
                let combo = KeyCombo::from_egui(*key, mods);
                if let Some(target) = data.keymap_bindings.get(&combo) {
                    match target {
                        KeyTarget::Action(ActionId::Undo) => actions.session.undo_requested = true,
                        KeyTarget::Action(ActionId::Redo) => actions.session.redo_requested = true,
                        KeyTarget::Action(ActionId::Save) => actions.session.save_requested = true,
                        KeyTarget::Action(ActionId::ToggleLibrary) => {
                            if !ui.egui_wants_keyboard_input() {
                                actions.session.toggle_library_panel = true;
                            }
                        }
                        KeyTarget::Action(ActionId::ToggleStageEditor) => {
                            actions.session.toggle_stage_editor = true;
                        }
                        KeyTarget::Action(ActionId::ToggleMidiLearn) => {
                            actions.session.midi_learn_toggle = true;
                        }
                        KeyTarget::Action(ActionId::ToggleKeyboardLearn) => {
                            actions.session.keyboard_learn_toggle = true;
                        }
                        KeyTarget::ParamPath(path) => {
                            actions
                                .commands
                                .push(EngineCommand::ToggleParam { path: path.clone() });
                        }
                        // Stage-context actions are handled in stage.rs
                        KeyTarget::Action(_) => {}
                    }
                }
            }
        }
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: `render_ui` doesn't panic with the test fixture.
    #[test]
    fn render_ui_smoke_default_fixture() {
        let data = UIData::test_fixture();
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui, &data);
        });
        // Running the harness processes a frame — if render_ui panics, this test fails.
        let _ = harness;
    }

    /// Every preview and stage canvas now sizes itself from the render
    /// resolution, so a portrait or square project drives layout arithmetic
    /// that a 16:9 fixture never reaches.
    #[test]
    fn render_ui_smoke_non_landscape_resolutions() {
        for (w, h) in [(1080u32, 1920u32), (1080, 1080), (1080, 1350), (0, 0)] {
            let mut data = UIData::test_fixture();
            data.render_width = w;
            data.render_height = h;
            data.stage_editor_open = true;
            let harness = egui_kittest::Harness::new_ui(|ui| {
                let _ = render_ui(ui, &data);
            });
            let _ = harness;
        }
    }

    /// Smoke test: `render_ui` with empty channels doesn't panic.
    #[test]
    fn render_ui_smoke_empty_channels() {
        let mut data = UIData::test_fixture();
        data.channels.clear();
        data.channel_count = 0;
        data.selected_deck = None;
        data.selected_channel = None;
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui, &data);
        });
        let _ = harness;
    }

    /// Smoke test: `render_ui` with library panel closed doesn't panic.
    #[test]
    fn render_ui_smoke_library_closed() {
        let mut data = UIData::test_fixture();
        data.library_panel_open = false;
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui, &data);
        });
        let _ = harness;
    }

    /// Smoke test: `render_ui` with stage editor open doesn't panic.
    #[test]
    fn render_ui_smoke_stage_editor_open() {
        let mut data = UIData::test_fixture();
        data.stage_editor_open = true;
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui, &data);
        });
        let _ = harness;
    }

    /// Smoke test: `render_ui` with master selected doesn't panic.
    #[test]
    fn render_ui_smoke_master_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_master = true;
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui, &data);
        });
        let _ = harness;
    }

    /// Smoke test: `render_ui` with channel selected doesn't panic.
    #[test]
    fn render_ui_smoke_channel_selected() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = Some(0);
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui, &data);
        });
        let _ = harness;
    }

    /// Smoke test: `render_ui` with MIDI learn active doesn't panic.
    #[test]
    fn render_ui_smoke_midi_learn() {
        let mut data = UIData::test_fixture();
        data.midi_learn_active = true;
        data.midi_learn_target = Some("crossfader".to_string());
        let harness = egui_kittest::Harness::new_ui(|ui| {
            let _ = render_ui(ui, &data);
        });
        let _ = harness;
    }
}
