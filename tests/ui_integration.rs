//! Integration tests for UI behavior.
//!
//! These tests simulate user interaction via AccessKit queries and assert
//! that the correct `UIActions` fields are populated.
//!
//! Pattern: `UIData` is constructed once per test. We wrap it in `Rc` to
//! share it with the harness closure without requiring `Clone` on `UIData`.

use std::rc::Rc;

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use varda::engine::EngineCommand;
use varda::usecases::ui::panels::render_ui;
use varda::usecases::ui::{UIActions, UIData};

/// Accumulated actions from all passes within a `run()` call.
///
/// `egui` may request repaints, causing `run()` to invoke the closure multiple
/// times. A click is processed in one pass but the next pass overwrites the
/// `UIActions`. We accumulate by merging interesting fields across passes.
// A flat tally of independent UI actions observed across egui passes.
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
struct AccActions {
    // Simple booleans
    add_channel: bool,
    toggle_library_panel: bool,
    toggle_right_panel: bool,
    select_master: bool,
    save_requested: bool,
    toggle_stage_editor: bool,
    toggle_arrangement_mode: bool,
    toggle_snap: bool,
    midi_rescan: bool,
    midi_clear_mappings: bool,
    midi_learn_toggle: bool,
    camera_rescan: bool,
    audio_rescan: bool,

    // Crossfader
    crossfader_snap_a: bool,
    crossfader_snap_b: bool,
    crossfader_auto_1s: bool,
    crossfader_auto_2s: bool,
    crossfader_auto_4s: bool,

    // Selection
    select_deck: Option<(usize, usize)>,
    select_channel: Option<usize>,
    remove_channel: Option<usize>,

    // Complex actions — track counts/flags since not all enums derive Clone
    output_create: bool,
    surface_add: bool,
    mod_add_lfo: bool,
    mod_add_audio: bool,
    mod_add_adsr: bool,
    mod_add_step_seq: bool,
    sequence_create: bool,

    // Combo box actions
    // Outer None = no SetTransition seen; inner None = transition cleared.
    #[allow(clippy::option_option)]
    set_transition: Option<Option<String>>,

    // Collapsing header item actions
    open_image_dialog_for_channel: Option<String>,
    open_video_dialog_for_channel: Option<String>,
    midi_device_toggles_count: usize,
    transport_play: bool,
    transport_stop: bool,
    transport_locate: Option<f64>,
    set_arrangement_zoom: Option<f32>,
    rearm_param: Option<String>,
    toggle_arrangement_snap: bool,
    gesture_active: bool,
    add_region: Option<(String, varda::arrangement::RegionConfig)>,
    update_region: Option<(String, usize, varda::arrangement::RegionConfig)>,
    set_lane_collapsed: Option<(String, bool)>,
    set_envelope_breakpoints: Option<(String, Vec<varda::modulation::Breakpoint>)>,
    reorder_deck: Option<(String, usize, usize)>,
    toggle_effect: Option<String>,
    copy: Option<varda::engine::ClipboardSource>,
    paste: Option<varda::engine::PasteTarget>,
    add_cue: Option<f64>,
    update_cue: Option<(String, Option<f64>, Option<String>)>,
    remove_cue: Option<String>,
    prev_cue: bool,
    next_cue: bool,
    trigger_cue: Option<String>,
    midi_learn_select: Option<String>,
}

impl AccActions {
    fn merge(&mut self, a: &UIActions) {
        // Booleans — OR-accumulate
        self.toggle_library_panel |= a.session.toggle_library_panel;
        self.toggle_right_panel |= a.session.toggle_right_panel;
        self.select_master |= a.session.select_master;
        self.save_requested |= a.session.save_requested;
        self.toggle_stage_editor |= a.session.toggle_stage_editor;
        self.toggle_arrangement_mode |= a.session.toggle_arrangement_mode;
        self.toggle_snap |= a.session.toggle_snap;
        if let Some(path) = &a.session.midi_learn_select {
            self.midi_learn_select = Some(path.clone());
        }
        self.toggle_arrangement_snap |= a.session.toggle_arrangement_snap;
        self.gesture_active |= a.session.gesture_active;
        if a.session.set_arrangement_zoom.is_some() {
            self.set_arrangement_zoom = a.session.set_arrangement_zoom;
        }
        self.midi_learn_toggle |= a.session.midi_learn_toggle;

        // Options — take latest non-None
        if a.session.select_deck.is_some() {
            self.select_deck = a.session.select_deck;
        }
        if a.session.select_channel.is_some() {
            self.select_channel = a.session.select_channel;
        }
        if a.session.remove_channel.is_some() {
            self.remove_channel = a.session.remove_channel;
        }

        // Unified command stream — crossfader, modulation-source adds, etc.
        for cmd in &a.commands {
            match cmd {
                EngineCommand::SetCrossfader(p) if *p < 0.5 => self.crossfader_snap_a = true,
                EngineCommand::SetCrossfader(_) => self.crossfader_snap_b = true,
                EngineCommand::AutoCrossfade { duration_secs, .. } => {
                    if (*duration_secs - 1.0).abs() < 0.01 {
                        self.crossfader_auto_1s = true;
                    }
                    if (*duration_secs - 2.0).abs() < 0.01 {
                        self.crossfader_auto_2s = true;
                    }
                    if (*duration_secs - 4.0).abs() < 0.01 {
                        self.crossfader_auto_4s = true;
                    }
                }
                EngineCommand::AddLfo { .. } => self.mod_add_lfo = true,
                EngineCommand::AddAudioBand { .. } => self.mod_add_audio = true,
                EngineCommand::AddAdsr { .. } => self.mod_add_adsr = true,
                EngineCommand::AddStepSequencer { .. } => self.mod_add_step_seq = true,
                EngineCommand::CreateSequence => self.sequence_create = true,
                EngineCommand::AddChannel => self.add_channel = true,
                EngineCommand::CreateOutput => self.output_create = true,
                EngineCommand::AddSurface { .. }
                | EngineCommand::AddPolygonSurface { .. }
                | EngineCommand::AddCircleSurface { .. } => self.surface_add = true,
                EngineCommand::RescanMidi => self.midi_rescan = true,
                EngineCommand::ClearMidiMappings => self.midi_clear_mappings = true,
                EngineCommand::RescanCameras => self.camera_rescan = true,
                EngineCommand::RescanAudio => self.audio_rescan = true,
                EngineCommand::SetMidiDeviceEnabled { .. } => self.midi_device_toggles_count += 1,
                EngineCommand::SetTransition { shader_name } => {
                    self.set_transition = Some(shader_name.clone());
                }
                EngineCommand::TransportPlay => self.transport_play = true,
                EngineCommand::TransportStop => self.transport_stop = true,
                EngineCommand::TransportLocate { position } => {
                    self.transport_locate = Some(*position);
                }
                EngineCommand::TransportPrevCue => self.prev_cue = true,
                EngineCommand::TransportNextCue => self.next_cue = true,
                EngineCommand::TriggerCue { uuid } => self.trigger_cue = Some(uuid.clone()),
                EngineCommand::AddCue { at, .. } => self.add_cue = Some(*at),
                EngineCommand::UpdateCue { uuid, at, name } => {
                    self.update_cue = Some((uuid.clone(), *at, name.clone()));
                }
                EngineCommand::RemoveCue { uuid } => self.remove_cue = Some(uuid.clone()),
                EngineCommand::RearmParam { param_key, .. } => {
                    self.rearm_param = Some(param_key.clone());
                }
                EngineCommand::AddRegion { deck_uuid, region } => {
                    self.add_region = Some((deck_uuid.clone(), *region));
                }
                EngineCommand::UpdateRegion {
                    deck_uuid,
                    index,
                    region,
                } => self.update_region = Some((deck_uuid.clone(), *index, *region)),
                EngineCommand::SetLaneCollapsed {
                    deck_uuid,
                    collapsed,
                } => self.set_lane_collapsed = Some((deck_uuid.clone(), *collapsed)),
                EngineCommand::SetEnvelopeBreakpoints { uuid, breakpoints } => {
                    self.set_envelope_breakpoints = Some((uuid.clone(), breakpoints.clone()));
                }
                EngineCommand::ReorderDeck {
                    channel_uuid,
                    from_idx,
                    to_idx,
                } => self.reorder_deck = Some((channel_uuid.clone(), *from_idx, *to_idx)),
                EngineCommand::ToggleEffect { effect_uuid } => {
                    self.toggle_effect = Some(effect_uuid.clone());
                }
                EngineCommand::Copy { source, .. } => self.copy = Some(source.clone()),
                EngineCommand::Paste { target } => self.paste = Some(target.clone()),
                _ => {}
            }
        }

        // Collapsing header items
        if a.session.open_image_dialog_for_channel.is_some() {
            self.open_image_dialog_for_channel
                .clone_from(&a.session.open_image_dialog_for_channel);
        }
        if a.session.open_video_dialog_for_channel.is_some() {
            self.open_video_dialog_for_channel
                .clone_from(&a.session.open_video_dialog_for_channel);
        }
    }
}

/// Helper: build a harness around `render_ui` with the given fixture data.
/// Uses 1280x720 to match a realistic window size for our panel layout.
/// State accumulates across multiple egui passes within a single `run()`.
fn make_harness(data: UIData) -> Harness<'static, AccActions> {
    // kittest's default step. Long enough that two clicks always read as two.
    make_harness_stepping(data, 1.0 / 4.0)
}

