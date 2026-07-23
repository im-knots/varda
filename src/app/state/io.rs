//! External I/O deck creation and stream library mutations.

use super::super::VardaApp;
use crate::engine::{CommandResult, ErrorCode};

impl VardaApp {
    pub fn cmd_add_ndi_deck(&mut self, channel_idx: usize, source_name: String) -> CommandResult {
        match self
            .external_io
            .ndi_manager
            .start_receive(&source_name, &self.context.device)
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
                    &source_name,
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
                message: format!("Failed to start NDI receive for '{}'", source_name),
            },
        }
    }

    pub fn cmd_add_syphon_deck(
        &mut self,
        channel_idx: usize,
        server_name: String,
    ) -> CommandResult {
        #[cfg(target_os = "macos")]
        {
            // Idempotency: if this channel already carries a Syphon deck for this
            // server, do nothing. An external controller may re-subscribe on every
            // reconnect, and our own reconcile may also bind it — both must
            // converge to a single deck, not stack duplicates.
            let display_name = format!("🔗 {}", server_name);
            if let Some(ch) = self.mixer.channels().get(channel_idx) {
                if ch
                    .decks
                    .iter()
                    .any(|s| s.deck.source_name() == display_name)
                {
                    log::debug!(
                        "Syphon deck '{}' already present on channel {}; add is a no-op",
                        server_name,
                        channel_idx
                    );
                    return CommandResult::Ok;
                }
            }
            match self
                .external_io
                .syphon_manager
                .start_receive(&server_name, &self.context.device)
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
                        &server_name,
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
                    message: format!("Failed to start Syphon receive for '{}'", server_name),
                },
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (channel_idx, server_name);
            CommandResult::Err {
                code: ErrorCode::Unavailable,
                message: "Syphon is only available on macOS".into(),
            }
        }
    }

    pub fn cmd_add_srt_deck(
        &mut self,
        channel_idx: usize,
        url: String,
        mode: crate::stream::SrtMode,
    ) -> CommandResult {
        match self
            .external_io
            .stream_manager
            .start_srt_receive(&url, mode, &self.context.device)
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
                    &url,
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
                message: format!("Failed to start SRT receive for '{}'", url),
            },
        }
    }

    pub fn cmd_add_hls_deck(&mut self, channel_idx: usize, url: String) -> CommandResult {
        match self.external_io.stream_manager.start_receive(
            &url,
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
                    &url,
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
                message: format!("Failed to start HLS receive for '{}'", url),
            },
        }
    }

    pub fn cmd_add_html_deck(&mut self, channel_idx: usize, url: String) -> CommandResult {
        match self.external_io.html_manager.start_render(
            &url,
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
                    &url,
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
                message: format!("Failed to start HTML render for '{}'", url),
            },
        }
    }

    /// Reload the HTML deck at `(channel_idx, deck_idx)`, re-fetching its URL.
    pub fn cmd_reload_html_deck(&mut self, channel_idx: usize, deck_idx: usize) -> CommandResult {
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

    pub fn cmd_add_dash_deck(&mut self, channel_idx: usize, url: String) -> CommandResult {
        match self.external_io.stream_manager.start_receive(
            &url,
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
                    &url,
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
                message: format!("Failed to start DASH receive for '{}'", url),
            },
        }
    }

    pub fn cmd_add_rtmp_deck(
        &mut self,
        channel_idx: usize,
        url: String,
        mode: crate::stream::RtmpMode,
    ) -> CommandResult {
        match self
            .external_io
            .stream_manager
            .start_rtmp_receive(&url, mode, &self.context.device)
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
                    &url,
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
                message: format!("Failed to start RTMP receive for '{}'", url),
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

    pub fn cmd_remove_stream_library_entry(&mut self, url: String) -> CommandResult {
        self.external_io.stream_library.retain(|(u, _)| u != &url);
        CommandResult::Ok
    }

    pub fn cmd_add_hls_library_entry(&mut self, url: String) -> CommandResult {
        if !self.external_io.hls_library.contains(&url) {
            log::info!("Added HLS source to library via API: {}", url);
            self.external_io.hls_library.push(url);
        }
        CommandResult::Ok
    }

    pub fn cmd_remove_hls_library_entry(&mut self, url: String) -> CommandResult {
        self.external_io.hls_library.retain(|u| u != &url);
        CommandResult::Ok
    }

    pub fn cmd_add_dash_library_entry(&mut self, url: String) -> CommandResult {
        if !self.external_io.dash_library.contains(&url) {
            log::info!("Added DASH source to library via API: {}", url);
            self.external_io.dash_library.push(url);
        }
        CommandResult::Ok
    }

    pub fn cmd_remove_dash_library_entry(&mut self, url: String) -> CommandResult {
        self.external_io.dash_library.retain(|u| u != &url);
        CommandResult::Ok
    }

    pub fn cmd_add_rtmp_library_entry(
        &mut self,
        url: String,
        mode: crate::stream::RtmpMode,
    ) -> CommandResult {
        if !self.external_io.rtmp_library.iter().any(|(u, _)| u == &url) {
            log::info!("Added RTMP source to library via API: {} ({})", url, mode);
            self.external_io.rtmp_library.push((url, mode));
        }
        CommandResult::Ok
    }

    pub fn cmd_remove_rtmp_library_entry(&mut self, url: String) -> CommandResult {
        self.external_io.rtmp_library.retain(|(u, _)| u != &url);
        CommandResult::Ok
    }

    pub fn cmd_add_html_library_entry(&mut self, url: String) -> CommandResult {
        if !self.external_io.html_library.contains(&url) {
            log::info!("Added HTML source to library: {}", url);
            self.external_io.html_library.push(url);
        }
        CommandResult::Ok
    }

    pub fn cmd_remove_html_library_entry(&mut self, url: String) -> CommandResult {
        self.external_io.html_library.retain(|u| u != &url);
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
    ///      named server appears. No black_hole placeholder, no failed-restore.
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
            let ch_idx = p.channel_idx;
            match self.cmd_add_syphon_deck(ch_idx, server_name.clone()) {
                CommandResult::Ok
                | CommandResult::OkWithId { .. }
                | CommandResult::OkWithData { .. } => {
                    // Re-apply the persisted slot props onto the deck we just bound
                    // (matched by name so an idempotent no-op doesn't mis-target).
                    let display_name = format!("🔗 {}", server_name);
                    if let Some(ch) = self.mixer.channel_mut(ch_idx) {
                        if let Some(slot) = ch
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
                    }
                    log::info!(
                        "Syphon deck '{}' late-bound to channel {}",
                        server_name,
                        ch_idx
                    );
                }
                CommandResult::Err { message, .. } => {
                    log::warn!(
                        "Syphon late-bind for '{}' (channel {}) failed: {}; will retry",
                        server_name,
                        ch_idx,
                        message
                    );
                    // Requeue to retry on the next reconcile.
                    self.external_io.pending_syphon.push(p);
                }
            }
        }
    }
}
