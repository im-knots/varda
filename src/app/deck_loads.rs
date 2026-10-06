//! Background deck loading. Slow source builds (shader compile, media decode)
//! run off the render thread through the provider's loader; finished decks are
//! finalized and attached at the start of a later frame. Every consumer uses
//! this path for such decks.

use super::VardaApp;
use crate::deck::Deck;
use crate::engine::types::{DeckLoadSnapshot, DeckLoadStatus};
use crate::renderer::GpuContext;
use crate::source::SourceLoader;
use anyhow::Context;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long a failed load stays listed, so a client that polls state can see
/// why its deck never appeared.
const FAILED_LOAD_RETENTION: Duration = Duration::from_secs(60);

/// A load that has been requested and not yet attached or failed.
struct Load {
    uuid: String,
    channel_uuid: String,
    name: String,
}

struct Finished {
    uuid: String,
    deck: anyhow::Result<Deck>,
}

struct Failure {
    load: Load,
    message: String,
    at: Instant,
}

/// Tracks background deck loads from request to attach.
pub(crate) struct DeckLoader {
    tx: mpsc::Sender<Finished>,
    rx: mpsc::Receiver<Finished>,
    in_flight: Vec<Load>,
    failed: Vec<Failure>,
}

impl DeckLoader {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            in_flight: Vec::new(),
            failed: Vec::new(),
        }
    }

    /// Start building a deck for `channel_uuid`. Returns the deck's future UUID.
    fn spawn(
        &mut self,
        context: &GpuContext,
        channel_uuid: &str,
        name: String,
        loader: SourceLoader,
        width: u32,
        height: u32,
    ) -> String {
        let uuid = uuid::Uuid::new_v4().to_string();
        let tx = self.tx.clone();
        let context = context.clone();
        let deck_uuid = uuid.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("deck-load-{name}"))
            .spawn(move || {
                let deck = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    loader(&context, width, height)
                        .map(|source| Deck::from_source(&context, source, width, height))
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the loader panicked")))
                .map(|mut deck| {
                    deck.set_uuid(deck_uuid.clone());
                    deck
                });
                // Fails only if the engine has shut down.
                let _ = tx.send(Finished {
                    uuid: deck_uuid,
                    deck,
                });
            });
        let load = Load {
            uuid: uuid.clone(),
            channel_uuid: channel_uuid.to_string(),
            name,
        };
        match spawned {
            Ok(_) => self.in_flight.push(load),
            Err(e) => self.record_failure(load, format!("could not start a loader thread: {e}")),
        }
        uuid
    }

    /// Loads that finished since the last call, paired with what they built.
    fn take_finished(&mut self) -> Vec<(Load, anyhow::Result<Deck>)> {
        let mut done = Vec::new();
        while let Ok(finished) = self.rx.try_recv() {
            if let Some(pos) = self.in_flight.iter().position(|l| l.uuid == finished.uuid) {
                done.push((self.in_flight.swap_remove(pos), finished.deck));
            }
        }
        done
    }

    fn record_failure(&mut self, load: Load, message: String) {
        self.failed.push(Failure {
            load,
            message,
            at: Instant::now(),
        });
    }

    fn prune_failures(&mut self, now: Instant) {
        self.failed
            .retain(|f| now.saturating_duration_since(f.at) < FAILED_LOAD_RETENTION);
    }

    /// Loads in flight, then recent failures.
    pub(crate) fn snapshot(&self) -> Vec<DeckLoadSnapshot> {
        let loading = self.in_flight.iter().map(|l| DeckLoadSnapshot {
            uuid: l.uuid.clone(),
            channel_uuid: l.channel_uuid.clone(),
            name: l.name.clone(),
            status: DeckLoadStatus::Loading,
        });
        let failed = self.failed.iter().map(|f| DeckLoadSnapshot {
            uuid: f.load.uuid.clone(),
            channel_uuid: f.load.channel_uuid.clone(),
            name: f.load.name.clone(),
            status: DeckLoadStatus::Failed {
                message: f.message.clone(),
            },
        });
        loading.chain(failed).collect()
    }

    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.len()
    }
}

impl VardaApp {
    /// Start a background load into `channel_uuid` at the current render size.
    /// Returns the deck's future UUID. `name` labels the load until then.
    pub(crate) fn spawn_deck_load(
        &mut self,
        channel_uuid: &str,
        name: String,
        loader: SourceLoader,
    ) -> String {
        self.sources.deck_loader.spawn(
            &self.render.context,
            channel_uuid,
            name,
            loader,
            self.render.width,
            self.render.height,
        )
    }

    /// Attach every deck whose load finished since the last frame, reporting failures.
    pub(crate) fn attach_finished_deck_loads(&mut self) {
        for (load, deck) in self.sources.deck_loader.take_finished() {
            match self.attach_loaded_deck(&load, deck) {
                Ok(ch_idx) => {
                    self.session.notifications.info(format!(
                        "➕ {} → Ch {}",
                        load.name,
                        ch_idx + 1
                    ));
                }
                Err(e) => {
                    let message = format!("{e:#}");
                    self.session
                        .notifications
                        .error(format!("Failed to load deck '{}': {message}", load.name));
                    self.sources.deck_loader.record_failure(load, message);
                }
            }
        }
        self.sources.deck_loader.prune_failures(Instant::now());
    }

