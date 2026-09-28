//! Where each piece of text goes on the deck this frame, for every mode and
//! transition. Pure: no GPU and no fonts. The caller supplies each string's
//! measured extent and draws the quads. See /spec/text-source.md § Modes and
//! § Transitions.

use super::cue::{Cue, CueAlign, LineAlign, LineValue, Placement, PositionAlign};

/// How the text moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    Static,
    Crawl,
    Ticker,
    #[default]
    Step,
}

/// What Step shows at once and triggers move by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Unit {
    #[default]
    Line,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum HAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// How Step changes from one unit to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Transition {
    Cut,
    #[default]
    Fade,
    #[serde(rename = "Roll-up")]
    RollUp,
    #[serde(rename = "Paint-on")]
    PaintOn,
}

/// A string's measured size, in ems (multiples of the font size).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extent {
    pub width: f32,
    /// The right end of each word, in ems. `None` until measured.
    pub word_ends: Option<std::sync::Arc<[f32]>>,
}

impl Extent {
    pub fn new(width: f32) -> Self {
        Self {
            width,
            word_ends: None,
        }
    }

    /// A guess for a string not measured yet: about half an em per character.
    pub fn estimate(text: &str) -> Self {
        Self::new(text.chars().count() as f32 * 0.55)
    }
}

/// Whether a line reads right to left: its first strongly directional
/// character is Hebrew or Arabic script.
pub fn is_rtl(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(|c| matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF))
}

/// How much of a unit Paint-on has revealed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reveal {
    /// All of it.
    All,
    /// This fraction of its lines, in reading order, across all its lines.
    Fraction(f32),
    /// Its first this many words.
    Words(usize),
    /// Erasing from the start: this fraction is gone.
    Erased(f32),
}

/// One piece of text to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Quad<'a> {
    pub text: &'a str,
    /// Left edge and top of the line box, in deck pixels.
    pub x: f32,
    pub y: f32,
    pub color: [f32; 4],
    /// Opacity from a transition, 0 to 1.
    pub fade: f32,
    /// Horizontal part of the string shown, in ems from its left edge, and
    /// whether the part after `edge` is shown rather than the part before.
    pub clip: Clip,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    pub edge: f32,
    pub keep_after: bool,
}

impl Clip {
    pub const NONE: Self = Self {
        edge: f32::INFINITY,
        keep_after: false,
    };
}

/// A unit Step can show: a whole cue, or one word of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepUnit {
    pub cue: usize,
    pub word: Option<usize>,
}

/// The units Step moves through, for `unit`.
pub fn step_units(cues: &[Cue], unit: Unit) -> Vec<StepUnit> {
    match unit {
        Unit::Line => (0..cues.len())
            .map(|cue| StepUnit { cue, word: None })
            .collect(),
        Unit::Word => cues
            .iter()
            .enumerate()
            .flat_map(|(cue, c)| (0..c.words.len()).map(move |w| StepUnit { cue, word: Some(w) }))
            .collect(),
    }
}

/// What Step shows this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepView {
    /// Index into the step units; `None` shows nothing (before the first
    /// timed cue, after a cue's end, an empty text).
    pub current: Option<usize>,
    /// The unit shown before `current`, for Fade.
    pub previous: Option<usize>,
    /// How far the transition into `current` has run, 0 to 1.
    pub progress: f32,
    /// How far `current` has left at its cue's end, 0 to 1.
    pub ending: f32,
    /// Paint-on by word times: the number of the current cue's words due.
    pub words_due: Option<usize>,
}

/// Everything about the deck the layout reads this frame.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub mode: Mode,
    pub align: HAlign,
    pub valign: VAlign,
    /// Anchor in deck-normalized coordinates, (0, 0) at the top left.
    pub position: [f32; 2],
    pub width: f32,
    pub height: f32,
    /// Font size in pixels.
    pub em: f32,
    pub line_spacing: f32,
    pub scroll: f64,
    pub looping: bool,
    pub transition: Transition,
    pub rollup_lines: usize,
    pub color: [f32; 4],
    pub voice_colors: &'a [[f32; 4]; 4],
}

