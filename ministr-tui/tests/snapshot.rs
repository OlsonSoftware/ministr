//! Deterministic frame renders pinned as insta snapshots — the harness
//! every console state ships in (GUI-BLUEPRINT-v8 §8: a state without a
//! snapshot does not ship). Sizes match the scrutiny doctrine: spacious
//! 120×36, compressed 80×24 (§3: strips tighten below 90 columns), and
//! narrow 60×20 (stacked bars). Char snapshots pin composition; styled
//! (debug-buffer) snapshots pin selection brightness and the ladder
//! rungs, where the signal is intensity and color.

use std::time::{Duration, Instant};

use ministr_tui::app::App;
use ministr_tui::console::{ConsoleModel, Standing, Strip};
use ministr_tui::ease::GLIDE;
use ministr_tui::engine::{EngineState, ProgressTarget};
use ministr_tui::motion::MAX_TRANSITION;
use ministr_tui::palette::ColorDepth;
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

#[test]
fn selection_clamps_when_the_selected_project_leaves() {
    let mut app = App::with_engine(populated());
    app.selected = 3;
    let shrunk = running(vec![strip("ministr", Standing::UpToDate, 812)]);
    assert!(app.absorb(shrunk));
    assert!(app.selected < app.strips().len());
}
