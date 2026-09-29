//! Text decks: words, lyrics and captions in any installed font.
//!
//! The text is parsed into cues ([`cue`]), laid out for the deck's mode and
//! transition each frame ([`layout`]), and drawn from coverage masks that are
//! rasterized only when a string, its style or its size bucket changes
//! ([`raster`], [`pass`]).

pub mod cue;
pub mod formats {
    pub mod lrc;
    pub mod subrip;
    pub mod webvtt;
}
pub mod layout;
pub mod pass;
pub mod raster;

use crate::engine::value::source::{DeckTransportSync, TransportSyncMode};
use crate::renderer::GpuContext;
use crate::source::{
    ControlError, ControlSpec, ControlStatus, ControlValue, DeckSourceInstance, DeckSourceProvider,
    LibraryEntry, LibrarySection, SourceConfig, SourceControl, SourceEnv, SourceFrame,
    SourceLoader, SourceQuery, WidgetHint, choice_index, choice_value, decode_config,
    encode_config, expect_color, expect_norm, expect_text,
};
use anyhow::{Context, Result};
use cue::{Format, Parsed};
use layout::{Extent, HAlign, Mode, StepUnit, StepView, Transition, Unit, VAlign};
use pass::{Draw, Mask, TextPass};
use raster::Style;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

pub const SOURCE_TYPE: &str = "Text";

/// Largest text a deck holds, in bytes. A song's lyrics are a few KiB; the cap
/// bounds `scene.json`, undo entries and state publication.
pub const MAX_TEXT: usize = 64 * 1024;

/// File extensions the text source imports.
pub const EXTENSIONS: &[&str] = &["txt", "lrc", "vtt", "srt"];

/// How long a frame may spend rasterizing, visible units first.
const RASTER_BUDGET: std::time::Duration = std::time::Duration::from_millis(2);

/// The baseline's depth below the top of a line's em box, in ems.
const ASCENT: f32 = 0.8;

const SPEED: (f32, f32) = (-4.0, 4.0);
const SCROLL: (f32, f32) = (-16.0, 16.0);
const TRANSITION_TIME: (f32, f32) = (0.0, 4.0);
const SIZE: (f32, f32) = (0.01, 1.0);
const LINE_SPACING: (f32, f32) = (0.5, 3.0);
const WEIGHT: (f32, f32) = (100.0, 900.0);
const POSITION: (f32, f32) = (-1.0, 2.0);

const MODES: [Mode; 4] = [Mode::Static, Mode::Crawl, Mode::Ticker, Mode::Step];
const UNITS: [Unit; 2] = [Unit::Line, Unit::Word];
const CLOCKS: [Clock; 2] = [Clock::Rate, Clock::Beat];
const ALIGNS: [HAlign; 3] = [HAlign::Left, HAlign::Center, HAlign::Right];
const VALIGNS: [VAlign; 3] = [VAlign::Top, VAlign::Middle, VAlign::Bottom];
const TRANSITIONS: [Transition; 4] = [
    Transition::Cut,
    Transition::Fade,
    Transition::RollUp,
    Transition::PaintOn,
];
const CHASE: [TransportSyncMode; 3] = [
    TransportSyncMode::Auto,
    TransportSyncMode::Always,
    TransportSyncMode::Never,
];
const VOICE_CONTROLS: [&str; 4] = [
    "voice_1_color",
    "voice_2_color",
    "voice_3_color",
    "voice_4_color",
];

/// What the free-running advance follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Clock {
    /// Seconds.
    #[default]
    Rate,
    /// The Beat timebase.
    Beat,
}

