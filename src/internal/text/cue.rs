//! The cue model every text format parses into. Layout, modes, transitions
//! and chase read cues, never the format.

use super::formats;

/// Which format a deck's text is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Format {
    #[default]
    Plain,
    Lrc,
    WebVtt,
    SubRip,
}

impl Format {
    pub const ALL: [Self; 4] = [Self::Plain, Self::Lrc, Self::WebVtt, Self::SubRip];

    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain",
            Self::Lrc => "LRC",
            Self::WebVtt => "WebVTT",
            Self::SubRip => "SubRip",
        }
    }

    /// The format a file's extension names, if any.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "txt" => Some(Self::Plain),
            "lrc" => Some(Self::Lrc),
            "vtt" => Some(Self::WebVtt),
            "srt" => Some(Self::SubRip),
            _ => None,
        }
    }

    /// Whether cues carry times.
    pub fn is_timed(self) -> bool {
        self != Self::Plain
    }
}

/// One piece of text shown as a unit: a line of plain text or LRC, or a
/// WebVTT or SubRip cue, which may span several lines.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cue {
    /// Seconds. `None` for plain text.
    pub start: Option<f64>,
    /// Seconds. WebVTT and SubRip only; an LRC line lasts until the next.
    pub end: Option<f64>,
    /// The cue's lines with markup removed.
    pub lines: Vec<String>,
    /// Every word of every line, in reading order.
    pub words: Vec<Word>,
    /// WebVTT cue settings.
    pub placement: Option<Placement>,
    /// WebVTT speaker, as an index into [`Parsed::voices`].
    pub voice: Option<usize>,
}

impl Cue {
    /// An untimed cue of `lines`, split into untimed words.
    pub fn untimed(lines: Vec<String>) -> Self {
        let words = split_words(&lines);
        Self {
            lines,
            words,
            ..Self::default()
        }
    }
}

/// One word of a cue.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    /// Which of the cue's lines it is on.
    pub line: usize,
    pub text: String,
    /// When the word is sung or spoken, from enhanced LRC or WebVTT inline
    /// timestamps. `None` when the cue has no word timing.
    pub time: Option<f64>,
}

/// Split `lines` into untimed words on whitespace.
pub fn split_words(lines: &[String]) -> Vec<Word> {
    lines
        .iter()
        .enumerate()
        .flat_map(|(line, text)| {
            text.split_whitespace().map(move |w| Word {
                line,
                text: w.to_string(),
                time: None,
            })
        })
        .collect()
}

/// WebVTT cue settings. A field is `None` when the cue did not set it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Placement {
    /// `position:P%`: the cue box's horizontal anchor, 0.0 to 1.0.
    pub position: Option<(f32, PositionAlign)>,
    /// `line:`: the cue box's vertical anchor.
    pub line: Option<(LineValue, LineAlign)>,
    /// `align:`: alignment of the lines inside the box.
    pub align: Option<CueAlign>,
    /// `size:S%`: the box's width, 0.0 to 1.0.
    pub size: Option<f32>,
}

impl Placement {
    /// Whether the cue sets anything that places it.
    pub fn places(&self) -> bool {
        self.position.is_some() || self.line.is_some() || self.align.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineValue {
    /// `line:L%`, 0.0 to 1.0 of the deck height.
    Percent(f32),
    /// `line:N`: line N from the top, or from the bottom when negative.
    Number(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionAlign {
    LineLeft,
    #[default]
    Center,
    LineRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineAlign {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CueAlign {
    Start,
    #[default]
    Center,
    End,
    Left,
    Right,
}

/// Something wrong with one line of the source text. Never fails the parse.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// 1-based line number in the source text.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// A parsed text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    /// Sorted by start time for timed formats; in source order for plain.
    pub cues: Vec<Cue>,
    pub problems: Vec<Problem>,
    /// WebVTT speaker names in order of first appearance.
    pub voices: Vec<String>,
}

/// Parse `text` as `format`.
pub fn parse(format: Format, text: &str) -> Parsed {
    match format {
        Format::Plain => parse_plain(text),
        Format::Lrc => formats::lrc::parse(text),
        Format::WebVtt => formats::webvtt::parse(text),
        Format::SubRip => formats::subrip::parse(text),
    }
}

/// Each line is one untimed cue. Blank lines are kept, so a crawl keeps the
/// spacing the performer typed.
fn parse_plain(text: &str) -> Parsed {
    Parsed {
        cues: text
            .lines()
            .map(|line| Cue::untimed(vec![line.to_string()]))
            .collect(),
        ..Parsed::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_one_untimed_cue_per_line() {
        let parsed = parse(Format::Plain, "hello world\n\nsecond line");
        assert_eq!(parsed.cues.len(), 3);
        assert_eq!(parsed.cues[0].lines, ["hello world"]);
        assert_eq!(parsed.cues[0].start, None);
        let words: Vec<_> = parsed.cues[0]
            .words
            .iter()
            .map(|w| w.text.as_str())
            .collect();
        assert_eq!(words, ["hello", "world"]);
        assert_eq!(parsed.cues[1].words.len(), 0);
        assert_eq!(parsed.problems.len(), 0);
    }

    #[test]
    fn formats_come_from_extensions() {
        assert_eq!(Format::from_extension("LRC"), Some(Format::Lrc));
        assert_eq!(Format::from_extension("vtt"), Some(Format::WebVtt));
        assert_eq!(Format::from_extension("srt"), Some(Format::SubRip));
        assert_eq!(Format::from_extension("txt"), Some(Format::Plain));
        assert_eq!(Format::from_extension("png"), None);
    }
}
