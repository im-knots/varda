//! Lighting: the LIGHTS band, the lighting deck card, and the lighting deck detail.
//!
//! A lighting deck is a deck. Its detail bar has the same left-to-right shape as a video deck's
//! — a picture of what it is doing, then its parameters, then the mix — and its parameters go
//! through the same `widgets::render_params` a shader's uniforms do, so they are modulatable,
//! automatable and learnable with no lighting-specific machinery.
//!
//! Reads the same [`crate::dmx::LightingSnapshot`] that `/api/state/lighting` serves and emits
//! the same `EngineCommand`s the REST routes do, so neither consumer holds logic the other
//! lacks. See /spec/lighting-routing.md § API Parity.

use super::super::widgets;
use crate::engine::EngineCommand;
use crate::modulation::DEFAULT_ASSIGNMENT_AMOUNT;
use crate::params::ParamValue;
use crate::usecases::ui::data::ParamUIInfo;
use crate::usecases::ui::{UIActions, UIData};

/// Warm amber, the lighting band's identity color. Deliberately a section accent rather than a
/// per-widget color: the existing amber dot means "held by hand" and must keep that meaning
/// here. See /spec/lighting-routing.md § Color language.
pub(super) fn lighting_accent() -> egui::Color32 {
    egui::Color32::from_rgb(255, 190, 90)
}

fn health_color(snapshot: &crate::dmx::LightingSnapshot) -> egui::Color32 {
    if !snapshot.running {
        egui::Color32::from_rgb(120, 120, 130)
    } else if snapshot.healthy {
        egui::Color32::from_rgb(80, 200, 80)
    } else {
        egui::Color32::from_rgb(200, 80, 80)
    }
}

/// The rig: transport status, patch findings, the patched fixtures, and the patch form.
///
/// Lives in the library's Lighting tree, because a rig is the lighting equivalent of the shader
/// and media lists: it is the content you have available to work with.
/// The DMX transport: is it running, is it healthy, is anything timing out.
///
/// Split from the patch because they answer different questions at different times. What lights
/// exist and how they are grouped is show content and is authored on a deck; whether Art-Net is
/// actually reaching them is output plumbing and belongs with the other outputs.
pub(super) fn render_transport_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let snapshot = &data.lighting;
    render_status_row(ui, snapshot, data.lights_band_open, actions);
    if !snapshot.watchdog.is_empty() {
        ui.add_space(6.0);
        render_watchdog(ui, snapshot);
    }
}

/// Palettes, for the library's Palettes tree.
pub(super) fn render_palette_section(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    render_palettes(ui, data, actions);
}

fn render_status_row(
    ui: &mut egui::Ui,
    snapshot: &crate::dmx::LightingSnapshot,
    band_open: bool,
    actions: &mut UIActions,
) {
    ui.horizontal(|ui| {
        let mut enabled = snapshot.enabled;
        if ui.checkbox(&mut enabled, "Enabled").changed() {
            actions
                .commands
                .push(EngineCommand::SetLightingEnabled(enabled));
        }

        if snapshot.enabled {
            let mut open = band_open;
            if ui
                .checkbox(&mut open, "Band")
                .on_hover_text("Show the LIGHTS band in the central area")
                .changed()
            {
                actions.session.toggle_lights_band = true;
            }
        }

        ui.separator();
        ui.colored_label(health_color(snapshot), "●");
        let label = if snapshot.running {
            if snapshot.transports.is_empty() {
                "no transport".to_string()
            } else {
                snapshot.transports.join(", ")
            }
        } else {
            "stopped".to_string()
        };
        ui.label(egui::RichText::new(label).small());
    });

    ui.horizontal(|ui| {
        let mut blackout = snapshot.blackout;
        let button = egui::Button::new(egui::RichText::new("BLACKOUT").color(if blackout {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_rgb(200, 80, 80)
        }))
        .fill(if blackout {
            egui::Color32::from_rgb(160, 40, 40)
        } else {
            egui::Color32::from_rgb(40, 40, 50)
        });
        if ui.add(button).clicked() {
            blackout = !blackout;
            actions
                .commands
                .push(EngineCommand::SetLightingBlackout(blackout));
        }

        if snapshot.programmer_roles > 0
            && ui
                .button("Release")
                .on_hover_text(format!(
                    "{} programmer value(s) held by hand; hand them back to the looks",
                    snapshot.programmer_roles
                ))
                .clicked()
        {
            actions
                .commands
                .push(EngineCommand::ReleaseLightingProgrammer);
        }

        if snapshot.consumer_wedged {
            ui.colored_label(egui::Color32::from_rgb(255, 170, 60), "⚠ transmit stalled");
        }
        if snapshot.packets_sent > 0 {
            ui.label(
                egui::RichText::new(format!("{} pkt", snapshot.packets_sent))
                    .small()
                    .color(egui::Color32::from_rgb(120, 120, 130)),
            );
        }
    });
}

pub(super) fn render_messages(ui: &mut egui::Ui, snapshot: &crate::dmx::LightingSnapshot) {
    for message in &snapshot.warnings {
        let (color, glyph) = if message.severity == "error" {
            (egui::Color32::from_rgb(220, 90, 90), "✖")
        } else {
            (egui::Color32::from_rgb(220, 180, 80), "⚠")
        };
        let text = match &message.fixture {
            Some(fixture) => format!("{glyph} {fixture}: {}", message.text),
            None => format!("{glyph} {}", message.text),
        };
        ui.colored_label(color, egui::RichText::new(text).small());
    }
}

/// The patch, as configured.
///
/// Lists `snapshot.patch` rather than `snapshot.fixtures`: one bad entry fails validation for the
/// whole rig, which resolves to zero fixtures, and listing only resolved fixtures meant a broken
/// patch showed as "No fixtures patched" while the config still held it. The entry that needs
/// deleting was the one entry that could not be seen.
pub(super) fn render_fixture_list(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let snapshot = &data.lighting;
    if snapshot.patch.is_empty() {
        ui.label(
            egui::RichText::new("No lights yet")
                .small()
                .color(egui::Color32::from_rgb(120, 120, 130)),
        );
        return;
    }

    // Resolved fixtures carry detail the config does not (vendor, model, channel count), so
    // pair them up where the entry actually patched.
    for entry in &snapshot.patch {
        let resolved = snapshot.fixtures.iter().find(|f| f.id == entry.id);
        let broken = entry.error.is_some();
        let name_color = if broken {
            egui::Color32::from_rgb(230, 120, 110)
        } else {
            lighting_accent()
        };

        ui.horizontal(|ui| {
            if broken {
                ui.label(egui::RichText::new("⚠").color(name_color));
            }
            ui.label(
                egui::RichText::new(entry.name.as_str())
                    .color(name_color)
                    .strong(),
            );
            let channels =
                resolved.map_or_else(String::new, |f| format!(" ({}ch)", f.channel_count));
            ui.label(
                egui::RichText::new(format!("U{} @{}{channels}", entry.universe, entry.address))
                    .small()
                    .color(egui::Color32::from_rgb(150, 150, 160)),
            );
            // Plain `x`, matching the project's unified remove-button language.
            if ui
                .small_button("x")
                .on_hover_text("Remove this fixture from the patch")
                .clicked()
            {
                actions.commands.push(EngineCommand::RemoveFixture {
                    uuid: entry.id.clone(),
                });
            }
        });

        let detail = resolved.map_or_else(
            || format!("   {} · {}", entry.profile, entry.mode),
            |f| format!("   {} {} · {}", f.vendor, f.model, f.mode),
        );
        ui.label(
            egui::RichText::new(detail)
                .small()
                .color(egui::Color32::from_rgb(110, 110, 120)),
        );

        // The reason, on the row it belongs to. A rig-wide message list makes an operator match
        // errors to fixtures by name during load-in.
        if let Some(err) = &entry.error {
            ui.label(
                egui::RichText::new(format!("   {err}"))
                    .small()
                    .color(egui::Color32::from_rgb(210, 110, 100)),
            );
        }
    }
}

/// The profile chooser, with search.
///
/// Searchable because the bundled library is 653 definitions and an operator owns a dozen
/// lights. Changing profile clears the chosen mode, since modes belong to the profile.
fn render_profile_picker(
    ui: &mut egui::Ui,
    id: egui::Id,
    data: &UIData,
    draft: &mut AddFixtureDraft,
) {
    ui.horizontal(|ui| {
        ui.label("Profile");
        let before = draft.profile.clone();
        egui::ComboBox::from_id_salt(id.with("profile"))
            .selected_text(if draft.profile.is_empty() {
                "select…".to_string()
            } else {
                draft.profile.clone()
            })
            .width(150.0)
            .show_ui(ui, |ui| {
                // Search, because the bundled library is 653 definitions and an
                // operator owns a dozen lights.
                ui.add(
                    egui::TextEdit::singleline(&mut draft.filter)
                        .hint_text("search")
                        .desired_width(140.0),
                );
                let needle = draft.filter.to_ascii_lowercase();
                let mut shown = 0usize;
                for reference in &data.lighting_profiles {
                    if !needle.is_empty() && !reference.to_ascii_lowercase().contains(&needle) {
                        continue;
                    }
                    ui.selectable_value(&mut draft.profile, reference.clone(), reference);
                    shown += 1;
                    if shown >= 200 {
                        ui.label(
                            egui::RichText::new("…keep typing to narrow")
                                .small()
                                .color(egui::Color32::from_rgb(120, 120, 130)),
                        );
                        break;
                    }
                }
            });
        // Changing profile invalidates the chosen mode: the modes belong to the profile.
        if draft.profile != before {
            draft.mode = modes_for(data, &draft.profile)
                .first()
                .cloned()
                .unwrap_or_default();
        }
    });
}

