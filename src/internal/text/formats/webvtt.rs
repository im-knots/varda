//! `.vtt` caption files.

use crate::text::cue::{
    Cue, CueAlign, LineAlign, LineValue, Parsed, Placement, PositionAlign, Problem, Word,
};

/// Parse `text` as a `.vtt` file.
pub fn parse(text: &str) -> Parsed {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parsed = Parsed::default();
    let mut blocks = blocks(text);
    if text.starts_with("WEBVTT") {
        // The header block may carry metadata lines; none of it is used.
        if !blocks.is_empty() {
            blocks.remove(0);
        }
    } else {
        problem(&mut parsed, 1, "missing WEBVTT header");
    }
    for block in blocks {
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

fn problem(parsed: &mut Parsed, line: usize, message: &str) {
    parsed.problems.push(Problem {
        line,
        message: message.to_string(),
    });
}

/// Whether `line` opens a block of kind `keyword` (`NOTE`, `STYLE`, `REGION`).
fn opens(line: &str, keyword: &str) -> bool {
    line.strip_prefix(keyword)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
}

fn parse_block(block: &[(usize, &str)], parsed: &mut Parsed) {
    let (first_number, first) = block[0];
    if ["NOTE", "STYLE", "REGION"]
        .iter()
        .any(|keyword| opens(first, keyword))
    {
        return;
    }
    // An identifier line may come before the timing line.
    let timing_at = usize::from(!first.contains("-->"));
    let Some(&(number, timing_line)) = block.get(timing_at).filter(|(_, l)| l.contains("-->"))
    else {
        problem(parsed, first_number, "no cue timing");
        return;
    };
    let Some((start, end, settings)) = timing(timing_line) else {
        problem(parsed, number, "bad cue timing");
        return;
    };
    let placement = placement(settings, number, parsed);
    let lines: Vec<&str> = block[timing_at + 1..].iter().map(|&(_, l)| l).collect();
    let mut cue = cue_text(&lines, start);
    cue.start = Some(start);
    cue.end = Some(end);
    cue.placement = placement;
    cue.voice = voice(lines.first().copied()).map(|name| voice_index(parsed, name));
    parsed.cues.push(cue);
}

/// `start --> end [settings]`, with the settings text after the end time.
fn timing(line: &str) -> Option<(f64, f64, &str)> {
    let (left, right) = line.split_once("-->")?;
    let start = timestamp(left.trim())?;
    let right = right.trim_start();
    let (end_text, settings) = right.split_once(char::is_whitespace).unwrap_or((right, ""));
    let end = timestamp(end_text)?;
    (end >= start).then_some((start, end, settings))
}

/// `[hh:]mm:ss.ttt` in seconds.
fn timestamp(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [m, s] => ("0", *m, *s),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    let (whole, fraction) = seconds.split_once('.')?;
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

/// Cue settings. `None` unless at least one setting was recognized.
fn placement(settings: &str, number: usize, parsed: &mut Parsed) -> Option<Placement> {
    let mut placement = Placement::default();
    for setting in settings.split_whitespace() {
        let Some((key, value)) = setting.split_once(':') else {
            continue;
        };
        match key {
            "position" => placement.position = position(value),
            "line" => placement.line = line_setting(value),
            "align" => placement.align = align(value),
            "size" => placement.size = percent(value),
            "vertical" => problem(parsed, number, "vertical text is not supported"),
            "region" => problem(parsed, number, "regions are not supported"),
            _ => {}
        }
    }
    (placement.position.is_some()
        || placement.line.is_some()
        || placement.align.is_some()
        || placement.size.is_some())
    .then_some(placement)
}

/// `P%` as a fraction from 0 to 1.
fn percent(value: &str) -> Option<f32> {
    let number: f32 = value.strip_suffix('%')?.parse().ok()?;
    (0.0..=100.0).contains(&number).then_some(number / 100.0)
}

fn position(value: &str) -> Option<(f32, PositionAlign)> {
    let (at, align) = value.split_once(',').unwrap_or((value, "center"));
    let align = match align {
        "line-left" => PositionAlign::LineLeft,
        "center" => PositionAlign::Center,
        "line-right" => PositionAlign::LineRight,
        _ => return None,
    };
    Some((percent(at)?, align))
}

fn line_setting(value: &str) -> Option<(LineValue, LineAlign)> {
    let (at, align) = value.split_once(',').unwrap_or((value, "start"));
    let align = match align {
        "start" => LineAlign::Start,
        "center" => LineAlign::Center,
        "end" => LineAlign::End,
        _ => return None,
    };
    let at = if at.ends_with('%') {
        LineValue::Percent(percent(at)?)
    } else {
        LineValue::Number(at.parse().ok()?)
    };
    Some((at, align))
}

fn align(value: &str) -> Option<CueAlign> {
    match value {
        "start" => Some(CueAlign::Start),
        "center" | "middle" => Some(CueAlign::Center),
        "end" => Some(CueAlign::End),
        "left" => Some(CueAlign::Left),
        "right" => Some(CueAlign::Right),
        _ => None,
    }
}

/// The speaker named by a `<v Name>` or `<v.class Name>` tag that opens the
/// cue text.
fn voice(first_line: Option<&str>) -> Option<&str> {
    let rest = first_line?.trim_start().strip_prefix("<v")?;
    if !rest.starts_with([' ', '\t', '.']) {
        return None;
    }
    let tag = &rest[..rest.find('>')?];
    let (_, name) = tag.split_once([' ', '\t'])?;
    let name = name.trim();
    (!name.is_empty()).then_some(name)
}

fn voice_index(parsed: &mut Parsed, name: &str) -> usize {
    if let Some(index) = parsed.voices.iter().position(|v| v == name) {
        return index;
    }
    parsed.voices.push(name.to_string());
    parsed.voices.len() - 1
}

/// The cue's lines with tags removed and references decoded, and its words.
/// With inline timestamps, every word is timed: words before the first take
/// `start`, and a word without a timestamp of its own takes the time of the
/// word before it.
fn cue_text(lines: &[&str], start: f64) -> Cue {
    let mut cue = Cue::default();
    let mut timed = false;
    for (index, line) in lines.iter().enumerate() {
        let (text, words) = clean_line(line);
        timed |= words.iter().any(|(_, time)| time.is_some());
        cue.lines.push(text);
        cue.words.extend(words.into_iter().map(|(text, time)| Word {
            line: index,
            text,
            time,
        }));
    }
    if timed {
        let mut current = start;
        for word in &mut cue.words {
            current = word.time.unwrap_or(current);
            word.time = Some(current);
        }
    }
    cue
}

/// One line of cue text: tags stripped, timestamps taken out, references
/// decoded. Returns the text and each word with the timestamp before it.
fn clean_line(line: &str) -> (String, Vec<(String, Option<f64>)>) {
    let mut text = String::new();
    let mut words: Vec<(String, Option<f64>)> = Vec::new();
    let mut pending = None;
    let mut in_word = false;
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        let (out, len) = if c == '<'
            && let Some(close) = rest.find('>')
        {
            if let Some(time) = timestamp(&rest[1..close]) {
                pending = Some(time);
            }
            rest = &rest[close + 1..];
            continue;
        } else if c == '&'
            && let Some((decoded, len)) = reference(rest)
        {
            (decoded, len)
        } else {
            (c, c.len_utf8())
        };
        if out.is_whitespace() {
            in_word = false;
        } else {
            if !in_word {
                words.push((String::new(), pending.take()));
                in_word = true;
            }
            if let Some((word, _)) = words.last_mut() {
                word.push(out);
            }
        }
        text.push(out);
        rest = &rest[len..];
    }
    (text.trim().to_string(), words)
}

/// A character reference at the start of `s`, and its length in bytes.
fn reference(s: &str) -> Option<(char, usize)> {
    let end = s.find(';')?;
    let name = &s[1..end];
    let decoded = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "nbsp" => '\u{a0}',
        "lrm" => '\u{200e}',
        "rlm" => '\u{200f}',
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some((decoded, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vtt(body: &str) -> Parsed {
        parse(&format!("WEBVTT\n\n{body}"))
    }

    fn one(body: &str) -> Cue {
        let parsed = vtt(body);
        assert_eq!(parsed.cues.len(), 1, "{parsed:?}");
        parsed.cues[0].clone()
    }

    fn placement_of(settings: &str) -> Option<Placement> {
        one(&format!("00:01.000 --> 00:02.000 {settings}\nhi")).placement
    }

    fn word_texts(cue: &Cue) -> Vec<&str> {
        cue.words.iter().map(|w| w.text.as_str()).collect()
    }

    fn word_times(cue: &Cue) -> Vec<Option<f64>> {
        cue.words.iter().map(|w| w.time).collect()
    }

    #[test]
    fn a_cue_has_start_end_lines_and_untimed_words() {
        let cue = one("00:01.000 --> 00:04.500\nHello world\nsecond line");
        assert_eq!(cue.start, Some(1.0));
        assert_eq!(cue.end, Some(4.5));
        assert_eq!(cue.lines, ["Hello world", "second line"]);
        assert_eq!(word_texts(&cue), ["Hello", "world", "second", "line"]);
        let lines: Vec<_> = cue.words.iter().map(|w| w.line).collect();
        assert_eq!(lines, [0, 0, 1, 1]);
        assert!(cue.words.iter().all(|w| w.time.is_none()));
        assert_eq!(cue.placement, None);
        assert_eq!(cue.voice, None);
    }

    #[test]
    fn hours_are_optional_in_timestamps() {
        let cue = one("01:02:03.250 --> 01:02:04.000\na");
        assert_eq!(cue.start, Some(3723.25));
        assert_eq!(cue.end, Some(3724.0));
    }

    #[test]
    fn the_header_may_carry_text_and_metadata() {
        let parsed = parse("WEBVTT - my captions\nKind: captions\n\n00:01.000 --> 00:02.000\na");
        assert_eq!(parsed.cues.len(), 1);
        assert!(parsed.problems.is_empty());
    }

    #[test]
    fn a_missing_header_is_reported_and_cues_still_parse() {
        let parsed = parse("00:01.000 --> 00:02.000\na\n\n00:03.000 --> 00:04.000\nb");
        assert_eq!(parsed.cues.len(), 2);
        assert_eq!(
            parsed.problems,
            [Problem {
                line: 1,
                message: "missing WEBVTT header".to_string()
            }]
        );
    }

    #[test]
    fn note_style_and_region_blocks_are_skipped() {
        let parsed = vtt(
            "NOTE a comment\nmore --> comment\n\nSTYLE\n::cue { color: red }\n\n\
                          REGION\nid:fred\n\nNOTE\n\n00:01.000 --> 00:02.000\na",
        );
        assert_eq!(parsed.cues.len(), 1);
        assert!(parsed.problems.is_empty());
    }

    #[test]
    fn identifier_lines_are_ignored() {
        let parsed = vtt("intro\n00:01.000 --> 00:02.000\na\n\n2\n00:03.000 --> 00:04.000\nb");
        assert_eq!(parsed.cues.len(), 2);
        assert_eq!(parsed.cues[0].lines, ["a"]);
        assert_eq!(parsed.cues[1].lines, ["b"]);
    }

    #[test]
    fn bad_timing_is_reported_on_its_line_and_the_block_skipped() {
        let parsed = vtt(
            "00:01.000 --> 00:02.000\na\n\nid\n00:03 --> 00:04.000\nb\n\n\
                          00:05.000 --> 00:06.000\nc",
        );
        assert_eq!(parsed.cues.len(), 2);
        assert_eq!(
            parsed.problems,
            [Problem {
                line: 7,
                message: "bad cue timing".to_string()
            }]
        );
    }

    #[test]
    fn a_block_without_timing_is_reported() {
        let parsed = vtt("just\ntext");
        assert!(parsed.cues.is_empty());
        assert_eq!(parsed.problems[0].line, 3);
        let parsed = vtt("lonely id");
        assert_eq!(parsed.problems[0].line, 3);
    }

    #[test]
    fn a_cue_that_ends_before_it_starts_is_bad_timing() {
        let parsed = vtt("00:05.000 --> 00:02.000\na");
        assert!(parsed.cues.is_empty());
        assert_eq!(parsed.problems[0].message, "bad cue timing");
    }

    #[test]
    fn cues_are_sorted_and_equal_times_keep_source_order() {
        let parsed = vtt(
            "00:05.000 --> 00:06.000\nlate\n\n00:01.000 --> 00:02.000\nfirst\n\n\
                          00:01.000 --> 00:03.000\nsecond",
        );
        let lines: Vec<_> = parsed.cues.iter().map(|c| c.lines[0].as_str()).collect();
        assert_eq!(lines, ["first", "second", "late"]);
    }

    #[test]
    fn position_sets_the_anchor_and_defaults_to_center() {
        let p = placement_of("position:25%").unwrap();
        assert_eq!(p.position, Some((0.25, PositionAlign::Center)));
        assert_eq!(p.line, None);
        assert_eq!(p.align, None);
        assert_eq!(p.size, None);
    }

    #[test]
    fn position_takes_each_alignment() {
        let get = |s: &str| placement_of(s).unwrap().position.unwrap().1;
        assert_eq!(get("position:10%,line-left"), PositionAlign::LineLeft);
        assert_eq!(get("position:10%,center"), PositionAlign::Center);
        assert_eq!(get("position:10%,line-right"), PositionAlign::LineRight);
        assert_eq!(placement_of("position:10%,start"), None);
    }

    #[test]
    fn line_takes_a_percent_or_a_number() {
        let get = |s: &str| placement_of(s).unwrap().line.unwrap();
        assert_eq!(get("line:80%"), (LineValue::Percent(0.8), LineAlign::Start));
        assert_eq!(get("line:3"), (LineValue::Number(3), LineAlign::Start));
        assert_eq!(get("line:-1,end"), (LineValue::Number(-1), LineAlign::End));
        assert_eq!(
            get("line:50%,center"),
            (LineValue::Percent(0.5), LineAlign::Center)
        );
        assert_eq!(
            get("line:0,start"),
            (LineValue::Number(0), LineAlign::Start)
        );
    }

    #[test]
    fn line_auto_leaves_the_line_unset() {
        assert_eq!(placement_of("line:auto"), None);
    }

    #[test]
    fn align_takes_every_value() {
        let get = |s: &str| placement_of(s).unwrap().align.unwrap();
        assert_eq!(get("align:start"), CueAlign::Start);
        assert_eq!(get("align:center"), CueAlign::Center);
        assert_eq!(get("align:middle"), CueAlign::Center);
        assert_eq!(get("align:end"), CueAlign::End);
        assert_eq!(get("align:left"), CueAlign::Left);
        assert_eq!(get("align:right"), CueAlign::Right);
        assert_eq!(placement_of("align:sideways"), None);
    }

    #[test]
    fn size_is_a_fraction_of_the_width() {
        let p = placement_of("size:40%").unwrap();
        assert_eq!(p.size, Some(0.4));
        assert!(!p.places());
    }

    #[test]
    fn several_settings_combine() {
        let p = placement_of("align:left position:10%,line-left line:90% size:50%").unwrap();
        assert_eq!(p.align, Some(CueAlign::Left));
        assert_eq!(p.position, Some((0.1, PositionAlign::LineLeft)));
        assert_eq!(p.line, Some((LineValue::Percent(0.9), LineAlign::Start)));
        assert_eq!(p.size, Some(0.5));
    }

    #[test]
    fn unknown_and_malformed_settings_are_ignored() {
        assert_eq!(
            placement_of("color:red line:x size:150% position:abc junk"),
            None
        );
        assert!(
            vtt("00:01.000 --> 00:02.000 color:red\nhi")
                .problems
                .is_empty()
        );
    }

    #[test]
    fn vertical_is_reported_and_the_cue_kept() {
        let parsed = vtt("00:01.000 --> 00:02.000 vertical:rl align:start\nhi");
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(
            parsed.cues[0].placement.unwrap().align,
            Some(CueAlign::Start)
        );
        assert_eq!(
            parsed.problems,
            [Problem {
                line: 3,
                message: "vertical text is not supported".to_string()
            }]
        );
    }

    #[test]
    fn region_is_reported_and_ignored() {
        let parsed = vtt("00:01.000 --> 00:02.000 region:fred\nhi");
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(parsed.cues[0].placement, None);
        assert_eq!(parsed.problems[0].message, "regions are not supported");
        assert_eq!(parsed.problems[0].line, 3);
    }

    #[test]
    fn inline_timestamps_time_the_following_words() {
        let cue = one(
            "00:01.000 --> 00:05.000\nNever <00:02.000>drink <00:02.500>liquid\n\
                       <00:03.000>nitrogen",
        );
        assert_eq!(cue.lines, ["Never drink liquid", "nitrogen"]);
        assert_eq!(
            word_times(&cue),
            [Some(1.0), Some(2.0), Some(2.5), Some(3.0)]
        );
        assert_eq!(cue.words[3].line, 1);
    }

    #[test]
    fn inline_timestamps_may_have_hours() {
        let cue = one("00:00:01.000 --> 00:00:05.000\n<00:00:02.000>a");
        assert_eq!(word_times(&cue), [Some(2.0)]);
    }

    #[test]
    fn a_word_without_its_own_timestamp_takes_the_previous_words_time() {
        let cue = one("00:01.000 --> 00:05.000\n<00:02.000>one two <00:03.000>three");
        assert_eq!(word_times(&cue), [Some(2.0), Some(2.0), Some(3.0)]);
    }

    #[test]
    fn a_voice_tag_sets_the_speaker_and_is_removed() {
        let parsed = vtt("00:01.000 --> 00:02.000\n<v Fred>Hello there</v>");
        assert_eq!(parsed.voices, ["Fred"]);
        assert_eq!(parsed.cues[0].voice, Some(0));
        assert_eq!(parsed.cues[0].lines, ["Hello there"]);
    }

    #[test]
    fn voices_are_numbered_in_order_of_first_appearance() {
        let parsed = vtt(
            "00:01.000 --> 00:02.000\n<v Bob>a\n\n00:03.000 --> 00:04.000\n\
                          <v.loud Alice Smith>b\n\n00:05.000 --> 00:06.000\n  <v Bob>c\n\n\
                          00:07.000 --> 00:08.000\nno voice",
        );
        assert_eq!(parsed.voices, ["Bob", "Alice Smith"]);
        let voices: Vec<_> = parsed.cues.iter().map(|c| c.voice).collect();
        assert_eq!(voices, [Some(0), Some(1), Some(0), None]);
    }

    #[test]
    fn only_a_voice_tag_that_opens_the_cue_counts() {
        let parsed = vtt("00:01.000 --> 00:02.000\nsaid <v Ann>hi");
        assert_eq!(parsed.cues[0].voice, None);
        assert!(parsed.voices.is_empty());
        let parsed = vtt("00:01.000 --> 00:02.000\n<v Ann>hi <v Ben>there");
        assert_eq!(parsed.voices, ["Ann"]);
    }

    #[test]
    fn voices_are_found_in_cues_listed_out_of_order() {
        let parsed =
            vtt("00:05.000 --> 00:06.000\n<v Late>a\n\n00:01.000 --> 00:02.000\n<v Early>b");
        assert_eq!(parsed.voices, ["Late", "Early"]);
        assert_eq!(parsed.cues[0].voice, Some(1));
    }

    #[test]
    fn tags_are_stripped_keeping_their_text() {
        let cue = one("00:01.000 --> 00:02.000\n<b>bold</b> <i>it</i> <u>un</u> \
                       <c.yellow.bg_blue>col</c> <lang en>la</lang> <ruby>漢<rt>kan</rt></ruby>");
        assert_eq!(cue.lines, ["bold it un col la 漢kan"]);
    }

    #[test]
    fn character_references_are_decoded() {
        let cue =
            one("00:01.000 --> 00:02.000\n&amp; &lt;b&gt; a&nbsp;b &lrm;&rlm; &#65;&#x42;&#X43;");
        assert_eq!(cue.lines, ["& <b> a\u{a0}b \u{200e}\u{200f} ABC"]);
    }

    #[test]
    fn a_decoded_angle_bracket_is_not_a_tag() {
        let cue = one("00:01.000 --> 00:02.000\n&lt;00:01.500&gt;x");
        assert_eq!(cue.lines, ["<00:01.500>x"]);
        assert!(cue.words.iter().all(|w| w.time.is_none()));
    }

    #[test]
    fn unknown_references_and_lone_ampersands_are_kept() {
        let cue = one("00:01.000 --> 00:02.000\nrock & roll &bogus; &#xZZ;");
        assert_eq!(cue.lines, ["rock & roll &bogus; &#xZZ;"]);
    }

    #[test]
    fn crlf_endings_and_a_bom_are_accepted() {
        let parsed = parse("\u{feff}WEBVTT\r\n\r\n1\r\n00:01.000 --> 00:02.000\r\na b\r\n\r\n");
        assert!(parsed.problems.is_empty());
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(parsed.cues[0].lines, ["a b"]);
    }

    #[test]
    fn a_cue_with_no_text_has_no_lines() {
        let cue = one("00:01.000 --> 00:02.000");
        assert!(cue.lines.is_empty());
        assert!(cue.words.is_empty());
    }
}