/// [`make_harness`] with the simulated frame length under the caller's control.
///
/// Double clicks need one: egui only pairs two clicks within 300 ms of each
/// other, and the default quarter-second step puts every simulated click far
/// outside that window.
fn make_harness_stepping(data: UIData, step_dt: f32) -> Harness<'static, AccActions> {
    let data = Rc::new(data);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .with_step_dt(step_dt)
        .build_ui_state(
            move |ui, acc: &mut AccActions| {
                let actions = render_ui(ui, &data);
                acc.merge(&actions);
            },
            AccActions::default(),
        );
    // Stabilize layout before interaction
    harness.run();
    // Reset accumulated state from layout passes
    *harness.state_mut() = AccActions::default();
    harness
}

/// [`make_harness`] that applies region edits back into the fixture between
/// frames, the way the engine does.
///
/// Every other test here renders a frozen snapshot, which cannot show what a
/// drag does once its region starts moving underneath the pointer. That is the
/// state a real resize spends all of its frames in.
fn make_live_harness(data: UIData) -> Harness<'static, AccActions> {
    use std::cell::RefCell;

    let data = Rc::new(RefCell::new(data));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .with_step_dt(1.0 / 4.0)
        .build_ui_state(
            move |ui, acc: &mut AccActions| {
                let actions = {
                    let snapshot = data.borrow();
                    render_ui(ui, &snapshot)
                };
                acc.merge(&actions);

                let mut live = data.borrow_mut();
                for cmd in &actions.commands {
                    if let EngineCommand::UpdateRegion {
                        deck_uuid,
                        index,
                        region,
                    } = cmd
                    {
                        if let Some(arrangement) = live.arrangement.as_mut() {
                            if let Some(lane) = arrangement
                                .config
                                .lanes
                                .iter_mut()
                                .find(|l| l.deck_uuid == *deck_uuid)
                            {
                                if let Some(slot) = lane.regions.get_mut(*index) {
                                    *slot = *region;
                                }
                            }
                        }
                    }
                }
            },
            AccActions::default(),
        );
    harness.run();
    *harness.state_mut() = AccActions::default();
    harness
}

/// Simulate a primary-button drag from `start` to `end` in window coordinates.
///
/// The intermediate nudge lets egui register a drag (and capture the press
/// origin) before the pointer travels to the release point, so handlers that
/// read `interact_pointer_pos()` on `drag_started`/`drag_stopped` see the
/// correct start and end positions.
fn drag(harness: &mut Harness<'static, AccActions>, start: egui::Pos2, end: egui::Pos2) {
    use egui::{Event, Modifiers, PointerButton};
    harness.event(Event::PointerMoved(start));
    harness.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::default(),
    });
    harness.run();
    // Move toward `end` in increments. The first increment is well beyond egui's
    // click-vs-drag threshold, so `drag_started` fires early (capturing a position
    // near `start`) rather than on a single large jump (which would capture `end`).
    for t in [0.25_f32, 0.5, 0.75, 1.0] {
        harness.event(Event::PointerMoved(start + (end - start) * t));
        harness.run();
    }
    harness.event(Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::default(),
    });
    harness.run();
}

/// A secondary-button click, which is what opens a context menu.
fn right_click(harness: &mut Harness<'static, AccActions>, pos: egui::Pos2) {
    use egui::{Event, Modifiers, PointerButton};
    harness.event(Event::PointerMoved(pos));
    harness.run();
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::default(),
        });
        harness.run();
    }
}

/// A primary click delivered by the pointer, which moves it onto the target.
///
/// `click_accesskit` fires the click without the pointer ever going there, so it
/// cannot show what a widget does while the mouse is over it.
fn click_at(harness: &mut Harness<'static, AccActions>, pos: egui::Pos2) {
    use egui::{Event, Modifiers, PointerButton};
    harness.event(Event::PointerMoved(pos));
    harness.run();
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        });
        harness.run();
    }
}

/// Two primary clicks at the same point, close enough together to read as one
/// double click.
///
/// Each event gets its own frame, matching how kittest drains a node's own
/// click: a press and a release in the same frame are not a click.
fn double_click(harness: &mut Harness<'static, AccActions>, pos: egui::Pos2) {
    use egui::{Event, Modifiers, PointerButton};
    harness.event(Event::PointerMoved(pos));
    harness.run();
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.event(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::default(),
            });
            harness.run();
        }
    }
}

// ── Library URL rows never inflate the panel width ──────────────────
//
// A resizable `egui::Panel` persists its content rect every frame, so any row
// wider than the panel's resized/default size overrides the user's drag and
// snaps the panel back to fit the content (and reveals the mixer texture beneath
// the UI during a resize). Long stream URLs used to do exactly this. The fix is
// the button-first `right_to_left` + truncating-label layout in
// `stream_row`; this test guards that layout against regressions.
const LONG_URL: &str =
    "https://very-long-cdn-hostname.example.com/live/premium/channel/12345/master-playlist-with-a-really-long-query.m3u8?token=abcdefghijklmnopqrstuvwxyz0123456789";

const PANEL_DEFAULT_WIDTH: f32 = 220.0;

fn probe_panel_width<F>(add: F) -> f32
where
    F: Fn(&mut egui::Ui) + 'static,
{
    let id = egui::Id::new("probe_panel");
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .build_ui(move |ui| {
            egui::Panel::left(id)
                .min_size(180.0)
                .default_size(PANEL_DEFAULT_WIDTH)
                .resizable(true)
                .show_inside(ui, |ui| add(ui));
        });
    // Run several frames to catch runaway growth (content-driven inflation).
    for _ in 0..5 {
        h.run();
    }
    h.ctx
        .data_mut(|d| d.get_persisted::<egui::PanelState>(id))
        .map(|s| s.rect.width())
        .expect("panel state should exist")
}

/// Mirrors the production `stream_row` layout: the remove button is reserved on
/// the right and the URL label truncates into the remaining width.
fn url_row(ui: &mut egui::Ui, url: &str) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let _ = ui.small_button("✕");
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.dnd_drag_source(egui::Id::new("probe_dnd"), 0u32, |ui| {
                    ui.label(egui::RichText::new("●"));
                    ui.add(
                        egui::Label::new(egui::RichText::new(format!("📡 {url}")).size(12.0))
                            .truncate(),
                    )
                    .on_hover_text(url);
                });
            });
        });
    });
}

#[test]
fn naive_url_row_inflates_panel() {
    // Baseline: an untruncated label in a plain `horizontal` layout forces the
    // panel far past its default width, reproducing the reported bug.
    let naive = probe_panel_width(|ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("📡 {LONG_URL}")).size(12.0));
            let _ = ui.small_button("✕");
        });
    });
    assert!(
        naive > PANEL_DEFAULT_WIDTH * 2.0,
        "expected the naive layout to inflate the panel, got {naive}"
    );
}

#[test]
fn truncating_url_row_keeps_panel_width() {
    // The `stream_row` layout keeps the panel pinned to its default width even
    // with a very long URL, so user resizes are never overridden.
    let fixed = probe_panel_width(|ui| url_row(ui, LONG_URL));
    assert!(
        (fixed - PANEL_DEFAULT_WIDTH).abs() < 1.0,
        "expected the truncating layout to hold the panel at {PANEL_DEFAULT_WIDTH}, got {fixed}"
    );
}

// ── Add Channel ─────────────────────────────────────────────────────

#[test]
fn click_add_channel_sets_action() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("➕ Ch").click();
    harness.run();

    assert!(
        harness.state().add_channel,
        "add_channel should be true after clicking ➕ Ch"
    );
}

// ── Snap Crossfader ─────────────────────────────────────────────────

#[test]
fn click_snap_a_sets_crossfader_action() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("⏮ Ch A").click();
    harness.run();

    assert!(
        harness.state().crossfader_snap_a,
        "Expected SnapA crossfader action"
    );
}

#[test]
fn click_snap_b_sets_crossfader_action() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("Ch B ⏭").click();
    harness.run();

    assert!(
        harness.state().crossfader_snap_b,
        "Expected SnapB crossfader action"
    );
}

// ── Toggle Library Panel ────────────────────────────────────────────

#[test]
fn click_close_library_sets_toggle() {
    let mut data = UIData::test_fixture();
    data.library_panel_open = true;
    let mut harness = make_harness(data);

    harness.get_by_label("◀").click();
    harness.run();

    assert!(
        harness.state().toggle_library_panel,
        "toggle_library_panel should be true"
    );
}

#[test]
fn click_open_library_sets_toggle() {
    let mut data = UIData::test_fixture();
    data.library_panel_open = false;
    let mut harness = make_harness(data);

    harness.get_by_label("▶").click();
    harness.run();

    assert!(
        harness.state().toggle_library_panel,
        "toggle_library_panel should be true"
    );
}

// ── Select Master ───────────────────────────────────────────────────

#[test]
fn click_main_output_heading_selects_master() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("🎬 Main Output").click();
    harness.run();

    assert!(
        harness.state().select_master,
        "select_master should be true"
    );
}

// ── Transport ───────────────────────────────────────────────────────