static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    let float = |name: &str, label: &str, (min, max): (f32, f32)| {
        ControlSpec::float(name, label, min, max)
            .routed(name)
            .modulatable()
    };
    let choice = |name: &str, label: &str, options: &[&str]| {
        ControlSpec::choice(name, label, options).routed(name)
    };
    let color =
        |name: &str, label: &str| ControlSpec::color(name, label).routed(name).modulatable();
    let mut params = vec![
        ControlSpec::text_block("text", "Text").routed("text"),
        ControlSpec::text("line", "Append line").routed("line"),
        ControlSpec::file("file", "Load file...", EXTENSIONS),
        choice("format", "Format", &Format::ALL.map(Format::label)),
        ControlSpec::text("font", "Font")
            .routed("font")
            .in_widget(WidgetHint::FontFamily),
        float("weight", "Weight", WEIGHT),
        ControlSpec::toggle("italic", "Italic").routed("italic"),
        choice("align", "Align", &["Left", "Center", "Right"]),
        choice("valign", "Vertical", &["Top", "Middle", "Bottom"]),
        choice("mode", "Mode", &["Static", "Crawl", "Ticker", "Step"]),
        choice("unit", "Unit", &["Line", "Word"]),
        choice("clock", "Clock", &["Rate", "Beat"]),
        float("speed", "Speed", SPEED),
        ControlSpec::toggle("loop", "Loop").routed("loop"),
        float("scroll", "Scroll", SCROLL),
        ControlSpec::action("next", "Next").routed("next"),
        ControlSpec::action("previous", "Previous").routed("previous"),
        ControlSpec::action("restart", "Restart").routed("restart"),
        choice(
            "transition",
            "Transition",
            &["Cut", "Fade", "Roll-up", "Paint-on"],
        ),
        float("transition_time", "Transition time", TRANSITION_TIME),
        choice("rollup_lines", "Roll-up lines", &["1", "2", "3", "4"]),
        float("size", "Size", SIZE),
        float("line_spacing", "Line spacing", LINE_SPACING),
        ControlSpec::point("position", "Position", POSITION.0, POSITION.1)
            .routed("position")
            .modulatable(),
        color("color", "Color"),
        color("background", "Background"),
    ];
    for (i, name) in VOICE_CONTROLS.iter().enumerate() {
        params.push(color(name, &format!("Speaker {} color", i + 1)));
    }
    params.push(ControlSpec::choice(
        "chase",
        "Chase",
        &["Auto", "Always", "Never"],
    ));
    params.push(ControlSpec::number(
        "chase_offset",
        "Chase offset",
        "s",
        0.01,
    ));
    params.push(ControlSpec::number("chase_delay", "Chase delay", "f", 1.0));
    // The GUI draws these as its text deck layout; the font keeps its picker
    // hint, which that layout uses too.
    params
        .into_iter()
        .map(|p| match p.widget {
            Some(_) => p,
            None => p.in_widget(WidgetHint::TextDeck),
        })
        .collect()
});

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Config {
    text: String,
    format: Format,
    /// Empty for the platform's sans-serif.
    font: String,
    weight: f32,
    italic: bool,
    align: HAlign,
    valign: VAlign,
    mode: Mode,
    unit: Unit,
    clock: Clock,
    speed: f32,
    #[serde(rename = "loop")]
    looping: bool,
    scroll: f32,
    transition: Transition,
    transition_time: f32,
    rollup_lines: u8,
    size: f32,
    line_spacing: f32,
    position: [f32; 2],
    color: [f32; 4],
    background: [f32; 4],
    voice_colors: [[f32; 4]; 4],
    transport_sync: DeckTransportSync,
    /// A file to import when the deck is created. Never saved: its contents
    /// are copied into `text`.
    #[serde(skip_serializing)]
    file: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            text: "TEXT".into(),
            format: Format::Plain,
            font: String::new(),
            weight: 400.0,
            italic: false,
            align: HAlign::Center,
            valign: VAlign::Middle,
            mode: Mode::Static,
            unit: Unit::Line,
            clock: Clock::Rate,
            speed: 1.0,
            looping: true,
            scroll: 0.0,
            transition: Transition::Fade,
            transition_time: 0.25,
            rollup_lines: 3,
            size: 0.12,
            line_spacing: 1.2,
            position: [0.5, 0.5],
            color: [1.0, 1.0, 1.0, 1.0],
            background: [0.0, 0.0, 0.0, 1.0],
            voice_colors: [
                [1.0, 1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [1.0, 0.0, 1.0, 1.0],
            ],
            transport_sync: DeckTransportSync::default(),
            file: None,
        }
    }
}

fn norm(value: f32, (min, max): (f32, f32)) -> f32 {
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

fn denorm(value: f32, (min, max): (f32, f32)) -> f32 {
    min + value.clamp(0.0, 1.0) * (max - min)
}

fn pick<T: Copy + PartialEq>(options: &[T], value: T) -> f32 {
    let index = options.iter().position(|o| *o == value).unwrap_or(0);
    choice_value(index, options.len())
}

fn chosen<T: Copy>(options: &[T], normalized: f32) -> T {
    options[choice_index(normalized, options.len())]
}

/// Read a text file for import: UTF-8, a leading byte order mark removed, at
/// most [`MAX_TEXT`].
///
/// # Errors
///
/// Fails when the file cannot be read, is not UTF-8, or is too large.
fn read_text_file(path: &str) -> Result<(String, Format)> {
    let bytes = std::fs::read(path).with_context(|| format!("Could not read {path}"))?;
    let text = String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{path} is not UTF-8 text"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_string();
    anyhow::ensure!(
        text.len() <= MAX_TEXT,
        "{path} is {} KiB; a text deck holds at most {} KiB",
        text.len() / 1024,
        MAX_TEXT / 1024
    );
    let format = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Format::from_extension)
        .unwrap_or_default();
    Ok((text, format))
}

/// The text source type.
pub struct TextProvider;

impl DeckSourceProvider for TextProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "Text"
    }

    fn icon(&self) -> &'static str {
        "T"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn library(&self, _query: &SourceQuery) -> LibrarySection {
        let mut options = std::collections::BTreeMap::new();
        if let Some(families) = crate::fonts::families() {
            options.insert("font".to_string(), families.to_vec());
        }
        LibrarySection {
            entries: vec![LibraryEntry::new(
                "Text",
                encode_config(SOURCE_TYPE, &Config::default()),
            )],
            options,
            ..LibrarySection::default()
        }
    }

    fn loader(&self, config: &SourceConfig, _query: &SourceQuery) -> Option<Result<SourceLoader>> {
        Some(decode_config::<Config>(config).and_then(|mut config| {
            if let Some(path) = config.file.take() {
                let (text, format) = read_text_file(&path)?;
                config.text = text;
                config.format = format;
            }
            let loader: SourceLoader = Box::new(move |gpu, width, height| {
                let fonts = crate::fonts::database();
                let mut deck = TextDeck::new(config);
                deck.attach(gpu, width, height)?;
                deck.warm(gpu, &fonts);
                Ok(Box::new(deck) as Box<dyn DeckSourceInstance>)
            });
            Ok(loader)
        }))
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let loader = self
            .loader(config, &env.query())
            .context("text source has a loader")??;
        loader(env.gpu, env.width, env.height)
    }

    fn identity(&self, _config: &SourceConfig) -> serde_json::Value {
        // Changing the text patches the deck instead of rebuilding it.
        serde_json::Value::Null
    }
}

