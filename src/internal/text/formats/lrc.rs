//! LRC lyrics, with enhanced LRC word tags.

use crate::text::cue::{Cue, Parsed, Problem, Word};

/// Parse `text` as LRC.
pub fn parse(text: &str) -> Parsed {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let offset = find_offset(text);
    let mut parsed = Parsed::default();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let (stamps, saw_tag, body) = leading_tags(line);
        if stamps.is_empty() {
            if !saw_tag {
                parsed.problems.push(Problem {
                    line: index + 1,
                    message: "no timestamp".to_string(),
                });
            }
            continue;
        }
        let (line_text, words) = timed_words(body);
        for stamp in stamps {
            parsed
                .cues
                .push(cue(shift(stamp, offset), &line_text, &words, offset));
        }
    }
    parsed
        .cues
        .sort_by(|a, b| a.start.unwrap_or(0.0).total_cmp(&b.start.unwrap_or(0.0)));
    parsed
}

/// The `[offset:ms]` value in milliseconds, or 0.
fn find_offset(text: &str) -> f64 {
    text.lines()
        .filter_map(|line| {
            let inner = line.trim().strip_prefix('[')?.split(']').next()?;
            let (key, value) = inner.split_once(':')?;
            if !key.trim().eq_ignore_ascii_case("offset") {
                return None;
            }
            value.trim().trim_start_matches('+').parse::<f64>().ok()
        })
        .next_back()
        .unwrap_or(0.0)
}

/// Apply the offset: a positive offset shows lyrics sooner.
fn shift(time: f64, offset_ms: f64) -> f64 {
    (time - offset_ms / 1000.0).max(0.0)
}

/// The leading `[...]` timestamps of `line`, whether it had a metadata tag,
/// and the text after them.
fn leading_tags(line: &str) -> (Vec<f64>, bool, &str) {
    let mut stamps = Vec::new();
    let mut saw_tag = false;
    let mut rest = line;
    while let Some(after) = rest.strip_prefix('[') {
        let Some(close) = after.find(']') else { break };
        let inner = &after[..close];
        if let Some(time) = timestamp(inner) {
            stamps.push(time);
        } else if is_metadata_tag(inner) {
            saw_tag = true;
        } else {
            break;
        }
        rest = after[close + 1..].trim_start();
    }
    (stamps, saw_tag, rest)
}