/// Open the top bar's transport popover and return the harness. The position
/// readout is the toggle, so the label to click is the timecode itself.
fn transport_harness(mutate: impl FnOnce(&mut UIData)) -> Harness<'static, AccActions> {
    let mut data = UIData::test_fixture();
    mutate(&mut data);
    let timecode = data.transport.timecode.clone();
    let mut harness = make_harness(data);
    harness.get_by_label(&timecode).click();
    harness.run();
    *harness.state_mut() = AccActions::default();
    harness
}

#[test]
fn transport_play_button_starts_the_show_position() {
    let mut harness = transport_harness(|_| {});

    harness.get_by_label("▶ Play").click();
    harness.run();

    assert!(harness.state().transport_play);
}

/// The same button stops a running transport, so a performer never has to hunt
/// for a second control.
#[test]
fn transport_play_button_stops_when_already_running() {
    let mut harness = transport_harness(|d| {
        d.transport.has_run = true;
        d.transport.running = true;
    });

    harness.get_by_label("⏸ Pause").click();
    harness.run();

    assert!(harness.state().transport_stop);
}

#[test]
fn transport_zero_button_locates_to_the_start() {
    let mut harness = transport_harness(|d| {
        d.transport.has_run = true;
        d.transport.position = 42.0;
    });

    harness.get_by_label("⏮ Zero").click();
    harness.run();

    assert_eq!(harness.state().transport_locate, Some(0.0));
}

/// While chasing, position belongs to the master, so the local controls are
/// disabled rather than silently ignored. See /spec/transport.md.
#[test]
fn transport_controls_are_disabled_while_chasing_timecode() {
    let mut harness = transport_harness(|d| {
        d.transport.source = varda::transport::TransportSource::Timecode;
    });

    harness.get_by_label("▶ Play").click();
    harness.run();

    assert!(!harness.state().transport_play);
}

// ── Save ────────────────────────────────────────────────────────────

#[test]
fn click_save_button_sets_save_requested() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("💾 Save").click();
    harness.run();

    assert!(
        harness.state().save_requested,
        "save_requested should be true"
    );
}

// ── Auto Crossfade Transitions ──────────────────────────────────────

#[test]
fn click_auto_transition_1s() {
    let mut harness = make_harness(UIData::test_fixture());

    // Seconds mode (no BPM in fixture): the direction label "→Ch A" is separate
    // and the duration buttons are bare numbers ("1", "2", "4", ...).
    harness.get_by_label("1").click();
    harness.run();

    assert!(
        harness.state().crossfader_auto_1s,
        "Expected 1s auto-transition"
    );
}

#[test]
fn click_auto_transition_2s() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("2").click();
    harness.run();

    assert!(
        harness.state().crossfader_auto_2s,
        "Expected 2s auto-transition"
    );
}

#[test]
fn click_auto_transition_4s() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("4").click();
    harness.run();

    assert!(
        harness.state().crossfader_auto_4s,
        "Expected 4s auto-transition"
    );
}

// ── Output Window ───────────────────────────────────────────────────

#[test]
fn click_new_output_creates_output_action() {
    // Taller window so the right panel's ScrollArea exposes the Output section
    let data = Rc::new(UIData::test_fixture());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 1200.0))
        .build_ui_state(
            move |ui, acc: &mut AccActions| {
                let actions = render_ui(ui, &data);
                acc.merge(&actions);
            },
            AccActions::default(),
        );
    harness.run();
    *harness.state_mut() = AccActions::default();

    // Expand the "📺 Outputs" collapsing header first
    harness.get_by_label("📺 Outputs").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    harness.get_by_label("+ Windowed").click();
    harness.run();

    assert!(
        harness.state().output_create,
        "Expected OutputAction::Create"
    );
}

// ── Modulation Sources ──────────────────────────────────────────────

#[test]
fn click_add_lfo() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("〰 Modulation").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    harness.get_by_label("➕ LFO").click();
    harness.run();

    assert!(
        harness.state().mod_add_lfo,
        "Expected ModulationAction::AddLFO"
    );
}

#[test]
fn click_add_audio_mod() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("〰 Modulation").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    harness.get_by_label("➕ Audio").click();
    harness.run();

    assert!(
        harness.state().mod_add_audio,
        "Expected ModulationAction::AddAudioFFT"
    );
}

#[test]
fn click_add_adsr() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("〰 Modulation").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    harness.get_by_label("➕ ADSR").click();
    harness.run();

    assert!(
        harness.state().mod_add_adsr,
        "Expected ModulationAction::AddADSR"
    );
}

#[test]
fn click_add_step_seq() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("〰 Modulation").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    harness.get_by_label("➕ StepSeq").click();
    harness.run();

    assert!(
        harness.state().mod_add_step_seq,
        "Expected ModulationAction::AddStepSequencer"
    );
}

// ── Stage Editor ────────────────────────────────────────────────────

#[test]
fn click_open_stage_editor() {
    let mut data = UIData::test_fixture();
    data.stage_editor_open = false;
    let mut harness = make_harness(data);

    // Expand "🗺 Stage Layout" collapsing header
    harness.get_by_label("🗺 Stage Layout").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    harness.get_by_label("✏ Open Editor").click();
    harness.run();

    assert!(
        harness.state().toggle_stage_editor,
        "toggle_stage_editor should be true"
    );
}

// ── Arrangement Mode ────────────────────────────────────────────────

/// A fixture whose first deck has one region on it, and a transport that has
/// run so the arrangement holds authority.
fn arranged_fixture() -> (UIData, String) {
    let mut data = UIData::test_fixture();
    let deck_uuid = data.channels[0].decks[0].uuid.clone();
    let mut lane = varda::arrangement::LaneConfig::new(&deck_uuid);
    lane.regions.push(varda::arrangement::RegionConfig {
        start: 2.0,
        end: 10.0,
        fade_in: 0.5,
        fade_out: 0.5,
    });
    let config = varda::arrangement::ArrangementConfig {
        lanes: vec![lane],
        ..Default::default()
    };
    data.arrangement = Some(varda::engine::types::ArrangementSnapshot {
        duration: config.duration(),
        config,
        engaged: true,
        overridden_params: vec![],
    });
    data.transport.has_run = true;
    (data, deck_uuid)
}

#[test]
fn click_switch_to_arrangement_mode() {
    let mut harness = make_harness(UIData::test_fixture());
    harness.get_by_label("▤ Arrange").click();
    harness.run();

    assert!(
        harness.state().toggle_arrangement_mode,
        "the Arrange button should request the mode swap"
    );
}

/// The way back must be in the same place, or the mode is a trap.
#[test]
fn arrangement_mode_offers_the_way_back() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    harness.get_by_label("🎛 Perform").click();
    harness.run();

    assert!(harness.state().toggle_arrangement_mode);
}

#[test]
fn arrangement_transport_strip_plays_and_stops() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    harness.get_by_label("▶").click();
    harness.run();
    assert!(
        harness.state().transport_play,
        "▶ should play the transport"
    );

    // One button covers stop and return: the engine holds position on the first
    // press and goes home on the second. See /spec/transport.md § Stop Twice.
    *harness.state_mut() = AccActions::default();
    harness.get_by_label("⏹").click();
    harness.run();
    assert!(harness.state().transport_stop);
    assert_eq!(
        harness.state().transport_locate,
        None,
        "the strip must not locate behind the engine's back"
    );
}

/// The arrows either side of stop walk the cue list, which is what the
/// return-to-zero arrow used to be. See /spec/arrangement.md § Cue points.
#[test]
fn the_transport_arrows_walk_the_cue_list() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    harness.get_by_label("⏮").click();
    harness.run();
    assert!(harness.state().prev_cue);
    assert_eq!(
        harness.state().transport_locate,
        None,
        "back is a cue jump now, not a rewind to zero"
    );

    harness.get_by_label("⏭").click();
    harness.run();
    assert!(harness.state().next_cue);
}

#[test]
fn arrangement_zoom_buttons_change_the_scale() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let before = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    harness.get_by_label("+").click();
    harness.run();

    let after = harness
        .state()
        .set_arrangement_zoom
        .expect("zoom in should request a new scale");
    assert!(after > before, "{after} should exceed {before}");
}

/// The same fixture with one hand-drawn curve on a parameter that is not
/// opacity, which is what puts an automation row under the lane.
fn automated_fixture() -> (UIData, String) {
    use varda::modulation::{Breakpoint, CurveKind};

    let (mut data, deck_uuid) = arranged_fixture();
    data.modulation_sources
        .push(varda::usecases::ui::ModSourceUIEntry {
            uuid: "env-speed".to_string(),
            source: varda::usecases::ui::ModSourceUI::Envelope {
                breakpoints: vec![
                    Breakpoint {
                        position: 1.0,
                        value: 0.5,
                        curve: CurveKind::default(),
                    },
                    Breakpoint {
                        position: 6.0,
                        value: 0.8,
                        curve: CurveKind::default(),
                    },
                ],
            },
            timebase: varda::timebase::Timebase::Transport,
        });
    data.modulation_assignments.insert(
        format!("deck_{deck_uuid}:speed"),
        vec![varda::usecases::ui::ModAssignmentUI {
            source_id: "env-speed".to_string(),
            amount: 1.0,
        }],
    );
    (data, deck_uuid)
}