pub(super) fn render_add_fixture(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let id = ui.id().with("add_fixture");
    let mut draft: AddFixtureDraft = ui
        .ctx()
        .data_mut(|d| d.get_temp::<AddFixtureDraft>(id))
        .unwrap_or_default();

    // "Add light", not "Patch fixture": the same rule that made the blend modes
    // Lighten/Normal instead of HTP/LTP. The console word survives on the hover, for the
    // operators who go looking for it.
    egui::CollapsingHeader::new(egui::RichText::new("➕ Add light").small())
        .id_salt(id)
        // Open when there is nothing yet: an empty rig must not hide the only way to fill it.
        .default_open(data.lighting.fixtures.is_empty())
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut draft.name);
            });
            render_profile_picker(ui, id, data, &mut draft);

            let modes = modes_for(data, &draft.profile);
            ui.horizontal(|ui| {
                ui.label("Mode");
                let enabled = !modes.is_empty();
                ui.add_enabled_ui(enabled, |ui| {
                    egui::ComboBox::from_id_salt(id.with("mode"))
                        .selected_text(if draft.mode.is_empty() {
                            "select…".to_string()
                        } else {
                            draft.mode.clone()
                        })
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for mode in &modes {
                                ui.selectable_value(&mut draft.mode, mode.clone(), mode);
                            }
                        });
                });
                if !enabled {
                    ui.label(
                        egui::RichText::new(if draft.profile.is_empty() {
                            "pick a profile first"
                        } else {
                            "this profile defines no modes"
                        })
                        .small()
                        .color(egui::Color32::from_rgb(120, 120, 130)),
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.label("Universe");
                ui.add(egui::DragValue::new(&mut draft.universe).range(1..=63_999));
                ui.label("Address");
                ui.add(egui::DragValue::new(&mut draft.address).range(1..=512));
            });

            let ready = !draft.name.trim().is_empty()
                && !draft.profile.trim().is_empty()
                && !draft.mode.trim().is_empty();
            if ui
                .add_enabled(ready, egui::Button::new("Add"))
                .on_hover_text("Patch this fixture into the rig")
                .on_disabled_hover_text("Name, profile and mode are required")
                .clicked()
            {
                actions.commands.push(EngineCommand::AddFixture {
                    name: draft.name.trim().to_string(),
                    profile: draft.profile.trim().to_string(),
                    mode: draft.mode.trim().to_string(),
                    universe: draft.universe,
                    address: draft.address,
                });
                // Advance the address past the fixture just patched, so patching a row of
                // identical pars is repeated clicks rather than repeated arithmetic.
                let span = data
                    .lighting
                    .fixtures
                    .iter()
                    .find(|f| f.mode == draft.mode)
                    .map_or(1, |f| u16::try_from(f.channel_count).unwrap_or(1));
                draft.address = draft.address.saturating_add(span).min(512);
                draft.name = next_name(&draft.name);
            }
        });

    ui.ctx().data_mut(|d| d.insert_temp(id, draft));
}

/// Increment a trailing number so patching `par-1` offers `par-2` next.
///
/// Zero padding is preserved: a rig numbered `par-01` through `par-12` should stay aligned
/// rather than turning into `par-8` halfway along.
fn next_name(name: &str) -> String {
    let trimmed = name.trim_end();
    let digits: String = trimmed
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        return trimmed.to_string();
    }
    let stem = &trimmed[..trimmed.len() - digits.len()];
    match digits.parse::<u32>() {
        Ok(n) => format!("{stem}{:0width$}", n + 1, width = digits.len()),
        Err(_) => trimmed.to_string(),
    }
}

fn render_watchdog(ui: &mut egui::Ui, snapshot: &crate::dmx::LightingSnapshot) {
    ui.label(
        egui::RichText::new("Watchdog")
            .small()
            .color(lighting_accent()),
    );
    for flag in &snapshot.watchdog {
        let kind = match flag.kind {
            crate::dmx::WatchdogKind::Dark => "dark",
            crate::dmx::WatchdogKind::Stuck => "stuck",
        };
        ui.colored_label(
            egui::Color32::from_rgb(220, 180, 80),
            egui::RichText::new(format!("⚠ {} {kind} {:.0}s", flag.name, flag.age_secs)).small(),
        );
    }
}

/// Modes a profile defines, or empty when it is unknown or defines none.
fn modes_for(data: &UIData, profile: &str) -> Vec<String> {
    data.lighting_profile_modes
        .get(profile)
        .cloned()
        .unwrap_or_default()
}

/// In-flight state of the patch form, parked in egui temp storage so it survives repaints
/// without the engine having to know about a half-typed fixture.
#[derive(Clone)]
struct AddFixtureDraft {
    name: String,
    profile: String,
    mode: String,
    /// Search text for the profile picker.
    filter: String,
    universe: u16,
    address: u16,
}

impl Default for AddFixtureDraft {
    fn default() -> Self {
        Self {
            name: "par-1".to_string(),
            profile: String::new(),
            mode: String::new(),
            filter: String::new(),
            universe: 1,
            address: 1,
        }
    }
}

// ── Looks and palettes ──────────────────────────────────────────────────────

/// In-flight state of the new-palette form.
#[derive(Clone)]
struct PaletteDraft {
    name: String,
    kind: String,
}

fn render_palettes(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions) {
    let programmer_held = data.lighting.programmer_roles > 0;
    ui.label(
        egui::RichText::new("Palettes")
            .small()
            .color(lighting_accent()),
    );

    if data.lighting.palettes.is_empty() {
        ui.label(
            egui::RichText::new("No palettes yet")
                .small()
                .color(egui::Color32::from_rgb(120, 120, 130)),
        );
    }
    // Both storages in one list: a look references a palette by UUID and does not care which
    // file it came from. The label says which, so an operator can see which of their palettes
    // will survive a move to another room.
    for palette in &data.lighting.palettes {
        render_palette_row(ui, palette, programmer_held, actions);
    }

    let id = ui.id().with("new_palette");
    let mut draft = ui
        .ctx()
        .data_mut(|d| d.get_temp::<PaletteDraft>(id))
        .unwrap_or(PaletteDraft {
            name: String::new(),
            kind: "color".into(),
        });
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut draft.name)
                .hint_text("new palette")
                .desired_width(90.0),
        );
        egui::ComboBox::from_id_salt(id.with("kind"))
            .selected_text(draft.kind.clone())
            .width(70.0)
            .show_ui(ui, |ui| {
                for kind in ["color", "beam", "position"] {
                    ui.selectable_value(&mut draft.kind, kind.to_string(), kind);
                }
            });
        if ui
            .add_enabled(!draft.name.trim().is_empty(), egui::Button::new("+"))
            .clicked()
        {
            actions.commands.push(EngineCommand::AddPalette {
                name: draft.name.trim().to_string(),
                kind: draft.kind.clone(),
            });
            draft.name.clear();
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, draft));
}

/// The color a palette holds, for its swatch.
///
/// Read from the roles it covers. A palette that covers no color roles has nothing to show, and
/// the caller draws an empty outline rather than a black square.
fn palette_swatch(palette: &crate::dmx::PaletteView) -> egui::Color32 {
    let role = |name: &str| {
        palette
            .default_roles
            .iter()
            .position(|r| r == name)
            .and_then(|i| palette.default_values.get(i).copied())
            .unwrap_or(0.0)
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    let (r, g, b) = (byte(role("red")), byte(role("green")), byte(role("blue")));
    if r == 0 && g == 0 && b == 0 {
        // A color palette can legitimately be built from white or amber emitters alone, so a
        // zero RGB triple is not necessarily an empty palette.
        let w = byte(role("white")).max(byte(role("amber")));
        return egui::Color32::from_rgb(w, w, w);
    }
    egui::Color32::from_rgb(r, g, b)
}

fn swatch_to_rgb(c: egui::Color32) -> [f32; 3] {
    [
        f32::from(c.r()) / 255.0,
        f32::from(c.g()) / 255.0,
        f32::from(c.b()) / 255.0,
    ]
}

fn render_palette_row(
    ui: &mut egui::Ui,
    palette: &crate::dmx::PaletteView,
    programmer_held: bool,
    actions: &mut UIActions,
) {
    ui.horizontal(|ui| {
        // The color it holds, so a palette is a thing you can see rather than a name with no
        // visible value. A palette that holds nothing reads as empty rather than as black.
        if palette.kind == "color" {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            if palette.default_roles.is_empty() {
                ui.painter().rect_stroke(
                    rect,
                    2.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 78, 86)),
                    egui::StrokeKind::Inside,
                );
            } else {
                ui.painter().rect_filled(rect, 2.0, palette_swatch(palette));
            }
        }
        ui.label(egui::RichText::new(palette.name.as_str()).small());
        let (label, hover) = if palette.venue {
            ("venue", "belongs to this room; re-point it at a new venue")
        } else {
            ("show", "travels with the show")
        };
        ui.label(
            egui::RichText::new(label)
                .small()
                .color(egui::Color32::from_rgb(120, 120, 130)),
        )
        .on_hover_text(hover);
        // Setting a color directly. Without this the only way to give a palette a value was to
        // hold one in the programmer first and store it, so a freshly created palette was inert
        // and there was nothing to indicate why.
        if palette.kind == "color" {
            let id = ui.id().with(("palette_rgb", &palette.id));
            let mut rgb: [f32; 3] = ui
                .ctx()
                .data_mut(|d| d.get_temp(id))
                .unwrap_or_else(|| swatch_to_rgb(palette_swatch(palette)));
            if ui
                .color_edit_button_rgb(&mut rgb)
                .on_hover_text("set this palette's color")
                .changed()
            {
                ui.ctx().data_mut(|d| d.insert_temp(id, rgb));
                for (role, value) in [("red", rgb[0]), ("green", rgb[1]), ("blue", rgb[2])] {
                    actions.commands.push(EngineCommand::SetPaletteValue {
                        palette: palette.id.clone(),
                        role: role.to_string(),
                        value,
                        fixture: None,
                    });
                }
            }
        }
        if ui
            .add_enabled(programmer_held, egui::Button::new("⏺").small())
            .on_hover_text("store the programmer into this palette")
            .on_disabled_hover_text("set some values by hand first")
            .clicked()
        {
            actions
                .commands
                .push(EngineCommand::StoreProgrammerToPalette {
                    palette: palette.id.clone(),
                });
        }
        if palette.override_count > 0 {
            ui.label(
                egui::RichText::new(format!("{}×", palette.override_count))
                    .small()
                    .color(egui::Color32::from_rgb(120, 120, 130)),
            )
            .on_hover_text("per-fixture or per-profile overrides");
        }
        if ui.small_button("x").clicked() {
            actions.commands.push(EngineCommand::RemovePalette {
                uuid: palette.id.clone(),
            });
        }
    });
}