/// A `key:value` tag such as `ar:Artist`, with a letter-only key.
fn is_metadata_tag(inner: &str) -> bool {
    inner
        .split_once(':')
        .is_some_and(|(key, _)| !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic()))
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss.xxx` in seconds. Minutes may exceed 59.
fn timestamp(s: &str) -> Option<f64> {
    let (minutes, seconds) = s.split_once(':')?;
    if minutes.is_empty() || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (whole, fraction) = match seconds.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (seconds, None),
    };
    if whole.is_empty() || whole.len() > 2 || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut secs: f64 = whole.parse().ok()?;
    if secs >= 60.0 {
        return None;
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty()
            || fraction.len() > 3
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        secs += format!("0.{fraction}").parse::<f64>().ok()?;
    }
    Some(minutes.parse::<f64>().ok()? * 60.0 + secs)
}

/// The line's text with `<mm:ss.xx>` word tags removed, and its words with
/// the raw time of the tag before each, if any.
fn timed_words(body: &str) -> (String, Vec<(String, Option<f64>)>) {
    let mut text = String::new();
    let mut words: Vec<(String, Option<f64>)> = Vec::new();
    let mut pending = None;
    let mut in_word = false;
    let mut rest = body;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(close) = rest.find('>')
            && let Some(time) = timestamp(&rest[1..close])
        {
            pending = Some(time);
            rest = &rest[close + 1..];
            continue;
        }
        if c.is_whitespace() {
            in_word = false;
        } else {
            if !in_word {
                words.push((String::new(), pending.take()));
                in_word = true;
            }
            if let Some((word, _)) = words.last_mut() {
                word.push(c);
            }
        }
        text.push(c);
        rest = &rest[c.len_utf8()..];
    }
    (text.trim().to_string(), words)
}

/// One cue at `start`. When the line has word tags, every word is timed:
/// words before the first tag take `start`, and a word without a tag of its
/// own takes the time of the word before it.
fn cue(start: f64, text: &str, words: &[(String, Option<f64>)], offset: f64) -> Cue {
    let timed = words.iter().any(|(_, time)| time.is_some());
    let mut current = start;
    let words = words
        .iter()
        .map(|(word, time)| {
            if let Some(time) = time {
                current = shift(*time, offset);
            }
            Word {
                line: 0,
                text: word.clone(),
                time: timed.then_some(current),
            }
        })
        .collect();
    Cue {
        start: Some(start),
        lines: vec![text.to_string()],
        words,
        ..Cue::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(parsed: &Parsed) -> Vec<f64> {
        parsed.cues.iter().map(|c| c.start.unwrap()).collect()
    }

    fn word_times(cue: &Cue) -> Vec<Option<f64>> {
        cue.words.iter().map(|w| w.time).collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn hundredths_thousandths_and_whole_seconds_all_parse() {
        let parsed = parse("[00:01.50]a\n[00:02.250]b\n[00:03]c");
        assert_eq!(starts(&parsed), [1.5, 2.25, 3.0]);
        assert!(parsed.problems.is_empty());
    }

    #[test]
    fn minutes_may_exceed_fifty_nine_and_have_three_digits() {
        let parsed = parse("[75:00.00]late\n[100:01]later");
        assert_eq!(starts(&parsed), [4500.0, 6001.0]);
    }

    #[test]
    fn a_line_is_one_cue_with_its_words_untimed() {
        let parsed = parse("[00:12.00]Hello there world");
        let cue = &parsed.cues[0];
        assert_eq!(cue.lines, ["Hello there world"]);
        assert_eq!(cue.end, None);
        assert_eq!(cue.placement, None);
        assert_eq!(cue.voice, None);
        let words: Vec<_> = cue.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(words, ["Hello", "there", "world"]);
        assert!(cue.words.iter().all(|w| w.line == 0 && w.time.is_none()));
    }

    #[test]
    fn a_line_with_two_timestamps_is_two_cues() {
        let parsed = parse("[00:10.00][00:30.00]Chorus line");
        assert_eq!(starts(&parsed), [10.0, 30.0]);
        assert_eq!(parsed.cues[0].lines, ["Chorus line"]);
        assert_eq!(parsed.cues[1].lines, ["Chorus line"]);
    }

    #[test]
    fn cues_are_sorted_by_start() {
        let parsed = parse("[00:20.00]b\n[00:05.00][00:30.00]a\n[00:10.00]c");
        assert_eq!(starts(&parsed), [5.0, 10.0, 20.0, 30.0]);
        assert_eq!(parsed.cues[0].lines, ["a"]);
        assert_eq!(parsed.cues[3].lines, ["a"]);
    }

    #[test]
    fn equal_times_keep_source_order() {
        let parsed = parse("[00:01.00]first\n[00:01.00]second");
        assert_eq!(parsed.cues[0].lines, ["first"]);
        assert_eq!(parsed.cues[1].lines, ["second"]);
    }

    #[test]
    fn a_positive_offset_shows_lyrics_sooner() {
        let parsed = parse("[offset:+500]\n[00:02.00]a");
        assert_eq!(starts(&parsed), [1.5]);
    }

    #[test]
    fn a_negative_offset_shows_lyrics_later() {
        let parsed = parse("[offset:-250]\n[00:02.00]a");
        assert_eq!(starts(&parsed), [2.25]);
    }

    #[test]
    fn an_offset_after_the_lines_still_applies() {
        let parsed = parse("[00:02.00]a\n[offset:1000]");
        assert_eq!(starts(&parsed), [1.0]);
    }

    #[test]
    fn an_offset_never_makes_a_time_negative() {
        let parsed = parse("[offset:5000]\n[00:01.00]a <00:02.00>b");
        assert_eq!(starts(&parsed), [0.0]);
        assert_eq!(word_times(&parsed.cues[0]), [Some(0.0), Some(0.0)]);
    }

    #[test]
    fn the_offset_shifts_word_times_too() {
        let parsed = parse("[offset:500]\n[00:02.00]<00:02.00>a <00:03.00>b");
        let times = word_times(&parsed.cues[0]);
        assert!(close(times[0].unwrap(), 1.5));
        assert!(close(times[1].unwrap(), 2.5));
    }

    #[test]
    fn metadata_tags_are_ignored_silently() {
        let text = "[ar:Artist]\n[ti:Title]\n[al:Album]\n[by:Me]\n[length:03:20]\n\
                    [re:Tool]\n[ve:1.0]\n[custom:thing]\n[00:01.00]a";
        let parsed = parse(text);
        assert_eq!(parsed.cues.len(), 1);
        assert!(parsed.problems.is_empty());
    }

    #[test]
    fn a_line_without_a_timestamp_is_reported_with_its_line_number() {
        let parsed = parse("[ti:Song]\n[00:01.00]a\n\njust words\n[00:02.00]b");
        assert_eq!(parsed.cues.len(), 2);
        assert_eq!(
            parsed.problems,
            [Problem {
                line: 4,
                message: "no timestamp".to_string()
            }]
        );
    }

    #[test]
    fn a_bracketed_word_that_is_not_a_tag_is_reported() {
        let parsed = parse("[Chorus]");
        assert!(parsed.cues.is_empty());
        assert_eq!(parsed.problems[0].line, 1);
    }

    #[test]
    fn a_bad_timestamp_is_reported() {
        let parsed = parse("[00:75.00]a\n[0a:10.00]b");
        assert!(parsed.cues.is_empty());
        assert_eq!(parsed.problems.len(), 2);
    }

    #[test]
    fn blank_lines_are_ignored() {
        let parsed = parse("\n  \n[00:01.00]a\n\n");
        assert_eq!(parsed.cues.len(), 1);
        assert!(parsed.problems.is_empty());
    }

    #[test]
    fn an_empty_timestamp_is_a_clearing_cue() {
        let parsed = parse("[00:01.00]a\n[00:04.00]");
        let clear = &parsed.cues[1];
        assert_eq!(clear.start, Some(4.0));
        assert_eq!(clear.lines, [""]);
        assert!(clear.words.is_empty());
    }

    #[test]
    fn word_tags_time_the_words_and_leave_the_text() {
        let parsed = parse("[00:01.00]<00:01.00>one <00:01.50>two <00:02.25>three");
        let cue = &parsed.cues[0];
        assert_eq!(cue.lines, ["one two three"]);
        assert_eq!(word_times(cue), [Some(1.0), Some(1.5), Some(2.25)]);
    }

    #[test]
    fn words_before_the_first_word_tag_take_the_line_time() {
        let parsed = parse("[00:01.00]one two <00:03.00>three");
        assert_eq!(parsed.cues[0].lines, ["one two three"]);
        assert_eq!(
            word_times(&parsed.cues[0]),
            [Some(1.0), Some(1.0), Some(3.0)]
        );
    }

    #[test]
    fn a_word_without_its_own_tag_takes_the_previous_words_time() {
        let parsed = parse("[00:01.00]<00:02.00>one two <00:03.00>three");
        assert_eq!(
            word_times(&parsed.cues[0]),
            [Some(2.0), Some(2.0), Some(3.0)]
        );
    }

    #[test]
    fn a_tag_inside_a_word_times_the_next_word() {
        let parsed = parse("[00:01.00]hel<00:02.00>lo world");
        let cue = &parsed.cues[0];
        assert_eq!(cue.lines, ["hello world"]);
        assert_eq!(word_times(cue), [Some(1.0), Some(2.0)]);
    }

    #[test]
    fn angle_brackets_that_are_not_times_are_kept() {
        let parsed = parse("[00:01.00]a <b> c");
        assert_eq!(parsed.cues[0].lines, ["a <b> c"]);
        assert!(parsed.cues[0].words.iter().all(|w| w.time.is_none()));
    }

    #[test]
    fn a_leading_bom_and_crlf_endings_are_accepted() {
        let parsed = parse("\u{feff}[ti:x]\r\n[00:01.00]a b\r\n[00:02.00]c\r\n");
        assert_eq!(starts(&parsed), [1.0, 2.0]);
        assert_eq!(parsed.cues[0].lines, ["a b"]);
        assert!(parsed.problems.is_empty());
    }

    #[test]
    fn text_is_trimmed_after_the_timestamps() {
        let parsed = parse("[00:01.00]   spaced out   ");
        assert_eq!(parsed.cues[0].lines, ["spaced out"]);
    }
}