/// The parsed text and what Step moves through.
#[derive(Default)]
struct Doc {
    parsed: Parsed,
    steps: Vec<StepUnit>,
    /// Whether WebVTT cue settings place the cues.
    placed: bool,
    /// Index of each cue's first line among all lines, for crawl chase.
    line_starts: Vec<usize>,
}

impl Doc {
    fn new(config: &Config) -> Self {
        let parsed = cue::parse(config.format, &config.text);
        let steps = layout::step_units(&parsed.cues, config.unit);
        let placed = config.format == Format::WebVtt
            && parsed
                .cues
                .iter()
                .any(|c| c.placement.is_some_and(|p| p.places()));
        let mut line_starts = Vec::with_capacity(parsed.cues.len());
        let mut lines = 0;
        for cue in &parsed.cues {
            line_starts.push(lines);
            lines += cue.lines.len();
        }
        Self {
            parsed,
            steps,
            placed,
            line_starts,
        }
    }

    /// Every string the layout may draw.
    fn strings(&self) -> HashSet<&str> {
        self.parsed
            .cues
            .iter()
            .flat_map(|c| {
                c.lines
                    .iter()
                    .map(String::as_str)
                    .chain(c.words.iter().map(|w| w.text.as_str()))
            })
            .collect()
    }
}

/// The controls' values with this frame's modulation applied.
#[derive(Debug, Clone, Copy)]
struct Live {
    speed: f32,
    scroll: f32,
    size: f32,
    weight: f32,
    line_spacing: f32,
    transition_time: f32,
    position: [f32; 2],
    color: [f32; 4],
    background: [f32; 4],
    voice_colors: [[f32; 4]; 4],
}

impl Live {
    fn from(config: &Config) -> Self {
        Self {
            speed: config.speed,
            scroll: config.scroll,
            size: config.size,
            weight: config.weight,
            line_spacing: config.line_spacing,
            transition_time: config.transition_time,
            position: config.position,
            color: config.color,
            background: config.background,
            voice_colors: config.voice_colors,
        }
    }
}

/// Where the text is in its motion.
#[derive(Debug, Clone, Copy, Default)]
struct Motion {
    /// Integrated or chased advance, in units.
    advance: f64,
    /// Seconds or beats run while free-running, for transitions.
    clock: f64,
    /// Transport seconds past the offset, while chasing.
    elapsed: Option<f64>,
    changed_at: f64,
    /// When the text last changed, on `clock`, for Static's transition.
    replaced_at: Option<f64>,
    view: Option<StepView>,
}

/// Cached masks for one style: per string, one mask per size bucket.
type Masks = HashMap<String, Vec<(i32, Mask)>>;

struct Gpu {
    pass: TextPass,
    max_dim: u32,
    /// The current style's masks, and the previous style's while the current
    /// one fills in.
    masks: HashMap<Style, Masks>,
}

/// One text deck.
pub struct TextDeck {
    config: Config,
    doc: Doc,
    live: Live,
    motion: Motion,
    gpu: Option<Gpu>,
    /// Measured width and word ends per string, by style. Size-independent.
    extents: HashMap<Style, HashMap<String, Extent>>,
    /// The style last drawn, kept as the fallback while a new one rasterizes.
    last_style: Option<Style>,
}

impl TextDeck {
    fn new(config: Config) -> Self {
        let doc = Doc::new(&config);
        let live = Live::from(&config);
        Self {
            config,
            doc,
            live,
            motion: Motion::default(),
            gpu: None,
            extents: HashMap::new(),
            last_style: None,
        }
    }

    /// A deck showing `text`, with no GPU state: for status and UI tests.
    pub fn detached(text: &str) -> Self {
        Self::new(Config {
            text: text.to_string(),
            ..Config::default()
        })
    }

    /// Scroll position and how many quads a `width` by `height` frame lays
    /// out now: for engine-level tests.
    #[cfg(test)]
    pub(crate) fn probe(&self, width: f32, height: f32) -> (f64, usize) {
        let mut quads = Vec::new();
        lay_out(
            &self.doc,
            &self.frame(width, height),
            &self.step_view(),
            None,
            &mut quads,
        );
        (self.scroll_position(), quads.len())
    }