// ── The LIGHTS band ─────────────────────────────────────────────────────────

/// Height of a collapsed band strip.
pub(super) const COLLAPSED_BAND_HEIGHT: f32 = 22.0;

/// True when the LIGHTS band is offered at all.
///
/// Offered as soon as the scene has anything lighting-related: a patched rig, or a lighting deck
/// in a channel. A user who never touches lighting sees nothing; a user who has touched it must
/// always have something to click.
///
/// An earlier version gated this on a patched rig alone, which made the band, its collapsed
/// strip, and its toggle *all* invisible until a fixture existed. That is a dead end: the only
/// way in was a control that was itself hidden.
#[must_use]
pub(super) fn lights_band_offered(_ctx: &egui::Context, _data: &UIData) -> bool {
    // Always. Two earlier attempts gated this on "is there anything lighting-related yet", and
    // both times the gate also hid the only way to create that something: the zone you drop a
    // lighting deck into. A channel's lighting section is part of the channel, the same way its
    // deck stack is, and it is present whether or not anything is in it.
    //
    // A performer who does not use lighting collapses the band once; that is one click, against
    // a class of bug that has now cost two rounds.
    true
}

/// True while a lighting item is being dragged out of the library.
#[must_use]
pub(super) fn lighting_drag_in_flight(ctx: &egui::Context) -> bool {
    egui::DragAndDrop::payload::<crate::usecases::ui::LibraryDrag>(ctx).is_some_and(|p| {
        matches!(
            *p,
            crate::usecases::ui::LibraryDrag::LightingDeck
                | crate::usecases::ui::LibraryDrag::Look(_)
        )
    })
}

/// True when the LIGHTS band should occupy vertical space in the central area.
#[must_use]
pub(super) fn lights_band_expanded(ctx: &egui::Context, data: &UIData) -> bool {
    data.lights_band_open || lighting_drag_in_flight(ctx)
}

/// Split the central height between the two bands.
///
/// Returns `(video_height, lights_height)`. A collapsed band gives its height to the other,
/// so with LIGHTS collapsed the VIDEO band occupies the full central area exactly as before.
#[must_use]
pub(super) fn band_heights(ctx: &egui::Context, data: &UIData, available: f32) -> (f32, f32) {
    let lights = lights_band_expanded(ctx, data);
    let video = data.video_band_open;
    match (video, lights) {
        (true, false) => (available - COLLAPSED_BAND_HEIGHT, COLLAPSED_BAND_HEIGHT),
        (false, true) => (COLLAPSED_BAND_HEIGHT, available - COLLAPSED_BAND_HEIGHT),
        (false, false) => (COLLAPSED_BAND_HEIGHT, COLLAPSED_BAND_HEIGHT),
        (true, true) => {
            let split = data.band_split.clamp(0.15, 0.85);
            let video_height = (available * split).max(COLLAPSED_BAND_HEIGHT);
            (
                video_height,
                (available - video_height).max(COLLAPSED_BAND_HEIGHT),
            )
        }
    }
}

/// A low-saturation warm wash distinguishing the LIGHTS band.
///
/// A large desaturated field, deliberately not a card or glyph color: the saturated amber dot
/// already means "held by hand" and must keep that meaning inside this band too.
fn band_tint() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(255, 170, 60, 18)
}

/// One channel's lighting decks, as a column in the LIGHTS band.
/// egui temp-memory key under which a channel's lighting zone publishes its screen rect.
///
/// Separate from `ch_drop_rect`, which the video column owns: both resolve to the same channel,
/// but publishing under one key would mean whichever column drew last silently became the only
/// droppable half.
pub(super) const LIGHT_DROP_RECT_KEY: &str = "light_drop_rect";

/// The border and wash one lighting lane wears.
///
/// The border comes from the same helper the VIDEO band uses, so a channel lane looks like one
/// column across both bands instead of two stacked containers. Without it the LIGHTS band had no
/// vertical rules between channels at all.
///
/// The fill is the band's, not the channel's: the border says "channel N", the warm wash says
/// "this half is lighting", and they answer different questions.
fn lighting_lane_frame(
    ch_idx: usize,
    dragging: bool,
    hovered: bool,
    selected: bool,
) -> egui::Frame {
    super::utils::channel_lane_frame(
        super::utils::channel_color(ch_idx),
        super::utils::LaneBorder {
            hovered,
            drag_active: dragging,
            selected,
        },
    )
    .fill(if dragging {
        // Brighter while a lighting item is in flight, so the zone reads as somewhere to aim
        // rather than as background.
        egui::Color32::from_rgba_unmultiplied(255, 170, 60, 46)
    } else {
        band_tint()
    })
}

pub(super) fn render_lighting_column(
    ui: &mut egui::Ui,
    channel_uuid: &str,
    ch_idx: usize,
    width: f32,
    height: f32,
    data: &UIData,
    actions: &mut UIActions,
) {
    let decks = data.lighting_show.decks(channel_uuid).to_vec();

    let dragging = lighting_drag_in_flight(ui.ctx());
    // Hover is tested against the rect this column published last frame, not `ui.max_rect()`:
    // this function is called straight from the band's horizontal layout, so `max_rect` is the
    // whole remaining row and every column would light up at once.
    let prev_rect: Option<egui::Rect> = ui.ctx().memory(|mem| {
        mem.data
            .get_temp(egui::Id::new(LIGHT_DROP_RECT_KEY).with(ch_idx))
    });
    let hovered = dragging
        && prev_rect.is_some_and(|r| ui.ctx().pointer_latest_pos().is_some_and(|p| r.contains(p)));

    let frame = lighting_lane_frame(
        ch_idx,
        dragging,
        hovered,
        data.selected_channel == Some(ch_idx),
    );

    let zone = ui.allocate_ui(egui::vec2(width, height), |ui| {
        frame.inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_min_size(egui::vec2(width - 8.0, height - 8.0));
            egui::ScrollArea::vertical()
                .id_salt(("lights_col", channel_uuid))
                .show(ui, |ui| {
                    if decks.is_empty() {
                        ui.label(
                            egui::RichText::new("drop a lighting deck")
                                .small()
                                .color(egui::Color32::from_rgb(150, 130, 100)),
                        );
                    }
                    for deck in &decks {
                        render_lighting_deck_card(ui, deck, width - 16.0, data, actions);
                    }
                });
        });
    });

    // Publish this zone so a drop lands on the channel it is drawn in. Without it the pointer
    // resolves to no channel at all and the drop is discarded in silence.
    ui.ctx().memory_mut(|mem| {
        mem.data.insert_temp(
            egui::Id::new(LIGHT_DROP_RECT_KEY).with(ch_idx),
            zone.response.rect,
        );
    });
}

