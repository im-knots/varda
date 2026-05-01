//! Video playback support for Varda
//! Uses FFmpeg for decoding video files to frames

use anyhow::{Context, Result};
use std::path::Path;

extern crate ffmpeg_next as ffmpeg;

use ffmpeg::format::{input, Pixel};
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg::util::frame::video::Video;

/// A video player that decodes frames from a video file
pub struct VideoPlayer {
    ictx: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Video,
    scaler: Scaler,
    video_stream_index: usize,
    
    // Video properties
    width: u32,
    height: u32,
    frame_rate: f64,
    duration: f64,
    
    // Playback state
    current_time: f64,
    is_playing: bool,
    is_looping: bool,
    
    // Current frame data (RGBA)
    frame_data: Vec<u8>,
}

impl VideoPlayer {
    /// Create a new video player from a file path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        ffmpeg::init().context("Failed to initialize FFmpeg")?;
        
        let ictx = input(&path).context("Failed to open video file")?;
        
        let video_stream = ictx
            .streams()
            .best(Type::Video)
            .context("No video stream found")?;
        let video_stream_index = video_stream.index();
        
        let context_decoder = ffmpeg::codec::context::Context::from_parameters(video_stream.parameters())?;
        let decoder = context_decoder.decoder().video()?;
        
        let width = decoder.width();
        let height = decoder.height();
        
        // Get frame rate
        let frame_rate = video_stream.rate();
        let fps = frame_rate.0 as f64 / frame_rate.1 as f64;
        
        // Get duration in seconds
        let duration = if ictx.duration() > 0 {
            ictx.duration() as f64 / ffmpeg::ffi::AV_TIME_BASE as f64
        } else {
            0.0
        };
        
        // Create scaler to convert to RGBA
        let scaler = Scaler::get(
            decoder.format(),
            width,
            height,
            Pixel::RGBA,
            width,
            height,
            Flags::BILINEAR,
        )?;
        
        let frame_data = vec![0u8; (width * height * 4) as usize];
        
        log::info!(
            "Loaded video: {}x{} @ {:.2} fps, duration: {:.2}s",
            width, height, fps, duration
        );
        
        Ok(Self {
            ictx,
            decoder,
            scaler,
            video_stream_index,
            width,
            height,
            frame_rate: fps,
            duration,
            current_time: 0.0,
            is_playing: true,
            is_looping: true,
            frame_data,
        })
    }
    
    /// Get the next frame as RGBA data
    /// Returns the frame data if a new frame is available
    pub fn next_frame(&mut self) -> Result<Option<&[u8]>> {
        loop {
            // Try to receive a decoded frame
            let mut decoded = Video::empty();
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                // Scale to RGBA
                let mut rgb_frame = Video::empty();
                self.scaler.run(&decoded, &mut rgb_frame)?;
                
                // Copy frame data
                let data = rgb_frame.data(0);
                let stride = rgb_frame.stride(0);
                
                for y in 0..self.height as usize {
                    let src_offset = y * stride;
                    let dst_offset = y * (self.width as usize * 4);
                    let row_bytes = self.width as usize * 4;
                    self.frame_data[dst_offset..dst_offset + row_bytes]
                        .copy_from_slice(&data[src_offset..src_offset + row_bytes]);
                }
                
                return Ok(Some(&self.frame_data));
            }
            
            // Need more packets
            match self.ictx.packets().next() {
                Some((stream, packet)) => {
                    if stream.index() == self.video_stream_index {
                        self.decoder.send_packet(&packet)?;
                    }
                }
                None => {
                    // End of file
                    if self.is_looping {
                        self.seek(0.0)?;
                        continue;
                    }
                    return Ok(None);
                }
            }
        }
    }
    
    /// Seek to a specific time in seconds
    pub fn seek(&mut self, time_secs: f64) -> Result<()> {
        let timestamp = (time_secs * ffmpeg::ffi::AV_TIME_BASE as f64) as i64;
        self.ictx.seek(timestamp, ..timestamp)?;
        self.decoder.flush();
        self.current_time = time_secs;
        Ok(())
    }
    
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn frame_rate(&self) -> f64 { self.frame_rate }
    pub fn duration(&self) -> f64 { self.duration }
    pub fn is_playing(&self) -> bool { self.is_playing }
    pub fn set_playing(&mut self, playing: bool) { self.is_playing = playing; }
    pub fn is_looping(&self) -> bool { self.is_looping }
    pub fn set_looping(&mut self, looping: bool) { self.is_looping = looping; }
}