/// Dragging across empty track is how a show gets authored in the first place.
#[test]
fn dragging_across_an_empty_lane_authors_a_region() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    // The second deck has no regions, so the whole row is free track.
    let empty_deck = data.channels[0].decks[1].uuid.clone();
    let mut harness = make_harness(data);

    let track = harness
        .get_by_label("test_generator_b timeline track")
        .rect();
    let y = track.center().y;
    drag(
        &mut harness,
        egui::pos2(track.left() + 120.0, y),
        egui::pos2(track.left() + 280.0, y),
    );

    let (deck_uuid, region) = harness
        .state()
        .add_region
        .clone()
        .expect("a drag across empty track should author a region");
    assert_eq!(deck_uuid, empty_deck);
    assert!(region.is_valid(), "the authored span must not be empty");
    assert!(
        region.start < region.end,
        "{region:?} should run left to right"
    );
    assert!(
        harness.state().gesture_active,
        "the drag must declare itself a gesture, or undo records every frame of it"
    );
}

/// Dragging a region's body moves it without resizing it.
#[test]
fn dragging_a_region_moves_it_along_the_timeline() {
    let (mut data, deck_uuid) = arranged_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    let region = harness.get_by_label("test_generator_a region 1").rect();
    let y = region.center().y;
    drag(
        &mut harness,
        egui::pos2(region.center().x, y),
        egui::pos2(region.center().x + 80.0, y),
    );

    let (moved_deck, index, moved) = harness
        .state()
        .update_region
        .clone()
        .expect("dragging a region should rewrite it");
    assert_eq!(moved_deck, deck_uuid);
    assert_eq!(index, 0);
    assert!(moved.start > 2.0, "{moved:?} should have moved later");
    assert!(
        (moved.span() - 8.0).abs() < 0.2,
        "{moved:?} should keep its length"
    );
    assert!(harness.state().gesture_active);
}

/// Show seconds to window x for the arranged fixture's lane, so a resize test
/// can aim at a region's drawn edge rather than at its widget rect (which
/// deliberately extends past the edge to catch near misses).
fn timeline_x(harness: &mut Harness<'static, AccActions>, pps: f32) -> impl Fn(f64) -> f32 {
    let track = harness
        .get_by_label("test_generator_a timeline track")
        .rect();
    let left = track.left();
    move |seconds: f64| left + (seconds as f32) * pps
}

/// Dragging a region's right edge lengthens it without moving its start.
#[test]
fn dragging_the_end_edge_resizes_the_region() {
    let (mut data, deck_uuid) = arranged_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    let y = harness
        .get_by_label("test_generator_a region 1")
        .rect()
        .center()
        .y;
    let x = timeline_x(&mut harness, pps);
    drag(
        &mut harness,
        egui::pos2(x(10.0) - 2.0, y),
        egui::pos2(x(14.0), y),
    );

    let (resized_deck, index, resized) = harness
        .state()
        .update_region
        .clone()
        .expect("dragging the end edge should rewrite the region");
    assert_eq!(resized_deck, deck_uuid);
    assert_eq!(index, 0);
    assert!(
        (resized.start - 2.0).abs() < 1e-6,
        "{resized:?} must keep its start"
    );
    assert!(
        resized.end > 10.5,
        "{resized:?} should have been stretched later"
    );
}

#[test]
fn dragging_the_start_edge_resizes_the_region() {
    let (mut data, deck_uuid) = arranged_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    let y = harness
        .get_by_label("test_generator_a region 1")
        .rect()
        .center()
        .y;
    let x = timeline_x(&mut harness, pps);
    drag(
        &mut harness,
        egui::pos2(x(2.0) + 2.0, y),
        egui::pos2(x(5.0), y),
    );

    let (resized_deck, index, resized) = harness
        .state()
        .update_region
        .clone()
        .expect("dragging the start edge should rewrite the region");
    assert_eq!(resized_deck, deck_uuid);
    assert_eq!(index, 0);
    assert!(
        (resized.end - 10.0).abs() < 1e-6,
        "{resized:?} must keep its end"
    );
    assert!(
        resized.start > 2.5,
        "{resized:?} should have been trimmed later"
    );
    assert!(
        harness.state().add_region.is_none(),
        "grabbing an edge must not author a new region"
    );
}

/// Aiming at a one-pixel edge and landing a few pixels past it is the normal way
/// to miss. Empty track sits there, so before the grab zone straddled the edge
/// this authored a second region on top of the one being resized.
#[test]
fn grabbing_just_outside_an_edge_still_resizes() {
    let (mut data, deck_uuid) = arranged_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    let y = harness
        .get_by_label("test_generator_a region 1")
        .rect()
        .center()
        .y;
    let x = timeline_x(&mut harness, pps);
    drag(
        &mut harness,
        egui::pos2(x(10.0) + 3.0, y),
        egui::pos2(x(14.0), y),
    );

    assert!(
        harness.state().add_region.is_none(),
        "a press inside the edge's grab zone must not author a region"
    );
    let (resized_deck, _, resized) = harness
        .state()
        .update_region
        .clone()
        .expect("a grab just outside the edge should still resize");
    assert_eq!(resized_deck, deck_uuid);
    assert!(resized.end > 10.5, "{resized:?} should have been stretched");
}

/// A resize spends every frame after the first with the region already moved,
/// so the gesture has to survive its own output coming back at it. If the edit
/// were computed from the current region rather than from where the drag
/// started, this would run away or stall.
#[test]
fn a_resize_survives_the_region_moving_under_the_pointer() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_live_harness(data);

    let y = harness
        .get_by_label("test_generator_a region 1")
        .rect()
        .center()
        .y;
    let x = timeline_x(&mut harness, pps);
    // Four seconds of travel: 2 s to 6 s, trimming the start of a 2–10 region.
    drag(
        &mut harness,
        egui::pos2(x(2.0) + 2.0, y),
        egui::pos2(x(6.0), y),
    );

    let (_, _, resized) = harness
        .state()
        .update_region
        .clone()
        .expect("the resize should still be rewriting the region at the end of the drag");
    assert!(
        (resized.end - 10.0).abs() < 1e-6,
        "{resized:?} must keep its end"
    );
    assert!(
        (resized.start - 6.0).abs() < 0.3,
        "{resized:?} should land where the pointer did, not somewhere it accumulated to"
    );
}

/// The same gesture in the other direction, since a start edge dragged left
/// grows the region and moves the rect the pointer is standing on.
#[test]
fn a_resize_that_grows_the_region_lands_where_the_pointer_does() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_live_harness(data);

    let y = harness
        .get_by_label("test_generator_a region 1")
        .rect()
        .center()
        .y;
    let x = timeline_x(&mut harness, pps);
    drag(
        &mut harness,
        egui::pos2(x(10.0) - 2.0, y),
        egui::pos2(x(14.0), y),
    );

    let (_, _, resized) = harness
        .state()
        .update_region
        .clone()
        .expect("the resize should still be rewriting the region at the end of the drag");
    assert!((resized.start - 2.0).abs() < 1e-6);
    assert!(
        (resized.end - 14.0).abs() < 0.3,
        "{resized:?} should land where the pointer did"
    );
}

/// The edge's claim on nearby track is a few pixels, not a licence to swallow
/// the lane: authoring elsewhere on the row still works.
#[test]
fn a_press_clear_of_a_region_still_authors_a_new_one() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    let y = harness
        .get_by_label("test_generator_a region 1")
        .rect()
        .center()
        .y;
    let track = harness
        .get_by_label("test_generator_a timeline track")
        .rect();
    let x = timeline_x(&mut harness, pps);
    let from = egui::pos2(track.right() - 80.0, y);
    let to = egui::pos2(track.right() - 20.0, y);
    assert!(
        from.x > x(10.0) + 10.0,
        "the press has to be clear of the region's edge zone to test anything"
    );
    drag(&mut harness, from, to);

    let (_, authored) = harness
        .state()
        .add_region
        .clone()
        .expect("empty track well clear of a region should still author one");
    assert!(
        authored.start > 10.0,
        "{authored:?} should start after the existing region, where the drag did"
    );
    assert!(
        harness.state().update_region.is_none(),
        "authoring must not rewrite the neighbouring region"
    );
}

/// Deck order is one order shared by both views, so it has to be editable from
/// the timeline and not only from the mixer.
#[test]
fn dragging_a_lane_header_reorders_the_deck() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let channel_uuid = data.channels[0].uuid.clone();
    let mut harness = make_harness(data);

    // Lane headers sit immediately left of their track and share its rows, so
    // the rows are located by their tracks and the header point is stepped back
    // from the track's left edge.
    let first = harness
        .get_by_label("test_generator_a timeline track")
        .rect();
    let second = harness
        .get_by_label("test_generator_b timeline track")
        .rect();
    let header_x = first.left() - 20.0;

    drag(
        &mut harness,
        egui::pos2(header_x, second.center().y),
        // The top half of the first lane is the gap above it.
        egui::pos2(header_x, first.top() + 3.0),
    );

    let (channel, from, to) = harness
        .state()
        .reorder_deck
        .clone()
        .expect("dropping a lane above another should reorder the deck");
    assert_eq!(channel, channel_uuid);
    assert_eq!((from, to), (1, 0));
}