/// One lighting deck, drawn as a deck card.
///
/// The same card as a video deck — same geometry, same chrome, same preview-plus-fader row, same
/// name and button rows — because a lighting deck *is* a deck. The only difference is what fills
/// the preview: video shows its rendered texture, lighting shows what the rig is emitting.
/// See /spec/lighting-routing.md § Performance-Mode UI.
fn render_lighting_deck_card(
    ui: &mut egui::Ui,
    deck: &crate::dmx::LightingDeck,
    _width: f32,
    data: &UIData,
    actions: &mut UIActions,
) {
    let uuid = deck.id.to_string();
    let selected = data.selected_lighting_deck.as_deref() == Some(uuid.as_str());
    // The deck's own content, so there is no such thing as a deck whose look is missing.
    let look_name = deck.content.name.as_str();
    let accent = lighting_accent();

    let card = super::utils::DeckCard::new(data.render_width, data.render_height);
    let (card_rect, card_resp) = ui.allocate_exact_size(card.size(), egui::Sense::click());

    let border = if selected {
        egui::Stroke::new(2.0, accent)
    } else {
        egui::Stroke::new(1.0, accent.linear_multiply(0.3))
    };
    super::utils::paint_deck_card_frame(ui.painter(), card_rect, border, 255);

    // Row 1: the preview, and the level fader beside it.
    render_deck_preview(ui, card.preview_rect(card_rect), deck, data);

    let mut level = deck.level;
    let mut slider_ui = ui.new_child(egui::UiBuilder::new().max_rect(card.slider_rect(card_rect)));
    let resp = slider_ui.add_sized(
        [card.slider_width, card.preview.y],
        egui::Slider::new(&mut level, 0.0..=1.0)
            .vertical()
            .show_value(false),
    );
    if resp.dragged() {
        // A held fader drag is a single undo gesture, exactly as on a video deck.
        actions.session.gesture_active = true;
    }
    if resp.changed() {
        actions.commands.push(EngineCommand::UpdateLightingDeck {
            uuid: uuid.clone(),
            level: Some(level),
            blend: None,
            mute: None,
            solo: None,
            independent: None,
            ltp_transition: None,
        });
    }

    // Row 2: the deck's name, which for a lighting deck is its look's name.
    ui.painter().text(
        card.name_pos(card_rect),
        egui::Align2::LEFT_TOP,
        super::utils::truncate_chars(look_name, 16),
        egui::FontId::proportional(11.0),
        accent,
    );

    // Row 3: M S and the rest.
    let mut btn_ui = ui.new_child(egui::UiBuilder::new().max_rect(card.button_rect(card_rect)));
    render_deck_toggles(&mut btn_ui, deck, &uuid, actions);

    if card_resp.clicked() {
        actions.session.select_lighting_deck = Some(uuid.clone());
    }
    card_resp.on_hover_text("Open the deck detail to say which lights are in this deck");
    ui.add_space(3.0);
}

/// What a lighting deck's preview window shows: the rig, in the color it is currently emitting.
///
/// The video counterpart is the deck's rendered texture. A lighting deck has no texture, so the
/// preview answers the same question a different way — *what is this deck putting on stage right
/// now* — by drawing one cell per fixture in patch order.
fn render_deck_preview(
    ui: &egui::Ui,
    rect: egui::Rect,
    _deck: &crate::dmx::LightingDeck,
    data: &UIData,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, egui::Color32::from_rgb(18, 16, 22));

    let fixtures = &data.lighting.fixtures;
    if fixtures.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "no rig",
            egui::FontId::proportional(10.0),
            egui::Color32::from_rgb(110, 100, 90),
        );
        return;
    }

    // A grid rather than a strip: the preview is square-ish, and a rig of 24 fixtures in one row
    // would give each of them two pixels.
    let count = fixtures.len().min(64);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let cols = (count as f32).sqrt().ceil().max(1.0) as usize;
    let rows = count.div_ceil(cols);
    #[allow(clippy::cast_precision_loss)]
    let cell_w = (rect.width() - 4.0) / cols as f32;
    #[allow(clippy::cast_precision_loss)]
    let cell_h = (rect.height() - 4.0) / rows as f32;

    for (i, fixture) in fixtures.iter().take(count).enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
        let cell = egui::Rect::from_min_size(
            egui::pos2(
                rect.min.x + 2.0 + cx * cell_w,
                rect.min.y + 2.0 + cy * cell_h,
            ),
            egui::vec2((cell_w - 1.0).max(1.0), (cell_h - 1.0).max(1.0)),
        );
        painter.rect_filled(cell, 1.0, fixture_color(fixture, data));
    }
}

/// Mute, solo, independent and remove, matching the video deck card's button row.
fn render_deck_toggles(
    ui: &mut egui::Ui,
    deck: &crate::dmx::LightingDeck,
    uuid: &str,
    actions: &mut UIActions,
) {
    /// A command that changes exactly one field of a deck, leaving the rest untouched.
    fn patch(uuid: &str) -> EngineCommand {
        EngineCommand::UpdateLightingDeck {
            uuid: uuid.to_owned(),
            level: None,
            blend: None,
            mute: None,
            solo: None,
            independent: None,
            ltp_transition: None,
        }
    }

    ui.horizontal(|ui| {
        let toggle = |ui: &mut egui::Ui, on: bool, text: &str, color: egui::Color32| {
            let button = egui::Button::new(egui::RichText::new(text).small().color(if on {
                egui::Color32::WHITE
            } else {
                color
            }))
            .fill(if on {
                color
            } else {
                egui::Color32::from_rgb(38, 36, 42)
            });
            ui.add(button).clicked()
        };

        if toggle(ui, deck.mute, "M", egui::Color32::from_rgb(200, 60, 60)) {
            let mut cmd = patch(uuid);
            if let EngineCommand::UpdateLightingDeck { ref mut mute, .. } = cmd {
                *mute = Some(!deck.mute);
            }
            actions.commands.push(cmd);
        }
        if toggle(ui, deck.solo, "S", egui::Color32::from_rgb(220, 170, 60)) {
            let mut cmd = patch(uuid);
            if let EngineCommand::UpdateLightingDeck { ref mut solo, .. } = cmd {
                *solo = Some(!deck.solo);
            }
            actions.commands.push(cmd);
        }
        // Independent: holds through a crossfade. House lights, blinders, audience wash.
        if toggle(
            ui,
            deck.independent,
            "I",
            egui::Color32::from_rgb(90, 160, 220),
        ) {
            let mut cmd = patch(uuid);
            if let EngineCommand::UpdateLightingDeck {
                ref mut independent,
                ..
            } = cmd
            {
                *independent = Some(!deck.independent);
            }
            actions.commands.push(cmd);
        }
        if ui.small_button("x").clicked() {
            actions.commands.push(EngineCommand::RemoveLightingDeck {
                uuid: uuid.to_owned(),
            });
        }
    });
}

/// The lighting analogue of a deck preview thumbnail.
///
/// Color a fixture is currently emitting, read from the last transmitted universe.
pub(super) fn fixture_color(fixture: &crate::dmx::FixtureView, data: &UIData) -> egui::Color32 {
    let Some(universe) = data
        .lighting
        .universes
        .iter()
        .find(|u| u.universe == fixture.universe)
    else {
        return egui::Color32::from_rgb(40, 38, 44);
    };
    let channel_of = |role: &str| {
        universe
            .slots
            .iter()
            .find(|s| s.fixture == fixture.name && s.role.as_deref() == Some(role))
            .map_or(0, |s| s.value)
    };
    let dimmer = universe
        .slots
        .iter()
        .find(|s| s.fixture == fixture.name && s.role.as_deref() == Some("dimmer"))
        .map_or(255, |s| s.value);
    let scale = f32::from(dimmer) / 255.0;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let scaled = |v: u8| (f32::from(v) * scale) as u8;
    let (r, g, b) = (
        scaled(channel_of("red")),
        scaled(channel_of("green")),
        scaled(channel_of("blue")),
    );
    if r == 0 && g == 0 && b == 0 {
        // A fixture with no color channels still shows its intensity, so a dimmer-only par or
        // a mover reads as lit rather than as absent.
        let white = scaled(channel_of("white")).max(scaled(dimmer));
        return egui::Color32::from_rgb(white, white, white);
    }
    egui::Color32::from_rgb(r, g, b)
}

/// The collapsed strip a band shows when it has no height.
pub(super) fn render_collapsed_band(
    ui: &mut egui::Ui,
    label: &str,
    warm: bool,
    open: &mut bool,
) -> bool {
    let mut toggled = false;
    egui::Frame::NONE
        .fill(if warm {
            band_tint()
        } else {
            egui::Color32::from_rgb(28, 28, 34)
        })
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button(if *open { "▾" } else { "▸" }).clicked() {
                    *open = !*open;
                    toggled = true;
                }
                ui.label(egui::RichText::new(label).small().color(if warm {
                    lighting_accent()
                } else {
                    egui::Color32::from_rgb(150, 150, 160)
                }));
            });
        });
    toggled
}

// ── Bottom bar: lighting deck detail ────────────────────────────────────────

/// Everything about one lighting deck, in the bottom bar where a video deck's detail goes.
///
/// This is the other half of the workflow: drag a group into a channel from the library, then
/// shape what it does here. Same shape as selecting a video deck and editing its shader
/// parameters. See /spec/lighting-routing.md § Performance-Mode UI.
pub(super) fn render_lighting_deck_detail(
    ui: &mut egui::Ui,
    uuid: &str,
    data: &UIData,
    actions: &mut UIActions,
) {
    let Ok(deck_id) = uuid::Uuid::parse_str(uuid) else {
        return;
    };
    let Some((channel, deck)) = data.lighting_show.find_deck(deck_id) else {
        ui.label(
            egui::RichText::new("That lighting deck is gone")
                .small()
                .color(egui::Color32::from_rgb(120, 120, 130)),
        );
        return;
    };
    let channel = channel.to_string();
    let deck = deck.clone();
    let look = Some(deck.content.clone());

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(look.as_ref().map_or("(missing look)", |l| l.name.as_str()))
                .color(lighting_accent())
                .strong(),
        );
        // The counterpart of a video deck's "Save Preset". The deck keeps its own content; the
        // library gets an independent copy. See /spec/lighting-routing.md § A deck owns its
        // content.
        let prompt_id = egui::Id::new(("look_save_prompt", uuid));
        let name_id = egui::Id::new(("look_save_name", uuid));
        let prompting: bool = ui.data(|d| d.get_temp(prompt_id)).unwrap_or(false);
        if prompting {
            let mut name: String = ui
                .data(|d| d.get_temp(name_id))
                .unwrap_or_else(|| deck.content.name.clone());
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0));
            if ui.small_button("✓ Save").clicked() && !name.trim().is_empty() {
                actions.commands.push(EngineCommand::SaveLook {
                    deck: uuid.to_string(),
                    name: name.trim().to_string(),
                });
                ui.data_mut(|d| d.insert_temp(prompt_id, false));
            }
            if ui.small_button("✕").clicked() {
                ui.data_mut(|d| d.insert_temp(prompt_id, false));
            }
            ui.data_mut(|d| d.insert_temp(name_id, name));
        } else if ui
            .small_button("💾 Save Look")
            .on_hover_text("copy this deck into the library as a Look")
            .clicked()
        {
            ui.data_mut(|d| {
                d.insert_temp(prompt_id, true);
                d.remove_temp::<String>(name_id);
            });
        }

        if ui.small_button("x").on_hover_text("close").clicked() {
            actions.session.select_lighting_deck = Some(String::new());
        }
    });
    ui.separator();

    // The same left-to-right shape as a video deck: a picture of what this deck is doing, then
    // its parameters. The mix sits under the plot, as Blend does in a video deck's params column.
    egui::ScrollArea::horizontal()
        .id_salt(("lighting_deck_hscroll", uuid))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                if let Some(ref look) = look {
                    render_stage_column(ui, look, &deck, uuid, &channel, data, actions);
                    ui.separator();
                    // One parameter column, because a deck is one behaviour. A video deck has
                    // one too; the N-column shape existed only while a deck named its targets.
                    // See /spec/lighting-routing.md § A deck does not address anything.
                    render_deck_params(ui, look, data, actions);
                }
            });
        });
}

