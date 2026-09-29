//! Interactive mode for HTML decks (feature `html`): a window showing one HTML
//! deck's live page and forwarding input to its `WebView`. At most one exists.

pub(crate) mod input;
mod window;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::engine::{CommandResult, ErrorCode};
use crate::html::HtmlInputEvent;

/// Resolved address + size of the HTML instance an interactive window drives.
#[derive(Debug, Clone)]
pub(crate) struct InteractiveTarget {
    pub deck_uuid: String,
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
    window: Option<window::InteractiveWindow>,
}

impl super::VardaApp {
    /// Open (or re-target) the interactive window for the HTML deck identified by
    /// `deck_uuid`. Window creation is deferred to the render loop.
    pub(crate) fn cmd_open_html_interactive(&mut self, deck_uuid: &str) -> CommandResult {
        let (channel_idx, deck_idx) = match self.mixer.resolve_deck(deck_uuid) {
            Ok(loc) => loc,
            Err(e) => return e.into(),
        };
        let html_idx = self
            .mixer
            .channels()
            .get(channel_idx)
            .and_then(|ch| ch.decks.get(deck_idx))
            .and_then(|slot| crate::html::provider::html_instance(slot.deck.source()));
        let Some(html_idx) = html_idx else {
            return CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "Deck is not an HTML source".into(),
            };
        };
        // Already showing this deck.
        if let Some(win) = &self.interactive.window
            && win.target.deck_uuid == deck_uuid
        {
            return CommandResult::Ok;
        }
        let (width, height) = self
            .sources
            .service::<crate::html::HtmlManager>()
            .instance_dimensions(html_idx)
            .unwrap_or((1920, 1080));
        // Close any existing window first.
        self.interactive.pending_close = true;
        self.interactive.pending_open = Some(InteractiveTarget {
            deck_uuid: deck_uuid.to_string(),
            html_idx,
            width,
            height,
        });
        CommandResult::Ok
    }

    /// Apply `interactive` action toggles from HTML decks since the last frame:
    /// open the window on that deck, or close it if already open there.
    pub(crate) fn poll_interactive_requests(&mut self) {
        let mut requested = Vec::new();
        for channel in self.mixer.channels_mut() {
            for slot in &mut channel.decks {
                if crate::html::provider::take_interactive_request(slot.deck.source_mut()).is_some()
                {
                    requested.push(slot.deck.uuid().to_string());
                }
            }
        }
        for deck_uuid in requested {
            if self.interactive_active_deck() == Some(deck_uuid.as_str()) {
                self.cmd_close_html_interactive();
            } else {
                self.cmd_open_html_interactive(&deck_uuid);
            }
        }
    }

    /// Close the interactive window, deferred to the render loop.
    pub(crate) fn cmd_close_html_interactive(&mut self) -> CommandResult {
        self.interactive.pending_open = None;
        self.interactive.pending_close = true;
        CommandResult::Ok
    }

    /// UUID of the deck the interactive window shows, if open.
    pub(crate) fn interactive_active_deck(&self) -> Option<&str> {
        self.interactive
            .window
            .as_ref()
            .map(|w| w.target.deck_uuid.as_str())
    }

    /// Apply pending open/close requests. Runs in the render loop, which has the
    /// `ActiveEventLoop`. The window is `Box::leak`ed; `destroy()` reclaims it.
    pub(crate) fn create_pending_interactive(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        if self.interactive.pending_close {
            self.interactive.pending_close = false;
            if let Some(win) = self.interactive.window.take() {
                self.sources
                    .service::<crate::html::HtmlManager>()
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
        let (deck_uuid, html_idx, width, height) = (
            target.deck_uuid.clone(),
            target.html_idx,
            target.width,
            target.height,
        );
        match window::InteractiveWindow::new(&self.render.context, window_static, target) {
            Ok(win) => {
                window_static.set_ime_allowed(true);
                self.sources
                    .service::<crate::html::HtmlManager>()
                    .send_input(html_idx, HtmlInputEvent::Focus(true));
                log::info!(
                    "Opened interactive HTML window for deck {deck_uuid} ({width}x{height})"
                );
                self.interactive.window = Some(win);
            }
            Err(e) => {
                log::error!("Failed to init interactive window surface: {e}");
                // Reclaim the leaked window box on failure.
                let ptr = std::ptr::from_ref::<Window>(window_static).cast_mut();
                unsafe {
                    let _ = Box::from_raw(ptr);
                }
            }
        }
    }

    /// Route a winit event to the interactive window. Returns `true` if the
    /// event was for that window and was consumed.
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
            self.sources
                .service::<crate::html::HtmlManager>()
                .send_input(html_idx, ev);
        }
        true
    }

    /// Blit the HTML texture into the interactive window. Call after the HTML
    /// provider's frame tick.
    pub(crate) fn render_interactive(&self) {
        if let Some(win) = &self.interactive.window
            && let Some(view) = self
                .sources
                .service::<crate::html::HtmlManager>()
                .texture_view(win.target.html_idx)
        {
            win.render(&self.render.context, view);
        }
    }
}