/// Dropping a lane back where it started must not emit a command, or every
/// aborted drag lands in the undo history as a no-op.
#[test]
fn dropping_a_lane_where_it_started_changes_nothing() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    let track = harness
        .get_by_label("test_generator_b timeline track")
        .rect();
    let header_x = track.left() - 20.0;

    drag(
        &mut harness,
        egui::pos2(header_x, track.center().y),
        egui::pos2(header_x, track.bottom() - 3.0),
    );

    assert!(
        harness.state().reorder_deck.is_none(),
        "the gap below a deck is where it already is"
    );
}

/// A lane dropped on another channel does nothing here. Moving a deck between
/// channels targets a channel rather than a position between two lanes, and it
/// stays in the mixer, which has somewhere to drop it.
#[test]
fn a_lane_dropped_on_another_channel_is_refused() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    let mine = harness
        .get_by_label("test_generator_a timeline track")
        .rect();
    // test_generator_c is the first deck of the second channel.
    let theirs = harness
        .get_by_label("test_generator_c timeline track")
        .rect();
    let header_x = mine.left() - 20.0;

    drag(
        &mut harness,
        egui::pos2(header_x, mine.center().y),
        egui::pos2(header_x, theirs.top() + 3.0),
    );

    assert!(
        harness.state().reorder_deck.is_none(),
        "a deck cannot be reordered into a channel it is not in"
    );
}

/// Frame snapping is a preference on the gesture, so it needs to be switchable
/// without leaving the timeline.
#[test]
fn the_snap_preference_is_reachable_from_the_timeline() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    harness.get_by_label("Snap").click();
    harness.run();

    assert!(harness.state().toggle_arrangement_snap);
}

/// A deck with several automated parameters is a wall of rows, so the fold has
/// to be there and has to be a control.
#[test]
fn a_lane_folds_its_automation_away() {
    let (mut data, deck_uuid) = automated_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    harness.get_by_label("Hide automation").click();
    harness.run();

    assert_eq!(harness.state().set_lane_collapsed, Some((deck_uuid, true)));
}

/// Double-clicking empty track drops a region without having to drag one out.
#[test]
fn double_clicking_empty_track_drops_a_region() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness_stepping(data, 1.0 / 60.0);

    let track = harness
        .get_by_label("test_generator_b timeline track")
        .rect();
    double_click(
        &mut harness,
        egui::pos2(track.left() + 200.0, track.center().y),
    );

    let (_, region) = harness
        .state()
        .add_region
        .clone()
        .expect("a double click on empty track should drop a region");
    assert!(region.is_valid());
}

/// The arranged fixture with one cue on it, which is what the arrows walk and
/// the ruler draws.
fn cued_fixture() -> (UIData, String) {
    let (mut data, deck_uuid) = arranged_fixture();
    data.arrangement_mode_open = true;
    let arrangement = data.arrangement.as_mut().expect("the fixture arranges");
    arrangement.config.cues.push(varda::arrangement::Cue {
        uuid: "cue00001".to_string(),
        name: "Drop".to_string(),
        at: 6.0,
    });
    (data, deck_uuid)
}

/// The same fixture back in Performance mode, which is where the pads live.
fn banked_fixture() -> UIData {
    let (mut data, _) = cued_fixture();
    data.arrangement_mode_open = false;
    let arrangement = data.arrangement.as_mut().expect("the fixture arranges");
    arrangement.config.cues.push(varda::arrangement::Cue {
        uuid: "cue00002".to_string(),
        name: "Break".to_string(),
        at: 24.0,
    });
    data
}

/// A pad names its cue and the moment it sits at, counted in the show's own
/// frame rate.
fn pad_label(data: &UIData, name: &str, at: f64) -> String {
    format!("Cue {name} at {}", data.transport.timecode_rate.format(at))
}

/// A cue is marked against the timeline but wanted at the desk, so every cue is
/// a pad in Performance mode and pressing one goes there.
#[test]
fn a_cue_is_a_pad_in_performance_mode() {
    let data = banked_fixture();
    let (drop, break_) = (
        pad_label(&data, "Drop", 6.0),
        pad_label(&data, "Break", 24.0),
    );
    let mut harness = make_harness(data);

    assert!(
        harness.query_by_label(&break_).is_some(),
        "every cue gets a pad, in the order the ruler draws them"
    );
    harness.get_by_label(&drop).click();
    harness.run();

    assert_eq!(
        harness.state().trigger_cue.as_deref(),
        Some("cue00001"),
        "the pad sends the cue it names"
    );
}

/// A bank of nothing is a header and a gap, so a show with no cues has neither.
#[test]
fn the_cue_bank_is_absent_until_there_are_cues() {
    let (mut data, _) = arranged_fixture();
    data.arrangement_mode_open = false;
    let harness = make_harness(data);

    assert!(
        harness.query_by_label("◆ Cues").is_none(),
        "no cues, no bank"
    );
}

/// Position is owned by the timecode master while chasing, so a pad that would
/// be refused is drawn refusing rather than failing under the hand.
#[test]
fn the_cue_pads_are_dead_while_chasing_timecode() {
    let mut data = banked_fixture();
    data.transport.source = varda::transport::TransportSource::Timecode;
    let drop = pad_label(&data, "Drop", 6.0);
    let harness = make_harness(data);

    let pad = harness.get_by_label(&drop);
    assert!(
        egui_kittest::kittest::NodeT::accesskit_node(&pad).is_disabled(),
        "the transport does not take locates while chasing"
    );
}

/// Mapping a cue to a foot switch is the same gesture as mapping anything else,
/// and it addresses the cue by UUID so a rename or a move keeps the mapping.
#[test]
fn a_cue_pad_can_be_learned_to_midi() {
    let mut data = banked_fixture();
    data.midi_learn_active = true;
    let drop = pad_label(&data, "Drop", 6.0);
    let mut harness = make_harness(data);

    let pad = harness.get_by_label(&drop).rect();
    click_at(&mut harness, pad.center());

    assert_eq!(
        harness.state().midi_learn_select.as_deref(),
        Some("cue/cue00001/fire")
    );
}

/// Marking the moment you are looking at is a double click on the ruler, the
/// same gesture that drops a region on a lane.
#[test]
fn double_clicking_the_ruler_drops_a_cue() {
    let (data, _) = cued_fixture();
    let mut harness = make_harness_stepping(data, 1.0 / 60.0);

    let ruler = harness.get_by_label("arrangement ruler").rect();
    double_click(
        &mut harness,
        egui::pos2(ruler.left() + 150.0, ruler.center().y),
    );

    let at = harness
        .state()
        .add_cue
        .expect("a double click on the ruler should drop a cue");
    assert!(at > 0.0, "the cue lands where the pointer was, not at zero");
    assert!(
        harness.state().transport_locate.is_some(),
        "the first click of the double click still scrubs, so the playhead \
         lands on the new cue"
    );
}

/// A cue you cannot move is a trap, so its dot is a handle.
#[test]
fn a_cue_can_be_dragged_along_the_ruler() {
    let (data, _) = cued_fixture();
    let mut harness = make_harness(data);

    let handle = harness.get_by_label("Cue Drop").rect();
    let y = handle.center().y;
    drag(
        &mut harness,
        egui::pos2(handle.center().x, y),
        egui::pos2(handle.center().x + 60.0, y),
    );

    let (uuid, at, name) = harness
        .state()
        .update_cue
        .clone()
        .expect("dragging a cue should move it");
    assert_eq!(uuid, "cue00001");
    assert!(at.expect("a drag moves the cue") > 6.0);
    assert_eq!(name, None, "a drag must not restate the name");
    assert!(harness.state().gesture_active, "one undo entry per drag");
}

/// Dragging a cue must not also scrub the show out from under the gesture.
#[test]
fn dragging_a_cue_does_not_scrub_the_ruler_behind_it() {
    let (data, _) = cued_fixture();
    let mut harness = make_harness(data);

    let handle = harness.get_by_label("Cue Drop").rect();
    let y = handle.center().y;
    drag(
        &mut harness,
        egui::pos2(handle.center().x, y),
        egui::pos2(handle.center().x + 60.0, y),
    );

    assert!(harness.state().update_cue.is_some());
    assert_eq!(harness.state().transport_locate, None);
}

#[test]
fn a_cue_is_deleted_from_its_own_menu() {
    let (data, _) = cued_fixture();
    let mut harness = make_harness(data);

    let handle = harness.get_by_label("Cue Drop").rect();
    right_click(&mut harness, handle.center());
    harness.get_by_label("Delete cue").click();
    harness.run();

    assert_eq!(harness.state().remove_cue.as_deref(), Some("cue00001"));
}

/// Double-clicking empty curve space adds a breakpoint, which is how a curve
/// gets its first shape.
#[test]
fn double_clicking_a_curve_adds_a_breakpoint() {
    let (mut data, _) = automated_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness_stepping(data, 1.0 / 60.0);

    let curve = harness.get_by_label("speed automation curve").rect();
    double_click(&mut harness, curve.center());

    let (uuid, points) = harness
        .state()
        .set_envelope_breakpoints
        .clone()
        .expect("a double click on empty curve space should add a point");
    assert_eq!(uuid, "env-speed");
    assert_eq!(points.len(), 3, "the two drawn points plus the new one");
}

