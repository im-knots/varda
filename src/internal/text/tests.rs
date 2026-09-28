use super::*;
use crate::modulation::ModulationEngine;
use crate::source::{ControlKind, SourceClock};
use crate::timebase::{TimeContext, TransportSample};

fn deck(text: &str) -> TextDeck {
    TextDeck::new(Config {
        text: text.into(),
        ..Config::default()
    })
}

fn with(config: Config) -> TextDeck {
    TextDeck::new(config)
}

fn run(deck: &mut TextDeck, clock: SourceClock) {
    let engine = ModulationEngine::new();
    let mut scratch = String::new();
    let mut ctx = SourceControl::new("d1", &engine, true, clock, 60, &mut scratch);
    deck.control(&mut ctx);
}

fn seconds(dt: f32) -> SourceClock {
    SourceClock {
        dt,
        ..SourceClock::default()
    }
}

fn transport(position: f64) -> SourceClock {
    SourceClock {
        transport: Some(TransportSample {
            position,
            running: true,
            discontinuity: false,
            fps: 30.0,
        }),
        ..SourceClock::default()
    }
}

fn current(deck: &TextDeck) -> Option<usize> {
    deck.step_view().current
}

#[test]
fn a_bare_config_is_a_valid_text_deck() {
    let config: SourceConfig = serde_json::from_str(r#"{"type":"Text"}"#).unwrap();
    let decoded: Config = decode_config(&config).unwrap();
    assert_eq!(decoded, Config::default());
}

#[test]
fn config_round_trips_without_the_import_path() {
    let mut config = Config {
        text: "hi".into(),
        format: Format::Lrc,
        file: Some("/tmp/x.lrc".into()),
        ..Config::default()
    };
    let deck = with(config.clone());
    let saved = deck.config();
    assert!(saved.get("file").is_none(), "an import path is never saved");
    config.file = None;
    assert_eq!(decode_config::<Config>(&saved).unwrap(), config);
    assert_eq!(saved.str("text"), Some("hi"));
}

#[test]
fn every_declared_control_reads_back() {
    let deck = deck("a");
    for spec in PARAMS.iter() {
        if matches!(spec.kind, ControlKind::Action) {
            continue;
        }
        assert!(deck.param(&spec.name).is_some(), "{} reads back", spec.name);
    }
}

#[test]
fn normalized_controls_round_trip_their_ranges() {
    let mut deck = deck("a");
    deck.set_param("speed", &ControlValue::Float(0.75)).unwrap();
    assert!((deck.config.speed - 2.0).abs() < 1e-5);
    deck.set_param("mode", &ControlValue::Float(choice_value(1, 4)))
        .unwrap();
    assert_eq!(deck.config.mode, Mode::Crawl);
    assert_eq!(
        deck.param("mode"),
        Some(ControlValue::Float(choice_value(1, 4)))
    );
    deck.set_param("rollup_lines", &ControlValue::Float(0.99))
        .unwrap();
    assert_eq!(deck.config.rollup_lines, 4);
    deck.set_param("weight", &ControlValue::Float(1.0)).unwrap();
    assert_eq!(deck.config.weight, 900.0);
}

#[test]
fn position_is_a_point_and_moves_per_axis_through_the_router() {
    let mut deck = deck("a");
    crate::source::write_route(&mut deck, "position/x", 0.0).unwrap();
    assert_eq!(deck.config.position, [POSITION.0, 0.5]);
    assert_eq!(
        crate::source::read_route(&deck, "position/y"),
        Some(norm(0.5, POSITION))
    );
}

#[test]
fn text_over_the_cap_is_refused() {
    let mut deck = deck("a");
    let big = "x".repeat(MAX_TEXT + 1);
    assert!(matches!(
        deck.set_param("text", &ControlValue::Text(big)),
        Err(ControlError::Invalid(_))
    ));
    assert_eq!(deck.config.text, "a");
}

#[test]
fn appended_lines_become_current_in_step() {
    let mut deck = with(Config {
        text: "one".into(),
        mode: Mode::Step,
        ..Config::default()
    });
    deck.set_param("line", &ControlValue::Text("two".into()))
        .unwrap();
    deck.set_param("line", &ControlValue::Text("three".into()))
        .unwrap();
    assert_eq!(deck.config.text, "one\ntwo\nthree");
    run(&mut deck, seconds(0.0));
    assert_eq!(current(&deck), Some(2));
}

#[test]
fn appending_past_the_cap_drops_the_oldest_lines() {
    let mut deck = deck("");
    let line = "y".repeat(1000);
    for _ in 0..80 {
        deck.set_param("line", &ControlValue::Text(line.clone()))
            .unwrap();
    }
    assert!(deck.config.text.len() <= MAX_TEXT);
    assert!(deck.config.text.lines().count() < 80);
}

#[test]
fn lines_cannot_be_appended_to_timed_text() {
    let mut deck = with(Config {
        text: "[00:01.00]a".into(),
        format: Format::Lrc,
        ..Config::default()
    });
    assert!(matches!(
        deck.set_param("line", &ControlValue::Text("b".into())),
        Err(ControlError::State(_))
    ));
}

#[test]
fn rate_clock_steps_through_lines_and_loops() {
    let mut deck = with(Config {
        text: "a\nb\nc".into(),
        mode: Mode::Step,
        speed: 2.0,
        ..Config::default()
    });
    run(&mut deck, seconds(0.0));
    assert_eq!(current(&deck), Some(0));
    run(&mut deck, seconds(0.5));
    assert_eq!(current(&deck), Some(1));
    run(&mut deck, seconds(1.0));
    assert_eq!(current(&deck), Some(0), "3 lines at 2/s wrap after 1.5 s");
}

#[test]
fn without_loop_step_holds_the_last_unit() {
    let mut deck = with(Config {
        text: "a\nb".into(),
        mode: Mode::Step,
        looping: false,
        speed: 1.0,
        ..Config::default()
    });
    run(&mut deck, seconds(5.0));
    assert_eq!(current(&deck), Some(1));
}

#[test]
fn beat_clock_advances_only_with_a_tempo() {
    let mut deck = with(Config {
        text: "a\nb\nc\nd".into(),
        mode: Mode::Step,
        clock: Clock::Beat,
        speed: 0.25,
        ..Config::default()
    });
    run(&mut deck, seconds(10.0));
    assert_eq!(current(&deck), Some(0), "no tempo, no advance");
    let beat = |dt| SourceClock {
        beat: Some(TimeContext {
            time: 0.0,
            dt,
            running: true,
            discontinuity: false,
        }),
        dt: 0.5,
        ..SourceClock::default()
    };
    run(&mut deck, beat(4.0));
    assert_eq!(current(&deck), Some(1), "one unit per bar at 0.25 per beat");
}

#[test]
fn triggers_step_manually() {
    let mut deck = with(Config {
        text: "a\nb\nc".into(),
        mode: Mode::Step,
        speed: 0.0,
        ..Config::default()
    });
    deck.trigger("next").unwrap();
    deck.trigger("next").unwrap();
    run(&mut deck, seconds(0.1));
    assert_eq!(current(&deck), Some(2));
    deck.trigger("previous").unwrap();
    run(&mut deck, seconds(0.1));
    assert_eq!(current(&deck), Some(1));
    deck.trigger("restart").unwrap();
    run(&mut deck, seconds(0.1));
    assert_eq!(current(&deck), Some(0));
    assert!(deck.trigger("nope").is_err());
}

#[test]
fn a_sleeping_deck_does_not_advance() {
    let mut deck = with(Config {
        text: "a\nb".into(),
        mode: Mode::Step,
        ..Config::default()
    });
    let engine = ModulationEngine::new();
    let mut scratch = String::new();
    let mut ctx = SourceControl::new("d1", &engine, false, seconds(5.0), 60, &mut scratch);
    deck.control(&mut ctx);
    assert_eq!(deck.motion.advance, 0.0);
}

#[test]
fn plain_text_chases_by_rate_and_repeats_exactly() {
    let config = Config {
        text: "a\nb\nc\nd".into(),
        mode: Mode::Step,
        speed: 0.5,
        looping: false,
        ..Config::default()
    };
    let mut deck = with(config.clone());
    run(&mut deck, transport(4.2));
    assert_eq!(current(&deck), Some(2), "4.2 s at 0.5 lines/s");
    let progress = deck.step_view().progress;

    // The same position after a different history shows the same thing.
    let mut other = with(config);
    run(&mut other, seconds(3.0));
    run(&mut other, transport(9.0));
    run(&mut other, transport(4.2));
    assert_eq!(current(&other), Some(2));
    assert_eq!(other.step_view().progress, progress);
}

#[test]
fn chase_offset_and_delay_shift_the_clock() {
    let mut deck = with(Config {
        text: "a\nb\nc".into(),
        mode: Mode::Step,
        speed: 1.0,
        looping: false,
        transport_sync: DeckTransportSync {
            mode: TransportSyncMode::Auto,
            offset: 10.0,
            delay_frames: 30,
        },
        ..Config::default()
    });
    run(&mut deck, transport(12.5));
    // 12.5 - 10 - 30/30 = 1.5 s
    assert_eq!(current(&deck), Some(1));
}

#[test]
fn never_does_not_chase() {
    let mut deck = with(Config {
        text: "a\nb\nc".into(),
        mode: Mode::Step,
        speed: 0.0,
        transport_sync: DeckTransportSync {
            mode: TransportSyncMode::Never,
            ..DeckTransportSync::default()
        },
        ..Config::default()
    });
    run(&mut deck, transport(100.0));
    assert!(!deck.chasing());
    assert_eq!(current(&deck), Some(0));
}

#[test]
fn lrc_cues_land_on_their_timestamps() {
    let mut deck = with(Config {
        text: "[00:02.00]first\n[00:05.00]second".into(),
        format: Format::Lrc,
        mode: Mode::Step,
        ..Config::default()
    });
    run(&mut deck, transport(1.0));
    assert_eq!(current(&deck), None, "nothing before the first cue");
    run(&mut deck, transport(2.1));
    assert_eq!(current(&deck), Some(0));
    run(&mut deck, transport(7.0));
    assert_eq!(current(&deck), Some(1));
}

#[test]
fn subrip_cues_leave_at_their_end() {
    let mut deck = with(Config {
        text: "1\n00:00:01,000 --> 00:00:02,000\nhello\n".into(),
        format: Format::SubRip,
        mode: Mode::Step,
        transition: Transition::Fade,
        transition_time: 0.5,
        ..Config::default()
    });
    run(&mut deck, transport(1.8));
    let view = deck.step_view();
    assert_eq!(view.current, Some(0));
    assert!((view.ending - 0.6).abs() < 1e-3, "{view:?}");
    run(&mut deck, transport(2.5));
    assert_eq!(current(&deck), None);
}

#[test]
fn word_times_drive_paint_on_and_word_steps() {
    let text = "[00:01.00]<00:01.00>one <00:02.00>two <00:03.00>three";
    let mut deck = with(Config {
        text: text.into(),
        format: Format::Lrc,
        mode: Mode::Step,
        transition: Transition::PaintOn,
        ..Config::default()
    });
    run(&mut deck, transport(2.5));
    assert_eq!(deck.step_view().words_due, Some(2));

    deck.set_param("unit", &ControlValue::Float(choice_value(1, 2)))
        .unwrap();
    run(&mut deck, transport(3.1));
    assert_eq!(current(&deck), Some(2), "third word");
}

#[test]
fn timed_crawl_moves_between_cues() {
    let mut deck = with(Config {
        text: "[00:00.00]a\n[00:02.00]b".into(),
        format: Format::Lrc,
        mode: Mode::Crawl,
        ..Config::default()
    });
    run(&mut deck, transport(1.0));
    assert!((deck.motion.advance - 0.5).abs() < 1e-9);
}

#[test]
fn inactive_controls_follow_the_text_and_mode() {
    let names = |deck: &TextDeck| {
        deck.inactive()
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    };
    let deck = deck("a");
    let plain = names(&deck);
    assert!(plain.contains(&"voice_1_color"));
    assert!(plain.contains(&"rollup_lines"));
    assert!(!plain.contains(&"line"));
    assert!(!plain.contains(&"position"));

    let placed = with(Config {
        text: "WEBVTT\n\n00:01.000 --> 00:02.000 line:10%\n<v Ann>hi\n".into(),
        format: Format::WebVtt,
        mode: Mode::Step,
        transition: Transition::RollUp,
        ..Config::default()
    });
    let list = names(&placed);
    assert!(list.contains(&"position") && list.contains(&"align") && list.contains(&"valign"));
    assert!(!list.contains(&"voice_1_color"), "{list:?}");
    assert!(list.contains(&"line"));
    assert!(!list.contains(&"rollup_lines"));

    let crawl = with(Config {
        mode: Mode::Crawl,
        ..Config::default()
    });
    assert!(names(&crawl).contains(&"transition"));
}

#[test]
fn a_placed_file_is_only_placed_in_step() {
    let text = "WEBVTT\n\n00:01.000 --> 00:02.000 position:10%\nhi\n";
    let deck = with(Config {
        text: text.into(),
        format: Format::WebVtt,
        mode: Mode::Static,
        ..Config::default()
    });
    assert!(deck.doc.placed);
    assert!(!deck.inactive().iter().any(|(n, _)| *n == "position"));
}

#[test]
fn a_file_import_sets_the_format_and_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("song.lrc");
    std::fs::write(&path, "\u{feff}[00:01.00]la").unwrap();
    let mut deck = deck("old");
    deck.set_param(
        "file",
        &ControlValue::Text(path.to_string_lossy().into_owned()),
    )
    .unwrap();
    assert_eq!(deck.config.format, Format::Lrc);
    assert_eq!(deck.config.text, "[00:01.00]la");
    assert_eq!(deck.param("file"), Some(ControlValue::Text(String::new())));
}

