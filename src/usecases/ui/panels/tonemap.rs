//! Tonemap mode and 3D LUT selection for the master output.
//!
//! Lives beside the main output preview rather than in the top bar: tonemapping
//! is the last thing that touches every pixel on the way out, and it was easy to
//! miss as a five-character abbreviation in a corner. Here the current curve is
//! named in full on the section header, and the preview it affects is directly
//! above it.

use super::super::{UIActions, UIData};
use crate::engine::EngineCommand;
use crate::renderer::tonemap::TonemapMode;

const TONEMAP_PRESETS: &[(&str, TonemapMode)] = &[
    ("Bypass (clamp)", TonemapMode::Bypass),
    ("ACES Filmic", TonemapMode::Aces),
    ("Reinhard", TonemapMode::Reinhard),
    ("Reinhard Extended", TonemapMode::ReinhardExtended),
    ("Hable Filmic", TonemapMode::HableFilmic),
    ("Uchimura (GT)", TonemapMode::Uchimura),
    ("Lottes (AMD)", TonemapMode::Lottes),
    ("AgX", TonemapMode::AgX),
    ("PBR Neutral", TonemapMode::KhronosPbrNeutral),
];

/// Full name of a mode, for the collapsed section header.
pub(super) fn tonemap_name(mode: TonemapMode) -> &'static str {
    TONEMAP_PRESETS
        .iter()
        .find(|(_, m)| *m == mode)
        .map_or("Unknown", |(label, _)| *label)
}

fn tonemap_description(mode: TonemapMode) -> &'static str {
    match mode {
        TonemapMode::Bypass => "Values >1.0 are clamped at the output boundary",
        TonemapMode::Aces => "Cinematic rolloff, warm highlight shift",
        TonemapMode::Reinhard => "Gentle curve, never reaches pure white",
        TonemapMode::ReinhardExtended => "Reinhard with white point, full SDR range",
        TonemapMode::HableFilmic => "Nice toe and shoulder, game-industry standard",
        TonemapMode::Uchimura => "Gran Turismo style, tunable shoulder",
        TonemapMode::Lottes => "Fast, invertible, high contrast",
        TonemapMode::AgX => "Neutral, minimal hue shift",
        TonemapMode::KhronosPbrNeutral => "Color-accurate, minimal look modification",
    }
}

pub(super) fn render_tonemap_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let current = data.tonemap_mode;

    for &(label, mode) in TONEMAP_PRESETS {
        if ui.radio(current == mode, label).clicked() && current != mode {
            actions.commands.push(EngineCommand::SetTonemapMode(mode));
        }
    }

    ui.label(
        egui::RichText::new(tonemap_description(current))
            .weak()
            .small(),
    );

    ui.separator();
    ui.label(egui::RichText::new("🎞 3D LUT").strong());

    let active_lut = data.active_lut_filename.as_deref();

    if ui.radio(active_lut.is_none(), "None").clicked() && active_lut.is_some() {
        actions.commands.push(EngineCommand::UnloadLut);
    }

    for lut_name in &data.available_luts {
        let is_active = active_lut == Some(lut_name.as_str());
        if ui.radio(is_active, lut_name).clicked() && !is_active {
            actions.commands.push(EngineCommand::LoadLut {
                filename: lut_name.clone(),
            });
        }
    }

    if data.available_luts.is_empty() {
        ui.label(
            egui::RichText::new("Place .cube/.3dl files in .varda/luts/")
                .weak()
                .small(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    /// Every mode must be listed, or a scene could load a curve the UI cannot
    /// display or change.
    #[test]
    fn every_mode_is_selectable_and_named() {
        for mode in [
            TonemapMode::Bypass,
            TonemapMode::Aces,
            TonemapMode::Reinhard,
            TonemapMode::ReinhardExtended,
            TonemapMode::HableFilmic,
            TonemapMode::Uchimura,
            TonemapMode::Lottes,
            TonemapMode::AgX,
            TonemapMode::KhronosPbrNeutral,
        ] {
            assert!(
                TONEMAP_PRESETS.iter().any(|(_, m)| *m == mode),
                "{mode:?} is missing from the preset list"
            );
            assert_ne!(tonemap_name(mode), "Unknown");
            assert!(!tonemap_description(mode).is_empty());
        }
    }

    #[test]
    fn selecting_a_different_mode_emits_a_command() {
        let mut data = UIData::test_fixture();
        data.tonemap_mode = TonemapMode::Aces;
        let mut actions = UIActions::new();
        {
            let mut harness = egui_kittest::Harness::new_ui(|ui| {
                render_tonemap_section(ui, &data, &mut actions);
            });
            harness.get_by_label("AgX").click();
            harness.run();
        }

        assert!(
            actions
                .commands
                .iter()
                .any(|c| matches!(c, EngineCommand::SetTonemapMode(TonemapMode::AgX)))
        );
    }

    /// Re-picking the active mode must not churn the engine with a no-op.
    #[test]
    fn reselecting_the_active_mode_emits_nothing() {
        let mut data = UIData::test_fixture();
        data.tonemap_mode = TonemapMode::Aces;
        let mut actions = UIActions::new();
        {
            let mut harness = egui_kittest::Harness::new_ui(|ui| {
                render_tonemap_section(ui, &data, &mut actions);
            });
            harness.get_by_label("ACES Filmic").click();
            harness.run();
        }

        assert!(actions.commands.is_empty());
    }
}