impl Frame<'_> {
    fn advance(&self) -> f32 {
        self.em * self.line_spacing
    }

    fn anchor(&self) -> (f32, f32) {
        (
            self.position[0] * self.width,
            self.position[1] * self.height,
        )
    }

    fn cue_color(&self, cue: &Cue) -> [f32; 4] {
        cue.voice.map_or(self.color, |v| {
            self.voice_colors[v % self.voice_colors.len()]
        })
    }

    fn block_top(&self, lines: usize) -> f32 {
        let height = lines as f32 * self.advance();
        let (_, y) = self.anchor();
        match self.valign {
            VAlign::Top => y,
            VAlign::Middle => y - height / 2.0,
            VAlign::Bottom => y - height,
        }
    }

    fn line_left(&self, width_px: f32) -> f32 {
        let (x, _) = self.anchor();
        match self.align {
            HAlign::Left => x,
            HAlign::Center => x - width_px / 2.0,
            HAlign::Right => x - width_px,
        }
    }

    fn visible_y(&self, y: f32) -> bool {
        y + self.advance() >= 0.0 && y <= self.height
    }
}

/// Every line of every cue, in order: what Static, Crawl and Ticker lay out.
pub fn all_lines(cues: &[Cue]) -> impl Iterator<Item = (&Cue, &str)> {
    cues.iter()
        .flat_map(|cue| cue.lines.iter().map(move |line| (cue, line.as_str())))
}

/// Lay out the frame's quads into `out`.
pub fn layout<'a>(
    cues: &'a [Cue],
    steps: &[StepUnit],
    view: &StepView,
    placed: bool,
    frame: &Frame,
    extent: &dyn Fn(&str) -> Extent,
    out: &mut Vec<Quad<'a>>,
) {
    out.clear();
    match frame.mode {
        Mode::Static => lay_block(cues, frame, 0.0, extent, 1.0, out),
        Mode::Crawl => lay_crawl(cues, frame, extent, out),
        Mode::Ticker => lay_ticker(cues, frame, extent, out),
        Mode::Step => lay_step(cues, steps, view, placed, frame, extent, out),
    }
}

/// All lines as one block, moved up by `scroll` lines.
fn lay_block<'a>(
    cues: &'a [Cue],
    frame: &Frame,
    scroll: f32,
    extent: &dyn Fn(&str) -> Extent,
    fade: f32,
    out: &mut Vec<Quad<'a>>,
) {
    let count = all_lines(cues).count();
    let top = frame.block_top(count) - scroll * frame.advance();
    for (i, (cue, line)) in all_lines(cues).enumerate() {
        let y = top + i as f32 * frame.advance();
        if line.is_empty() || !frame.visible_y(y) {
            continue;
        }
        let width = extent(line).width * frame.em;
        out.push(Quad {
            text: line,
            x: frame.line_left(width),
            y,
            color: frame.cue_color(cue),
            fade,
            clip: Clip::NONE,
        });
    }
}

fn lay_crawl<'a>(
    cues: &'a [Cue],
    frame: &Frame,
    extent: &dyn Fn(&str) -> Extent,
    out: &mut Vec<Quad<'a>>,
) {
    let count = all_lines(cues).count();
    if count == 0 {
        return;
    }
    let scroll = frame.scroll as f32;
    if !frame.looping {
        lay_block(cues, frame, scroll, extent, 1.0, out);
        return;
    }
    // The block, then a screen of empty space, then the block again: it has
    // left the deck before its next copy comes up from the bottom.
    let gap = (frame.height / frame.advance()).ceil();
    let period = count as f32 + gap;
    let scroll = scroll.rem_euclid(period);
    lay_block(cues, frame, scroll, extent, 1.0, out);
    lay_block(cues, frame, scroll - period, extent, 1.0, out);
}

