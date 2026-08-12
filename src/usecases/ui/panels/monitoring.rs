//! Live system telemetry: frame rate, GPU load, CPU, and memory.
//!
//! Grouped as one health cluster rather than scattered through the top bar,
//! because these are things you glance at when something feels wrong, not
//! settings you change. They sit at the bottom of the right panel, and stay
//! readable as a vertical strip when that panel is collapsed, so a performer
//! who reclaims the screen space does not lose the frame rate with it.

use super::super::{UIActions, UIData};
use super::popovers::{render_fps_popover, render_gpu_popover};

const GOOD: egui::Color32 = egui::Color32::from_rgb(100, 220, 100);
const WARN: egui::Color32 = egui::Color32::from_rgb(220, 200, 60);
const BAD: egui::Color32 = egui::Color32::from_rgb(220, 60, 60);

/// Frame rate is the one metric where high is good, so it reads inverted.
fn fps_color(fps: f32) -> egui::Color32 {
    if fps > 55.0 {
        GOOD
    } else if fps > 30.0 {
        WARN
    } else {
        BAD
    }
}

/// Shared by GPU, CPU, and RAM, which are all "percent of a budget consumed".
fn load_color(percent: f32) -> egui::Color32 {
    if percent < 50.0 {
        GOOD
    } else if percent < 80.0 {
        WARN
    } else {
        BAD
    }
}

fn ram_gb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

fn deck_count(data: &UIData) -> usize {
    data.channels.iter().map(|c| c.decks.len()).sum()
}

/// Colour targets the scene holds resident, in bytes.
///
/// Decks and channels each own a *pair* of full-resolution targets, since both
/// ping-pong through their effect chain (`Deck Texture` / `Deck Texture B`,
/// `composite_texture` / `effect_ping_texture`), and the mixer owns one more
/// pair. Deliberately an under-estimate of total VRAM: effect pass buffers,
/// video source textures, decoder pools, and camera buffers all sit on top of
/// it. The number exists to answer "is this scene getting heavy" rather than to
/// account for the card. Until residency windows land, an arrangement holds
/// every deck for the whole show, so this is the ceiling a long show runs
/// into. See /spec/arrangement.md § Deferred: Residency Windows.
fn estimated_vram(data: &UIData) -> u64 {
    let bytes_per_pixel = u64::from(
        crate::renderer::context::COLOR_PATH_FORMAT
            .block_copy_size(None)
            .unwrap_or(8),
    );
    let pairs = deck_count(data) as u64 + data.channels.len() as u64 + 1;
    pairs * 2 * u64::from(data.render_width) * u64::from(data.render_height) * bytes_per_pixel
}

fn ram_percent(data: &UIData) -> f32 {
    if data.ram_total == 0 {
        return 0.0;
    }
    data.ram_used as f32 / data.ram_total as f32 * 100.0
}

/// The full cluster, for the bottom of the expanded right panel.
///
/// `actions` is unused today but kept in the signature: the popovers this opens
/// are shared with paths that emit commands, and threading it later would touch
/// every call site.
pub(super) fn render_monitoring_section(
    ui: &mut egui::Ui,
    data: &UIData,
    _actions: &mut UIActions,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        let fps_response = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(format!("{:.0} FPS", data.fps))
                        .color(fps_color(data.fps))
                        .monospace()
                        .small(),
                )
                .sense(egui::Sense::click()),
            )
            .on_hover_text("Click for render timing details");
        egui::Popup::from_toggle_button_response(&fps_response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                render_fps_popover(ui, data);
            });

        let gpu = data.gpu_utilization;
        let gpu_response = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(format!("🖥 {gpu:.0}%"))
                        .color(load_color(gpu))
                        .monospace()
                        .small(),
                )
                .sense(egui::Sense::click()),
            )
            .on_hover_text("GPU utilization — click for details");
        egui::Popup::from_toggle_button_response(&gpu_response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                render_gpu_popover(ui, data);
            });

        ui.label(
            egui::RichText::new(format!("CPU {:.0}%", data.cpu_usage))
                .color(load_color(data.cpu_usage))
                .monospace()
                .small(),
        );

        ui.label(
            egui::RichText::new(format!(
                "RAM {:.1}/{:.0}G",
                ram_gb(data.ram_used),
                ram_gb(data.ram_total)
            ))
            .color(load_color(ram_percent(data)))
            .monospace()
            .small(),
        );

        let decks = deck_count(data);
        ui.label(
            egui::RichText::new(format!(
                "{decks} decks ~{:.1}G",
                ram_gb(estimated_vram(data))
            ))
            .monospace()
            .small()
            .weak(),
        )
        .on_hover_text(format!(
            "{decks} decks across {} channels, holding at least {:.2} GB of {}x{} colour targets. \
             Effect buffers and video decoders sit on top of that. Decks stay resident for the \
             whole show, so a long arrangement pays this the entire time.",
            data.channels.len(),
            ram_gb(estimated_vram(data)),
            data.render_width,
            data.render_height,
        ));
    });
}