#[test]
fn a_non_utf8_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.txt");
    std::fs::write(&path, [0xff, 0xfe, 0x00, 0x41]).unwrap();
    let mut deck = deck("old");
    assert!(
        deck.set_param(
            "file",
            &ControlValue::Text(path.to_string_lossy().into_owned())
        )
        .is_err()
    );
    assert_eq!(deck.config.text, "old");
}

#[test]
fn modulation_moves_live_values_but_not_the_config() {
    use crate::modulation::{AnalyzerValues, AudioValues, ModulationSource};
    let mut engine = ModulationEngine::new();
    let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
    engine.update_free_running(0.25, &AudioValues::default(), &AnalyzerValues::default());
    engine.assign("deck/d1/size", &uuid, 1.0);
    engine.assign("deck/d1/color/g", &uuid, -1.0);
    engine.assign("deck/d1/position/y", &uuid, 1.0);
    let mut deck = deck("a");
    let mut scratch = String::new();
    let mut ctx = SourceControl::new("d1", &engine, true, seconds(0.0), 60, &mut scratch);
    deck.control(&mut ctx);
    assert!(deck.live.size > deck.config.size);
    assert!(deck.live.color[1] < 1.0);
    assert!(deck.live.position[1] > 0.5);
    assert_eq!(
        deck.config,
        Config {
            text: "a".into(),
            ..Config::default()
        }
    );
}

