//! Stage editor input handling, one module per drawing tool.
//!
//! Split to mirror the `DrawingTool` variants exactly, so a gesture bug has one
//! obvious home. Every handler takes the frame's [`CanvasGeometry`] rather than
//! recomputing the screen ↔ normalized mapping, and mutates only
//! [`StageEditorState`] plus the outgoing [`UIActions`].

mod bezier;
mod draw;
mod keyboard;
mod select;

use super::super::super::{UIActions, UIData};
use super::hit_test::CanvasGeometry;
use super::state::{DrawingTool, StageEditorState};

/// Dispatch one frame of canvas input to the active tool.
///
/// The `match` stays exhaustive over `DrawingTool`, so adding a tool fails to
/// compile until it is handled here.
pub(super) fn handle_canvas(
    ui: &egui::Ui,
    painter: &egui::Painter,
    resp: &egui::Response,
    data: &UIData,
    actions: &mut UIActions,
    state: &mut StageEditorState,
    geom: CanvasGeometry,
) {
    match state.tool {
        DrawingTool::Select => select::handle(ui, painter, resp, data, actions, state, geom),
        DrawingTool::Rectangle => draw::rectangle(resp, data, actions, state, geom),
        DrawingTool::Polygon => draw::polygon(resp, data, actions, state, geom),
        DrawingTool::Circle => draw::circle(resp, data, actions, state, geom),
        DrawingTool::Bezier => bezier::handle(ui, resp, data, actions, state, geom),
    }
}

/// Apply keymap-driven shortcuts. Skipped while the user is binding a key.
pub(super) fn handle_keyboard(
    ui: &egui::Ui,
    data: &UIData,
    actions: &mut UIActions,
    state: &mut StageEditorState,
) {
    if data.keyboard_learn_active {
        return;
    }
    keyboard::handle(ui, data, actions, state);
}
