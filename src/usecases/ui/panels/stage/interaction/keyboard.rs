//! Data-driven keyboard shortcuts for the stage editor, resolved through the
//! user's keymap rather than hard-coded bindings.

use super::super::super::super::{UIActions, UIData};
use super::super::state::{DrawingTool, StageEditorState};
use crate::engine::EngineCommand;

pub(super) fn handle(
    ui: &egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    state: &mut StageEditorState,
) {
    use crate::keymap::{collect_pressed_keys, ActionId, KeyCombo, KeyTarget};
    let pressed = collect_pressed_keys(ui.ctx());
    for (key, mods) in &pressed {
        let combo = KeyCombo::from_egui(*key, mods);
        if let Some(target) = data.keymap_bindings.get(&combo) {
            match target {
                KeyTarget::Action(ActionId::ToolSelect) => state.tool = DrawingTool::Select,
                KeyTarget::Action(ActionId::ToolRectangle) => {
                    state.tool = DrawingTool::Rectangle;
                }
                KeyTarget::Action(ActionId::ToolPolygon) => state.tool = DrawingTool::Polygon,
                KeyTarget::Action(ActionId::ToolCircle) => state.tool = DrawingTool::Circle,
                KeyTarget::Action(ActionId::ClearDrawing) => {
                    state.polygon_verts.clear();
                    state.rect_start = None;
                    state.circle_center = None;
                }
                KeyTarget::Action(ActionId::DeleteSurface) => {
                    if !state.selected_surfaces.is_empty() {
                        let uuids: Vec<String> = state.selected_surfaces.iter().cloned().collect();
                        for uuid in uuids {
                            actions.commands.push(EngineCommand::RemoveSurface { uuid });
                        }
                        state.selected_surfaces.clear();
                    }
                }
                KeyTarget::Action(ActionId::DuplicateSurface) => {
                    for uuid in &state.selected_surfaces {
                        actions
                            .commands
                            .push(EngineCommand::DuplicateSurface { uuid: uuid.clone() });
                    }
                }
                KeyTarget::Action(ActionId::FlipHorizontal) => {
                    for uuid in &state.selected_surfaces {
                        actions
                            .commands
                            .push(EngineCommand::FlipSurfaceHorizontal { uuid: uuid.clone() });
                    }
                }
                KeyTarget::Action(ActionId::FlipVertical) => {
                    for uuid in &state.selected_surfaces {
                        actions
                            .commands
                            .push(EngineCommand::FlipSurfaceVertical { uuid: uuid.clone() });
                    }
                }
                KeyTarget::Action(ActionId::CombineSurfaces)
                    if state.selected_surfaces.len() >= 2 =>
                {
                    let uuids: Vec<String> = state.selected_surfaces.iter().cloned().collect();
                    actions
                        .commands
                        .push(EngineCommand::CombineSurfaces { uuids });
                    state.selected_surfaces.clear();
                }
                _ => {}
            }
        }
    }
}
