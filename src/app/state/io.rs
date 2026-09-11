//! External I/O deck creation and stream library mutations.

use super::super::VardaApp;
use crate::engine::{CommandResult, ErrorCode};

impl VardaApp {
    pub fn cmd_add_ndi_deck(&mut self, channel_uuid: &str, source_name: &str) -> CommandResult {
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        match self
            .external_io
            .ndi_manager
            .start_receive(source_name, &self.context.device)
        {
            Some(receiver_idx) => {
                let (src_w, src_h) = self
                    .external_io
                    .ndi_manager
                    .receiver_dimensions(receiver_idx)
                    .unwrap_or((1920, 1080));
                match crate::deck::Deck::new_from_ndi(
                    &self.context,
                    receiver_idx,
                    source_name,
                    src_w,
                    src_h,
                    self.render_width,
                    self.render_height,
                ) {
                    Ok(deck) => {
                        let uuid = deck.uuid().to_string();
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::OkWithId { uuid }
                        } else {
                            CommandResult::Err {
                                code: ErrorCode::NotFound,
                                message: "Channel not found".into(),
                            }
                        }
                    }
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InternalError,
                        message: e.to_string(),
                    },
                }
            }
            None => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("Failed to start NDI receive for '{source_name}'"),
            },
        }
    }

    /// Attach a Spout sender to a channel as a live deck.
    ///
    /// The Windows counterpart to [`Self::cmd_add_syphon_deck`], and idempotent
    /// for the same reason: a reconnecting controller and the reconcile pass can
    /// both try to bind the same sender, and they must converge on one deck.
    ///
    /// Not `cfg`-gated, because `SpoutManager` reports unavailable off Windows,
    /// so the command simply refuses there rather than failing to compile.
    /// See /spec/spout-output.md.
    pub fn cmd_add_spout_deck(&mut self, channel_uuid: &str, sender_name: &str) -> CommandResult {
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        let display_name = format!("🔗 {sender_name}");
        if let Some(ch) = self.mixer.channels().get(channel_idx)
            && ch
                .decks
                .iter()
                .any(|s| s.deck.source_name() == display_name)
        {
            log::debug!(
                "Spout deck '{sender_name}' already present on channel {channel_idx}; add is a no-op"
            );
            return CommandResult::Ok;
        }
        let Some(receiver_idx) = self
            .external_io
            .spout_manager
            .start_receive(sender_name, &self.context.device)
        else {
            return CommandResult::Err {
                code: ErrorCode::Unavailable,
                message: "Spout is unavailable on this system".into(),
            };
        };
        let (src_w, src_h) = self
            .external_io
            .spout_manager
            .client_dimensions(receiver_idx)
            .unwrap_or((1920, 1080));
        match crate::deck::Deck::new_from_spout(
            &self.context,
            receiver_idx,
            sender_name,
            src_w,
            src_h,
            self.render_width,
            self.render_height,
        ) {
            Ok(deck) => {
                let uuid = deck.uuid().to_string();
                if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                    ch.add_deck(deck);
                    CommandResult::OkWithId { uuid }
                } else {
                    CommandResult::Err {
                        code: ErrorCode::NotFound,
                        message: "Channel not found".into(),
                    }
                }
            }
            Err(e) => CommandResult::Err {
                code: ErrorCode::InternalError,
                message: e.to_string(),
            },
        }
    }

    pub fn cmd_add_syphon_deck(&mut self, channel_uuid: &str, server_name: &str) -> CommandResult {
        #[cfg(target_os = "macos")]
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        #[cfg(target_os = "macos")]
        {
            // Idempotency: if this channel already carries a Syphon deck for this
            // server, do nothing. An external controller may re-subscribe on every
            // reconnect, and our own reconcile may also bind it — both must
            // converge to a single deck, not stack duplicates.
            let display_name = format!("🔗 {server_name}");
            if let Some(ch) = self.mixer.channels().get(channel_idx)
                && ch
                    .decks
                    .iter()
                    .any(|s| s.deck.source_name() == display_name)
            {
                log::debug!(
                    "Syphon deck '{server_name}' already present on channel {channel_idx}; add is a no-op"
                );
                return CommandResult::Ok;
            }
            match self
                .external_io
                .syphon_manager
                .start_receive(server_name, &self.context.device)
            {
                Some(client_idx) => {
                    let (src_w, src_h) = self
                        .external_io
                        .syphon_manager
                        .client_dimensions(client_idx)
                        .unwrap_or((1920, 1080));
                    match crate::deck::Deck::new_from_syphon(
                        &self.context,
                        client_idx,
                        server_name,
                        src_w,
                        src_h,
                        self.render_width,
                        self.render_height,
                    ) {
                        Ok(deck) => {
                            let uuid = deck.uuid().to_string();
                            if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                                ch.add_deck(deck);
                                CommandResult::OkWithId { uuid }
                            } else {
                                CommandResult::Err {
                                    code: ErrorCode::NotFound,
                                    message: "Channel not found".into(),
                                }
                            }
                        }
                        Err(e) => CommandResult::Err {
                            code: ErrorCode::InternalError,
                            message: e.to_string(),
                        },
                    }
                }
                None => CommandResult::Err {
                    code: ErrorCode::InvalidInput,
                    message: format!("Failed to start Syphon receive for '{server_name}'"),
                },
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (channel_uuid, server_name);
            CommandResult::Err {
                code: ErrorCode::Unavailable,
                message: "Syphon is only available on macOS".into(),
            }
        }
    }

    pub fn cmd_add_srt_deck(
        &mut self,
        channel_uuid: &str,
        url: &str,
        mode: crate::stream::SrtMode,
    ) -> CommandResult {
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        match self
            .external_io
            .stream_manager
            .start_srt_receive(url, mode, &self.context.device)
        {
            Some(receiver_idx) => {
                let (src_w, src_h) = self
                    .external_io
                    .stream_manager
                    .receiver_dimensions(receiver_idx)
                    .unwrap_or((1920, 1080));
                match crate::deck::Deck::new_from_srt(
                    &self.context,
                    receiver_idx,
                    url,
                    src_w,
                    src_h,
                    self.render_width,
                    self.render_height,
                ) {
                    Ok(deck) => {
                        let uuid = deck.uuid().to_string();
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::OkWithId { uuid }
                        } else {
                            CommandResult::Err {
                                code: ErrorCode::NotFound,
                                message: "Channel not found".into(),
                            }
                        }
                    }
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InternalError,
                        message: e.to_string(),
                    },
                }
            }
            None => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("Failed to start SRT receive for '{url}'"),
            },
        }
    }

    pub fn cmd_add_hls_deck(&mut self, channel_uuid: &str, url: &str) -> CommandResult {
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        match self.external_io.stream_manager.start_receive(
            url,
            crate::stream::StreamProtocol::Hls,
            &self.context.device,
        ) {
            Some(receiver_idx) => {
                let (src_w, src_h) = self
                    .external_io
                    .stream_manager
                    .receiver_dimensions(receiver_idx)
                    .unwrap_or((1920, 1080));
                match crate::deck::Deck::new_from_hls(
                    &self.context,
                    receiver_idx,
                    url,
                    src_w,
                    src_h,
                    self.render_width,
                    self.render_height,
                ) {
                    Ok(deck) => {
                        let uuid = deck.uuid().to_string();
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::OkWithId { uuid }
                        } else {
                            CommandResult::Err {
                                code: ErrorCode::NotFound,
                                message: "Channel not found".into(),
                            }
                        }
                    }
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InternalError,
                        message: e.to_string(),
                    },
                }
            }
            None => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("Failed to start HLS receive for '{url}'"),
            },
        }
    }

    pub fn cmd_add_html_deck(&mut self, channel_uuid: &str, url: &str) -> CommandResult {
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        match self.external_io.html_manager.start_render(
            url,
            self.render_width,
            self.render_height,
            &self.context.device,
        ) {
            Some(instance_idx) => {
                let (src_w, src_h) = self
                    .external_io
                    .html_manager
                    .instance_dimensions(instance_idx)
                    .unwrap_or((1920, 1080));
                match crate::deck::Deck::new_from_html(
                    &self.context,
                    instance_idx,
                    url,
                    src_w,
                    src_h,
                    self.render_width,
                    self.render_height,
                ) {
                    Ok(deck) => {
                        let uuid = deck.uuid().to_string();
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::OkWithId { uuid }
                        } else {
                            CommandResult::Err {
                                code: ErrorCode::NotFound,
                                message: "Channel not found".into(),
                            }
                        }
                    }
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InternalError,
                        message: e.to_string(),
                    },
                }
            }
            None => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("Failed to start HTML render for '{url}'"),
            },
        }
    }

    /// Reload the HTML deck at `(channel_idx, deck_idx)`, re-fetching its URL.
    pub fn cmd_reload_html_deck(&mut self, deck_uuid: &str) -> CommandResult {
        let (channel_idx, deck_idx) = match self.resolve_deck(deck_uuid) {
            Ok(loc) => loc,
            Err(e) => return e.into(),
        };
        let kind = self
            .mixer
            .channels()
            .get(channel_idx)
            .and_then(|ch| ch.decks.get(deck_idx))
            .map(|slot| slot.deck.external_source_kind());
        match kind {
            Some(Some(crate::deck::ExternalSourceKind::Html(idx))) => {
                self.external_io.html_manager.reload(idx);
                CommandResult::Ok
            }
            _ => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: "Deck is not an HTML source".into(),
            },
        }
    }

    pub fn cmd_add_dash_deck(&mut self, channel_uuid: &str, url: &str) -> CommandResult {
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        match self.external_io.stream_manager.start_receive(
            url,
            crate::stream::StreamProtocol::Dash,
            &self.context.device,
        ) {
            Some(receiver_idx) => {
                let (src_w, src_h) = self
                    .external_io
                    .stream_manager
                    .receiver_dimensions(receiver_idx)
                    .unwrap_or((1920, 1080));
                match crate::deck::Deck::new_from_dash(
                    &self.context,
                    receiver_idx,
                    url,
                    src_w,
                    src_h,
                    self.render_width,
                    self.render_height,
                ) {
                    Ok(deck) => {
                        let uuid = deck.uuid().to_string();
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::OkWithId { uuid }
                        } else {
                            CommandResult::Err {
                                code: ErrorCode::NotFound,
                                message: "Channel not found".into(),
                            }
                        }
                    }
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InternalError,
                        message: e.to_string(),
                    },
                }
            }
            None => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("Failed to start DASH receive for '{url}'"),
            },
        }
    }

    pub fn cmd_add_rtmp_deck(
        &mut self,
        channel_uuid: &str,
        url: &str,
        mode: crate::stream::RtmpMode,
    ) -> CommandResult {
        let channel_idx = match self.resolve_channel(channel_uuid) {
            Ok(idx) => idx,
            Err(e) => return e.into(),
        };
        match self
            .external_io
            .stream_manager
            .start_rtmp_receive(url, mode, &self.context.device)
        {
            Some(receiver_idx) => {
                let (src_w, src_h) = self
                    .external_io
                    .stream_manager
                    .receiver_dimensions(receiver_idx)
                    .unwrap_or((1920, 1080));
                match crate::deck::Deck::new_from_rtmp(
                    &self.context,
                    receiver_idx,
                    url,
                    src_w,
                    src_h,
                    self.render_width,
                    self.render_height,
                ) {
                    Ok(deck) => {
                        let uuid = deck.uuid().to_string();
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::OkWithId { uuid }
                        } else {
                            CommandResult::Err {
                                code: ErrorCode::NotFound,
                                message: "Channel not found".into(),
                            }
                        }
                    }
                    Err(e) => CommandResult::Err {
                        code: ErrorCode::InternalError,
                        message: e.to_string(),
                    },
                }
            }
            None => CommandResult::Err {
                code: ErrorCode::InvalidInput,
                message: format!("Failed to start RTMP receive for '{url}'"),
            },
        }
    }

    // ── Stream Library ─────────────────────────────────────

    pub fn cmd_add_stream_library_entry(
        &mut self,
        url: String,
        mode: crate::stream::SrtMode,
    ) -> CommandResult {
        if !self
            .external_io
            .stream_library
            .iter()
            .any(|(u, _)| u == &url)
        {
            self.external_io.stream_library.push((url, mode));
        }
        CommandResult::Ok
    }

    pub fn cmd_remove_stream_library_entry(&mut self, url: &str) -> CommandResult {
        self.external_io.stream_library.retain(|(u, _)| u != url);
        CommandResult::Ok
    }

    pub fn cmd_add_hls_library_entry(&mut self, url: String) -> CommandResult {
        if !self.external_io.hls_library.contains(&url) {
            log::info!("Added HLS source to library via API: {url}");
            self.external_io.hls_library.push(url);
        }
        CommandResult::Ok
    }

    pub fn cmd_remove_hls_library_entry(&mut self, url: &str) -> CommandResult {
        self.external_io.hls_library.retain(|u| u != url);
        CommandResult::Ok
    }

    pub fn cmd_add_dash_library_entry(&mut self, url: String) -> CommandResult {
        if !self.external_io.dash_library.contains(&url) {
            log::info!("Added DASH source to library via API: {url}");
            self.external_io.dash_library.push(url);
        }
        CommandResult::Ok
    }

    pub fn cmd_remove_dash_library_entry(&mut self, url: &str) -> CommandResult {
        self.external_io.dash_library.retain(|u| u != url);
        CommandResult::Ok
    }

    pub fn cmd_add_rtmp_library_entry(
        &mut self,
        url: String,
        mode: crate::stream::RtmpMode,
    ) -> CommandResult {
        if !self.external_io.rtmp_library.iter().any(|(u, _)| u == &url) {
            log::info!("Added RTMP source to library via API: {url} ({mode})");
            self.external_io.rtmp_library.push((url, mode));
        }
        CommandResult::Ok
    }

    pub fn cmd_remove_rtmp_library_entry(&mut self, url: &str) -> CommandResult {
        self.external_io.rtmp_library.retain(|(u, _)| u != url);
        CommandResult::Ok
    }

    pub fn cmd_add_html_library_entry(&mut self, url: String) -> CommandResult {
        if !self.external_io.html_library.contains(&url) {
            log::info!("Added HTML source to library: {url}");
            self.external_io.html_library.push(url);
        }
        CommandResult::Ok
    }

    pub fn cmd_remove_html_library_entry(&mut self, url: &str) -> CommandResult {
        self.external_io.html_library.retain(|u| u != url);
        CommandResult::Ok
    }

    /// Render-thread Syphon maintenance, called ~1×/sec (see `render_mixer_frame`).
    ///
    /// Two jobs, both removing the start/stop-ordering fragility that used to
    /// require coordinated launch and manual re-probes:
    ///   1. **Auto-rediscover** — re-scan `SyphonServerDirectory` so a producer
    ///      that starts, restarts, or republishes *after* Varda is noticed
    ///      without an external rescan poke. Keeps the snapshot/library list
    ///      fresh for the UI and any external controller's GET fallback.
    ///   2. **Late-bind pending decks** — any Syphon deck deferred at restore
    ///      time (`persistence::PendingSyphonDeck`) is attached the moment its
    ///      named server appears. No `black_hole` placeholder, no failed-restore.
    #[cfg(target_os = "macos")]
    pub fn reconcile_syphon(&mut self) {
        const SCAN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
        if self.external_io.last_syphon_scan.elapsed() < SCAN_INTERVAL {
            return;
        }
        self.external_io.last_syphon_scan = std::time::Instant::now();

        // (1) Re-scan. Cheap directory query; safe to run every tick-second.
        self.external_io.syphon_manager.discover();

        if self.external_io.pending_syphon.is_empty() {
            return;
        }

        // (2) Bind any pending deck whose server is now available.
        let available: std::collections::HashSet<String> = self
            .external_io
            .syphon_manager
            .sources()
            .iter()
            .map(|s| s.name.clone())
            .collect();

        // Pull the ready ones out; leave the rest pending for the next pass.
        let mut ready: Vec<crate::persistence::PendingSyphonDeck> = Vec::new();
        self.external_io
            .pending_syphon
            .retain(|p| match &p.config.source {
                crate::scene::SourceConfig::Syphon { name } if available.contains(name) => {
                    ready.push(p.clone());
                    false
                }
                _ => true,
            });

        for p in ready {
            let crate::scene::SourceConfig::Syphon { name } = &p.config.source else {
                continue;
            };
            let server_name = name.clone();
            let channel_uuid = p.channel_uuid.clone();
            match self.cmd_add_syphon_deck(&channel_uuid, &server_name) {
                CommandResult::Ok
                | CommandResult::OkWithId { .. }
                | CommandResult::OkWithData { .. } => {
                    // Re-apply the persisted slot props onto the deck we just bound
                    // (matched by name so an idempotent no-op doesn't mis-target).
                    let display_name = format!("🔗 {server_name}");
                    if let Ok(ch_idx) = self.resolve_channel(&channel_uuid)
                        && let Some(ch) = self.mixer.channel_mut(ch_idx)
                        && let Some(slot) = ch
                            .decks
                            .iter_mut()
                            .find(|s| s.deck.source_name() == display_name)
                    {
                        slot.opacity = p.config.opacity;
                        slot.blend_mode = p.config.blend_mode.into();
                        slot.mute = p.config.mute;
                        slot.solo = p.config.solo;
                        slot.z_index = p.config.z_index;
                    }
                    log::info!("Syphon deck '{server_name}' late-bound to channel {channel_uuid}");
                }
                CommandResult::Err { code, message } => {
                    // A missing channel means the user deleted it after restore —
                    // there is nothing left to bind to, so drop the pending deck
                    // rather than retrying forever.
                    if code == ErrorCode::NotFound {
                        log::info!(
                            "Dropping Syphon late-bind for '{server_name}': channel {channel_uuid} no longer exists"
                        );
                        continue;
                    }
                    log::warn!(
                        "Syphon late-bind for '{server_name}' (channel {channel_uuid}) failed: {message}; will retry"
                    );
                    // Requeue to retry on the next reconcile.
                    self.external_io.pending_syphon.push(p);
                }
            }
        }
    }

    /// Rediscover Spout senders and late-bind any deck waiting for one.
    ///
    /// The Windows counterpart to [`Self::reconcile_syphon`], and the same two
    /// jobs: keep the library list fresh so a producer that starts after Varda is
    /// noticed, and attach decks deferred at restore once their sender appears.
    ///
    /// Runs unconditionally because discovery is a no-op off Windows, which keeps
    /// the render loop free of a platform gate.
    pub fn reconcile_spout(&mut self) {
        const SCAN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
        if !self.external_io.spout_manager.is_available() {
            return;
        }
        if self.external_io.last_spout_scan.elapsed() < SCAN_INTERVAL {
            return;
        }
        self.external_io.last_spout_scan = std::time::Instant::now();
        self.external_io.spout_manager.discover();

        if self.external_io.pending_spout.is_empty() {
            return;
        }
        let available: std::collections::HashSet<String> = self
            .external_io
            .spout_manager
            .sources()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        let mut ready: Vec<crate::persistence::PendingSpoutDeck> = Vec::new();
        self.external_io
            .pending_spout
            .retain(|p| match &p.config.source {
                crate::scene::SourceConfig::Spout { name } if available.contains(name) => {
                    ready.push(p.clone());
                    false
                }
                _ => true,
            });

        for p in ready {
            let crate::scene::SourceConfig::Spout { name } = &p.config.source else {
                continue;
            };
            let sender_name = name.clone();
            let channel_uuid = p.channel_uuid.clone();
            match self.cmd_add_spout_deck(&channel_uuid, &sender_name) {
                CommandResult::Ok
                | CommandResult::OkWithId { .. }
                | CommandResult::OkWithData { .. } => {
                    let display_name = format!("🔗 {sender_name}");
                    if let Ok(ch_idx) = self.resolve_channel(&channel_uuid)
                        && let Some(ch) = self.mixer.channel_mut(ch_idx)
                        && let Some(slot) = ch
                            .decks
                            .iter_mut()
                            .find(|s| s.deck.source_name() == display_name)
                    {
                        slot.opacity = p.config.opacity;
                        slot.blend_mode = p.config.blend_mode.into();
                        slot.mute = p.config.mute;
                        slot.solo = p.config.solo;
                        slot.z_index = p.config.z_index;
                    }
                    log::info!("Spout deck '{sender_name}' late-bound to channel {channel_uuid}");
                }
                CommandResult::Err { code, message } => {
                    if code == ErrorCode::NotFound {
                        log::info!(
                            "Dropping Spout late-bind for '{sender_name}': channel {channel_uuid} no longer exists"
                        );
                        continue;
                    }
                    log::warn!(
                        "Spout late-bind for '{sender_name}' (channel {channel_uuid}) failed: {message}; will retry"
                    );
                    self.external_io.pending_spout.push(p);
                }
            }
        }
    }
}