fn lay_ticker<'a>(
    cues: &'a [Cue],
    frame: &Frame,
    extent: &dyn Fn(&str) -> Extent,
    out: &mut Vec<Quad<'a>>,
) {
    let gap = 2.0 * frame.advance();
    let lines: Vec<(&Cue, &str, f32)> = all_lines(cues)
        .map(|(cue, line)| (cue, line, extent(line).width * frame.em))
        .collect();
    if lines.is_empty() {
        return;
    }
    // Scroll is in lines: line k passes in the time it takes its own width
    // and the gap after it to go by.
    let length: f32 = lines.iter().map(|(_, _, w)| w + gap).sum();
    let mut offset = offset_px(&lines, frame.scroll, gap, length);
    let period = length + frame.width;
    if frame.looping {
        offset = offset.rem_euclid(period);
    }
    let (x0, _) = frame.anchor();
    let y = frame.block_top(1);
    let copies: &[f32] = if frame.looping { &[0.0, 1.0] } else { &[0.0] };
    for &copy in copies {
        let mut x = x0 - offset + copy * period;
        for &(cue, line, width) in &lines {
            if x + width >= 0.0 && x <= frame.width && !line.is_empty() {
                out.push(Quad {
                    text: line,
                    x,
                    y,
                    color: frame.cue_color(cue),
                    fade: 1.0,
                    clip: Clip::NONE,
                });
            }
            x += width + gap;
        }
    }
}

/// How far the ticker row has moved for a scroll of `scroll` lines.
fn offset_px(lines: &[(&Cue, &str, f32)], scroll: f64, gap: f32, length: f32) -> f32 {
    let whole = scroll.floor();
    let frac = (scroll - whole) as f32;
    if whole < 0.0 {
        let first = lines[0].2 + gap;
        return scroll as f32 * first;
    }
    let whole = whole as usize;
    if whole >= lines.len() {
        // Past the end, keep moving at the average line's pace.
        let average = length / lines.len() as f32;
        return length + (scroll as f32 - lines.len() as f32) * average;
    }
    let before: f32 = lines[..whole].iter().map(|(_, _, w)| w + gap).sum();
    before + frac * (lines[whole].2 + gap)
}

/// The lines a step unit shows.
fn unit_lines(cues: &[Cue], unit: StepUnit) -> Vec<&str> {
    let cue = &cues[unit.cue];
    match unit.word {
        Some(w) => cue
            .words
            .get(w)
            .map(|w| w.text.as_str())
            .into_iter()
            .collect(),
        None => cue.lines.iter().map(String::as_str).collect(),
    }
}

fn lay_step<'a>(
    cues: &'a [Cue],
    steps: &[StepUnit],
    view: &StepView,
    placed: bool,
    frame: &Frame,
    extent: &dyn Fn(&str) -> Extent,
    out: &mut Vec<Quad<'a>>,
) {
    let Some(current) = view.current.and_then(|i| steps.get(i).map(|u| (i, *u))) else {
        return;
    };
    let entering = view.progress.clamp(0.0, 1.0);
    let leaving = 1.0 - view.ending.clamp(0.0, 1.0);
    match frame.transition {
        Transition::Cut => {
            lay_unit(
                cues,
                current.1,
                placed,
                frame,
                extent,
                1.0,
                Reveal::All,
                out,
            );
        }
        Transition::Fade => {
            if entering < 1.0
                && let Some(prev) = view.previous.and_then(|i| steps.get(i))
            {
                lay_unit(
                    cues,
                    *prev,
                    placed,
                    frame,
                    extent,
                    1.0 - entering,
                    Reveal::All,
                    out,
                );
            }
            lay_unit(
                cues,
                current.1,
                placed,
                frame,
                extent,
                entering * leaving,
                Reveal::All,
                out,
            );
        }
        Transition::PaintOn => {
            let reveal = if view.ending > 0.0 {
                Reveal::Erased(view.ending.clamp(0.0, 1.0))
            } else if let Some(due) = view.words_due {
                Reveal::Words(due)
            } else {
                Reveal::Fraction(entering)
            };
            lay_unit(cues, current.1, placed, frame, extent, 1.0, reveal, out);
        }
        Transition::RollUp => lay_rollup(cues, steps, current.0, entering, frame, extent, out),
    }
}

