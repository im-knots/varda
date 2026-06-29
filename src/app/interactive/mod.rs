//! Interactive mode for HTML decks (feature `html`).
//!
//! A separate Varda-owned window displays a selected HTML deck's live page and
//! forwards mouse/keyboard/scroll/IME input into the offscreen Servo `WebView`.
//! See `/spec/html-source.md` §4. Only one interactive window exists at a time;
//! it is fixed 1:1 to the deck's WebView size (non-resizable).

pub(crate) mod input;
mod window;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::deck::ExternalSourceKind;
use crate::engine::{CommandResult, ErrorCode};
use crate::html::HtmlInputEvent;

/// Resolved address + size of the HTML instance an interactive window drives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InteractiveTarget {
    pub channel_idx: usize,
    pub deck_idx: usize,
    pub html_idx: usize,
    pub width: u32,
    pub height: u32,
}

/// Engine-side state for the single interactive HTML window (one at a time).
#[derive(Default)]
pub(crate) struct InteractiveHtmlState {
    /// Resolved open request, drained in the render loop (needs the event loop).
    pending_open: Option<InteractiveTarget>,
    /// Request to close the current window (set by command or `CloseRequested`).
    pending_close: bool,
    /// The live interactive window, if any.
    window: Option<window::InteractiveWindow>,
}

impl super::VardaApp {
    /// Open (or re-target) the interactive window for the HTML deck at
    /// `(channel_idx, deck_idx)`. Window creation is deferred to the render loop.
    pub(crate) fn cmd_open_html_interactive(
        &mut self,
        channel_idx: usize,
        deck_idx: usize,
    ) -> CommandResult {
        let kind = self
            .mixer
            .channels()
            .get(channel_idx)
            .and_then(|ch| ch.decks.get(deck_idx))
            .map(|slot| slot.deck.external_source_kind());
        match kind {
            Some(Some(ExternalSourceKind::Html(html_idx))) => {
                // Already showing this deck → no-op.
                if let Some(win) = &self.interactive.window {
                    if win.target.channel_idx == channel_idx && win.target.deck_idx == deck_idx {
                        return CommandResult::Ok;
                    }
                }
                let (width, height) = self
                    .external_io
                    .html_manager
                    .instance_dimensions(html_idx)
                    .unwrap_or((1920, 1080));
                // One at a time: close any existing window, then open the new one.
                self.interactive.pending_close = true;
                self.interactive.pending_open = Some(InteractiveTarget {
                    channel_idx,
                    deck_idx,
                    html_idx,
                    width,
                    height,
                });
                CommandResult::Ok
            }
            Some(_) => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "Deck is not an HTML source".into(),
            },
            None => CommandResult::Err {
                code: ErrorCode::NotFound,
                message: "Deck not found".into(),
            },
        }
    }

    /// Close the interactive window (if any). Deferred to the render loop.
    pub(crate) fn cmd_close_html_interactive(&mut self) -> CommandResult {
        self.interactive.pending_open = None;
        self.interactive.pending_close = true;
        CommandResult::Ok
    }

    /// The `(channel_idx, deck_idx)` of the deck the interactive window is bound
    /// to, if one is open. Used to reflect the toggle state in snapshots.
    pub(crate) fn interactive_active_deck(&self) -> Option<(usize, usize)> {
        self.interactive
            .window
            .as_ref()
            .map(|w| (w.target.channel_idx, w.target.deck_idx))
    }

    /// Apply pending interactive open/close requests. Runs in the render loop so
    /// it has the `ActiveEventLoop` needed to create a window. Mirrors
    /// `create_pending_outputs` (`Box::leak` for `'static`; `destroy()` reclaims).
    pub(crate) fn create_pending_interactive(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        if self.interactive.pending_close {
            self.interactive.pending_close = false;
            if let Some(win) = self.interactive.window.take() {
                self.external_io
                    .html_manager
                    .send_input(win.target.html_idx, HtmlInputEvent::Focus(false));
                win.destroy();
            }
        }
        let Some(target) = self.interactive.pending_open.take() else {
            return;
        };
        let attrs = Window::default_attributes()
            .with_title("Varda — Interactive HTML")
            .with_inner_size(PhysicalSize::new(target.width, target.height))
            .with_resizable(false);
        let window = match event_loop.create_window(attrs) {
            Ok(w) => w,
            Err(e) => {
                log::error!("Failed to create interactive window: {e}");
                self.session
                    .notifications
                    .warn(format!("Could not open interactive window: {e}"));
                return;
            }
        };
        let window_static: &'static Window = Box::leak(Box::new(window));
        match window::InteractiveWindow::new(&self.context, window_static, target) {
            Ok(win) => {
                window_static.set_ime_allowed(true);
                self.external_io
                    .html_manager
                    .send_input(target.html_idx, HtmlInputEvent::Focus(true));
                log::info!(
                    "Opened interactive HTML window for deck {}/{} ({}x{})",
                    target.channel_idx,
                    target.deck_idx,
                    target.width,
                    target.height
                );
                self.interactive.window = Some(win);
            }
            Err(e) => {
                log::error!("Failed to init interactive window surface: {e}");
                // Reclaim the leaked window box on failure.
                let ptr = window_static as *const Window as *mut Window;
                unsafe {
                    let _ = Box::from_raw(ptr);
                }
            }
        }
    }

    /// Route a winit window event to the interactive window. Returns `true` if
    /// the event targeted the interactive window (and was consumed), so the
    /// caller skips main/output handling. Mouse/keyboard/scroll/IME are
    /// translated and forwarded to the servo thread; `CloseRequested` closes.
    pub(crate) fn handle_interactive_event(
        &mut self,
        window_id: winit::window::WindowId,
        event: &winit::event::WindowEvent,
    ) -> bool {
        let is_ours = self
            .interactive
            .window
            .as_ref()
            .is_some_and(|w| w.id() == window_id);
        if !is_ours {
            return false;
        }
        if matches!(event, winit::event::WindowEvent::CloseRequested) {
            self.interactive.pending_close = true;
            return true;
        }
        let html_idx = self.interactive.window.as_ref().unwrap().target.html_idx;
        let events = self
            .interactive
            .window
            .as_mut()
            .unwrap()
            .process_event(event);
        for ev in events {
            self.external_io.html_manager.send_input(html_idx, ev);
        }
        true
    }

    /// Blit the current HTML texture into the interactive window (per frame).
    /// Call after `html_manager.update()` so the texture is fresh.
    pub(crate) fn render_interactive(&self) {
        if let Some(win) = &self.interactive.window {
            if let Some(view) = self
                .external_io
                .html_manager
                .texture_view(win.target.html_idx)
            {
                win.render(&self.context, view);
            }
        }
    }
}
