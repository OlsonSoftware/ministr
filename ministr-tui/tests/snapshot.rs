//! Deterministic frame renders pinned as insta snapshots — the harness
//! every console state ships in (GUI-BLUEPRINT-v8 §8: a state without a
//! snapshot does not ship). Sizes match the scrutiny doctrine: spacious
//! 120×36, compressed 80×24 (§3: strips tighten below 90 columns), and
//! narrow 60×20 (stacked bars). Char snapshots pin composition; styled
//! (debug-buffer) snapshots pin selection brightness and the ladder
//! rungs, where the signal is intensity and color.

use std::time::{Duration, Instant};

use ministr_tui::app::{App, View};
use ministr_tui::console::{ConsoleModel, Standing, Strip};
use ministr_tui::detail::{Detail, Facts, PathsEditor};
use ministr_tui::ease::GLIDE;
use ministr_tui::engine::{Action, EngineState, ProgressTarget};
use ministr_tui::motion::MAX_TRANSITION;
use ministr_tui::palette::ColorDepth;
use ministr_tui::patchin::PatchIn;
use ministr_tui::strings;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// Render one frame as it stands at `now` and return the buffer as
/// text. A zero delta keeps any playing transition at its current
/// instant, and the eased meters are a pure function of `now`, so
/// every render stays deterministic.
fn render_at(app: &mut App, width: u16, height: u16, now: Instant) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| app.draw(frame, Duration::ZERO, now))
        .expect("draw frame");
    terminal.backend().to_string()
}

/// [`render_at`] for the states with no glide in play, where the
/// instant cannot matter.
fn render(app: &mut App, width: u16, height: u16) -> String {
    render_at(app, width, height, Instant::now())
}

/// Render one frame and return the full buffer, styles included — the
/// harness for what chars alone cannot pin (brightness, the ladder).
fn render_styled(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    let now = Instant::now();
    terminal
        .draw(|frame| app.draw(frame, Duration::ZERO, now))
        .expect("draw frame");
    terminal.backend().buffer().clone()
}

fn strip(name: &str, standing: Standing, files: usize) -> Strip {
    Strip {
        id: name.to_owned(),
        name: name.to_owned(),
        standing,
        files,
    }
}

/// The populated console: the three everyday standings, at a fixed
/// meter percent (live motion is the next chunk). Three strips so
/// every one is on screen at 120 columns — the rarer standings
/// (waiting, warming up, failed) carry their own snapshot.
fn populated() -> EngineState {
    EngineState::Running(ConsoleModel {
        version: "0.7.0".to_owned(),
        strips: vec![
            strip("ministr", Standing::UpToDate, 812),
            strip("cohaero", Standing::Building { fraction: 0.43 }, 1204),
            strip("tock", Standing::NeedsUpdate, 640),
        ],
    })
}

fn running(strips: Vec<Strip>) -> EngineState {
    EngineState::Running(ConsoleModel {
        version: "0.7.0".to_owned(),
        strips,
    })
}

// --- S1 designed states, spacious / compressed / stacked ---------------