/// Which groups are listening to this deck's channel.
///
/// Read-only, and outward: a deck names nobody, so this reports who has pointed themselves at it.
/// Routing is done in the Stage editor, where the group lives.
fn render_listeners(ui: &mut egui::Ui, channel: &str, data: &UIData) {
    let listening: Vec<&str> = data
        .lighting_group_sources
        .iter()
        .filter(|(_, source)| source.as_deref() == Some(channel))
        .filter_map(|(uuid, _)| {
            data.lighting_group_names
                .iter()
                .find(|(g, _, _)| g == uuid)
                .map(|(_, name, _)| name.as_str())
        })
        .collect();

    ui.label(egui::RichText::new("Heard by").small().strong());
    if listening.is_empty() {
        ui.label(
            egui::RichText::new("nothing yet — point a group at this channel in the Stage editor")
                .small()
                .color(egui::Color32::from_rgb(120, 120, 130)),
        );
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for name in listening {
            ui.label(
                egui::RichText::new(format!("◈ {name}"))
                    .small()
                    .color(lighting_accent()),
            );
        }
    });
}

/// The deck's parameters: the roles this behaviour drives.
///
/// One column, because a deck is one behaviour. Which lamps hear it is the listening groups'
/// business, so the roles offered are the union of what the groups on this deck's channel can
/// actually do — a deck in a channel no group listens to has nothing to offer, and says so.
///
/// Rendered through `widgets::render_params`, the same function a shader's uniforms go through,
/// so modulation, automation, MIDI learn and keyboard learn work on a pan exactly as on a shader
/// uniform. See /spec/lighting-routing.md § Roles are parameter paths.
fn render_deck_params(
    ui: &mut egui::Ui,
    look: &crate::dmx::Look,
    data: &UIData,
    actions: &mut UIActions,
) {
    let look_uuid = look.id.to_string();
    let params = deck_params(look, data);

    egui::Frame::default()
        .inner_margin(6.0)
        .corner_radius(4.0)
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_min_width(230.0);
            ui.set_max_width(260.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.label(
                    egui::RichText::new(look.name.as_str())
                        .strong()
                        .color(lighting_accent()),
                );
                if params.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "No group listens to this channel yet — point one at it in the Stage \
                             editor",
                        )
                        .small()
                        .color(egui::Color32::from_rgb(120, 120, 130)),
                    );
                    return;
                }

                let set_look = {
                    let look_uuid = look_uuid.clone();
                    move |name: &str, val: ParamValue| {
                        // Every role is a normalized float. The other variants cannot arise from
                        // a param this module builds, but the match must be total.
                        let value = match val {
                            ParamValue::Float(v) => v,
                            ParamValue::Bool(b) => f32::from(b),
                            #[allow(clippy::cast_precision_loss)]
                            ParamValue::Long(v) => v as f32,
                            ParamValue::Color(c) => c[0],
                            ParamValue::Point2D(p) => p[0],
                        };
                        EngineCommand::SetLookValue {
                            look: look_uuid.clone(),
                            role: name.to_string(),
                            value: Some(value),
                            palette: None,
                        }
                    }
                };

                let prefix = format!("look/{look_uuid}");
                let (assign, unassign, remove, automate) = (
                    prefix.clone(),
                    prefix.clone(),
                    prefix.clone(),
                    prefix.clone(),
                );

                widgets::render_params(
                    ui,
                    &params,
                    &data.modulation_sources,
                    &set_look,
                    Some(
                        &|name: &str, source_uuid: &str| EngineCommand::AssignModulation {
                            target: format!("{assign}/{name}"),
                            source_id: source_uuid.to_string(),
                            amount: DEFAULT_ASSIGNMENT_AMOUNT,
                        },
                    ),
                    Some(
                        &|name: &str, source_uuid: &str| EngineCommand::ClearModulationSource {
                            target: format!("{unassign}/{name}"),
                            source_id: source_uuid.to_string(),
                        },
                    ),
                    Some(&|name: &str| EngineCommand::ClearModulation {
                        target: format!("{remove}/{name}"),
                    }),
                    Some(&|name: &str| EngineCommand::AddAutomationLane {
                        target: format!("{automate}/{name}"),
                        timebase: crate::timebase::Timebase::Transport,
                    }),
                    &mut actions.commands,
                    &mut actions.session.gesture_active,
                    &format!("look_{look_uuid}"),
                    Some(&prefix),
                    data.midi_learn_active,
                    &mut actions.session.midi_learn_select,
                    data.midi_learn_target.as_deref(),
                    &data.modulation_assignments,
                    &data.modulation_current_values,
                    &prefix,
                    data.keyboard_learn_active,
                    &mut actions.session.keyboard_learn_select,
                    data.keyboard_learn_target.as_deref(),
                );
            });
        });
}

/// The roles a deck can drive: the union of what the listening groups' fixtures declare.
///
/// A group offers the union of its members' roles for the same reason it always did — fanning a
/// value across a mixed group is ordinary, and fixtures that cannot do it ignore it.
fn deck_params(look: &crate::dmx::Look, data: &UIData) -> Vec<ParamUIInfo> {
    let mut roles: Vec<String> = Vec::new();
    for fixture in &data.lighting.fixtures {
        for role in &fixture.roles {
            if !roles.iter().any(|r| r == role) {
                roles.push(role.clone());
            }
        }
    }
    // Declaration order, so Dimmer leads and the color roles stay together, matching the order an
    // operator reads them on the fixture itself.
    roles.sort_by_key(|r| {
        crate::dmx::Role::ALL
            .iter()
            .position(|k| k.as_str() == r)
            .unwrap_or(usize::MAX)
    });

    roles
        .into_iter()
        .map(|role| {
            let value = look
                .assignments()
                .iter()
                .find(|a| a.role.as_str() == role)
                .and_then(|a| a.value.as_literal())
                .unwrap_or(0.0);
            ParamUIInfo {
                name: role.clone(),
                label: Some(role_label(&role)),
                value: ParamValue::Float(value),
                min: Some(0.0),
                max: Some(1.0),
                group: role_group_name(&role).map(str::to_string),
                choices: Vec::new(),
            }
        })
        .collect()
}

/// The rig, drawn on a stage. The lighting deck's answer to the video deck preview.
///
/// A video deck's leftmost column is a picture of what that deck is putting out. This is the
/// same column for a lighting deck: the fixtures of the rig, laid out as they stand on stage,
/// each lit in the color it is currently emitting. It is what makes "what is this deck doing"
/// answerable at a glance, and it is where fixtures are selected, added and grouped.
/// See /spec/lighting-routing.md § The stage view.
fn render_stage_column(
    ui: &mut egui::Ui,
    _look: &crate::dmx::Look,
    deck: &crate::dmx::LightingDeck,
    deck_uuid: &str,
    deck_channel: &str,
    data: &UIData,
    actions: &mut UIActions,
) {
    // Which lamps hear this deck. The deck names nobody — the groups listening to its channel
    // do — so this is resolved outward from the channel, not inward from the deck.
    // See /spec/lighting-routing.md § A group is the lighting surface.
    let driven: std::collections::BTreeSet<String> = data
        .lighting_group_sources
        .iter()
        .filter(|(_, source)| source.as_deref() == Some(deck_channel))
        .flat_map(|(uuid, _)| {
            data.lighting_group_members
                .iter()
                .find(|(g, _)| g == uuid)
                .map(|(_, m)| m.clone())
                .unwrap_or_default()
        })
        .collect();
    let in_look = |id: &str| driven.contains(id);

    // Width is fixed, height follows the bar, exactly as the video preview column does.
    let width = 300.0_f32;
    let available_height = (ui.available_height() - 12.0).max(80.0);

    egui::Frame::default()
        .inner_margin(6.0)
        .corner_radius(4.0)
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_min_width(width + 12.0);
            ui.set_max_width(width + 12.0);
            // The parent is a `horizontal_top`, so without an explicit top-down layout every
            // widget in this column lays out sideways and a text field renders one character
            // per line.
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.label(egui::RichText::new("🎭 Stage").strong());

                // A preview, not an editor. Placing a lamp, patching one, or changing which
                // group it is in are facts about the room and happen in the Stage editor; this
                // shows what the deck is doing with them.
                // See /spec/lighting-routing.md § The deck detail.
                let stage = egui::vec2(width, (available_height - 150.0).clamp(90.0, 260.0));
                let (rect, _) = ui.allocate_exact_size(stage, egui::Sense::hover());
                draw_stage(ui, rect, data, &in_look);

                ui.label(
                    egui::RichText::new(if data.lighting.fixtures.is_empty() {
                        "no lights yet — rig them in the Stage editor"
                    } else {
                        "what this deck is putting on stage"
                    })
                    .small()
                    .color(egui::Color32::from_rgb(120, 120, 130)),
                );

                ui.add_space(4.0);
                render_listeners(ui, deck_channel, data);

                // The deck's own controls, under the picture of what the deck is doing. A video
                // deck puts Blend at the top of its params column because that column *is* the
                // deck; a lighting deck addresses many targets and so has many params columns,
                // and none of them is the deck. This column is.
                ui.add_space(4.0);
                ui.separator();
                render_deck_mix(ui, deck, deck_uuid, actions);
            });
        });
}

