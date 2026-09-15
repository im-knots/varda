//! The lighting half of the stage: fixtures drawn, placed and selected on the same canvas as
//! the surfaces.
//!
//! A fixture is the same kind of inhabitant as a surface — a physical thing standing in the room
//! that receives a signal — so it is edited in the same place, in the same normalized
//! coordinates. Which lamps exist and where they stand is Stage state, not deck content: a deck
//! defining the rig is a video deck defining your projector setup.
//! See /spec/lighting-routing.md § The Stage holds both kinds of physical thing.

use super::super::super::{UIActions, UIData};
use super::hit_test::CanvasGeometry;
use crate::engine::EngineCommand;

/// Radius of a lamp on the canvas, in pixels.
///
/// Fixed rather than scaled with the canvas: a lamp is a point in the room, and drawing it larger
/// on a bigger canvas would imply it covers more of the stage.
const LAMP_RADIUS: f32 = 9.0;

/// Where a fixture sits on the canvas, in normalized stage coordinates.
///
/// An unplaced fixture falls back to a grid along the top of the stage, so a freshly patched rig
/// is visible and draggable rather than invisible until someone thinks to place it. Nobody is
/// made to lay out a plot before they can use a light.
#[must_use]
pub(super) fn fixture_point(
    fixture: &crate::dmx::FixtureView,
    index: usize,
    count: usize,
) -> [f32; 2] {
    fixture.position.unwrap_or_else(|| {
        #[allow(clippy::cast_precision_loss)]
        let step = 1.0 / (count.max(1) as f32 + 1.0);
        #[allow(clippy::cast_precision_loss)]
        let x = step * (index as f32 + 1.0);
        [x, 0.08]
    })
}

/// Draw every fixture, lit in the color it is currently emitting.
pub(super) fn paint(
    painter: &egui::Painter,
    data: &UIData,
    geom: CanvasGeometry,
    selected: &std::collections::BTreeSet<String>,
) {
    let fixtures = &data.lighting.fixtures;
    for (i, fixture) in fixtures.iter().enumerate() {
        let [nx, ny] = fixture_point(fixture, i, fixtures.len());
        let center = geom.to_screen([nx, ny]);
        let is_selected = selected.contains(&fixture.id);

        // An unlit lamp draws over a dim body, or a rig sitting at zero would be an empty
        // canvas: the plot has to show where the lights *are* before what they are doing.
        painter.circle_filled(center, LAMP_RADIUS, egui::Color32::from_rgb(38, 36, 42));
        painter.circle_filled(
            center,
            LAMP_RADIUS,
            super::super::lighting::fixture_color(fixture, data),
        );
        painter.circle_stroke(
            center,
            LAMP_RADIUS + 1.5,
            if is_selected {
                egui::Stroke::new(2.0, super::super::lighting::lighting_accent())
            } else {
                egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 66, 76))
            },
        );
        // Unplaced lamps are marked, so "it is sitting in the default row" is legible as a fact
        // rather than looking like a deliberate placement.
        if fixture.position.is_none() {
            painter.circle_stroke(
                center,
                LAMP_RADIUS + 4.0,
                egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 84, 60)),
            );
        }
        painter.text(
            egui::pos2(center.x, center.y + LAMP_RADIUS + 3.0),
            egui::Align2::CENTER_TOP,
            super::super::utils::truncate_chars(&fixture.name, 10),
            egui::FontId::proportional(9.0),
            egui::Color32::from_rgb(150, 145, 155),
        );
    }
}

/// The fixture under a point, if any.
#[must_use]
pub(super) fn hit(data: &UIData, geom: CanvasGeometry, pos: egui::Pos2) -> Option<String> {
    let fixtures = &data.lighting.fixtures;
    // Reverse order, so the lamp drawn last — visually on top — is the one picked.
    fixtures.iter().enumerate().rev().find_map(|(i, fixture)| {
        let center = geom.to_screen(fixture_point(fixture, i, fixtures.len()));
        ((center - pos).length() <= LAMP_RADIUS + 3.0).then(|| fixture.id.clone())
    })
}