    /// A config for a deck showing `text`.
    pub fn config_for(text: &str) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                text: text.to_string(),
                ..Config::default()
            },
        )
    }

    fn attach(&mut self, gpu: &GpuContext, _width: u32, _height: u32) -> Result<()> {
        self.gpu = Some(Gpu {
            pass: TextPass::new(gpu)?,
            max_dim: gpu.device.limits().max_texture_dimension_2d,
            masks: HashMap::new(),
        });
        Ok(())
    }

    fn style(&self) -> Style {
        Style::new(&self.config.font, self.live.weight, self.config.italic)
    }

    /// Rasterize what the first frame shows, without the frame budget: this
    /// runs on the loader thread.
    fn warm(&mut self, gpu: &GpuContext, fonts: &Arc<usvg::fontdb::Database>) {
        let (width, height) = (1920.0, 1080.0);
        let wanted = self.wanted(width, height);
        let style = self.style();
        let em = self.live.size * height;
        for text in wanted {
            self.rasterize(gpu, fonts, &style, &text, em);
        }
    }

    /// The strings the current frame lays out, visible first.
    fn wanted(&self, width: f32, height: f32) -> Vec<String> {
        let style = self.style();
        let mut quads = Vec::new();
        lay_out(
            &self.doc,
            &self.frame(width, height),
            &self.step_view(),
            self.extents.get(&style),
            &mut quads,
        );
        let mut seen = HashSet::new();
        quads
            .iter()
            .filter(|q| seen.insert(q.text))
            .map(|q| q.text.to_string())
            .collect()
    }

    /// The strings in `quads` with no mask at `bucket` in `style`, visible
    /// first. Empty, and allocation-free, once everything is rasterized.
    fn missing(&self, quads: &[layout::Quad], style: &Style, bucket: i32) -> Vec<String> {
        let masks = self.gpu.as_ref().and_then(|g| g.masks.get(style));
        let extents = self.extents.get(style);
        let mut out: Vec<String> = Vec::new();
        for quad in quads {
            let have = masks
                .and_then(|m| m.get(quad.text))
                .is_some_and(|v| v.iter().any(|(b, _)| *b == bucket));
            // Measured with no ink (all whitespace): nothing to rasterize.
            let blank = extents
                .and_then(|e| e.get(quad.text))
                .is_some_and(|e| e.width == 0.0);
            if !have && !blank && !out.iter().any(|t| t == quad.text) {
                out.push(quad.text.to_string());
            }
        }
        out
    }

    /// Shape and upload `text` at the bucket for `em`, recording its extent.
    fn rasterize(
        &mut self,
        gpu: &GpuContext,
        fonts: &Arc<usvg::fontdb::Database>,
        style: &Style,
        text: &str,
        em: f32,
    ) {
        let Some(state) = self.gpu.as_mut() else {
            return;
        };
        let bucket = raster::size_bucket(em);
        let raster =
            match raster::rasterize(fonts, text, style, raster::bucket_px(bucket), state.max_dim) {
                Ok(Some(raster)) => raster,
                Ok(None) => {
                    self.extents
                        .entry(style.clone())
                        .or_default()
                        .insert(text.to_string(), Extent::new(0.0));
                    return;
                }
                Err(e) => {
                    log::warn!("Could not rasterize text '{text}': {e}");
                    return;
                }
            };
        self.extents.entry(style.clone()).or_default().insert(
            text.to_string(),
            Extent {
                width: raster.advance,
                word_ends: Some(raster.word_ends.clone()),
            },
        );
        let mask = state.pass.upload(gpu, &raster);
        let per_text = state
            .masks
            .entry(style.clone())
            .or_default()
            .entry(text.to_string())
            .or_default();
        per_text.retain(|(b, _)| *b != bucket);
        per_text.push((bucket, mask));
        // Keep the newest and the ones either side of it.
        if per_text.len() > 3 {
            per_text.sort_by_key(|(b, _)| (b - bucket).abs());
            per_text.truncate(3);
        }
    }

    /// Drop cached masks and extents for text not in the document, and for
    /// styles other than the current and previous one.
    fn evict(&mut self) {
        let keep = self.doc.strings();
        let current = self.style();
        let styles = [Some(&current), self.last_style.as_ref()];
        let keep_style = |s: &Style| styles.contains(&Some(s));
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.masks.retain(|s, _| keep_style(s));
            for masks in gpu.masks.values_mut() {
                masks.retain(|t, _| keep.contains(t.as_str()));
            }
        }
        self.extents.retain(|s, _| keep_style(s));
        for extents in self.extents.values_mut() {
            extents.retain(|t, _| keep.contains(t.as_str()));
        }
    }

    fn set_text(&mut self, text: String, format: Option<Format>) -> Result<(), ControlError> {
        if text.len() > MAX_TEXT {
            return Err(ControlError::Invalid(format!(
                "a text deck holds at most {} KiB",
                MAX_TEXT / 1024
            )));
        }
        self.config.text = text;
        if let Some(format) = format {
            self.config.format = format;
        }
        self.reparse();
        self.motion.replaced_at = Some(self.motion.clock);
        Ok(())
    }

    fn reparse(&mut self) {
        self.doc = Doc::new(&self.config);
        self.evict();
    }

    /// Append one plain line, dropping the oldest lines past the cap, and
    /// make it the current unit in Step.
    fn append_line(&mut self, line: &str) -> Result<(), ControlError> {
        if self.config.format != Format::Plain {
            return Err(ControlError::State(
                "lines can only be appended to plain text".into(),
            ));
        }
        let line = line.replace(['\r', '\n'], " ");
        let mut text = std::mem::take(&mut self.config.text);
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line);
        while text.len() > MAX_TEXT {
            match text.find('\n') {
                Some(i) => {
                    text.drain(..=i);
                }
                None => text.truncate(MAX_TEXT),
            }
        }
        self.config.text = text;
        self.reparse();
        let last = self.doc.steps.len().saturating_sub(1) as f64;
        self.motion.advance = last - f64::from(self.live.scroll).floor() + 1e-6;
        Ok(())
    }

    /// Start the free-running motion from the first unit, as a new mode
    /// does.
    fn restart_motion(&mut self) {
        self.motion.advance = 0.0;
        self.motion.view = None;
        self.motion.changed_at = self.motion.clock;
    }

    fn chasing(&self) -> bool {
        self.motion.elapsed.is_some()
    }

    fn scroll_position(&self) -> f64 {
        self.motion.advance + f64::from(self.live.scroll)
    }

    /// The layout frame for a `width` by `height` deck.
    fn frame(&self, width: f32, height: f32) -> layout::Frame<'_> {
        layout::Frame {
            mode: self.config.mode,
            align: self.config.align,
            valign: self.config.valign,
            position: self.live.position,
            width,
            height,
            em: self.live.size * height,
            line_spacing: self.live.line_spacing,
            scroll: self.scroll_position(),
            looping: self.config.looping,
            transition: self.config.transition,
            rollup_lines: usize::from(self.config.rollup_lines.clamp(1, 4)),
            color: self.live.color,
            voice_colors: &self.live.voice_colors,
        }
    }

    fn step_view(&self) -> StepView {
        self.motion.view.unwrap_or(StepView {
            current: (!self.doc.steps.is_empty()).then_some(0),
            previous: None,
            progress: 1.0,
            ending: 0.0,
            words_due: None,
        })
    }

    /// Transition progress for something `since` long ago.
    fn progress(&self, since: f64) -> f32 {
        let time = f64::from(self.live.transition_time);
        if time <= 0.0 {
            1.0
        } else {
            (since / time).clamp(0.0, 1.0) as f32
        }
    }

    /// Work out this frame's step, transition and chase state.
    fn update_view(&mut self) {
        let units = self.doc.steps.len();
        let timed = self.config.format.is_timed();
        let view = match self.motion.elapsed {
            Some(elapsed) if timed => self.timed_view(elapsed),
            _ => {
                let position = self.scroll_position();
                let current = self.step_index(position, units);
                if current != self.motion.view.and_then(|v| v.current) {
                    self.motion.changed_at = self.motion.clock;
                    let previous = self.motion.view.and_then(|v| v.current);
                    self.motion.view = Some(StepView {
                        current,
                        previous,
                        progress: 0.0,
                        ending: 0.0,
                        words_due: None,
                    });
                }
                let since = if self.chasing() && self.live.speed.abs() > f32::EPSILON {
                    // Chasing is deterministic: time since the unit began.
                    position.fract().rem_euclid(1.0) / f64::from(self.live.speed.abs())
                } else {
                    self.motion.clock - self.motion.changed_at
                };
                StepView {
                    progress: self.progress(since),
                    ..self.motion.view.unwrap_or(StepView {
                        current,
                        previous: None,
                        progress: 1.0,
                        ending: 0.0,
                        words_due: None,
                    })
                }
            }
        };
        self.motion.view = Some(view);
    }

    fn step_index(&self, position: f64, units: usize) -> Option<usize> {
        if units == 0 {
            return None;
        }
        let k = position.floor() as i64;
        // A deck's text is capped at 64 KiB, so its unit count fits.
        let n = i64::try_from(units).unwrap_or(i64::MAX);
        let index = if self.config.looping {
            k.rem_euclid(n)
        } else {
            k.clamp(0, n - 1)
        };
        Some(index as usize)
    }

    /// Step state for timed text chasing the transport at `elapsed` seconds.
    fn timed_view(&self, elapsed: f64) -> StepView {
        let cues = &self.doc.parsed.cues;
        let steps = &self.doc.steps;
        let time_of = |u: &StepUnit| {
            let cue = &cues[u.cue];
            u.word
                .and_then(|w| cue.words.get(w).and_then(|w| w.time))
                .or(cue.start)
                .unwrap_or(0.0)
        };
        let index = steps.partition_point(|u| time_of(u) <= elapsed);
        let empty = StepView {
            current: None,
            previous: None,
            progress: 1.0,
            ending: 0.0,
            words_due: None,
        };
        let Some(current) = index.checked_sub(1) else {
            return empty;
        };
        let unit = steps[current];
        let cue = &cues[unit.cue];
        let time = f64::from(self.live.transition_time);
        if let Some(end) = cue.end
            && elapsed >= end
        {
            return empty;
        }
        let ending = cue.end.map_or(0.0, |end| {
            if time <= 0.0 {
                0.0
            } else {
                ((elapsed - (end - time)) / time).clamp(0.0, 1.0) as f32
            }
        });
        let words_due =
            (unit.word.is_none() && cue.words.iter().any(|w| w.time.is_some())).then(|| {
                cue.words
                    .iter()
                    .filter(|w| w.time.is_some_and(|t| t <= elapsed))
                    .count()
            });
        StepView {
            current: Some(current),
            previous: current.checked_sub(1),
            progress: self.progress(elapsed - time_of(&unit)),
            ending,
            words_due,
        }
    }

    /// For timed text chasing in Crawl or Ticker: lines scrolled so far, with
    /// the fraction of the way to the next cue.
    fn timed_line_advance(&self, elapsed: f64) -> f64 {
        let cues = &self.doc.parsed.cues;
        let index = cues.partition_point(|c| c.start.unwrap_or(0.0) <= elapsed);
        let Some(current) = index.checked_sub(1) else {
            return 0.0;
        };
        let start = cues[current].start.unwrap_or(0.0);
        let lines = cues[current].lines.len() as f64;
        let fraction = cues
            .get(current + 1)
            .and_then(|next| next.start)
            .filter(|next| *next > start)
            .map_or(0.0, |next| {
                ((elapsed - start) / (next - start)).clamp(0.0, 1.0)
            });
        self.doc.line_starts[current] as f64 + fraction * lines
    }

    fn inactive_list(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = Vec::new();
        if self.config.mode == Mode::Step && self.doc.placed {
            for name in ["position", "align", "valign"] {
                out.push((name, "Placed by the WebVTT file's cue settings"));
            }
        }
        if self.doc.parsed.voices.is_empty() {
            for name in VOICE_CONTROLS {
                out.push((name, "No speaker tags in this text"));
            }
        }
        if self.config.format != Format::Plain {
            out.push(("line", "Lines can only be appended to plain text"));
        }
        if self.config.transition != Transition::RollUp {
            out.push(("rollup_lines", "Used by the Roll-up transition"));
        }
        if matches!(self.config.mode, Mode::Crawl | Mode::Ticker) {
            out.push(("transition", "Crawl and Ticker move continuously"));
            out.push(("transition_time", "Crawl and Ticker move continuously"));
        }
        out
    }
}

