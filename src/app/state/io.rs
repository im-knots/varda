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
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::Ok
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
                            if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                                ch.add_deck(deck);
                                CommandResult::Ok
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
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::Ok
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
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::Ok
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
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::Ok
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
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::Ok
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
                        if let Some(ch) = self.mixer.channel_mut(channel_idx) {
                            ch.add_deck(deck);
                            CommandResult::Ok
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
}