#[test]
fn the_label_is_the_first_line() {
    assert_eq!(deck("\n  hello world  \nmore").label(), "hello world");
    assert_eq!(deck("").label(), "Text");
    assert_eq!(deck(&"x".repeat(40)).label().chars().count(), 35);
}

#[test]
fn problems_are_reported_under_the_text() {
    let deck = with(Config {
        text: "[00:01.00]a\nno time here".into(),
        format: Format::Lrc,
        ..Config::default()
    });
    let status = deck.status();
    assert!(
        status.display["text"].contains("line 2"),
        "{:?}",
        status.display
    );
}

// ── GPU ─────────────────────────────────────────────────────────────

fn render_deck(config: Config, width: u32, height: u32) -> Option<Vec<[f32; 4]>> {
    let gpu = crate::testing::headless_gpu()?;
    let fonts = {
        let mut db = usvg::fontdb::Database::new();
        crate::fonts::add_bundled(&mut db);
        Arc::new(db)
    };
    let mut deck = with(Config {
        font: "Ubuntu".into(),
        ..config
    });
    deck.attach(&gpu, width, height).expect("pipelines");
    let em = deck.live.size * height as f32;
    let style = deck.style();
    for text in deck.wanted(width as f32, height as f32) {
        deck.rasterize(&gpu, &fonts, &style, &text, em);
    }
    Some(crate::testing::render_source_pixels(
        &gpu, &mut deck, width, height,
    ))
}

