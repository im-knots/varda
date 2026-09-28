//! `.srt` subtitle files. See /spec/text-source.md § Timed text parsing.

use crate::text::cue::{Cue, Parsed, Problem};

/// Parse `text` as an `.srt` file.
pub fn parse(text: &str) -> Parsed {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parsed = Parsed::default();
    for block in blocks(text) {
        parse_block(&block, &mut parsed);
    }
    parsed
        .cues
        .sort_by(|a, b| a.start.unwrap_or(0.0).total_cmp(&b.start.unwrap_or(0.0)));
    parsed
}

/// Runs of non-blank lines, each line with its 1-based line number.
fn blocks(text: &str) -> Vec<Vec<(usize, &str)>> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push((index + 1, line));
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

fn parse_block(block: &[(usize, &str)], parsed: &mut Parsed) {
    // The index line comes before the timing line, and may be missing.
    let timing_at = usize::from(!block[0].1.contains("-->"));
    let Some(&(number, timing_line)) = block.get(timing_at).filter(|(_, l)| l.contains("-->"))
    else {
        parsed.problems.push(Problem {
            line: block[0].0,
            message: "no cue timing".to_string(),
        });
        return;
    };
    let Some((start, end)) = timing(timing_line) else {
        parsed.problems.push(Problem {
            line: number,
            message: "bad cue timing".to_string(),
        });
        return;
    };
    let lines = block[timing_at + 1..]
        .iter()
        .map(|&(_, line)| strip_markup(line))
        .collect();
    let mut cue = Cue::untimed(lines);
    cue.start = Some(start);
    cue.end = Some(end);
    parsed.cues.push(cue);
}

/// `start --> end`, ignoring anything after the end time such as
/// `X1:... X2:...` coordinates.
fn timing(line: &str) -> Option<(f64, f64)> {
    let (left, right) = line.split_once("-->")?;
    let start = timestamp(left.trim())?;
    let end = timestamp(right.split_whitespace().next()?)?;
    (end >= start).then_some((start, end))
}

/// `hh:mm:ss,mmm` or `hh:mm:ss.mmm` in seconds.
fn timestamp(s: &str) -> Option<f64> {
    let mut parts = s.split(':');
    let (hours, minutes, seconds) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let (whole, fraction) = seconds.split_once([',', '.'])?;
    let digits = |t: &str, max: usize| {
        !t.is_empty() && t.len() <= max && t.bytes().all(|b| b.is_ascii_digit())
    };
    if !digits(hours, usize::MAX)
        || !digits(minutes, 2)
        || !digits(whole, 2)
        || !digits(fraction, 3)
    {
        return None;
    }
    let minutes: f64 = minutes.parse().ok()?;
    let whole: f64 = whole.parse().ok()?;
    if minutes >= 60.0 || whole >= 60.0 {
        return None;
    }
    let fraction: f64 = format!("0.{fraction}").parse().ok()?;
    Some(hours.parse::<f64>().ok()? * 3600.0 + minutes * 60.0 + whole + fraction)
}