/// Where a fixture sits on the stage plot.
///
/// Derived from universe and address rather than stored: a rig is patched in the order it is
/// rigged, so address order is already the physical order in almost every case, and asking an
/// operator to lay out a plot before they can use a light would be a setup step video has no
/// equivalent of. A real XY plot is Phase 61 work.
fn stage_slot_for(
    rect: egui::Rect,
    fixture: &crate::dmx::FixtureView,
    index: usize,
    count: usize,
) -> egui::Rect {
    let Some([x, y]) = fixture.position else {
        return stage_slot(rect, index, count);
    };
    // A placed fixture gets a cell the size the grid would have given it, centred on its point,
    // so placed and unplaced lights stay the same size and the plot does not reflow as one is
    // dragged.
    let cell = stage_slot(rect, 0, count).size();
    egui::Rect::from_center_size(
        egui::pos2(
            rect.min.x + x.clamp(0.0, 1.0) * rect.width(),
            rect.min.y + y.clamp(0.0, 1.0) * rect.height(),
        ),
        cell,
    )
}

fn stage_slot(rect: egui::Rect, index: usize, count: usize) -> egui::Rect {
    let count = count.max(1);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let cols = ((count as f32).sqrt().ceil() as usize).max(1);
    let rows = count.div_ceil(cols);
    #[allow(clippy::cast_precision_loss)]
    let cell = egui::vec2(rect.width() / cols as f32, rect.height() / rows as f32);
    #[allow(clippy::cast_precision_loss)]
    let (cx, cy) = ((index % cols) as f32, (index / cols) as f32);
    egui::Rect::from_min_size(
        egui::pos2(rect.min.x + cx * cell.x, rect.min.y + cy * cell.y),
        cell,
    )
    .shrink(3.0)
}

fn draw_stage(ui: &egui::Ui, rect: egui::Rect, data: &UIData, in_look: &dyn Fn(&str) -> bool) {
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(16, 15, 20));

    let fixtures = &data.lighting.fixtures;
    if fixtures.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "no lights yet — patch one below",
            egui::FontId::proportional(11.0),
            egui::Color32::from_rgb(110, 100, 90),
        );
        return;
    }

    for (i, fixture) in fixtures.iter().enumerate() {
        let cell = stage_slot_for(rect, fixture, i, fixtures.len());
        let color = fixture_color(fixture, data);
        let member = in_look(&fixture.id);

        // The emitted color fills the lamp; membership of this deck is the ring around it. A
        // light can be lit by another deck without belonging to this one, and the two facts have
        // to stay separately readable.
        // An unlit lamp draws over a dim body rather than as its literal black, or a patched rig
        // sitting at zero would be an empty box: the plot has to show where the lights *are*
        // before it can show what they are doing.
        let radius = cell.width().min(cell.height()) * 0.32;
        painter.circle_filled(cell.center(), radius, egui::Color32::from_rgb(38, 36, 42));
        painter.circle_filled(cell.center(), radius, color);
        painter.circle_stroke(
            cell.center(),
            cell.width().min(cell.height()) * 0.38,
            if member {
                egui::Stroke::new(2.0, lighting_accent())
            } else {
                egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 58, 66))
            },
        );
        painter.text(
            egui::pos2(cell.center().x, cell.max.y - 1.0),
            egui::Align2::CENTER_BOTTOM,
            super::utils::truncate_chars(&fixture.name, 8),
            egui::FontId::proportional(9.0),
            egui::Color32::from_rgb(150, 145, 155),
        );
    }
}