/// A window of the last `rollup_lines` units, newest at the bottom, moving
/// up by the newest unit's height as it rolls in. The oldest fades as it
/// leaves the top.
fn lay_rollup<'a>(
    cues: &'a [Cue],
    steps: &[StepUnit],
    current: usize,
    entering: f32,
    frame: &Frame,
    extent: &dyn Fn(&str) -> Extent,
    out: &mut Vec<Quad<'a>>,
) {
    let window = frame.rollup_lines.max(1);
    let first = current.saturating_sub(window);
    let units: Vec<StepUnit> = steps[first..=current].to_vec();
    let rows: Vec<usize> = units
        .iter()
        .map(|u| unit_lines(cues, *u).len().max(1))
        .collect();
    let newest_rows = *rows.last().unwrap_or(&1) as f32;
    let shown_rows: usize = rows.iter().rev().take(window).sum();
    // Bottom of the window sits where a block of `shown_rows` would end.
    let bottom = frame.block_top(shown_rows) + shown_rows as f32 * frame.advance();
    let lift = (1.0 - entering) * newest_rows * frame.advance();
    let mut y_bottom = bottom + lift;
    let leaving = units.len() > window;
    for (i, unit) in units.iter().enumerate().rev() {
        let height = rows[i] as f32 * frame.advance();
        let top = y_bottom - height;
        let fade = if leaving && i == 0 {
            1.0 - entering
        } else {
            1.0
        };
        if fade > 0.0 {
            lay_lines(cues, *unit, top, frame, extent, fade, Reveal::All, out);
        }
        y_bottom = top;
    }
}

/// One step unit as a block at its anchor (or where its cue places it).
#[allow(clippy::too_many_arguments)]
fn lay_unit<'a>(
    cues: &'a [Cue],
    unit: StepUnit,
    placed: bool,
    frame: &Frame,
    extent: &dyn Fn(&str) -> Extent,
    fade: f32,
    reveal: Reveal,
    out: &mut Vec<Quad<'a>>,
) {
    if fade <= 0.0 {
        return;
    }
    let rows = unit_lines(cues, unit).len();
    if placed {
        let placement = cues[unit.cue].placement.unwrap_or_default();
        lay_placed(cues, unit, &placement, frame, extent, fade, reveal, out);
    } else {
        let top = frame.block_top(rows);
        lay_lines(cues, unit, top, frame, extent, fade, reveal, out);
    }
}

/// A unit's lines stacked from `top`, aligned by the deck's `align`.
#[allow(clippy::too_many_arguments)]
fn lay_lines<'a>(
    cues: &'a [Cue],
    unit: StepUnit,
    top: f32,
    frame: &Frame,
    extent: &dyn Fn(&str) -> Extent,
    fade: f32,
    reveal: Reveal,
    out: &mut Vec<Quad<'a>>,
) {
    let lines = unit_lines(cues, unit);
    let extents: Vec<Extent> = lines.iter().map(|l| extent(l)).collect();
    let clips = clips(cues, unit, &lines, &extents, reveal);
    let color = frame.cue_color(&cues[unit.cue]);
    for (row, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let width = extents[row].width * frame.em;
        out.push(Quad {
            text: line,
            x: frame.line_left(width),
            y: top + row as f32 * frame.advance(),
            color,
            fade,
            clip: clips[row],
        });
    }
}