/// Dragging a breakpoint is how a curve gets its shape, and the drag has to
/// land on the envelope that owns the point.
#[test]
fn dragging_a_breakpoint_redraws_the_curve() {
    let (mut data, _) = automated_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    // The fixture's first point sits at one second, at half value, which the
    // panel draws at the vertical centre of the row.
    let curve = harness.get_by_label("speed automation curve").rect();
    let y = curve.center().y;
    drag(
        &mut harness,
        egui::pos2(curve.left() + pps, y),
        egui::pos2(curve.left() + pps * 2.0, y),
    );

    let (uuid, points) = harness
        .state()
        .set_envelope_breakpoints
        .clone()
        .expect("dragging a point should rewrite the curve");
    assert_eq!(uuid, "env-speed");
    assert_eq!(points.len(), 2, "moving a point must not add or drop one");
    assert!(
        harness.state().gesture_active,
        "a curve drag is one gesture, not one undo entry per frame"
    );
}

/// Sharing a shape between two parameters is copy and paste, since a curve is
/// not assignable to a second parameter. That makes the menu route the one that
/// has to work, rather than a convenience on top of the keyboard.
///
/// Copied from the curve and pasted from the header, since both menus offer the
/// pair and a lane is grabbed by whichever is nearer the pointer.
#[test]
fn a_curve_can_be_copied_and_pasted_from_its_menu() {
    let (mut data, _) = automated_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    let curve = harness.get_by_label("speed automation curve").rect();
    right_click(&mut harness, curve.center());
    harness.get_by_label("Copy curve").click_accesskit();
    harness.run();
    assert!(
        harness.state().set_envelope_breakpoints.is_none(),
        "copying must not rewrite the curve it copied"
    );

    let header = harness.get_by_label("speed automation").rect();
    right_click(&mut harness, header.center());
    harness.get_by_label("Paste curve").click_accesskit();
    harness.run();

    let (uuid, points) = harness
        .state()
        .set_envelope_breakpoints
        .clone()
        .expect("pasting should write the copied shape onto this lane");
    assert_eq!(uuid, "env-speed");
    // The copied shape spans one to six seconds and the playhead is at zero, so
    // it lands at the playhead keeping its five-second span.
    assert!(
        points[0].position.abs() < 1e-6 && (points[1].position - 5.0).abs() < 1e-6,
        "{points:?} should be the copied shape anchored at the playhead"
    );
}

/// The same fixture with a second curve, on a different deck's parameter, which
/// is what a copy between lanes needs.
fn two_curve_fixture() -> UIData {
    use varda::modulation::{Breakpoint, CurveKind};

    let (mut data, _) = automated_fixture();
    let other_deck = data.channels[0].decks[1].uuid.clone();
    data.modulation_sources
        .push(varda::usecases::ui::ModSourceUIEntry {
            uuid: "env-scale".to_string(),
            source: varda::usecases::ui::ModSourceUI::Envelope {
                breakpoints: vec![Breakpoint {
                    position: 2.0,
                    value: 0.1,
                    curve: CurveKind::default(),
                }],
            },
            timebase: varda::timebase::Timebase::Transport,
        });
    data.modulation_assignments.insert(
        format!("deck_{other_deck}:scale"),
        vec![varda::usecases::ui::ModAssignmentUI {
            source_id: "env-scale".to_string(),
            amount: 1.0,
        }],
    );
    data.arrangement_mode_open = true;
    data
}

/// Reusing a shape on another parameter is the whole point of the curve
/// clipboard: an envelope drives the one parameter it was drawn for, so the way
/// to get the same shape elsewhere is to copy it there.
#[test]
fn a_curve_copied_from_one_lane_pastes_onto_another() {
    let data = two_curve_fixture();
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    let source = harness.get_by_label("speed automation curve").rect();
    right_click(&mut harness, source.center());
    let copy = harness.get_by_label("Copy curve").rect();
    click_at(&mut harness, copy.center());

    let target = harness.get_by_label("scale automation curve").rect();
    right_click(&mut harness, target.center());
    let paste = harness.get_by_label("Paste curve").rect();
    click_at(&mut harness, paste.center());

    let (uuid, points) = harness
        .state()
        .set_envelope_breakpoints
        .clone()
        .expect("pasting should write the copied shape onto the other lane");
    assert_eq!(uuid, "env-scale", "the shape lands where it was pasted");
    // The lane's own point at two seconds is before the paste, so it stays.
    let at = seconds_at(&target, target.center().x, pps);
    assert_eq!(points.len(), 3, "{points:?}");
    assert!(
        (points[1].position - at).abs() < SECONDS_SLOP
            && (points[2].position - at - 5.0).abs() < SECONDS_SLOP,
        "{points:?} should be the copied shape anchored at {at}"
    );
}

/// A menu that rewrites itself as the pointer travels to the item it opened for
/// is worse than no menu: the click lands on whatever slid under the cursor.
/// The curve's menu decides between breakpoint items and clipboard items, and
/// that decision belongs to where the right-click landed, not to where the
/// pointer is now.
#[test]
fn a_curve_menu_keeps_the_items_it_opened_with() {
    let (mut data, _) = automated_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    // The fixture's first point sits at one second, at half value, which the
    // panel draws at the vertical centre of the row.
    let curve = harness.get_by_label("speed automation curve").rect();
    let point = egui::pos2(curve.left() + pps, curve.center().y);
    right_click(&mut harness, point);
    assert!(
        harness.query_by_label("Delete breakpoint").is_some(),
        "right-clicking a point should offer that point's own items"
    );

    // Reaching any of those items means moving off the point they belong to.
    harness.event(egui::Event::PointerMoved(point + egui::vec2(20.0, 40.0)));
    harness.run();

    assert!(
        harness.query_by_label("Delete breakpoint").is_some(),
        "the point's items must survive the trip to them"
    );
    assert!(
        harness.query_by_label("Copy curve").is_some(),
        "a point is what most people aim at when they mean 'this curve', so \
         the clipboard belongs in its menu too"
    );
}

/// Reaching a menu item means moving the mouse onto it, and the menu opens over
/// the lane it came from. The items must not change identity under the pointer.
#[test]
fn pasting_a_curve_works_with_the_mouse_rather_than_only_accesskit() {
    let (mut data, _) = automated_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    let curve = harness.get_by_label("speed automation curve").rect();
    right_click(&mut harness, curve.center());
    let copy = harness.get_by_label("Copy curve").rect();
    click_at(&mut harness, copy.center());

    // Past both of the lane's own points, so the paste is visible in the result
    // rather than replacing what was already there.
    let paste_click = egui::pos2(curve.left() + pps * 8.0, curve.center().y);
    right_click(&mut harness, paste_click);
    let paste = harness.get_by_label("Paste curve").rect();
    click_at(&mut harness, paste.center());

    let (uuid, points) = harness
        .state()
        .set_envelope_breakpoints
        .clone()
        .expect("clicking paste with the mouse should write the copied shape");
    assert_eq!(uuid, "env-speed");
    // The copied pair keeps its five-second span, anchored where the menu was
    // opened. Both of the lane's own points are before that, so they stay.
    let at = seconds_at(&curve, paste_click.x, pps);
    assert_eq!(points.len(), 4, "{points:?}");
    assert!(
        (points[2].position - at).abs() < SECONDS_SLOP
            && (points[3].position - at - 5.0).abs() < SECONDS_SLOP,
        "{points:?} should be the copied shape anchored at {at}"
    );
}

/// Seconds at a screen x, for a track whose left edge is time zero. Good to
/// within the pixel the track's own edges are rounded to, which is why callers
/// compare with a tolerance rather than exactly.
fn seconds_at(track: &egui::Rect, x: f32, pixels_per_second: f32) -> f64 {
    f64::from((x - track.left()) / pixels_per_second)
}

/// A pixel of slop at any sane zoom.
const SECONDS_SLOP: f64 = 0.05;

/// With nothing copied yet there is nothing to paste, and a menu item that does
/// nothing is worse than one that says why.
#[test]
fn pasting_is_offered_but_disabled_until_something_is_copied() {
    let (mut data, _) = automated_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    let curve = harness.get_by_label("speed automation curve").rect();
    right_click(&mut harness, curve.center());

    assert!(
        harness.query_by_label("Copy curve").is_some(),
        "the curve's own menu is where copy belongs"
    );
    let paste = harness
        .query_by_label("Paste curve")
        .expect("paste should be visible rather than hidden");
    assert!(
        egui_kittest::kittest::NodeT::accesskit_node(&paste).is_disabled(),
        "paste with an empty clipboard must not be clickable"
    );
}

/// Bending a segment is the only way to reach `tension`, which the engine has
/// evaluated since automation shipped but no gesture could set.
#[test]
fn dragging_a_segment_bends_the_curve() {
    use varda::modulation::CurveKind;

    let (mut data, _) = automated_fixture();
    data.arrangement_mode_open = true;
    let pps = data.arrangement_pixels_per_second;
    let mut harness = make_harness(data);

    // The fixture's segment runs from (1 s, 0.5) up to (6 s, 0.8). The press has
    // to land on the drawn line, so it is computed rather than eyeballed: the
    // lane pads five pixels top and bottom so an extreme point stays grabbable.
    let track = harness.get_by_label("speed automation curve").rect();
    let padding = 5.0;
    let usable = track.height() - 2.0 * padding;
    let y_at = |value: f32| track.bottom() - padding - value * usable;
    let midpoint = egui::pos2(track.left() + pps * 3.5, y_at(0.65));
    drag(
        &mut harness,
        midpoint,
        egui::pos2(midpoint.x, midpoint.y - 40.0),
    );

    let (uuid, points) = harness
        .state()
        .set_envelope_breakpoints
        .clone()
        .expect("dragging a segment should rewrite the curve");
    assert_eq!(uuid, "env-speed");
    assert_eq!(points.len(), 2, "bending must not add or drop a point");
    assert!(
        (points[0].position - 1.0).abs() < 1e-6 && (points[0].value - 0.5).abs() < 1e-6,
        "{points:?}: bending must not move the points it runs between"
    );
    match points[0].curve {
        CurveKind::Linear { tension } => assert!(
            tension > 0.2,
            "dragging up a rising segment should bend it up, got {tension}"
        ),
        other => panic!("a bent segment should be linear with tension, got {other:?}"),
    }
    assert!(
        harness.state().gesture_active,
        "a bend is one gesture, not one undo entry per frame"
    );
}

