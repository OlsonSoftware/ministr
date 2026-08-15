//! Deterministic frame renders pinned as insta snapshots — the harness
//! every console state ships in (GUI-BLUEPRINT-v8 §8: a state without a
//! snapshot does not ship). Sizes match the scrutiny doctrine: spacious
//! 120×36 and narrow 60×20.

use ministr_tui::app::App;
use ministr_tui::engine::EngineState;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// Render one frame at the given size and return the buffer as text.
fn render(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw frame");
    terminal.backend().to_string()
}

#[test]
fn scaffold_frame_running_spacious() {
    let app = App::with_engine(EngineState::Running {
        version: "0.7.0".to_owned(),
        projects: 3,
    });
    insta::assert_snapshot!(render(&app, 120, 36));
}

#[test]
fn scaffold_frame_running_no_projects() {
    let app = App::with_engine(EngineState::Running {
        version: "0.7.0".to_owned(),
        projects: 0,
    });
    insta::assert_snapshot!(render(&app, 120, 36));
}

#[test]
fn scaffold_frame_starting_narrow() {
    let app = App::with_engine(EngineState::Starting);
    insta::assert_snapshot!(render(&app, 60, 20));
}

#[test]
fn scaffold_frame_unreachable() {
    let app = App::with_engine(EngineState::Unreachable);
    insta::assert_snapshot!(render(&app, 120, 36));
}