/// A WebVTT-placed cue. See /spec/text-source.md § WebVTT cue placement.
#[allow(clippy::too_many_arguments)]
fn lay_placed<'a>(
    cues: &'a [Cue],
    unit: StepUnit,
    placement: &Placement,
    frame: &Frame,
    extent: &dyn Fn(&str) -> Extent,
    fade: f32,
    reveal: Reveal,
    out: &mut Vec<Quad<'a>>,
) {
    let lines = unit_lines(cues, unit);
    let advance = frame.advance();
    let block = lines.len() as f32 * advance;
    let box_width = placement.size.unwrap_or(1.0).clamp(0.0, 1.0) * frame.width;
    let (position, position_align) = placement.position.unwrap_or((0.5, PositionAlign::Center));
    let anchor_x = position * frame.width;
    let box_left = match position_align {
        PositionAlign::LineLeft => anchor_x,
        PositionAlign::Center => anchor_x - box_width / 2.0,
        PositionAlign::LineRight => anchor_x - box_width,
    };
    let top = match placement.line {
        None => frame.height - block,
        Some((LineValue::Number(n), _)) if n >= 0 => n as f32 * advance,
        Some((LineValue::Number(n), _)) => frame.height + (n + 1) as f32 * advance - block,
        Some((LineValue::Percent(p), align)) => {
            let y = p * frame.height;
            match align {
                LineAlign::Start => y,
                LineAlign::Center => y - block / 2.0,
                LineAlign::End => y - block,
            }
        }
    };
    let extents: Vec<Extent> = lines.iter().map(|l| extent(l)).collect();
    let clips = clips(cues, unit, &lines, &extents, reveal);
    let color = frame.cue_color(&cues[unit.cue]);
    for (row, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let width = extents[row].width * frame.em;
        let x = match placement.align.unwrap_or_default() {
            CueAlign::Start | CueAlign::Left => box_left,
            CueAlign::Center => box_left + (box_width - width) / 2.0,
            CueAlign::End | CueAlign::Right => box_left + box_width - width,
        };
        out.push(Quad {
            text: line,
            x,
            y: top + row as f32 * advance,
            color,
            fade,
            clip: clips[row],
        });
    }
}

/// Per-line clips for a reveal across a unit's lines, in reading order. A
/// right-to-left line is revealed from its right edge.
fn clips(
    cues: &[Cue],
    unit: StepUnit,
    lines: &[&str],
    extents: &[Extent],
    reveal: Reveal,
) -> Vec<Clip> {
    let widths: Vec<f32> = extents.iter().map(|e| e.width).collect();
    let total: f32 = widths.iter().sum();
    let by_distance = |shown: f32, keep_after: bool| -> Vec<Clip> {
        let mut left = shown;
        widths
            .iter()
            .map(|&w| {
                let edge = left.clamp(0.0, w);
                left -= w;
                Clip { edge, keep_after }
            })
            .collect()
    };
    let reading_order = match reveal {
        Reveal::All => return vec![Clip::NONE; widths.len()],
        Reveal::Fraction(f) => by_distance(f.clamp(0.0, 1.0) * total, false),
        Reveal::Erased(f) => by_distance(f.clamp(0.0, 1.0) * total, true),
        Reveal::Words(due) if unit.word.is_some() => {
            let edge = if due > 0 { f32::INFINITY } else { 0.0 };
            vec![
                Clip {
                    edge,
                    keep_after: false
                };
                widths.len()
            ]
        }
        Reveal::Words(due) => {
            // Each line shows up to the end of its last due word.
            let cue = &cues[unit.cue];
            (0..widths.len())
                .map(|row| {
                    let due_on_row = cue
                        .words
                        .iter()
                        .take(due)
                        .filter(|word| word.line == row)
                        .count();
                    Clip {
                        edge: word_end(lines[row], &extents[row], due_on_row),
                        keep_after: false,
                    }
                })
                .collect()
        }
    };
    reading_order
        .into_iter()
        .zip(lines.iter().zip(&widths))
        .map(|(clip, (line, &w))| {
            if is_rtl(line) && clip.edge.is_finite() {
                Clip {
                    edge: w - clip.edge,
                    keep_after: !clip.keep_after,
                }
            } else {
                clip
            }
        })
        .collect()
}

/// Where the first `words` words of `line` end, in ems: measured when the
/// extent has word ends, otherwise by the share of characters before it.
fn word_end(line: &str, extent: &Extent, words: usize) -> f32 {
    if words == 0 {
        return 0.0;
    }
    if let Some(ends) = &extent.word_ends
        && let Some(&end) = ends.get(words - 1)
    {
        return end;
    }
    let chars = line.chars().count().max(1) as f32;
    extent.width * word_end_chars(line, words) as f32 / chars
}