    /// Finalize a built deck and add it to its channel. Returns the channel's
    /// position.
    fn attach_loaded_deck(
        &mut self,
        load: &Load,
        deck: anyhow::Result<Deck>,
    ) -> anyhow::Result<usize> {
        let mut deck = deck?;
        let ch_idx = self
            .mixer
            .resolve_channel(&load.channel_uuid)
            .context("its channel was removed while it loaded")?;
        self.finalize_new_deck(&mut deck)?;
        let channel = self
            .mixer
            .channel_mut(ch_idx)
            .context("its channel was removed while it loaded")?;
        channel.add_deck(deck);
        Ok(ch_idx)
    }
}

#[cfg(test)]
impl VardaApp {
    /// Run frames until no deck load is in flight.
    pub(crate) fn settle_deck_loads(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.sources.deck_loader.in_flight() > 0 {
            assert!(Instant::now() < deadline, "deck loads never finished");
            std::thread::sleep(Duration::from_millis(5));
            self.process_commands();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::types::DeckLoadStatus;
    use crate::engine::{CommandResult, EngineCommand as C, ErrorCode};

    fn headless_app() -> Option<super::VardaApp> {
        crate::testing::headless_app()
    }

    fn channel_uuid(app: &super::VardaApp, idx: usize) -> String {
        app.mixer_ref().channels()[idx].uuid().to_string()
    }

    fn shader(name: &str) -> crate::source::SourceConfig {
        crate::source::SourceConfig::new("Shader").with("name", name)
    }

    fn has_deck(app: &super::VardaApp, uuid: &str) -> bool {
        app.mixer_ref()
            .channels()
            .iter()
            .flat_map(|ch| ch.decks.iter())
            .any(|slot| slot.deck.uuid() == uuid)
    }

    #[test]
    fn a_deck_add_answers_with_the_uuid_the_deck_later_has() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch = channel_uuid(&app, 0);
        let uuid = app
            .add_deck(&ch, &shader("liquid_light"))
            .expect("known shader");

        assert!(!has_deck(&app, &uuid), "decks attach on a later frame");
        let loads = app.build_engine_state().deck_loads;
        assert!(matches!(
            loads.as_slice(),
            [l] if l.uuid == uuid && l.channel_uuid == ch && l.status == DeckLoadStatus::Loading
        ));

        app.settle_deck_loads();
        assert!(has_deck(&app, &uuid));
        assert_eq!(app.build_engine_state().deck_loads.len(), 0);
    }

    #[test]
    fn a_write_to_a_deck_still_loading_is_not_found() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch = channel_uuid(&app, 0);
        let uuid = app
            .add_deck(&ch, &shader("liquid_light"))
            .expect("known shader");

        let result = app.execute_command(C::SetDeckOpacity {
            deck_uuid: uuid,
            opacity: 0.5,
        });
        assert!(matches!(
            result,
            CommandResult::Err {
                code: ErrorCode::NotFound,
                ..
            }
        ));
    }

    #[test]
    fn a_failed_load_is_listed_and_reported() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-an-image.png");
        std::fs::write(&path, b"this is not a png").unwrap();
        let ch = channel_uuid(&app, 0);
        let uuid = app
            .add_deck(
                &ch,
                &crate::still::Image::config_for(&path.to_string_lossy()),
            )
            .expect("the file exists");

        app.settle_deck_loads();
        assert!(!has_deck(&app, &uuid));
        let loads = app.build_engine_state().deck_loads;
        assert!(matches!(
            loads.as_slice(),
            [l] if l.uuid == uuid && matches!(l.status, DeckLoadStatus::Failed { .. })
        ));
        assert!(
            app.session
                .notifications
                .visible()
                .iter()
                .any(|n| n.message.contains("not-an-image.png")),
            "the operator is told which deck failed"
        );
    }

    #[test]
    fn a_load_whose_channel_is_removed_fails_instead_of_landing_elsewhere() {
        let Some(mut app) = headless_app() else {
            return;
        };
        app.execute_command(C::AddChannel);
        let doomed = channel_uuid(&app, 2);
        let uuid = app
            .add_deck(&doomed, &shader("liquid_light"))
            .expect("known shader");
        assert!(matches!(
            app.execute_command(C::RemoveChannel {
                channel_uuid: doomed
            }),
            CommandResult::Ok
        ));

        app.settle_deck_loads();
        assert!(!has_deck(&app, &uuid));
        assert!(matches!(
            app.build_engine_state().deck_loads.as_slice(),
            [l] if matches!(l.status, DeckLoadStatus::Failed { .. })
        ));
    }

    #[test]
    fn a_missing_media_file_is_refused_at_once() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch = channel_uuid(&app, 0);
        let missing = "/nonexistent/clip.mp4";
        assert!(
            app.add_deck(&ch, &crate::video::provider::Video::config_for(missing))
                .is_err()
        );
        assert!(
            app.add_deck(&ch, &crate::still::Image::config_for(missing))
                .is_err()
        );
        assert_eq!(app.build_engine_state().deck_loads.len(), 0);
    }
}