fn role_label(role: &str) -> String {
    let mut out = String::with_capacity(role.len());
    for (i, ch) in role.chars().enumerate() {
        if i == 0 {
            out.extend(ch.to_uppercase());
        } else if ch == '_' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn role_group_name(role: &str) -> Option<&'static str> {
    let parsed = crate::dmx::Role::ALL.iter().find(|r| r.as_str() == role)?;
    Some(match parsed.group() {
        crate::dmx::RoleGroup::Intensity => "Intensity",
        crate::dmx::RoleGroup::Color => "Color",
        crate::dmx::RoleGroup::Position => "Position",
        crate::dmx::RoleGroup::Beam => "Beam",
        crate::dmx::RoleGroup::Control => "Control",
    })
}

/// Level, blend, transition and the flags: how this deck mixes.
///
/// Rendered inside the Stage column rather than as a column of its own, following the video deck
/// detail, which puts Blend at the top of its params column instead of giving the mix a column.
/// A video deck has one params column and so has an obvious home for deck-wide settings; a
/// lighting deck has one per target and so has none — the Stage column is the one that means
/// "this deck".
fn render_deck_mix(
    ui: &mut egui::Ui,
    deck: &crate::dmx::LightingDeck,
    uuid: &str,
    actions: &mut UIActions,
) {
    /// A command touching exactly one field, leaving the rest alone.
    fn patch(uuid: &str) -> EngineCommand {
        EngineCommand::UpdateLightingDeck {
            uuid: uuid.to_owned(),
            level: None,
            blend: None,
            mute: None,
            solo: None,
            independent: None,
            ltp_transition: None,
        }
    }

    // No frame of its own: it sits inside the Stage column, which already has one.
    {
        {
            {
                ui.label(egui::RichText::new("Mix").small().strong());

                // Options only. Level is the fader on the deck card in the band, exactly as a
                // video deck's opacity is: the detail carries the settings you reach for while
                // building a look, not a second copy of the control you perform with.

                ui.horizontal(|ui| {
            ui.label("Blend");
            egui::ComboBox::from_id_salt((uuid, "blend"))
                .selected_text(deck.blend.label())
                .width(96.0)
                .show_ui(ui, |ui| {
                    for mode in crate::dmx::LightingBlend::ALL {
                        let mut chosen = deck.blend;
                        if ui
                            .selectable_value(&mut chosen, *mode, mode.label())
                            .clicked()
                        {
                            let mut cmd = patch(uuid);
                            if let EngineCommand::UpdateLightingDeck { ref mut blend, .. } = cmd {
                                *blend = Some(*mode);
                            }
                            actions.commands.push(cmd);
                        }
                    }
                });
        })
        .response
        .on_hover_text(
            "Auto is HTP for intensity and LTP for everything else, which is what a console does",
        );

                ui.horizontal(|ui| {
            ui.label("On fade");
            egui::ComboBox::from_id_salt((uuid, "ltp"))
                .selected_text(deck.ltp_transition.label())
                .width(96.0)
                .show_ui(ui, |ui| {
                    for mode in crate::dmx::LtpTransition::ALL {
                        let mut chosen = deck.ltp_transition;
                        if ui
                            .selectable_value(&mut chosen, *mode, mode.label())
                            .clicked()
                        {
                            let mut cmd = patch(uuid);
                            if let EngineCommand::UpdateLightingDeck {
                                ref mut ltp_transition,
                                ..
                            } = cmd
                            {
                                *ltp_transition = Some(*mode);
                            }
                            actions.commands.push(cmd);
                        }
                    }
                });
        })
        .response
        .on_hover_text(
            "Fade sweeps a moving head through the intermediate angles; Snap cuts to the new one",
        );

                let mut independent = deck.independent;
                if ui
                    .checkbox(&mut independent, "Independent")
                    .on_hover_text(
                        "Hold through a crossfade: house lights, blinders, audience wash",
                    )
                    .changed()
                {
                    let mut cmd = patch(uuid);
                    if let EngineCommand::UpdateLightingDeck {
                        independent: ref mut i,
                        ..
                    } = cmd
                    {
                        *i = Some(independent);
                    }
                    actions.commands.push(cmd);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    /// Render the lighting deck detail to a PNG so the layout can actually be looked at.
    ///
    /// Ignored by default: it is a development aid, not a contract. Run with
    /// `cargo test -- --ignored render_deck_detail_to_png --nocapture`.
    #[test]
    #[ignore = "development aid: writes PNGs to $SHOT_DIR rather than asserting"]
    fn render_deck_detail_to_png() {
        for (name, fixtures) in [("empty", 0_usize), ("rigged", 6)] {
            let mut data = UIData::test_fixture();
            for i in 0..fixtures {
                let mut f = fixture_view(
                    &uuid::Uuid::new_v4().to_string(),
                    &format!("par-{}", i + 1),
                    &["dimmer", "red", "green", "blue", "pan", "tilt"],
                );
                f.address = u16::try_from(i * 8 + 1).unwrap_or(1);
                // Both halves, as the running app has them: `fixtures` is the resolved rig and
                // `patch` is what was configured. A shot with only one populated is not the
                // screen anyone sees.
                data.lighting.patch.push(crate::dmx::PatchEntryView {
                    id: f.id.clone(),
                    name: f.name.clone(),
                    profile: "generic/rgbw".into(),
                    mode: f.mode.clone(),
                    universe: f.universe,
                    address: f.address,
                    error: None,
                });
                data.lighting.fixtures.push(f);
            }
            let mut look = crate::dmx::Look::new("Wash");
            look.set(
                crate::dmx::Role::Dimmer,
                crate::dmx::AttrValue::literal(1.0),
            );
            let deck = crate::dmx::LightingDeck::new(look.clone());
            let uuid = deck.id.to_string();
            data.lighting_show.looks.push(look);
            data.lighting_show.decks_mut("ch-1").push(deck);

            let mut actions = UIActions::new();
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1360.0, 380.0))
                .build_ui(|ui| {
                    render_lighting_deck_detail(ui, &uuid, &data, &mut actions);
                });
            harness.run();
            let image = harness.render().expect("render");
            let path = format!(
                "{}/deck_detail_{name}.png",
                std::env::var("SHOT_DIR").unwrap()
            );
            image.save(&path).expect("save");
            println!("wrote {path}");

            // The card, at the size it is drawn in the LIGHTS band.
            let mut card_actions = UIActions::new();
            let card_deck = data.lighting_show.decks("ch-1")[0].clone();
            let mut card = egui_kittest::Harness::builder()
                .with_size(egui::vec2(170.0, 160.0))
                .build_ui(|ui| {
                    render_lighting_deck_card(ui, &card_deck, 130.0, &data, &mut card_actions);
                });
            card.run();
            let card_path = format!(
                "{}/deck_card_{name}.png",
                std::env::var("SHOT_DIR").unwrap()
            );
            card.render()
                .expect("render")
                .save(&card_path)
                .expect("save");
            println!("wrote {card_path}");
        }
    }

    /// Modes belong to a profile, so the patch form offers what the chosen profile defines
    /// instead of asking an operator to remember and type one.
    #[test]
    fn modes_come_from_the_selected_profile() {
        let mut data = UIData::test_fixture();
        let mut index = std::collections::HashMap::new();
        index.insert(
            "generic/rgbw".to_string(),
            vec!["4ch".to_string(), "8ch".to_string()],
        );
        data.lighting_profile_modes = std::sync::Arc::new(index);

        assert_eq!(modes_for(&data, "generic/rgbw"), vec!["4ch", "8ch"]);
        assert!(
            modes_for(&data, "nope/unknown").is_empty(),
            "an unknown profile offers nothing rather than a stale list"
        );
        assert!(modes_for(&data, "").is_empty(), "no profile chosen yet");
    }

    /// A channel's lighting section is part of the channel, present whether or not anything is
    /// in it. Two earlier versions gated it on "is there anything lighting-related yet", and
    /// both times the gate also hid the only way to create that something.
    /// The vertical rules between channels. The LIGHTS band shipped with `Frame::NONE`, so its
    /// lanes had no column boundary and the band read as one undivided field.
    #[test]
    fn a_lighting_lane_draws_the_same_column_rule_as_a_video_lane() {
        let resting = lighting_lane_frame(0, false, false, false);
        let video = super::super::utils::channel_lane_frame(
            super::super::utils::channel_color(0),
            super::super::utils::LaneBorder::default(),
        );
        assert_eq!(
            resting.stroke, video.stroke,
            "the two bands must draw the same lane border, or a channel stops reading as one column"
        );
        assert!(resting.stroke.width > 0.0);
    }

    /// The border tracks the channel, so two channels never wear the same one.
    #[test]
    fn each_lighting_lane_wears_its_own_channel_accent() {
        let a = lighting_lane_frame(0, false, false, true);
        let b = lighting_lane_frame(1, false, false, true);
        assert_ne!(a.stroke.color, b.stroke.color);
    }

    /// The wash is the band's and survives every border state, so the LIGHTS half stays visually
    /// distinct from the VIDEO half whatever the channel is doing.
    #[test]
    fn the_band_wash_survives_every_border_state() {
        for (dragging, hovered, selected) in [
            (false, false, false),
            (false, false, true),
            (true, false, false),
            (true, true, false),
        ] {
            let frame = lighting_lane_frame(0, dragging, hovered, selected);
            assert_ne!(
                frame.fill,
                egui::Color32::TRANSPARENT,
                "lighting lane lost its band wash at ({dragging}, {hovered}, {selected})"
            );
        }
    }

    /// A deck's parameters come from what the rig can actually do, as declared by the fixtures'
    /// Open Fixture Library profiles — not from a target, because a deck names none.
    #[test]
    fn deck_parameters_come_from_the_rigs_capabilities() {
        let mut data = UIData::test_fixture();
        data.lighting.fixtures.push(fixture_view(
            "f1",
            "par",
            &["dimmer", "red", "green", "blue"],
        ));
        let params = deck_params(&crate::dmx::Look::new("Wash"), &data);
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["dimmer", "red", "green", "blue"], "{names:?}");
    }

    /// Role declaration order, not alphabetical: an operator reads Dimmer first and the color
    /// roles together, the way they are printed on the fixture.
    #[test]
    fn parameters_are_ordered_the_way_a_fixture_is_read() {
        let mut data = UIData::test_fixture();
        data.lighting.fixtures.push(fixture_view(
            "f1",
            "mover",
            &["tilt", "red", "dimmer", "pan"],
        ));
        let params = deck_params(&crate::dmx::Look::new("Wash"), &data);
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["dimmer", "red", "pan", "tilt"], "{names:?}");
    }

    /// Parameters carry the role's own group, so lighting is sectioned in the inspector the way
    /// shader parameters are rather than arriving as one flat list.
    #[test]
    fn parameters_are_sectioned_by_role_group() {
        let mut data = UIData::test_fixture();
        data.lighting.fixtures.push(fixture_view(
            "f1",
            "mover",
            &["dimmer", "red", "pan", "zoom"],
        ));
        let params = deck_params(&crate::dmx::Look::new("Wash"), &data);
        let group = |name: &str| {
            params
                .iter()
                .find(|p| p.name == name)
                .and_then(|p| p.group.clone())
        };
        assert_eq!(group("dimmer").as_deref(), Some("Intensity"));
        assert_eq!(group("red").as_deref(), Some("Color"));
        assert_eq!(group("pan").as_deref(), Some("Position"));
        assert_eq!(group("zoom").as_deref(), Some("Beam"));
    }

    /// The union of what the rig can do, so a mixed rig offers every role some fixture carries.
    /// Fixtures that cannot do one simply ignore it.
    #[test]
    fn parameters_are_the_union_of_a_mixed_rig() {
        let mut data = UIData::test_fixture();
        data.lighting
            .fixtures
            .push(fixture_view("f1", "par", &["dimmer", "red"]));
        data.lighting
            .fixtures
            .push(fixture_view("f2", "mover", &["dimmer", "pan", "tilt"]));
        let params = deck_params(&crate::dmx::Look::new("Wash"), &data);
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["dimmer", "red", "pan", "tilt"], "{names:?}");
    }

    /// Every fixture gets a distinct place on the plot, or two lights would share one hit box
    /// and clicking either would toggle the same one.
    #[test]
    fn every_fixture_gets_its_own_place_on_the_stage() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(260.0, 160.0));
        for count in 1..=24_usize {
            let slots: Vec<egui::Rect> = (0..count).map(|i| stage_slot(rect, i, count)).collect();
            for (i, a) in slots.iter().enumerate() {
                assert!(
                    a.width() > 0.0 && a.height() > 0.0,
                    "empty slot at {i}/{count}"
                );
                for b in slots.iter().skip(i + 1) {
                    assert!(
                        !a.intersects(*b),
                        "slots overlap at count {count}: {a:?} vs {b:?}"
                    );
                }
            }
        }
    }

    /// A palette's swatch is read from the values it holds, not guessed from the roles it
    /// covers. A palette you cannot see the color of is indistinguishable from one that does
    /// nothing, which is how the first version read.
    #[test]
    fn a_palette_shows_the_color_it_holds() {
        let p = palette_view("warm", &[("red", 1.0), ("green", 0.5), ("blue", 0.0)]);
        assert_eq!(palette_swatch(&p), egui::Color32::from_rgb(255, 127, 0));
    }

    /// A color palette built from white or amber emitters alone is not an empty palette.
    #[test]
    fn a_white_only_palette_is_not_read_as_black() {
        let p = palette_view("house", &[("white", 1.0)]);
        assert_eq!(palette_swatch(&p), egui::Color32::from_rgb(255, 255, 255));
    }

    fn fixture_view(id: &str, name: &str, roles: &[&str]) -> crate::dmx::FixtureView {
        crate::dmx::FixtureView {
            id: id.to_string(),
            name: name.to_string(),
            vendor: "generic".into(),
            model: "test".into(),
            mode: "4ch".into(),
            universe: 1,
            address: 1,
            channel_count: roles.len(),
            roles: roles.iter().map(|r| (*r).to_string()).collect(),
            invert_pan: false,
            invert_tilt: false,
            swap_pan_tilt: false,
            position: None,
        }
    }

    fn palette_view(name: &str, values: &[(&str, f32)]) -> crate::dmx::PaletteView {
        crate::dmx::PaletteView {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            kind: "color".into(),
            venue: false,
            default_roles: values.iter().map(|(r, _)| (*r).to_string()).collect(),
            default_values: values.iter().map(|(_, v)| *v).collect(),
            override_count: 0,
        }
    }

    /// A deck names no target and does not define the rig. It reports which groups hear it, the
    /// way a video deck has no idea which surfaces show it.
    /// See /spec/lighting-routing.md § A group is the lighting surface.
    #[test]
    fn the_deck_detail_names_no_target_and_does_not_patch() {
        let mut data = UIData::test_fixture();
        let look = crate::dmx::Look::new("Wash");
        let deck = crate::dmx::LightingDeck::new(look.clone());
        let uuid = deck.id.to_string();
        data.lighting_show.looks.push(look);
        data.lighting_show.decks_mut("ch-1").push(deck);

        let mut actions = UIActions::new();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            render_lighting_deck_detail(ui, &uuid, &data, &mut actions);
        });
        harness.run();
        // A deck names nobody; it reports who has pointed themselves at its channel.
        assert!(
            harness.query_by_label("Heard by").is_some(),
            "the deck must report which groups hear it"
        );
        for banned in ["➕ Add light", "💡 Lights", "Groups"] {
            assert!(
                harness.query_by_label(banned).is_none(),
                "the rig is Stage state and must not be editable from a deck: {banned}"
            );
        }
    }

    #[test]
    fn the_band_is_always_offered() {
        let ctx = egui::Context::default();
        let mut data = UIData::test_fixture();
        assert!(
            lights_band_offered(&ctx, &data),
            "an empty scene still has somewhere to put a lighting deck"
        );

        let look = crate::dmx::Look::new("wash");
        data.lighting_show.looks.push(look);
        data.lighting_show
            .decks_mut("abc12345")
            .push(crate::dmx::LightingDeck::new(crate::dmx::Look::new("Wash")));
        assert!(lights_band_offered(&ctx, &data));
    }

    #[test]
    fn the_band_is_offered_once_fixtures_are_patched() {
        let ctx = egui::Context::default();
        let mut data = UIData::test_fixture();
        data.lighting.fixtures.push(crate::dmx::FixtureView {
            id: "f1".into(),
            name: "par-1".into(),
            vendor: "Generic".into(),
            model: "RGBW".into(),
            mode: "4ch".into(),
            universe: 1,
            address: 1,
            channel_count: 4,
            roles: vec!["red".into()],
            invert_pan: false,
            invert_tilt: false,
            swap_pan_tilt: false,
            position: None,
        });
        assert!(lights_band_offered(&ctx, &data));
    }

    #[test]
    fn an_offered_but_closed_band_still_leaves_video_almost_all_the_height() {
        let ctx = egui::Context::default();
        let mut data = UIData::test_fixture();
        data.lighting.enabled = true;
        data.lights_band_open = false;
        let (video, lights) = band_heights(&ctx, &data, 600.0);
        assert!(video > 570.0, "video keeps the height: {video}");
        assert!(lights <= COLLAPSED_BAND_HEIGHT);
    }

    /// Even with the band deliberately collapsed, dragging a lighting deck opens it: there has
    /// to be somewhere to aim, and expanding it mid-drag is not something a performer can do.
    #[test]
    fn a_lighting_drag_opens_a_collapsed_band() {
        let ctx = egui::Context::default();
        let mut data = UIData::test_fixture();
        data.lights_band_open = false;
        assert!(!lights_band_expanded(&ctx, &data));

        egui::DragAndDrop::set_payload(&ctx, crate::usecases::ui::LibraryDrag::LightingDeck);
        assert!(
            lights_band_expanded(&ctx, &data),
            "a lighting drag must open the zone it needs to land in"
        );
    }

    /// Dragging a shader must not reopen a band the performer collapsed.
    #[test]
    fn a_visual_drag_does_not_open_a_collapsed_band() {
        let ctx = egui::Context::default();
        let mut data = UIData::test_fixture();
        data.lights_band_open = false;
        egui::DragAndDrop::set_payload(&ctx, crate::usecases::ui::LibraryDrag::Generator(0));
        assert!(!lights_band_expanded(&ctx, &data));
    }

    #[test]
    fn the_deck_detail_survives_a_missing_look() {
        let mut data = UIData::test_fixture();
        let deck = crate::dmx::LightingDeck::new(crate::dmx::Look::new("l"));
        let uuid = deck.id.to_string();
        data.lighting_show.decks_mut("abc12345").push(deck);
        let mut actions = UIActions::new();
        let _h = egui_kittest::Harness::new_ui(|ui| {
            render_lighting_deck_detail(ui, &uuid, &data, &mut actions);
        });
    }

    #[test]
    fn the_deck_detail_reports_a_deck_that_is_gone() {
        let data = UIData::test_fixture();
        let mut actions = UIActions::new();
        let missing = uuid::Uuid::new_v4().to_string();
        {
            let _h = egui_kittest::Harness::new_ui(|ui| {
                render_lighting_deck_detail(ui, &missing, &data, &mut actions);
            });
        }
        assert!(actions.commands.is_empty(), "no commands for a dead deck");
    }

    fn data_with_rig(enabled: bool, band_open: bool) -> UIData {
        let mut data = UIData::test_fixture();
        data.lighting.enabled = enabled;
        data.lights_band_open = band_open;
        data
    }

    /// The hard requirement: with LIGHTS collapsed the central area is exactly what it was
    /// before lighting existed.
    #[test]
    fn a_collapsed_lights_band_gives_its_height_to_video() {
        let data = data_with_rig(true, false);
        let ctx = egui::Context::default();
        let (video, lights) = band_heights(&ctx, &data, 600.0);
        assert!((video - (600.0 - COLLAPSED_BAND_HEIGHT)).abs() < f32::EPSILON);
        assert!((lights - COLLAPSED_BAND_HEIGHT).abs() < f32::EPSILON);
    }

    /// Collapsing is the performer's choice, not something the scene decides for them.
    #[test]
    fn the_band_collapses_only_when_the_performer_collapses_it() {
        let ctx = egui::Context::default();
        let mut data = data_with_rig(false, false);
        assert!(
            !lights_band_expanded(&ctx, &data),
            "closed stays closed with no drag in flight"
        );
        data.lights_band_open = true;
        assert!(lights_band_expanded(&ctx, &data));
    }

    #[test]
    fn both_bands_open_split_evenly_by_default() {
        let data = data_with_rig(true, true);
        let ctx = egui::Context::default();
        let (video, lights) = band_heights(&ctx, &data, 600.0);
        assert!((video - 300.0).abs() < 1.0, "video {video}");
        assert!((lights - 300.0).abs() < 1.0, "lights {lights}");
    }

    #[test]
    fn the_split_is_clamped_so_neither_band_vanishes() {
        let mut data = data_with_rig(true, true);
        let ctx = egui::Context::default();
        data.band_split = 0.0;
        let (video, lights) = band_heights(&ctx, &data, 600.0);
        assert!(video >= COLLAPSED_BAND_HEIGHT, "video {video}");
        assert!(lights > 0.0);
        data.band_split = 1.0;
        let (video, lights) = band_heights(&ctx, &data, 600.0);
        assert!(lights >= COLLAPSED_BAND_HEIGHT, "lights {lights}");
        assert!(video > 0.0);
    }

    #[test]
    fn a_lighting_only_layout_collapses_video() {
        let mut data = data_with_rig(true, true);
        let ctx = egui::Context::default();
        data.video_band_open = false;
        let (video, lights) = band_heights(&ctx, &data, 600.0);
        assert!((video - COLLAPSED_BAND_HEIGHT).abs() < f32::EPSILON);
        assert!(lights > video);
    }

    #[test]
    fn both_heights_always_sum_to_the_available_space() {
        let ctx = egui::Context::default();
        for split in [0.2_f32, 0.5, 0.8] {
            let mut data = data_with_rig(true, true);
            data.band_split = split;
            let (video, lights) = band_heights(&ctx, &data, 600.0);
            assert!(
                (video + lights - 600.0).abs() < 1.0,
                "split {split} gave {video} + {lights}"
            );
        }
    }
    use super::*;

    #[test]
    fn next_name_increments_a_trailing_number() {
        assert_eq!(next_name("par-1"), "par-2");
        assert_eq!(next_name("par-9"), "par-10");
        assert_eq!(next_name("wash 07"), "wash 08", "zero padding is preserved");
        assert_eq!(
            next_name("par-09"),
            "par-10",
            "padding gives way when it must"
        );
    }

    #[test]
    fn next_name_leaves_an_unnumbered_name_alone() {
        assert_eq!(next_name("key light"), "key light");
        assert_eq!(next_name(""), "");
    }

    #[test]
    fn health_color_distinguishes_stopped_from_unhealthy() {
        let stopped = crate::dmx::LightingSnapshot::default();
        let mut unhealthy = crate::dmx::LightingSnapshot {
            running: true,
            healthy: false,
            ..Default::default()
        };
        let healthy = crate::dmx::LightingSnapshot {
            running: true,
            healthy: true,
            ..Default::default()
        };
        assert_ne!(health_color(&stopped), health_color(&unhealthy));
        assert_ne!(health_color(&unhealthy), health_color(&healthy));
        unhealthy.running = false;
        assert_eq!(health_color(&unhealthy), health_color(&stopped));
    }

    #[test]
    fn the_default_draft_is_patchable_shaped() {
        let d = AddFixtureDraft::default();
        assert_eq!(d.universe, 1);
        assert_eq!(d.address, 1);
        assert!(d.profile.is_empty(), "profile must be chosen explicitly");
    }
}