/// Select and place lamps. Returns true when the gesture was a lamp's, so the caller leaves
/// surface handling alone.
///
/// Lamps are hit-tested *before* surfaces: a lamp is a small target drawn on top of a large one,
/// and a surface would otherwise swallow every click meant for a light standing on it.
pub(super) fn handle(
    ui: &egui::Ui,
    resp: &egui::Response,
    data: &UIData,
    actions: &mut UIActions,
    dragging: &mut Option<String>,
    selected: &mut std::collections::BTreeSet<String>,
    geom: CanvasGeometry,
) -> bool {
    // Shift extends, matching surface selection. Grouping is a multi-lamp act, so this is the
    // common case rather than a power-user one.
    let shift_held = ui.input(|i| i.modifiers.shift);

    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(id) = hit(data, geom, pos)
    {
        if !shift_held && !selected.contains(&id) {
            selected.clear();
        }
        selected.insert(id.clone());
        *dragging = Some(id);
    }

    if let Some(id) = dragging.clone() {
        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            actions.commands.push(EngineCommand::SetFixturePosition {
                uuid: id,
                position: Some(geom.to_norm(pos)),
            });
        }
        if resp.drag_stopped() {
            *dragging = None;
        }
        return true;
    }

    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(id) = hit(data, geom, pos)
    {
        if shift_held {
            if !selected.remove(&id) {
                selected.insert(id);
            }
        } else {
            selected.clear();
            selected.insert(id);
        }
        return true;
    }
    false
}

/// The stage's lighting panel: which lamps to place, and the groups they form.
///
/// Deliberately **not** a patch editor. A lamp is a physical device that emits light onto the
/// stage, which makes it an *output* — the same kind of thing as a projector — so what profile it
/// is, what address it answers on and which universe carries it live with the other outputs. What
/// belongs here is what belongs to the stage: where each lamp stands, and which lamps form a
/// group. A projector is configured in Outputs and *placed* on the stage; a lamp is the same.
/// See /spec/lighting-routing.md § The Stage holds both kinds of physical thing.
pub(super) fn render_panel(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    selected: &mut std::collections::BTreeSet<String>,
) {
    egui::Frame::default()
        .inner_margin(6.0)
        .corner_radius(4.0)
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.label(egui::RichText::new("💡 Lights").strong());
                ui.label(
                    egui::RichText::new(if data.lighting.fixtures.is_empty() {
                        "no lights yet — add them under DMX Output"
                    } else {
                        "drag a lamp on the canvas to place it"
                    })
                    .small()
                    .color(egui::Color32::from_rgb(120, 120, 130)),
                );
                ui.separator();

                let list_height = (ui.available_height() - 160.0).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt("stage_lights_scroll")
                    .max_height(list_height)
                    .show(ui, |ui| {
                        render_lamp_list(ui, data, selected);
                    });

                ui.add_space(4.0);
                ui.separator();
                render_groups(ui, data, actions, selected);
            });
        });
}

/// The lamps, as things to select and place. Names only — the patch detail is output config.
fn render_lamp_list(
    ui: &mut egui::Ui,
    data: &UIData,
    selected: &mut std::collections::BTreeSet<String>,
) {
    for fixture in &data.lighting.fixtures {
        ui.horizontal(|ui| {
            // Selecting a row selects the lamp on the canvas, so the list and the plot are two
            // views of one selection rather than two independent ones.
            let is_selected = selected.contains(&fixture.id);
            if ui
                .selectable_label(
                    is_selected,
                    egui::RichText::new(fixture.name.as_str()).small(),
                )
                .clicked()
            {
                if is_selected {
                    selected.remove(&fixture.id);
                } else {
                    selected.insert(fixture.id.clone());
                }
            }
            // Unplaced lamps sit in a default row on the canvas. Saying so here means "they are
            // all in a line at the top" reads as a fact rather than as someone's layout.
            if fixture.position.is_none() {
                ui.label(
                    egui::RichText::new("unplaced")
                        .small()
                        .color(egui::Color32::from_rgb(150, 140, 100)),
                )
                .on_hover_text("drag it on the canvas to place it");
            }
        });
    }
}