/// Characters up to the end of the first `words` words of `line`.
fn word_end_chars(line: &str, words: usize) -> usize {
    if words == 0 {
        return 0;
    }
    let mut seen = 0;
    let mut in_word = false;
    for (i, c) in line.chars().enumerate() {
        if c.is_whitespace() {
            if in_word {
                seen += 1;
                if seen == words {
                    return i;
                }
            }
            in_word = false;
        } else {
            in_word = true;
        }
    }
    line.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cues(lines: &[&str]) -> Vec<Cue> {
        lines
            .iter()
            .map(|l| Cue::untimed(vec![(*l).to_string()]))
            .collect()
    }

    const VOICES: [[f32; 4]; 4] = [[1.0, 1.0, 0.0, 1.0]; 4];

    fn frame(mode: Mode) -> Frame<'static> {
        Frame {
            mode,
            align: HAlign::Left,
            valign: VAlign::Top,
            position: [0.0, 0.0],
            width: 1000.0,
            height: 500.0,
            em: 100.0,
            line_spacing: 1.0,
            scroll: 0.0,
            looping: false,
            transition: Transition::Cut,
            rollup_lines: 2,
            color: [1.0; 4],
            voice_colors: &VOICES,
        }
    }

    fn width_one(_: &str) -> Extent {
        Extent::new(1.0)
    }

    fn view(current: usize) -> StepView {
        StepView {
            current: Some(current),
            previous: None,
            progress: 1.0,
            ending: 0.0,
            words_due: None,
        }
    }

    fn run<'a>(cues: &'a [Cue], unit: Unit, view: &StepView, frame: &Frame) -> Vec<Quad<'a>> {
        let steps = step_units(cues, unit);
        let mut out = Vec::new();
        layout(cues, &steps, view, false, frame, &width_one, &mut out);
        out
    }

    #[test]
    fn static_stacks_lines_from_the_anchor() {
        let cues = cues(&["a", "b", "c"]);
        let quads = run(&cues, Unit::Line, &view(0), &frame(Mode::Static));
        let ys: Vec<f32> = quads.iter().map(|q| q.y).collect();
        assert_eq!(ys, [0.0, 100.0, 200.0]);
    }

    #[test]
    fn alignment_picks_the_anchor_point_of_the_block() {
        let cues = cues(&["a", "b"]);
        let mut f = frame(Mode::Static);
        f.position = [0.5, 0.5];
        f.align = HAlign::Center;
        f.valign = VAlign::Middle;
        let quads = run(&cues, Unit::Line, &view(0), &f);
        // Two 100 px lines centered on (500, 250); each 100 px wide.
        assert_eq!((quads[0].x, quads[0].y), (450.0, 150.0));
        assert_eq!(quads[1].y, 250.0);
        f.align = HAlign::Right;
        f.valign = VAlign::Bottom;
        let quads = run(&cues, Unit::Line, &view(0), &f);
        assert_eq!((quads[0].x, quads[0].y), (400.0, 50.0));
    }

    #[test]
    fn crawl_moves_the_block_up_one_line_per_unit() {
        let cues = cues(&["a", "b", "c"]);
        let mut f = frame(Mode::Crawl);
        f.scroll = 1.5;
        let quads = run(&cues, Unit::Line, &view(0), &f);
        // "a" is fully above the deck and culled; "b" is half off the top.
        assert_eq!(quads[0].text, "b");
        assert_eq!(quads[0].y, -50.0);
        assert_eq!(quads.len(), 2);
    }

    #[test]
    fn a_looping_crawl_comes_back_from_the_bottom() {
        let cues = cues(&["a", "b"]);
        let mut f = frame(Mode::Crawl);
        f.looping = true;
        // Two lines plus a 5-line screen gap: the period is 7 lines.
        f.scroll = 6.5;
        let quads = run(&cues, Unit::Line, &view(0), &f);
        assert!(
            quads.iter().any(|q| q.text == "a" && q.y == 50.0),
            "{quads:?}"
        );
        f.scroll = 13.5;
        let again = run(&cues, Unit::Line, &view(0), &f);
        assert_eq!(quads, again);
    }

    #[test]
    fn a_non_looping_crawl_ends_empty() {
        let cues = cues(&["a", "b"]);
        let mut f = frame(Mode::Crawl);
        f.scroll = 3.0;
        assert!(run(&cues, Unit::Line, &view(0), &f).is_empty());
    }

    #[test]
    fn ticker_passes_each_line_at_its_own_width() {
        let cues = cues(&["a", "b"]);
        let mut f = frame(Mode::Ticker);
        let wide = |t: &str| Extent::new(if t == "a" { 3.0 } else { 1.0 });
        let steps = step_units(&cues, Unit::Line);
        let mut out = Vec::new();
        f.scroll = 1.0;
        layout(&cues, &steps, &view(0), false, &f, &wide, &mut out);
        // Line "a" (300 px) plus a 200 px gap has gone by.
        let b = out.iter().find(|q| q.text == "b").unwrap();
        assert_eq!(b.x, 0.0);
        f.scroll = 0.5;
        layout(&cues, &steps, &view(0), false, &f, &wide, &mut out);
        assert_eq!(out[0].x, -250.0);
    }

    #[test]
    fn step_shows_one_unit_and_word_units_split_cues() {
        let cues = cues(&["hello there", "bye"]);
        let quads = run(&cues, Unit::Line, &view(1), &frame(Mode::Step));
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].text, "bye");
        let quads = run(&cues, Unit::Word, &view(1), &frame(Mode::Step));
        assert_eq!(quads[0].text, "there");
        assert_eq!(step_units(&cues, Unit::Word).len(), 3);
    }

    #[test]
    fn nothing_current_draws_nothing() {
        let cues = cues(&["a"]);
        let mut v = view(0);
        v.current = None;
        assert!(run(&cues, Unit::Line, &v, &frame(Mode::Step)).is_empty());
    }

    #[test]
    fn fade_crosses_the_outgoing_and_incoming_units() {
        let cues = cues(&["a", "b"]);
        let mut f = frame(Mode::Step);
        f.transition = Transition::Fade;
        let v = StepView {
            current: Some(1),
            previous: Some(0),
            progress: 0.25,
            ending: 0.0,
            words_due: None,
        };
        let quads = run(&cues, Unit::Line, &v, &f);
        assert_eq!(quads.len(), 2);
        assert_eq!((quads[0].text, quads[0].fade), ("a", 0.75));
        assert_eq!((quads[1].text, quads[1].fade), ("b", 0.25));
    }

    #[test]
    fn fade_out_at_a_cue_end() {
        let cues = cues(&["a"]);
        let mut f = frame(Mode::Step);
        f.transition = Transition::Fade;
        let mut v = view(0);
        v.ending = 0.4;
        let quads = run(&cues, Unit::Line, &v, &f);
        assert!((quads[0].fade - 0.6).abs() < 1e-6);
    }

    #[test]
    fn paint_on_reveals_across_lines_in_reading_order() {
        let cue = Cue::untimed(vec!["ab".into(), "cd".into()]);
        let cues = vec![cue];
        let mut f = frame(Mode::Step);
        f.transition = Transition::PaintOn;
        let mut v = view(0);
        v.progress = 0.75;
        let quads = run(&cues, Unit::Line, &v, &f);
        assert_eq!(quads[0].clip.edge, 1.0);
        assert_eq!(quads[1].clip.edge, 0.5);
        assert!(!quads[0].clip.keep_after);
        v.ending = 0.25;
        let quads = run(&cues, Unit::Line, &v, &f);
        assert!(quads[0].clip.keep_after);
        assert_eq!(quads[0].clip.edge, 0.5);
    }

    #[test]
    fn paint_on_by_word_times_reveals_whole_words() {
        let cues = vec![Cue::untimed(vec!["one two three".into()])];
        let mut f = frame(Mode::Step);
        f.transition = Transition::PaintOn;
        let mut v = view(0);
        v.words_due = Some(2);
        let quads = run(&cues, Unit::Line, &v, &f);
        // "one two" is 7 of 13 characters.
        assert!((quads[0].clip.edge - 7.0 / 13.0).abs() < 1e-6);
    }

    #[test]
    fn paint_on_uses_measured_word_ends_when_known() {
        let cues = vec![Cue::untimed(vec!["one two three".into()])];
        let mut f = frame(Mode::Step);
        f.transition = Transition::PaintOn;
        let mut v = view(0);
        v.words_due = Some(1);
        let measured = |_: &str| Extent {
            width: 6.0,
            word_ends: Some(vec![1.5, 3.0, 6.0].into()),
        };
        let steps = step_units(&cues, Unit::Line);
        let mut out = Vec::new();
        layout(&cues, &steps, &v, false, &f, &measured, &mut out);
        assert_eq!(out[0].clip.edge, 1.5);
    }

    #[test]
    fn right_to_left_lines_are_revealed_from_the_right() {
        let cues = vec![Cue::untimed(vec!["שלום".into()])];
        let mut f = frame(Mode::Step);
        f.transition = Transition::PaintOn;
        let mut v = view(0);
        v.progress = 0.25;
        let quads = run(&cues, Unit::Line, &v, &f);
        assert!(quads[0].clip.keep_after);
        assert_eq!(quads[0].clip.edge, 0.75);
        assert!(is_rtl("مرحبا"));
        assert!(!is_rtl("hello"));
    }

    #[test]
    fn roll_up_keeps_a_window_newest_at_the_bottom() {
        let cues = cues(&["a", "b", "c"]);
        let mut rollup = frame(Mode::Step);
        rollup.transition = Transition::RollUp;
        rollup.rollup_lines = 2;
        let quads = run(&cues, Unit::Line, &view(2), &rollup);
        let texts: Vec<&str> = quads
            .iter()
            .filter(|q| q.fade > 0.0)
            .map(|q| q.text)
            .collect();
        assert_eq!(texts, ["c", "b"]);
        let newest = quads.iter().find(|q| q.text == "c").unwrap();
        let second = quads.iter().find(|q| q.text == "b").unwrap();
        assert_eq!(newest.y - second.y, 100.0);

        // Halfway in: the window sits half a line low and the oldest fades.
        let mut halfway = view(2);
        halfway.progress = 0.5;
        let quads = run(&cues, Unit::Line, &halfway, &rollup);
        let rolling = quads.iter().find(|q| q.text == "c").unwrap();
        assert_eq!(rolling.y - newest.y, 50.0);
        let oldest = quads.iter().find(|q| q.text == "a").unwrap();
        assert_eq!(oldest.fade, 0.5);
    }

    #[test]
    fn a_speaker_colors_its_cue() {
        let mut cues = cues(&["a"]);
        cues[0].voice = Some(0);
        let quads = run(&cues, Unit::Line, &view(0), &frame(Mode::Step));
        assert_eq!(quads[0].color, VOICES[0]);
    }

    #[test]
    fn webvtt_placement_positions_the_cue_box() {
        let mut cue = Cue::untimed(vec!["x".into()]);
        cue.placement = Some(Placement {
            position: Some((0.1, PositionAlign::LineLeft)),
            line: Some((LineValue::Percent(0.2), LineAlign::Start)),
            align: Some(CueAlign::Start),
            size: Some(0.5),
        });
        let cues = vec![cue];
        let steps = step_units(&cues, Unit::Line);
        let mut out = Vec::new();
        layout(
            &cues,
            &steps,
            &view(0),
            true,
            &frame(Mode::Step),
            &width_one,
            &mut out,
        );
        assert_eq!((out[0].x, out[0].y), (100.0, 100.0));
    }

    #[test]
    fn webvtt_defaults_put_an_unplaced_cue_on_the_bottom_line_centered() {
        let cues = vec![Cue::untimed(vec!["x".into()])];
        let steps = step_units(&cues, Unit::Line);
        let mut out = Vec::new();
        layout(
            &cues,
            &steps,
            &view(0),
            true,
            &frame(Mode::Step),
            &width_one,
            &mut out,
        );
        assert_eq!((out[0].x, out[0].y), (450.0, 400.0));
    }

    #[test]
    fn webvtt_line_numbers_count_from_top_and_bottom() {
        let mut cue = Cue::untimed(vec!["x".into()]);
        cue.placement = Some(Placement {
            line: Some((LineValue::Number(-2), LineAlign::Start)),
            ..Placement::default()
        });
        let cues = vec![cue];
        let steps = step_units(&cues, Unit::Line);
        let mut out = Vec::new();
        layout(
            &cues,
            &steps,
            &view(0),
            true,
            &frame(Mode::Step),
            &width_one,
            &mut out,
        );
        assert_eq!(out[0].y, 300.0);
    }
}