/// `line` without `<b>`, `<i>`, `<u>` and `<font>` tags or `{\...}` override
/// codes, trimmed. Other angle brackets and braces are kept.
fn strip_markup(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(close) = rest.find('>')
            && is_format_tag(&rest[1..close])
        {
            rest = &rest[close + 1..];
            continue;
        }
        if rest.starts_with("{\\")
            && let Some(close) = rest.find('}')
        {
            rest = &rest[close + 1..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out.trim().to_string()
}

/// Whether the text between `<` and `>` is an opening or closing `b`, `i`,
/// `u` or `font` tag.
fn is_format_tag(inner: &str) -> bool {
    let name = inner.strip_prefix('/').unwrap_or(inner);
    let name = name.split_whitespace().next().unwrap_or("");
    ["b", "i", "u", "font"]
        .iter()
        .any(|tag| name.eq_ignore_ascii_case(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(parsed: &Parsed) -> Vec<f64> {
        parsed.cues.iter().map(|c| c.start.unwrap()).collect()
    }

    #[test]
    fn a_block_is_one_cue_with_start_end_and_lines() {
        let parsed = parse("1\n00:00:01,500 --> 00:00:04,250\nHello world\nsecond line\n");
        assert!(parsed.problems.is_empty());
        let cue = &parsed.cues[0];
        assert_eq!(cue.start, Some(1.5));
        assert_eq!(cue.end, Some(4.25));
        assert_eq!(cue.lines, ["Hello world", "second line"]);
        let words: Vec<_> = cue
            .words
            .iter()
            .map(|w| (w.line, w.text.as_str()))
            .collect();
        assert_eq!(
            words,
            [(0, "Hello"), (0, "world"), (1, "second"), (1, "line")]
        );
        assert!(cue.words.iter().all(|w| w.time.is_none()));
        assert_eq!(cue.placement, None);
        assert_eq!(cue.voice, None);
    }

    #[test]
    fn hours_count_in_the_time() {
        let parsed = parse("1\n01:02:03,004 --> 10:00:00,000\na");
        assert_eq!(parsed.cues[0].start, Some(3723.004));
        assert_eq!(parsed.cues[0].end, Some(36000.0));
    }

    #[test]
    fn several_blocks_are_several_cues() {
        let parsed =
            parse("1\n00:00:01,000 --> 00:00:02,000\na\n\n2\n00:00:03,000 --> 00:00:04,000\nb");
        assert_eq!(starts(&parsed), [1.0, 3.0]);
        assert_eq!(parsed.cues[1].lines, ["b"]);
    }

    #[test]
    fn several_blank_lines_separate_blocks() {
        let parsed = parse(
            "1\n00:00:01,000 --> 00:00:02,000\na\n\n\n  \n2\n00:00:03,000 --> 00:00:04,000\nb",
        );
        assert_eq!(parsed.cues.len(), 2);
        assert!(parsed.problems.is_empty());
    }

    #[test]
    fn a_period_may_separate_the_milliseconds() {
        let parsed = parse("1\n00:00:01.500 --> 00:00:02.000\na");
        assert_eq!(parsed.cues[0].start, Some(1.5));
    }

    #[test]
    fn coordinates_after_the_end_time_are_ignored() {
        let parsed = parse("1\n00:00:01,000 --> 00:00:02,000  X1:100 X2:600 Y1:10 Y2:50\na");
        assert_eq!(parsed.cues[0].end, Some(2.0));
        assert!(parsed.problems.is_empty());
    }

    #[test]
    fn a_missing_index_is_fine() {
        let parsed = parse("00:00:01,000 --> 00:00:02,000\na\n\n00:00:03,000 --> 00:00:04,000\nb");
        assert_eq!(starts(&parsed), [1.0, 3.0]);
        assert!(parsed.problems.is_empty());
    }

    #[test]
    fn a_wrong_index_is_ignored() {
        let parsed = parse("seven\n00:00:01,000 --> 00:00:02,000\na");
        assert_eq!(parsed.cues.len(), 1);
    }

    #[test]
    fn bad_timing_is_reported_on_its_line_and_the_block_skipped() {
        let parsed = parse(
            "1\n00:00:01,000 --> 00:00:02,000\na\n\n2\n00:01,000 --> 00:00:04,000\nb\n\n\
                            3\n00:00:05,000 --> 00:00:06,000\nc",
        );
        assert_eq!(starts(&parsed), [1.0, 5.0]);
        assert_eq!(
            parsed.problems,
            [Problem {
                line: 6,
                message: "bad cue timing".to_string()
            }]
        );
    }

    #[test]
    fn a_block_without_timing_is_reported_on_its_first_line() {
        let parsed = parse("1\n00:00:01,000 --> 00:00:02,000\na\n\n2\nno timing here\n");
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(
            parsed.problems,
            [Problem {
                line: 5,
                message: "no cue timing".to_string()
            }]
        );
    }

    #[test]
    fn out_of_range_or_backward_times_are_bad_timing() {
        let parsed = parse("00:00:61,000 --> 00:00:62,000\na\n\n00:00:05,000 --> 00:00:01,000\nb");
        assert!(parsed.cues.is_empty());
        assert_eq!(parsed.problems.len(), 2);
    }

    #[test]
    fn cues_are_sorted_and_equal_times_keep_source_order() {
        let parsed = parse(
            "1\n00:00:09,000 --> 00:00:10,000\nlate\n\n2\n00:00:01,000 --> 00:00:02,000\n\
                            first\n\n3\n00:00:01,000 --> 00:00:03,000\nsecond",
        );
        let lines: Vec<_> = parsed.cues.iter().map(|c| c.lines[0].as_str()).collect();
        assert_eq!(lines, ["first", "second", "late"]);
    }

    #[test]
    fn format_tags_are_stripped_keeping_their_text() {
        let parsed = parse(
            "1\n00:00:01,000 --> 00:00:02,000\n<b>bold</b> <i>it</i> <U>un</U>\n\
             <font color=\"#ff0000\" face=\"Arial\">red</font>",
        );
        assert_eq!(parsed.cues[0].lines, ["bold it un", "red"]);
        let words: Vec<_> = parsed.cues[0]
            .words
            .iter()
            .map(|w| w.text.as_str())
            .collect();
        assert_eq!(words, ["bold", "it", "un", "red"]);
    }

    #[test]
    fn position_codes_are_stripped() {
        let parsed = parse("1\n00:00:01,000 --> 00:00:02,000\n{\\an8}top {\\pos(10,20)}text");
        assert_eq!(parsed.cues[0].lines, ["top text"]);
    }

    #[test]
    fn other_brackets_and_braces_are_kept() {
        let parsed = parse("1\n00:00:01,000 --> 00:00:02,000\n<3 you {sic} a<b");
        assert_eq!(parsed.cues[0].lines, ["<3 you {sic} a<b"]);
    }

    #[test]
    fn crlf_endings_and_a_bom_are_accepted() {
        let parsed = parse(
            "\u{feff}1\r\n00:00:01,000 --> 00:00:02,000\r\na b\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\nc\r\n",
        );
        assert!(parsed.problems.is_empty());
        assert_eq!(starts(&parsed), [1.0, 3.0]);
        assert_eq!(parsed.cues[0].lines, ["a b"]);
    }
}