/// Groups: the mapping from what a deck addresses to the lamps that answer.
///
/// This is the surface-mapping step for lighting. A deck says "front truss at 60%"; a group says
/// which physical lamps the front truss is *tonight*. Defining membership therefore belongs to
/// the Stage, beside the lamps themselves — it is the fact that changes when the show travels.
/// See /spec/lighting-routing.md § A group is the lighting surface.
fn render_groups(
    ui: &mut egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    selected: &std::collections::BTreeSet<String>,
) {
    ui.label(egui::RichText::new("Groups").small().strong());

    let id = ui.id().with("stage_group_name");
    let mut name: String = ui.ctx().data_mut(|d| d.get_temp(id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut name)
                .hint_text("name selected lights")
                .desired_width(130.0),
        );
        if ui
            .add_enabled(
                !name.trim().is_empty() && !selected.is_empty(),
                egui::Button::new("+").small(),
            )
            .on_disabled_hover_text("select some lamps on the canvas, then name them")
            .clicked()
        {
            actions.commands.push(EngineCommand::AddLightingGroup {
                name: name.trim().to_string(),
                fixtures: selected.iter().cloned().collect(),
            });
            name.clear();
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, name));

    if data.lighting_group_names.is_empty() {
        ui.label(
            egui::RichText::new("No groups yet. A deck drives groups, so make at least one.")
                .small()
                .color(egui::Color32::from_rgb(120, 120, 130)),
        );
        return;
    }

    for (uuid, gname, members) in &data.lighting_group_names {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("◈ {gname}")).small());
            ui.label(
                egui::RichText::new(format!("({members})"))
                    .small()
                    .color(egui::Color32::from_rgb(130, 130, 140)),
            );
            // Re-point a group at the current selection: the one move a touring show makes on
            // arrival, once the lamps in this room are placed.
            if ui
                .add_enabled(!selected.is_empty(), egui::Button::new("set").small())
                .on_hover_text("replace this group's members with the selected lamps")
                .on_disabled_hover_text("select some lamps first")
                .clicked()
            {
                actions
                    .commands
                    .push(EngineCommand::SetLightingGroupMembers {
                        uuid: uuid.clone(),
                        fixtures: selected.iter().cloned().collect(),
                    });
            }
            if ui.small_button("x").clicked() {
                actions
                    .commands
                    .push(EngineCommand::RemoveLightingGroup { uuid: uuid.clone() });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom() -> CanvasGeometry {
        CanvasGeometry::new(
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 200.0)),
            0.0,
            false,
        )
    }

    fn fixture(id: &str, position: Option<[f32; 2]>) -> crate::dmx::FixtureView {
        crate::dmx::FixtureView {
            id: id.to_string(),
            name: id.to_string(),
            vendor: "generic".into(),
            model: "test".into(),
            mode: "4ch".into(),
            universe: 1,
            address: 1,
            channel_count: 4,
            roles: vec!["dimmer".into()],
            invert_pan: false,
            invert_tilt: false,
            swap_pan_tilt: false,
            position,
        }
    }

    /// A freshly patched rig must be visible and draggable, not invisible until someone thinks
    /// to place it.
    #[test]
    fn unplaced_fixtures_get_distinct_default_places() {
        let seen: Vec<[f32; 2]> = (0..5)
            .map(|i| fixture_point(&fixture("f", None), i, 5))
            .collect();
        for (i, a) in seen.iter().enumerate() {
            assert!(a[0] > 0.0 && a[0] < 1.0, "off canvas at {i}: {a:?}");
            for b in seen.iter().skip(i + 1) {
                assert!(
                    (a[0] - b[0]).abs() > 0.01,
                    "two lamps share a default place"
                );
            }
        }
    }

    #[test]
    fn a_placed_fixture_keeps_its_own_point() {
        let f = fixture("f", Some([0.25, 0.75]));
        assert_eq!(fixture_point(&f, 3, 9), [0.25, 0.75]);
    }

    /// The lamp drawn on top is the one picked, or overlapping lamps would select the one
    /// underneath.
    #[test]
    fn hit_testing_picks_the_lamp_drawn_last() {
        let mut data = UIData::test_fixture();
        data.lighting.fixtures = vec![
            fixture("under", Some([0.5, 0.5])),
            fixture("over", Some([0.5, 0.5])),
        ];
        let at = geom().to_screen([0.5, 0.5]);
        assert_eq!(hit(&data, geom(), at).as_deref(), Some("over"));
    }

    #[test]
    fn empty_space_hits_no_lamp() {
        let mut data = UIData::test_fixture();
        data.lighting.fixtures = vec![fixture("a", Some([0.1, 0.1]))];
        let far = geom().to_screen([0.9, 0.9]);
        assert_eq!(hit(&data, geom(), far), None);
    }
}