#[test]
fn text_draws_its_color_over_the_background() {
    let Some(pixels) = render_deck(
        Config {
            text: "HHHH".into(),
            size: 0.5,
            color: [1.0, 0.0, 0.0, 1.0],
            background: [0.0, 0.0, 1.0, 1.0],
            ..Config::default()
        },
        128,
        64,
    ) else {
        eprintln!("Skipping: no headless GPU available");
        return;
    };
    let red = pixels.iter().filter(|p| p[0] > 0.9 && p[2] < 0.1).count();
    let blue = pixels.iter().filter(|p| p[2] > 0.9 && p[0] < 0.1).count();
    assert!(red > 50, "text pixels: {red}");
    assert!(blue > 1000, "background pixels: {blue}");
    assert!(
        pixels.iter().all(|p| (p[3] - 1.0).abs() < 1e-3),
        "opaque everywhere"
    );
}

#[test]
fn a_transparent_background_gives_transparent_pixels_and_straight_text() {
    let Some(pixels) = render_deck(
        Config {
            text: "HHHH".into(),
            size: 0.5,
            color: [0.0, 1.0, 0.0, 1.0],
            background: [0.0, 0.0, 0.0, 0.0],
            ..Config::default()
        },
        128,
        64,
    ) else {
        eprintln!("Skipping: no headless GPU available");
        return;
    };
    let clear = pixels.iter().filter(|p| p[3] < 1e-3).count();
    assert!(clear > 1000, "uncovered pixels are transparent: {clear}");
    // Straight alpha: a partly covered edge pixel still carries full green.
    let edge = pixels
        .iter()
        .find(|p| p[3] > 0.2 && p[3] < 0.8)
        .expect("an antialiased edge");
    assert!(edge[1] > 0.95, "straight color at the edge: {edge:?}");
}