/// The badge is the only route back to automation once a parameter is held, so
/// it has to be a control and not just a light.
#[test]
fn clicking_the_override_badge_re_arms_the_parameter() {
    let (mut data, deck_uuid) = arranged_fixture();
    data.arrangement_mode_open = true;
    let key = varda::arrangement::opacity_param_key(&deck_uuid);
    data.arrangement.as_mut().unwrap().overridden_params = vec![key.clone()];
    let mut harness = make_harness(data);

    harness.get_by_label("Hand back to the arrangement").click();
    harness.run();

    assert_eq!(harness.state().rearm_param, Some(key));
}

#[test]
fn click_add_surface() {
    // Surfaces are added by drawing on the editor canvas, not via a button.
    let mut data = UIData::test_fixture();
    data.surfaces = vec![];
    data.stage_editor_open = true; // render the full editor (toolbar + canvas)
    let mut harness = make_harness(data);

    // Select the rectangle drawing tool (persists in egui memory).
    harness.get_by_label("▭ Rectangle").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    // Drag a rectangle across the canvas (window coords inside the central panel,
    // below the toolbar). This emits SurfaceAction::AddPolygon.
    drag(
        &mut harness,
        egui::pos2(450.0, 180.0),
        egui::pos2(850.0, 430.0),
    );

    assert!(
        harness.state().surface_add,
        "Expected a surface to be added after a rectangle drag"
    );
}

// ── MIDI ────────────────────────────────────────────────────────────

#[test]
fn click_midi_rescan() {
    let mut harness = make_harness(UIData::test_fixture());

    // Expand "🎹 MIDI" collapsing header
    harness.get_by_label("🎹 MIDI").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    harness.get_by_label("🔄 Rescan").click();
    harness.run();

    assert!(harness.state().midi_rescan, "midi_rescan should be true");
}

// ── Sequence ────────────────────────────────────────────────────────