impl DeckSourceInstance for TextDeck {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        let first = self
            .doc
            .parsed
            .cues
            .iter()
            .flat_map(|c| &c.lines)
            .find(|l| !l.trim().is_empty())
            .map_or("Text", |l| l.trim());
        let mut label: String = first.chars().take(32).collect();
        if first.chars().count() > 32 {
            label.push_str("...");
        }
        label
    }

    fn config(&self) -> SourceConfig {
        encode_config(SOURCE_TYPE, &self.config)
    }

    fn patch(&mut self, config: &SourceConfig) {
        let Ok(next) = config.decode::<Config>() else {
            return;
        };
        let reparse = next.text != self.config.text
            || next.format != self.config.format
            || next.unit != self.config.unit;
        let restart = next.mode != self.config.mode;
        self.config = next;
        if restart {
            self.restart_motion();
        }
        self.live = Live::from(&self.config);
        if reparse {
            self.reparse();
        }
    }

    fn control(&mut self, ctx: &mut SourceControl) {
        let c = &self.config;
        self.live = Live {
            speed: denorm(ctx.modulated_norm("speed", norm(c.speed, SPEED)), SPEED),
            scroll: denorm(ctx.modulated_norm("scroll", norm(c.scroll, SCROLL)), SCROLL),
            size: denorm(ctx.modulated_norm("size", norm(c.size, SIZE)), SIZE),
            weight: denorm(ctx.modulated_norm("weight", norm(c.weight, WEIGHT)), WEIGHT),
            line_spacing: denorm(
                ctx.modulated_norm("line_spacing", norm(c.line_spacing, LINE_SPACING)),
                LINE_SPACING,
            ),
            transition_time: denorm(
                ctx.modulated_norm("transition_time", norm(c.transition_time, TRANSITION_TIME)),
                TRANSITION_TIME,
            ),
            position: ctx.modulated_point("position", c.position, POSITION.0, POSITION.1),
            color: ctx.modulated_color("color", c.color),
            background: ctx.modulated_color("background", c.background),
            voice_colors: [0, 1, 2, 3]
                .map(|i| ctx.modulated_color(VOICE_CONTROLS[i], c.voice_colors[i])),
        };

        let sync = self.config.transport_sync;
        self.motion.elapsed = ctx
            .transport
            .filter(|t| sync.mode.is_chasing(t.running))
            .map(|t| {
                let fps = if t.fps > 0.0 { t.fps } else { 30.0 };
                t.position - sync.offset - f64::from(sync.delay_frames) / fps
            });

        if ctx.awake {
            let step = match self.config.clock {
                Clock::Rate => f64::from(ctx.dt),
                Clock::Beat => ctx.beat.map_or(0.0, |b| f64::from(b.dt)),
            };
            self.motion.clock += step;
            match self.motion.elapsed {
                Some(elapsed) if self.config.format.is_timed() => {
                    if matches!(self.config.mode, Mode::Crawl | Mode::Ticker) {
                        self.motion.advance = self.timed_line_advance(elapsed);
                    }
                }
                Some(elapsed) => {
                    self.motion.advance = elapsed * f64::from(self.config.speed);
                }
                // Static doesn't accumulate a position for a later Crawl or Ticker.
                None if self.config.mode != Mode::Static => {
                    self.motion.advance += f64::from(self.live.speed) * step;
                }
                None => {}
            }
            self.update_view();
        }
    }

    fn render(&mut self, frame: &mut SourceFrame) -> Result<()> {
        let (width, height) = (frame.width as f32, frame.height as f32);
        let em = self.live.size * height;
        let bucket = raster::size_bucket(em);
        let style = self.style();
        let view = self.step_view();
        let mut quads = Vec::new();
        lay_out(
            &self.doc,
            &self.frame(width, height),
            &view,
            self.extents.get(&style),
            &mut quads,
        );

        // Rasterize what this frame is missing, visible first, under budget,
        // and lay out again only if anything new was measured.
        let missing = self.missing(&quads, &style, bucket);
        if !missing.is_empty()
            && let Some(fonts) = crate::fonts::ready()
        {
            drop(quads);
            let started = std::time::Instant::now();
            for text in missing {
                if started.elapsed() > RASTER_BUDGET {
                    break;
                }
                self.rasterize(frame.gpu, &fonts, &style, &text, em);
            }
            quads = Vec::new();
            lay_out(
                &self.doc,
                &self.frame(width, height),
                &view,
                self.extents.get(&style),
                &mut quads,
            );
        }
        if self.last_style.as_ref() != Some(&style)
            && self.last_style.replace(style.clone()).is_some()
        {
            drop(quads);
            self.evict();
            quads = Vec::new();
            lay_out(
                &self.doc,
                &self.frame(width, height),
                &view,
                self.extents.get(&style),
                &mut quads,
            );
        }

        let intro = match (self.config.mode, self.motion.replaced_at) {
            (Mode::Static, Some(at)) if self.config.transition != Transition::Cut => {
                self.progress(self.motion.clock - at)
            }
            _ => 1.0,
        };
        let Some(gpu) = self.gpu.as_mut() else {
            return Ok(());
        };
        let advance = em * self.live.line_spacing;
        let draws: Vec<Draw> = quads
            .iter()
            .filter_map(|quad| {
                let mask = pick_mask(&gpu.masks, &style, quad.text, bucket)?;
                let scale = em / mask.px;
                let left = quad.x + mask.origin[0] * em;
                let baseline = quad.y + (advance - em) / 2.0 + ASCENT * em;
                let top = baseline + mask.origin[1] * em;
                let width_em = mask.width as f32 / mask.px;
                Some(Draw {
                    mask,
                    rect: [
                        left,
                        top,
                        left + mask.width as f32 * scale,
                        top + mask.height as f32 * scale,
                    ],
                    color: quad.color,
                    clip_u: (quad.clip.edge - mask.origin[0]) / width_em,
                    keep_after: quad.clip.keep_after,
                    fade: quad.fade * intro,
                })
            })
            .collect();
        gpu.pass.draw(frame, self.live.background, &draws);
        Ok(())
    }

    fn schema(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn param(&self, name: &str) -> Option<ControlValue> {
        let c = &self.config;
        let float = |v| Some(ControlValue::Float(v));
        match name {
            "text" => Some(ControlValue::Text(c.text.clone())),
            "line" | "file" => Some(ControlValue::Text(String::new())),
            "format" => float(pick(&Format::ALL, c.format)),
            "font" => Some(ControlValue::Text(c.font.clone())),
            "weight" => float(norm(c.weight, WEIGHT)),
            "italic" => Some(ControlValue::Bool(c.italic)),
            "align" => float(pick(&ALIGNS, c.align)),
            "valign" => float(pick(&VALIGNS, c.valign)),
            "mode" => float(pick(&MODES, c.mode)),
            "unit" => float(pick(&UNITS, c.unit)),
            "clock" => float(pick(&CLOCKS, c.clock)),
            "speed" => float(norm(c.speed, SPEED)),
            "loop" => Some(ControlValue::Bool(c.looping)),
            "scroll" => float(norm(c.scroll, SCROLL)),
            "transition" => float(pick(&TRANSITIONS, c.transition)),
            "transition_time" => float(norm(c.transition_time, TRANSITION_TIME)),
            "rollup_lines" => float(choice_value(usize::from(c.rollup_lines.clamp(1, 4)) - 1, 4)),
            "size" => float(norm(c.size, SIZE)),
            "line_spacing" => float(norm(c.line_spacing, LINE_SPACING)),
            "position" => Some(ControlValue::Point(c.position)),
            "color" => Some(ControlValue::Color(c.color)),
            "background" => Some(ControlValue::Color(c.background)),
            "chase" => float(pick(&CHASE, c.transport_sync.mode)),
            "chase_offset" => float(c.transport_sync.offset as f32),
            "chase_delay" => float(c.transport_sync.delay_frames as f32),
            _ => VOICE_CONTROLS
                .iter()
                .position(|v| *v == name)
                .map(|i| ControlValue::Color(c.voice_colors[i])),
        }
    }

    fn set_param(&mut self, name: &str, value: &ControlValue) -> Result<(), ControlError> {
        let n = || expect_norm(name, value);
        let c = &mut self.config;
        match name {
            "text" => {
                let text = expect_text(name, value)?.to_string();
                return self.set_text(text, None);
            }
            "line" => return self.append_line(expect_text(name, value)?),
            "file" => {
                let path = expect_text(name, value)?;
                let (text, format) =
                    read_text_file(path).map_err(|e| ControlError::State(e.to_string()))?;
                return self.set_text(text, Some(format));
            }
            "format" => {
                c.format = chosen(&Format::ALL, n()?);
                self.reparse();
                return Ok(());
            }
            "unit" => {
                c.unit = chosen(&UNITS, n()?);
                self.reparse();
                return Ok(());
            }
            "font" => c.font = expect_text(name, value)?.trim().to_string(),
            "weight" => c.weight = denorm(n()?, WEIGHT),
            "italic" => c.italic = n()? > 0.5,
            "align" => c.align = chosen(&ALIGNS, n()?),
            "valign" => c.valign = chosen(&VALIGNS, n()?),
            "mode" => {
                let mode = chosen(&MODES, n()?);
                if mode != c.mode {
                    c.mode = mode;
                    self.restart_motion();
                }
            }
            "clock" => c.clock = chosen(&CLOCKS, n()?),
            "speed" => c.speed = denorm(n()?, SPEED),
            "loop" => c.looping = n()? > 0.5,
            "scroll" => c.scroll = denorm(n()?, SCROLL),
            "transition" => c.transition = chosen(&TRANSITIONS, n()?),
            "transition_time" => c.transition_time = denorm(n()?, TRANSITION_TIME),
            "rollup_lines" => c.rollup_lines = choice_index(n()?, 4) as u8 + 1,
            "size" => c.size = denorm(n()?, SIZE),
            "line_spacing" => c.line_spacing = denorm(n()?, LINE_SPACING),
            "position" => match value {
                ControlValue::Point(p) => {
                    c.position = p.map(|v| v.clamp(POSITION.0, POSITION.1));
                }
                _ => return Err(ControlError::Invalid("'position' takes a point".into())),
            },
            "color" => c.color = expect_color(name, value)?,
            "background" => c.background = expect_color(name, value)?,
            "chase" => c.transport_sync.mode = chosen(&CHASE, n()?),
            "chase_offset" => {
                c.transport_sync.offset =
                    f64::from(value.as_f32().ok_or_else(|| {
                        ControlError::Invalid("'chase_offset' takes seconds".into())
                    })?);
            }
            "chase_delay" => {
                c.transport_sync.delay_frames = value
                    .as_f32()
                    .ok_or_else(|| ControlError::Invalid("'chase_delay' takes frames".into()))?
                    as i32;
            }
            _ => match VOICE_CONTROLS.iter().position(|v| *v == name) {
                Some(i) => c.voice_colors[i] = expect_color(name, value)?,
                None => return Err(ControlError::Unknown(name.to_string())),
            },
        }
        self.live = Live::from(&self.config);
        Ok(())
    }

    fn trigger(&mut self, action: &str) -> Result<(), ControlError> {
        match action {
            "next" => self.motion.advance = self.motion.advance.floor() + 1.0,
            "previous" => self.motion.advance = self.motion.advance.floor() - 1.0,
            "restart" => self.motion.advance = 0.0,
            _ => return Err(ControlError::Unknown(action.to_string())),
        }
        Ok(())
    }

    fn status(&self) -> ControlStatus {
        let mut status = crate::source::status_from_params(self);
        let view = self.step_view();
        let info = &mut status.info;
        info.insert("unit".into(), view.current.into());
        info.insert("units".into(), self.doc.steps.len().into());
        info.insert("chasing".into(), self.chasing().into());
        if let Some(cue) = view
            .current
            .and_then(|i| self.doc.steps.get(i))
            .map(|u| &self.doc.parsed.cues[u.cue])
        {
            info.insert("cue_start".into(), cue.start.into());
            info.insert("cue_end".into(), cue.end.into());
        }
        let problems = &self.doc.parsed.problems;
        if !problems.is_empty() {
            info.insert(
                "problems".into(),
                problems
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .into(),
            );
            let shown: Vec<String> = problems.iter().take(3).map(ToString::to_string).collect();
            let more = problems.len().saturating_sub(3);
            let mut line = shown.join("; ");
            if more > 0 {
                use std::fmt::Write as _;
                let _ = write!(line, "; {more} more");
            }
            status.display.insert("text".into(), line);
        }
        if !self.config.font.is_empty()
            && crate::fonts::families()
                .is_some_and(|f| !crate::fonts::has_family(&f, &self.config.font))
        {
            status.display.insert(
                "font".into(),
                format!(
                    "Not installed: {}. Drawing in sans-serif.",
                    self.config.font
                ),
            );
            info.insert("font_missing".into(), self.config.font.clone().into());
        }
        status.display.insert(
            "speed".into(),
            match self.config.clock {
                Clock::Rate => format!("{:.2} units/s", self.config.speed),
                Clock::Beat => format!("{:.2} units/beat", self.config.speed),
            },
        );
        status.display.insert(
            "transition_time".into(),
            match self.config.clock {
                Clock::Rate => format!("{:.2} s", self.config.transition_time),
                Clock::Beat => format!("{:.2} beats", self.config.transition_time),
            },
        );
        status
            .display
            .insert("weight".into(), format!("{:.0}", self.config.weight));
        status
    }

    fn inactive(&self) -> Vec<(&'static str, &'static str)> {
        self.inactive_list()
    }

    fn owns_alpha(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Lay out `doc` for `frame` into `out`, measuring strings by `extents` and
/// estimating the ones not measured yet.
fn lay_out<'a>(
    doc: &'a Doc,
    frame: &layout::Frame,
    view: &StepView,
    extents: Option<&HashMap<String, Extent>>,
    out: &mut Vec<layout::Quad<'a>>,
) {
    let extent = |t: &str| {
        extents
            .and_then(|e| e.get(t))
            .cloned()
            .unwrap_or_else(|| Extent::estimate(t))
    };
    layout::layout(
        &doc.parsed.cues,
        &doc.steps,
        view,
        doc.placed,
        frame,
        &extent,
        out,
    );
}

/// The best mask for `text`: the wanted bucket in the current style, else the
/// nearest bucket in any cached style.
fn pick_mask<'m>(
    masks: &'m HashMap<Style, Masks>,
    style: &Style,
    text: &str,
    bucket: i32,
) -> Option<&'m Mask> {
    let nearest = |v: &'m Vec<(i32, Mask)>| {
        v.iter()
            .min_by_key(|(b, _)| ((b - bucket).abs(), *b < bucket))
            .map(|(_, m)| m)
    };
    masks
        .get(style)
        .and_then(|m| m.get(text))
        .and_then(nearest)
        .or_else(|| masks.values().filter_map(|m| m.get(text)).find_map(nearest))
}

#[cfg(test)]
mod tests;