#[test]
fn the_library_offers_one_entry_and_no_file_button() {
    let services = crate::source::Services::new();
    let shaders = crate::registry::ShaderRegistry::new();
    let query = SourceQuery {
        services: &services,
        shaders: &shaders,
        channels: &[],
    };
    let section = TextProvider.library(&query);
    assert_eq!(section.entries.len(), 1);
    assert!(
        section.create.is_none(),
        "files load from the deck's own controls"
    );
}

#[test]
fn every_control_is_drawn_by_the_text_deck_layout() {
    for spec in PARAMS.iter() {
        let hint = spec.widget;
        assert!(
            hint == Some(WidgetHint::TextDeck)
                || (spec.name == "font" && hint == Some(WidgetHint::FontFamily)),
            "{} carries {hint:?}",
            spec.name
        );
    }
}

#[test]
fn crawl_and_ticker_draw_multi_line_text() {
    for mode in [Mode::Crawl, Mode::Ticker, Mode::Static] {
        let Some(pixels) = render_deck(
            Config {
                text: "HHHH\nHHHH\nHHHH".into(),
                mode,
                size: 0.2,
                color: [1.0, 0.0, 0.0, 1.0],
                ..Config::default()
            },
            256,
            128,
        ) else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let red = pixels.iter().filter(|p| p[0] > 0.9).count();
        assert!(red > 50, "{mode:?} draws its text: {red}");
    }
}

#[test]
fn switching_mode_starts_the_motion_from_the_top() {
    for (mode, looping) in [
        (Mode::Crawl, false),
        (Mode::Ticker, false),
        (Mode::Crawl, true),
    ] {
        let mut deck = with(Config {
            text: "first\nsecond\nthird".into(),
            looping,
            ..Config::default()
        });
        // A minute in Static first.
        for _ in 0..60 {
            run(&mut deck, seconds(1.0));
        }
        deck.set_param("mode", &ControlValue::Float(pick(&MODES, mode)))
            .unwrap();
        run(&mut deck, seconds(0.016));
        let (position, quads) = deck.probe(1920.0, 1080.0);
        assert!(position < 1.0, "{mode:?} starts at the top: {position}");
        assert!(quads > 0, "{mode:?} shows its text right after the switch");
    }
}