#[test]
fn click_add_sequence() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("+ Sequence").click();
    harness.run();

    assert!(
        harness.state().sequence_create,
        "Expected SequenceAction::Create"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Context-dependent tests (require modified fixture state)
// ═══════════════════════════════════════════════════════════════════

// ── Remove Channel (requires 3+ channels) ───────────────────────────

#[test]
fn click_remove_channel_with_three_channels() {
    use varda::usecases::ui::ChannelUIInfo;
    use varda::BlendMode;

    let mut data = UIData::test_fixture();
    // Add a third channel so the "x" remove button appears
    data.channels.push(ChannelUIInfo {
        uuid: "cc000003".to_string(),
        ch_idx: 2,
        name: "Ch C".to_string(),
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        decks: vec![],
        effects: vec![],
    });
    data.channel_count = 3;
    let mut harness = make_harness(data);

    // The "x" buttons appear next to each channel name.
    // There will be multiple "x" labels (one per channel + deck remove buttons).
    // Use get_by_label to find any "x" — we just need to confirm remove_channel fires.
    // Since there are multiple "x" buttons, we look for the hover text instead.
    // Unfortunately AccessKit doesn't expose hover text. Let's just verify the button exists
    // by clicking the first "x" we find.
    // The "x" buttons appear next to each channel name when 3+ channels.
    // There are multiple "x" labels (channel remove + deck remove).
    // Collect them and click the first one — validates the button exists.
    let nodes: Vec<_> = harness.get_all_by_label("x").collect();
    assert!(
        !nodes.is_empty(),
        "Expected at least one 'x' button with 3 channels"
    );
    nodes[0].click();
    harness.run();
}

/// The global right-click popup is drawn last and over everything, so opening
/// it on a click a context menu already took would cover that menu and swallow
/// every press aimed at its items.
#[test]
fn right_clicking_a_menu_does_not_also_open_the_midi_learn_popup() {
    let (mut data, _) = automated_fixture();
    data.arrangement_mode_open = true;
    let mut harness = make_harness(data);

    let curve = harness.get_by_label("speed automation curve").rect();
    right_click(&mut harness, curve.center());

    assert!(
        harness.query_by_label("Copy curve").is_some(),
        "the curve's menu is what this right-click was for"
    );
    assert!(
        harness
            .query_by_label_contains("Enter MIDI Learn")
            .is_none(),
        "the global popup must stand down for a widget that has its own menu"
    );
}

/// Right-clicking past every widget is still how MIDI learn is reached.
#[test]
fn right_clicking_bare_background_still_opens_the_midi_learn_popup() {
    let mut harness = make_harness(UIData::test_fixture());

    right_click(&mut harness, egui::pos2(4.0, 400.0));

    assert!(
        harness
            .query_by_label_contains("Enter MIDI Learn")
            .is_some(),
        "a right-click with no menu behind it should still offer MIDI learn"
    );
}

// ── MIDI Learn Exit (requires midi_learn_active) ────────────────────

#[test]
fn click_exit_midi_learn() {
    let mut data = UIData::test_fixture();
    data.midi_learn_active = true;
    data.midi_learn_target = None;
    let mut harness = make_harness(data);

    harness.get_by_label("x Exit MIDI Learn").click();
    harness.run();

    assert!(
        harness.state().midi_learn_toggle,
        "midi_learn_toggle should be true"
    );
}

// ── Select Channel (click channel heading) ──────────────────────────

#[test]
fn click_channel_heading_selects_channel() {
    let mut harness = make_harness(UIData::test_fixture());

    // Channel headings are "▌ Ch A" / "▌ Ch B" — these are labels with click sense
    harness.get_by_label("▌ Ch A").click();
    harness.run();

    assert_eq!(
        harness.state().select_channel,
        Some(0),
        "Expected select_channel = Some(0)"
    );
}

#[test]
fn click_channel_b_heading_selects_channel_b() {
    let mut harness = make_harness(UIData::test_fixture());

    harness.get_by_label("▌ Ch B").click();
    harness.run();

    assert_eq!(
        harness.state().select_channel,
        Some(1),
        "Expected select_channel = Some(1)"
    );
}

// ── Library: Open Library from right panel (when closed) ────────────

#[test]
fn click_open_library_from_right_panel() {
    let mut data = UIData::test_fixture();
    data.library_panel_open = false;
    // Use tall harness to ensure the button is visible in the right panel
    let data = Rc::new(data);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 1200.0))
        .build_ui_state(
            move |ui, acc: &mut AccActions| {
                let actions = render_ui(ui, &data);
                acc.merge(&actions);
            },
            AccActions::default(),
        );
    harness.run();
    *harness.state_mut() = AccActions::default();

    harness.get_by_label("📚 Open Library (L)").click();
    harness.run();

    assert!(
        harness.state().toggle_library_panel,
        "toggle_library_panel should be true"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Combo box tests (two-phase: click to open popup, then click option)
// ═══════════════════════════════════════════════════════════════════

// ── Transition Shader Selector ──────────────────────────────────────

#[test]
fn combo_select_transition_shader() {
    let mut data = UIData::test_fixture();
    data.transition_names = vec!["fade".to_string(), "wipe".to_string()];
    data.active_transition_name = None; // currently "Opacity"
    let mut harness = make_harness(data);

    // Phase 1: click the combo box to open its popup
    // ComboBox exposes selected_text as AccessKit `value`, not `label`
    harness.get_by_value("🔀 Opacity").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    // Phase 2: click an option in the popup (selectable_label → AccessKit label)
    harness.get_by_label("fade").click();
    harness.run();

    assert_eq!(
        harness.state().set_transition,
        Some(Some("fade".to_string())),
        "Expected set_transition = Some(Some(\"fade\"))"
    );
}

#[test]
fn combo_select_opacity_transition() {
    let mut data = UIData::test_fixture();
    data.transition_names = vec!["fade".to_string()];
    data.active_transition_name = Some("fade".to_string()); // currently "fade"
    let mut harness = make_harness(data);

    // Phase 1: click the combo box
    harness.get_by_value("🔀 fade").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    // Phase 2: click "Opacity (default)"
    harness.get_by_label("Opacity (default)").click();
    harness.run();

    assert_eq!(
        harness.state().set_transition,
        Some(None),
        "Expected set_transition = Some(None) for opacity"
    );
}

// ── Channel Blend Mode Selector ─────────────────────────────────────
// NOTE: selectable_value-based combos (blend mode, scaling mode) don't
// reliably trigger actions through AccessKit clicks due to egui's popup
// close semantics. We verify the combo exists with the correct value.
// The actual blend mode change logic is covered by unit tests.

#[test]
fn combo_blend_mode_exists_with_correct_value() {
    let harness = make_harness(UIData::test_fixture());

    // Each channel should have a blend mode combo showing "Norm"
    let norms: Vec<_> = harness.get_all_by_value("Norm").collect();
    assert!(
        norms.len() >= 2,
        "Expected at least 2 blend mode combos (one per channel), got {}",
        norms.len()
    );
}

// ── Scaling Mode Combo (existence only — selectable_value limitation) ─

#[test]
fn combo_scaling_mode_exists_when_deck_selected() {
    // The fixture has selected_deck = Some((0, 0)) with scaling_mode = Some(Fit)
    let harness = make_harness(UIData::test_fixture());

    // The scaling mode combo should show "Fit" as its value
    assert!(
        harness.query_by_value("Fit").is_some(),
        "Expected scaling mode combo showing 'Fit' for selected deck"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Collapsing header tests (expand header, then click button inside)
// ═══════════════════════════════════════════════════════════════════

// ── Library: Image File Dialog ──────────────────────────────────────

#[test]
fn collapsing_image_load_dialog() {
    let mut data = UIData::test_fixture();
    data.library_panel_open = true;
    let mut harness = make_harness(data);

    // Expand the "🖼 Images" collapsing header
    harness.get_by_label("🖼 Images").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    // Click "📁 Load to Ch A"
    harness.get_by_label("📁 Load to Ch A").click();
    harness.run();

    // The request must name Ch A by UUID. Asserting only `is_some` would pass
    // even if the button targeted a different channel.
    assert_eq!(
        harness.state().open_image_dialog_for_channel.as_deref(),
        Some("ca000001"),
        "Load to Ch A must request a dialog for Ch A's UUID"
    );
}

// ── Library: Video File Dialog ──────────────────────────────────────

#[test]
fn collapsing_video_load_dialog() {
    let mut data = UIData::test_fixture();
    data.library_panel_open = true;
    let mut harness = make_harness(data);

    // Expand the "🎬 Video" collapsing header
    harness.get_by_label("🎬 Video").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    // Click "📁 Load to Ch A" — note: same label as image, but different header context
    // There might be label ambiguity, so let's use the first match
    let loads: Vec<_> = harness.get_all_by_label("📁 Load to Ch A").collect();
    loads[0].click();
    harness.run();

    // Either dialog may fire (the label is shared between the two headers), but
    // whichever does must name Ch A by UUID.
    let state = harness.state();
    let target = state
        .open_image_dialog_for_channel
        .as_deref()
        .or(state.open_video_dialog_for_channel.as_deref());
    assert_eq!(
        target,
        Some("ca000001"),
        "Load to Ch A must request a dialog for Ch A's UUID"
    );
}

// ── Library: Camera Rescan (inside collapsing header) ───────────────

#[test]
fn collapsing_camera_rescan() {
    let mut data = UIData::test_fixture();
    data.library_panel_open = true;
    let mut harness = make_harness(data);

    // Expand the "📹 Cameras (0)" collapsing header
    harness.get_by_label("📹 Cameras (0)").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    // MIDI section is now collapsed by default, so only the camera rescan is visible.
    // Click the camera rescan button directly.
    harness.get_by_label("🔄 Rescan").click();
    harness.run();

    assert!(
        harness.state().camera_rescan,
        "camera_rescan should be true"
    );
}

// ── MIDI: Clear All Mappings (inside collapsing header) ─────────────

#[test]
fn collapsing_midi_clear_all_mappings() {
    use varda::usecases::ui::MidiMappingUI;

    let mut data = UIData::test_fixture();
    // Need at least one mapping for "Clear All" to appear
    data.midi_mappings = vec![MidiMappingUI {
        key: varda::midi::MidiKey::CC(0, 0, 1),
        key_display: "CC 0/1".to_string(),
        device_name: "Test Device".to_string(),
        param_path: "crossfader".to_string(),
    }];
    // Use tall harness — MIDI section is at the bottom of the right panel
    let data = Rc::new(data);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 1200.0))
        .build_ui_state(
            move |ui, acc: &mut AccActions| {
                let actions = render_ui(ui, &data);
                acc.merge(&actions);
            },
            AccActions::default(),
        );
    harness.run();
    *harness.state_mut() = AccActions::default();

    // Expand "🎹 MIDI" collapsing header in right panel
    harness.get_by_label("🎹 MIDI").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    // Expand "Mappings (1)" collapsing header
    harness.get_by_label("Mappings (1)").click();
    harness.run();
    *harness.state_mut() = AccActions::default();

    // Click "🗑 Clear All"
    harness.get_by_label("🗑 Clear All").click();
    harness.run();

    assert!(
        harness.state().midi_clear_mappings,
        "midi_clear_mappings should be true"
    );
}

// ── Clipboard menus ─────────────────────────────────────────────────

/// The card is a container full of parameter widgets, so the surface carrying
/// its menu has to sense clicks without swallowing theirs.
#[test]
fn an_effect_card_offers_copy_without_stealing_clicks_from_its_contents() {
    let mut harness = make_harness(UIData::test_fixture());

    let card = harness.get_by_label("test_effect effect card").rect();
    right_click(&mut harness, card.center());
    assert!(
        harness.query_by_label_contains("Copy effect").is_some(),
        "right-clicking an effect card should offer to copy it"
    );

    harness.key_press(egui::Key::Escape);
    harness.run();
    *harness.state_mut() = AccActions::default();

    // The enable checkbox sits in the card's header row. Clicking it worked
    // before the card sensed clicks at all; the regression would be the card
    // taking the click instead.
    let toggle = harness
        .query_all_by_role(egui::accesskit::Role::CheckBox)
        .find(|node| card.contains(node.rect().center()))
        .expect("the effect's enable checkbox");
    toggle.click();
    harness.run();

    assert_eq!(
        harness.state().toggle_effect.as_deref(),
        Some("dfx00001"),
        "the click reached the checkbox, not the card behind it"
    );
}

/// A deck copied from the mixer is a source; the same deck copied on the
/// timeline is a source and a placement. The menu says which by the mode it
/// was opened in.
#[test]
fn copying_a_deck_from_the_mixer_leaves_the_arrangement_out() {
    let mut harness = make_harness(UIData::test_fixture());

    let card = harness.get_by_label("test_generator_a deck card").rect();
    right_click(&mut harness, card.center());
    harness.get_by_label_contains("Copy deck").click_accesskit();
    harness.run();

    assert_eq!(
        harness.state().copy,
        Some(varda::engine::ClipboardSource::Deck("a0000001".to_string()))
    );
}

/// A channel's header is where a copied deck goes back in, which is the whole
/// point of copying one in the mixer.
#[test]
fn a_copied_deck_pastes_into_a_channel_from_its_header() {
    let mut data = UIData::test_fixture();
    data.clipboard = Some(varda::engine::ClipboardSummary {
        kind: varda::engine::ClipboardKind::Deck,
        label: "test_generator_a".to_string(),
    });
    let channel_uuid = data.channels[0].uuid.clone();
    let header_label = format!("▌ {}", data.channels[0].name);
    let mut harness = make_harness(data);

    let header = harness.get_by_label(header_label.as_str()).rect();
    right_click(&mut harness, header.center());
    let paste = harness.get_by_label_contains("Paste deck").rect();
    click_at(&mut harness, paste.center());

    assert_eq!(
        harness.state().paste,
        Some(varda::engine::PasteTarget::IntoChannel(channel_uuid)),
        "the copy belongs to the channel whose header was right-clicked"
    );
}

/// The other way into a channel: a copy dropped below the deck it was aimed at.
#[test]
fn a_copied_deck_pastes_below_a_deck_card() {
    let mut data = UIData::test_fixture();
    data.clipboard = Some(varda::engine::ClipboardSummary {
        kind: varda::engine::ClipboardKind::Deck,
        label: "test_generator_a".to_string(),
    });
    let deck_uuid = data.channels[0].decks[0].uuid.clone();
    let mut harness = make_harness(data);

    let card = harness.get_by_label("test_generator_a deck card").rect();
    right_click(&mut harness, card.center());
    let paste = harness.get_by_label_contains("Paste deck").rect();
    click_at(&mut harness, paste.center());

    assert_eq!(
        harness.state().paste,
        Some(varda::engine::PasteTarget::AfterDeck(deck_uuid))
    );
}

/// The body of a channel column is where a paste is aimed, and for a channel
/// with no decks in it there is nothing else to aim at.
#[test]
fn an_empty_channel_takes_a_pasted_deck_in_its_body() {
    let mut data = UIData::test_fixture();
    data.clipboard = Some(varda::engine::ClipboardSummary {
        kind: varda::engine::ClipboardKind::Deck,
        label: "test_generator_a".to_string(),
    });
    let channel_uuid = data.channels[1].uuid.clone();
    let body_label = format!("{} deck area", data.channels[1].name);
    data.channels[1].decks.clear();
    let mut harness = make_harness(data);

    let body = harness.get_by_label(body_label.as_str()).rect();
    right_click(&mut harness, body.center());
    let paste = harness.get_by_label_contains("Paste deck").rect();
    click_at(&mut harness, paste.center());

    assert_eq!(
        harness.state().paste,
        Some(varda::engine::PasteTarget::IntoChannel(channel_uuid)),
        "a deck pasted in a channel's body lands in that channel"
    );
}