/// The same four values stacked for the collapsed rail, which is 36 px wide.
///
/// Values only, no labels: at this width a label costs a whole row, and colour
/// plus position already say which is which. Hover gives the full text.
pub(super) fn render_monitoring_strip(ui: &mut egui::Ui, data: &UIData) {
    let gpu = data.gpu_utilization;
    let rows = [
        (
            format!("{:.0}", data.fps),
            fps_color(data.fps),
            format!("{:.0} FPS", data.fps),
        ),
        (
            format!("{gpu:.0}%"),
            load_color(gpu),
            format!("GPU {gpu:.0}%"),
        ),
        (
            format!("{:.0}%", data.cpu_usage),
            load_color(data.cpu_usage),
            format!("CPU {:.0}%", data.cpu_usage),
        ),
        (
            format!("{:.1}G", ram_gb(data.ram_used)),
            load_color(ram_percent(data)),
            format!(
                "RAM {:.1} of {:.0} GB",
                ram_gb(data.ram_used),
                ram_gb(data.ram_total)
            ),
        ),
    ];

    for (text, color, hover) in rows {
        ui.label(egui::RichText::new(text).color(color).monospace().small())
            .on_hover_text(hover);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_colour_is_inverted_relative_to_load() {
        assert_eq!(fps_color(60.0), GOOD);
        assert_eq!(fps_color(45.0), WARN);
        assert_eq!(fps_color(12.0), BAD);

        assert_eq!(load_color(10.0), GOOD);
        assert_eq!(load_color(60.0), WARN);
        assert_eq!(load_color(95.0), BAD);
    }

    /// A machine reporting no total memory must not divide by zero.
    #[test]
    fn ram_percent_survives_an_unknown_total() {
        let mut data = UIData::test_fixture();
        data.ram_total = 0;
        data.ram_used = 1024;
        assert_eq!(ram_percent(&data), 0.0);
    }

    /// The estimate has to move with the scene, or it is decoration.
    #[test]
    fn the_vram_estimate_grows_with_the_scene() {
        let mut data = UIData::test_fixture();
        data.render_width = 1920;
        data.render_height = 1080;
        let before = estimated_vram(&data);
        assert!(before > 0, "a scene with decks must estimate something");

        // A deck is a pair of 1080p Rgba16Float targets, about 33 MB. Anything
        // that costs less than that is counting the wrong thing.
        let per_deck = estimated_vram(&data) / (deck_count(&data) + data.channels.len() + 1) as u64;
        assert!(
            (30..40).contains(&(per_deck / (1024 * 1024))),
            "expected roughly 33 MB per deck at 1080p, got {} MB",
            per_deck / (1024 * 1024)
        );

        let decks_before = deck_count(&data);
        let deck = data.channels[0].decks[0].clone();
        data.channels[0].decks.push(deck);
        assert!(
            estimated_vram(&data) > before,
            "another deck must cost another pair of targets"
        );
        assert_eq!(deck_count(&data), decks_before + 1);
    }

    #[test]
    fn both_layouts_render() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        {
            let _expanded = egui_kittest::Harness::new_ui(|ui| {
                render_monitoring_section(ui, &data, &mut actions);
            });
        }
        let _collapsed = egui_kittest::Harness::new_ui(|ui| {
            render_monitoring_strip(ui, &data);
        });
        assert!(actions.commands.is_empty());
    }
}