#[test]
fn console_populated_spacious() {
    let mut app = App::with_engine(populated());
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn console_populated_compressed() {
    let mut app = App::with_engine(populated());
    insta::assert_snapshot!(render(&mut app, 80, 24));
}

#[test]
fn console_populated_stacked() {
    let mut app = App::with_engine(populated());
    insta::assert_snapshot!(render(&mut app, 60, 20));
}

#[test]
fn console_first_run_spacious() {
    let mut app = App::with_engine(running(Vec::new()));
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn console_first_run_narrow() {
    let mut app = App::with_engine(running(Vec::new()));
    insta::assert_snapshot!(render(&mut app, 60, 20));
}

#[test]
fn console_engine_starting() {
    let mut app = App::with_engine(EngineState::Starting);
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn console_engine_unreachable() {
    let mut app = App::with_engine(EngineState::Unreachable);
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn console_waiting_warming_failed() {
    let mut app = App::with_engine(running(vec![
        strip("ministr", Standing::Waiting, 812),
        strip("cohaero", Standing::Warming, 0),
        strip("tock", Standing::Failed, 97),
    ]));
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

// --- meters at fixed fractional percents (sub-cell precision) ----------

#[test]
fn console_meters_at_fractional_percents() {
    let mut app = App::with_engine(running(vec![
        strip("alpha", Standing::Building { fraction: 0.125 }, 100),
        strip("beta", Standing::Building { fraction: 0.43 }, 100),
        strip("gamma", Standing::Building { fraction: 0.87 }, 100),
    ]));
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

// --- overflow travel ----------------------------------------------------

#[test]
fn console_overflow_edge_markers_spacious() {
    let strips = (1..=10)
        .map(|i| strip(&format!("project-{i:02}"), Standing::UpToDate, i * 100))
        .collect();
    let mut app = App::with_engine(running(strips));
    app.selected = 9;
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn console_overflow_edge_markers_stacked() {
    let strips = (1..=12)
        .map(|i| strip(&format!("project-{i:02}"), Standing::UpToDate, i * 100))
        .collect();
    let mut app = App::with_engine(running(strips));
    app.selected = 11;
    insta::assert_snapshot!(render(&mut app, 60, 12));
}

// --- what chars cannot pin: selection brightness + the ladder ----------

#[test]
fn console_selection_brightens_the_frame_not_a_color() {
    let mut app = App::with_engine(populated());
    app.selected = 1;
    insta::assert_debug_snapshot!(render_styled(&mut app, 80, 24));
}

#[test]
fn console_ladder_rung_256() {
    let mut app = App::with_engine(populated()).with_depth(ColorDepth::Ansi256);
    insta::assert_debug_snapshot!(render_styled(&mut app, 80, 24));
}

#[test]
fn console_ladder_rung_16() {
    let mut app = App::with_engine(populated()).with_depth(ColorDepth::Ansi16);
    insta::assert_debug_snapshot!(render_styled(&mut app, 80, 24));
}

#[test]
fn console_ladder_rung_mono() {
    let mut app = App::with_engine(populated()).with_depth(ColorDepth::Mono);
    insta::assert_debug_snapshot!(render_styled(&mut app, 80, 24));
}

// --- topology: selection moves, patch-in / remove choreography ---------

#[test]
fn selection_moves_left_right_and_clamps() {
    let mut app = App::with_engine(populated());
    assert!(
        !app.on_key(KeyEvent::from(KeyCode::Left)),
        "left edge holds"
    );
    assert!(app.on_key(KeyEvent::from(KeyCode::Right)));
    assert!(app.on_key(KeyEvent::from(KeyCode::Right)));
    assert_eq!(app.selected, 2);
    assert!(
        !app.on_key(KeyEvent::from(KeyCode::Right)),
        "right edge holds"
    );
}

#[test]
fn a_patched_in_project_materializes() {
    let mut app = App::with_engine(populated());
    let mut grown = populated();
    if let EngineState::Running(model) = &mut grown {
        model.strips.push(strip("fresh", Standing::Waiting, 0));
    }
    let now = Instant::now();
    assert!(app.absorb(grown));
    assert!(app.animating(now), "patch-in plays a transition");
    // The transition obeys the motion law: over within the ceiling.
    let _ = render_at(&mut app, 120, 36, now);
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).expect("test terminal");
    terminal
        .draw(|frame| app.draw(frame, MAX_TRANSITION, now))
        .expect("draw frame");
    terminal
        .draw(|frame| app.draw(frame, Duration::ZERO, now))
        .expect("draw frame");
    assert!(!app.animating(now), "transitions never loop");
}

#[test]
fn a_removed_project_dissolves_in_place() {
    let mut app = App::with_engine(populated());
    let mut shrunk = populated();
    if let EngineState::Running(model) = &mut shrunk {
        model.strips.remove(1);
    }
    let now = Instant::now();
    assert!(app.absorb(shrunk));
    assert!(app.animating(now), "removal plays a transition");
    assert_eq!(
        app.strips().len(),
        3,
        "the ghost strip holds its place while dissolving"
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).expect("test terminal");
    terminal
        .draw(|frame| app.draw(frame, MAX_TRANSITION, now))
        .expect("draw frame");
    terminal
        .draw(|frame| app.draw(frame, Duration::ZERO, now))
        .expect("draw frame");
    assert!(!app.animating(now));
    assert_eq!(app.strips().len(), 2, "the ghost leaves with the dissolve");
}

// --- live meters: the needle glides, never steps -----------------------

/// One project mid-build at 20%, as the probe reported it.
fn one_building() -> EngineState {
    running(vec![strip(
        "cohaero",
        Standing::Building { fraction: 0.20 },
        1204,
    )])
}

/// A progress report pointing cohaero's needle at `fraction`.
fn report(fraction: f64) -> Vec<ProgressTarget> {
    vec![ProgressTarget {
        id: "cohaero".to_owned(),
        fraction,
    }]
}

#[test]
fn meter_glides_between_progress_reports() {
    let t0 = Instant::now();
    let mut app = App::with_engine(one_building());
    assert!(app.absorb_progress(&report(0.60), t0));

    // The needle leaves from where it sat, crosses, and settles on the
    // report — three fixed instants, three exact frames.
    insta::assert_snapshot!("meter_glide_start", render_at(&mut app, 120, 36, t0));
    insta::assert_snapshot!(
        "meter_glide_mid",
        render_at(&mut app, 120, 36, t0 + GLIDE / 2)
    );
    insta::assert_snapshot!(
        "meter_glide_settled",
        render_at(&mut app, 120, 36, t0 + GLIDE)
    );
}

#[test]
fn a_gliding_needle_keeps_the_frame_clock_running_then_rests() {
    let t0 = Instant::now();
    let mut app = App::with_engine(one_building());
    assert!(!app.animating(t0), "no report yet: the console rests");
    assert!(app.absorb_progress(&report(0.60), t0));
    assert!(app.animating(t0 + GLIDE / 2), "mid-glide the clock runs");
    assert!(
        !app.animating(t0 + GLIDE),
        "a settled needle returns the console to rest"
    );
}

#[test]
fn an_unchanged_report_does_not_wake_the_console() {
    let t0 = Instant::now();
    let mut app = App::with_engine(one_building());
    assert!(
        !app.absorb_progress(&report(0.20), t0),
        "a report matching the probe's position moves nothing"
    );
    assert!(!app.animating(t0), "and the console stays at rest");
}

#[test]
fn a_report_for_a_strip_that_is_not_building_is_ignored() {
    let t0 = Instant::now();
    let mut app = App::with_engine(running(vec![strip("cohaero", Standing::UpToDate, 1204)]));
    assert!(!app.absorb_progress(&report(0.60), t0));
    assert!(!app.animating(t0 + GLIDE / 2));
}

#[test]
fn a_finished_build_takes_its_needle_with_it() {
    let t0 = Instant::now();
    let mut app = App::with_engine(one_building());
    assert!(app.absorb_progress(&report(0.60), t0));
    assert!(app.animating(t0 + GLIDE / 2));
    assert!(app.absorb(running(vec![strip("cohaero", Standing::UpToDate, 1204,)])));
    assert!(
        !app.animating(t0 + GLIDE / 2),
        "the strip stopped building: its glide is gone, the console rests"
    );
    assert!(!app.building());
}

#[test]
fn building_reports_only_while_a_strip_builds() {
    let mut app = App::with_engine(one_building());
    assert!(app.building());
    let _ = app.absorb(running(vec![strip("cohaero", Standing::UpToDate, 1204)]));
    assert!(!app.building());
    let _ = app.absorb(EngineState::Unreachable);
    assert!(!app.building());
}

// --- S2 strip detail: designed states -----------------------------------

/// The opened project's slower facts, every phrase fixed so the render
/// is deterministic.
fn cohaero_facts() -> Facts {
    Facts {
        id: "cohaero".to_owned(),
        paths: vec![
            "/Users/alrik/Code/cohaero".to_owned(),
            "/Users/alrik/Code/cohaero-content".to_owned(),
        ],
        sections: 5210,
        symbols: 903,
        updated: Some("4 minutes ago".to_owned()),
        attention: Some("3 files changed · 1 new since the last build".to_owned()),
    }
}

/// The console with cohaero opened as S2.
fn opened(facts: Option<Facts>) -> App {
    let mut app = App::with_engine(populated());
    app.selected = 1;
    app.view = View::Detail(Detail {
        id: "cohaero".to_owned(),
        name: "cohaero".to_owned(),
        standing: Standing::NeedsUpdate,
        files: 1204,
        facts,
        editing: None,
    });
    app
}

#[test]
fn detail_still_loading_spacious() {
    let mut app = opened(None);
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn detail_filled_spacious() {
    let mut app = opened(Some(cohaero_facts()));
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn detail_filled_compressed() {
    let mut app = opened(Some(cohaero_facts()));
    insta::assert_snapshot!(render(&mut app, 80, 24));
}

#[test]
fn detail_building_shows_the_live_meter() {
    let mut app = opened(Some(Facts {
        attention: None,
        ..cohaero_facts()
    }));
    if let View::Detail(open) = &mut app.view {
        open.standing = Standing::Building { fraction: 0.43 };
    }
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn detail_editing_paths() {
    let mut app = opened(Some(cohaero_facts()));
    if let View::Detail(open) = &mut app.view {
        open.editing = Some(PathsEditor::new(&cohaero_facts().paths));
    }
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn detail_remove_confirms_inline() {
    let mut app = opened(Some(cohaero_facts()));
    app.confirming_remove = true;
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn detail_verb_failure_notice() {
    let mut app = opened(Some(cohaero_facts()));
    app.notice = Some(strings::NOTICE_REBUILD_FAILED.to_owned());
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

// --- S3 patch in: designed states ---------------------------------------

/// The console with the patch-in panel up over it, at a fixed path.
fn patching_in() -> App {
    let mut app = App::with_engine(populated());
    app.view = View::PatchIn(PatchIn::new("/Users/alrik/Code/newproject"));
    app
}

#[test]
fn patch_in_panel_spacious() {
    let mut app = patching_in();
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn patch_in_panel_narrow() {
    let mut app = patching_in();
    insta::assert_snapshot!(render(&mut app, 60, 20));
}

#[test]
fn patch_in_failure_notice() {
    let mut app = patching_in();
    app.notice = Some(strings::NOTICE_ADD_FAILED.to_owned());
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

// --- inline remove confirm on the console strip -------------------------

#[test]
fn console_remove_confirms_on_the_strip_spacious() {
    let mut app = App::with_engine(populated());
    app.selected = 1;
    app.confirming_remove = true;
    insta::assert_snapshot!(render(&mut app, 120, 36));
}

#[test]
fn console_remove_confirms_on_the_bar_stacked() {
    let mut app = App::with_engine(populated());
    app.selected = 1;
    app.confirming_remove = true;
    insta::assert_snapshot!(render(&mut app, 60, 20));
}

// --- transitions: start and end frames at fixed instants ----------------

/// Play the current transition out: one frame at the motion-law
/// ceiling, one settling frame.
fn play_out(app: &mut App, now: Instant) {
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).expect("test terminal");
    terminal
        .draw(|frame| app.draw(frame, MAX_TRANSITION, now))
        .expect("draw frame");
    terminal
        .draw(|frame| app.draw(frame, Duration::ZERO, now))
        .expect("draw frame");
}

#[test]
fn opening_a_strip_sweeps_start_and_end_frames() {
    let mut app = App::with_engine(populated());
    assert!(app.on_key(KeyEvent::from(KeyCode::Enter)));
    assert!(matches!(app.pending, Some(Action::OpenDetail { .. })));
    let t0 = Instant::now();
    assert!(app.animating(t0), "opening plays the sweep");
    insta::assert_snapshot!("sweep_open_start", render_at(&mut app, 120, 36, t0));
    play_out(&mut app, t0);
    assert!(!app.animating(t0), "the sweep never loops");
    insta::assert_snapshot!("sweep_open_end", render_at(&mut app, 120, 36, t0));
}

/// Two projects — the console before cohaero patches in.
fn without_cohaero() -> EngineState {
    running(vec![
        strip("ministr", Standing::UpToDate, 812),
        strip("tock", Standing::NeedsUpdate, 640),
    ])
}

#[test]
fn patch_in_materializes_start_and_end_frames() {
    let mut app = App::with_engine(without_cohaero());
    assert!(app.absorb(populated()));
    let t0 = Instant::now();
    assert!(app.animating(t0), "patch-in plays the materialize");
    insta::assert_snapshot!("materialize_start", render_at(&mut app, 120, 36, t0));
    play_out(&mut app, t0);
    assert!(!app.animating(t0), "the materialize never loops");
    insta::assert_snapshot!("materialize_end", render_at(&mut app, 120, 36, t0));
}

#[test]
fn remove_dissolves_start_and_end_frames() {
    let mut app = App::with_engine(populated());
    assert!(app.absorb(without_cohaero()));
    let t0 = Instant::now();
    assert!(app.animating(t0), "removal plays the dissolve");
    insta::assert_snapshot!("dissolve_start", render_at(&mut app, 120, 36, t0));
    play_out(&mut app, t0);
    assert!(!app.animating(t0), "the dissolve never loops");
    insta::assert_snapshot!("dissolve_end", render_at(&mut app, 120, 36, t0));
}

// --- verbs: keys queue actions, views route keys ------------------------

#[test]
fn enter_opens_the_selected_strip_and_esc_returns() {
    let mut app = App::with_engine(populated());
    assert!(app.on_key(KeyEvent::from(KeyCode::Enter)));
    assert!(matches!(app.view, View::Detail(_)));
    assert_eq!(
        app.pending,
        Some(Action::OpenDetail {
            id: "ministr".to_owned()
        })
    );
    assert!(app.on_key(KeyEvent::from(KeyCode::Esc)));
    assert!(matches!(app.view, View::Console));
    assert!(!app.should_quit, "esc pops the view, it does not quit");
}

#[test]
fn x_then_y_queues_the_remove() {
    let mut app = App::with_engine(populated());
    assert!(app.on_key(KeyEvent::from(KeyCode::Char('x'))));
    assert!(app.confirming_remove);
    assert!(app.on_key(KeyEvent::from(KeyCode::Char('y'))));
    assert!(!app.confirming_remove);
    assert_eq!(
        app.pending,
        Some(Action::Remove {
            id: "ministr".to_owned()
        })
    );
}

#[test]
fn x_then_any_other_key_keeps_the_project() {
    let mut app = App::with_engine(populated());
    assert!(app.on_key(KeyEvent::from(KeyCode::Char('x'))));
    assert!(app.on_key(KeyEvent::from(KeyCode::Left)));
    assert!(!app.confirming_remove);
    assert_eq!(app.pending, None);
    assert_eq!(app.selected, 0, "the keep key is consumed, not acted on");
}

#[test]
fn r_queues_the_rebuild() {
    let mut app = App::with_engine(populated());
    assert!(app.on_key(KeyEvent::from(KeyCode::Char('r'))));
    assert_eq!(
        app.pending,
        Some(Action::Rebuild {
            id: "ministr".to_owned()
        })
    );
}

#[test]
fn a_opens_patch_in_prefilled_with_the_current_directory() {
    let mut app = App::with_engine(populated());
    assert!(app.on_key(KeyEvent::from(KeyCode::Char('a'))));
    let here = std::env::current_dir().expect("cwd").display().to_string();
    let View::PatchIn(form) = &app.view else {
        panic!("a opens the patch-in panel");
    };
    assert_eq!(form.path.text(), here);
}

#[test]
fn patch_in_letters_type_instead_of_acting() {
    let mut app = patching_in();
    assert!(app.on_key(KeyEvent::from(KeyCode::Char('q'))));
    assert!(!app.should_quit, "q is a character here, not a verb");
    let View::PatchIn(form) = &app.view else {
        panic!("still on the panel");
    };
    assert!(form.path.text().ends_with('q'));
    assert!(app.on_key(KeyEvent::from(KeyCode::Esc)));
    assert!(matches!(app.view, View::Console));
}

#[test]
fn patch_in_enter_queues_the_add() {
    let mut app = patching_in();
    assert!(app.on_key(KeyEvent::from(KeyCode::Enter)));
    assert_eq!(
        app.pending,
        Some(Action::PatchIn {
            path: "/Users/alrik/Code/newproject".to_owned()
        })
    );
}

#[test]
fn editing_paths_saves_the_grown_set_and_drops_blanks() {
    let mut app = opened(Some(cohaero_facts()));
    assert!(app.on_key(KeyEvent::from(KeyCode::Char('e'))));
    // Down twice: past the second path onto the trailing blank row.
    assert!(app.on_key(KeyEvent::from(KeyCode::Down)));
    assert!(app.on_key(KeyEvent::from(KeyCode::Down)));
    for c in "/x".chars() {
        assert!(app.on_key(KeyEvent::from(KeyCode::Char(c))));
    }
    assert!(app.on_key(KeyEvent::from(KeyCode::Enter)));
    assert_eq!(
        app.pending,
        Some(Action::SavePaths {
            id: "cohaero".to_owned(),
            paths: vec![
                "/Users/alrik/Code/cohaero".to_owned(),
                "/Users/alrik/Code/cohaero-content".to_owned(),
                "/x".to_owned(),
            ],
        })
    );
    let View::Detail(open) = &app.view else {
        panic!("still on the panel");
    };
    assert!(open.editing.is_none(), "save closes the editor");
}

#[test]
fn esc_cancels_the_path_editor_without_saving() {
    let mut app = opened(Some(cohaero_facts()));
    assert!(app.on_key(KeyEvent::from(KeyCode::Char('e'))));
    assert!(app.on_key(KeyEvent::from(KeyCode::Backspace)));
    assert!(app.on_key(KeyEvent::from(KeyCode::Esc)));
    assert_eq!(app.pending, None);
    assert!(
        matches!(app.view, View::Detail(_)),
        "esc leaves the editor, not the panel"
    );
}

#[test]
fn a_probe_updates_the_open_panel() {
    let mut app = opened(Some(cohaero_facts()));
    let mut next = populated();
    if let EngineState::Running(model) = &mut next {
        model.strips[1].standing = Standing::Building { fraction: 0.66 };
    }
    assert!(app.absorb(next));
    let View::Detail(open) = &app.view else {
        panic!("the panel holds while its strip lives");
    };
    assert_eq!(
        open.standing,
        Standing::Building { fraction: 0.66 },
        "the probe's standing flows into the open panel"
    );
}

#[test]
fn the_panel_closes_when_its_project_leaves() {
    let mut app = opened(Some(cohaero_facts()));
    assert!(app.absorb(without_cohaero()));
    assert!(matches!(app.view, View::Console));
}

#[test]
fn the_panel_closes_when_the_engine_goes_away() {
    let mut app = opened(Some(cohaero_facts()));
    assert!(app.absorb(EngineState::Unreachable));
    assert!(matches!(app.view, View::Console));
}

#[test]
fn a_notice_clears_on_the_next_key() {
    let mut app = App::with_engine(populated());
    app.notice = Some(strings::NOTICE_REBUILD_FAILED.to_owned());
    assert!(app.on_key(KeyEvent::from(KeyCode::Right)));
    assert_eq!(app.notice, None);
}

#[test]
fn fresh_facts_land_once_and_identical_refetches_are_quiet() {
    let mut app = opened(None);
    assert!(app.absorb_detail(cohaero_facts()));
    assert!(
        !app.absorb_detail(cohaero_facts()),
        "an identical refetch draws nothing"
    );
}

#[test]
fn selection_clamps_when_the_selected_project_leaves() {
    let mut app = App::with_engine(populated());
    app.selected = 3;
    let shrunk = running(vec![strip("ministr", Standing::UpToDate, 812)]);
    assert!(app.absorb(shrunk));
    assert!(app.selected < app.strips().len());
}
