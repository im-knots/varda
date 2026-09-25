//! Background deck loading. Shader compiles and media decodes run off the
//! render thread; finished decks attach at the start of a later frame, where
//! they are finalized. This is the only path that builds a deck from a shader,
//! image, or video, whichever consumer asked. See /spec/ui-engine-boundary.md
//! (Decision #15).

use super::VardaApp;
use crate::deck::Deck;
use crate::engine::types::{DeckLoadSnapshot, DeckLoadStatus};
use crate::renderer::GpuContext;
use anyhow::Context;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long a failed load stays listed, so a client that polls state can see
/// why its deck never appeared.
const FAILED_LOAD_RETENTION: Duration = Duration::from_secs(60);

/// What a background load builds.
pub(crate) enum DeckSource {
    Shader(Box<crate::isf::ISFShader>),
    Image(PathBuf),
    Video(PathBuf),
}

impl DeckSource {
    fn name(&self) -> String {
        match self {
            Self::Shader(shader) => shader.name(),
            Self::Image(path) | Self::Video(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |f| f.to_string_lossy().into_owned(),
            ),
        }
    }

    fn build(self, context: &GpuContext, width: u32, height: u32) -> anyhow::Result<Deck> {
        match self {
            Self::Shader(shader) if shader.metadata.is_compute() => {
                Deck::new_from_compute_shader(context, *shader, width, height)
            }
            Self::Shader(shader) => Deck::new(context, *shader, width, height),
            Self::Image(path) => Deck::new_from_image(context, &path, width, height),
            Self::Video(path) => Deck::new_from_video(context, &path, width, height),
        }
    }
}

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

    /// Start building a deck for `channel_uuid` and return the UUID it will
    /// have once attached.
    fn spawn(
        &mut self,
        context: &GpuContext,
        channel_uuid: &str,
        source: DeckSource,
        width: u32,
        height: u32,
    ) -> String {
        let uuid = uuid::Uuid::new_v4().to_string();
        let name = source.name();
        let tx = self.tx.clone();
        let context = context.clone();
        let deck_uuid = uuid.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("deck-load-{name}"))
            .spawn(move || {
                let deck = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    source.build(&context, width, height)
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the loader panicked")))
                .map(|mut deck| {
                    deck.set_uuid(deck_uuid.clone());
                    deck
                });
                // The engine may have shut down; nothing is waiting then.
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

    /// Number of loads still in flight.
    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.len()
    }
}

impl VardaApp {
    /// Start a background load of `source` into `channel_uuid` at the current
    /// render size, returning the UUID the deck will have.
    pub(crate) fn spawn_deck_load(&mut self, channel_uuid: &str, source: DeckSource) -> String {
        self.deck_loader.spawn(
            &self.context,
            channel_uuid,
            source,
            self.render_width,
            self.render_height,
        )
    }

    /// Attach every deck whose load finished since the last frame, and report
    /// the ones that could not be attached.
    pub(crate) fn attach_finished_deck_loads(&mut self) {
        for (load, deck) in self.deck_loader.take_finished() {
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
                    self.deck_loader.record_failure(load, message);
                }
            }
        }
        self.deck_loader.prune_failures(Instant::now());
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
    /// Run frames until no deck load is in flight. Shader, image, and video
    /// decks attach on a later frame than the command that asked for them.
    pub(crate) fn settle_deck_loads(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.deck_loader.in_flight() > 0 {
            assert!(Instant::now() < deadline, "deck loads never finished");
            std::thread::sleep(Duration::from_millis(5));
            self.process_commands();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::traits::MixerCommands;
    use crate::engine::types::DeckLoadStatus;
    use crate::engine::{CommandResult, EngineCommand as C, ErrorCode};

    fn headless_app() -> Option<super::VardaApp> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        super::VardaApp::new(gpu, &crate::testing::headless_config()).ok()
    }

    fn channel_uuid(app: &super::VardaApp, idx: usize) -> String {
        app.mixer_ref().channels()[idx].uuid().to_string()
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
        let uuid = app.add_deck(&ch, "liquid_light").expect("known shader");

        assert!(!has_deck(&app, &uuid), "decks attach on a later frame");
        let loads = app.build_engine_state().deck_loads;
        assert!(matches!(
            loads.as_slice(),
            [l] if l.uuid == uuid && l.channel_uuid == ch && l.status == DeckLoadStatus::Loading
        ));

        app.settle_deck_loads();
        assert!(has_deck(&app, &uuid));
        assert!(app.build_engine_state().deck_loads.is_empty());
    }

    #[test]
    fn a_write_to_a_deck_still_loading_is_not_found() {
        let Some(mut app) = headless_app() else {
            return;
        };
        let ch = channel_uuid(&app, 0);
        let uuid = app.add_deck(&ch, "liquid_light").expect("known shader");

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
        let uuid = app.add_image_deck(&ch, &path).expect("the file exists");

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
        let uuid = app.add_deck(&doomed, "liquid_light").expect("known shader");
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
        let missing = std::path::Path::new("/nonexistent/clip.mp4");
        assert!(app.add_video_deck(&ch, missing).is_err());
        assert!(app.add_image_deck(&ch, missing).is_err());
        assert!(app.build_engine_state().deck_loads.is_empty());
    }
}
