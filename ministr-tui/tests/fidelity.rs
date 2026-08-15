//! The fidelity core's harness (GUI-BLUEPRINT-v8 Amendment A):
//! synchronized-update wrapping proven in the emitted byte stream,
//! ladder rungs and sub-cell meters pinned as snapshots, and the
//! motion law proven mechanically — every transition ends on time and
//! renders deterministically at fixed timestamps.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ministr_tui::app::App;
use ministr_tui::meter::{GlyphSet, Meter};
use ministr_tui::motion::{MAX_TRANSITION, Motion, Transition};
use ministr_tui::palette::{ColorDepth, accent_ramp};
use ministr_tui::sync;
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Widget;
use ratatui::{Terminal, TerminalOptions, Viewport};

// --- synchronized frames ------------------------------------------------

/// The DEC 2026 sequences the wrapper must emit around every frame.
const BEGIN_SYNC: &str = "\x1b[?2026h";
const END_SYNC: &str = "\x1b[?2026l";

/// One byte stream shared by the backend and the sync wrapper — the
/// same both-hands-on-stdout arrangement the real console uses.
#[derive(Clone, Default)]
struct SharedStream(Arc<Mutex<Vec<u8>>>);

impl SharedStream {
    fn bytes(&self) -> Vec<u8> {
        self.0.lock().expect("stream lock").clone()
    }
}

impl Write for SharedStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("stream lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn every_draw_is_wrapped_in_a_synchronized_update() {
    // A crossterm backend writing into a byte buffer instead of a
    // terminal; the fixed viewport keeps it off the real tty entirely.
    let mut stream = SharedStream::default();
    let backend = CrosstermBackend::new(stream.clone());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 20, 3)),
        },
    )
    .expect("test terminal");

    sync::draw_synced(&mut stream, &mut terminal, |frame| {
        frame.render_widget(Line::from("steady"), frame.area());
    })
    .expect("synced draw");

    let bytes = stream.bytes();
    let text = String::from_utf8_lossy(&bytes);
    let begin = text.find(BEGIN_SYNC).expect("begin-sync in the stream");
    let end = text.rfind(END_SYNC).expect("end-sync in the stream");
    let content = text.find("steady").expect("frame content in the stream");
    assert!(
        begin < content && content < end,
        "frame content must sit between begin-sync and end-sync"
    );
}

// --- color capability ladder --------------------------------------------

#[test]
fn ladder_detection_reads_the_standard_variables() {
    // NO_COLOR (any non-empty value) beats everything.
    assert_eq!(
        ColorDepth::detect(Some("1"), Some("truecolor"), Some("xterm-256color")),
        ColorDepth::Mono
    );
    // An empty NO_COLOR does not count as set.
    assert_eq!(
        ColorDepth::detect(Some(""), Some("truecolor"), None),
        ColorDepth::TrueColor
    );
    assert_eq!(
        ColorDepth::detect(None, None, Some("dumb")),
        ColorDepth::Mono
    );
    assert_eq!(
        ColorDepth::detect(None, Some("truecolor"), Some("xterm")),
        ColorDepth::TrueColor
    );
    assert_eq!(
        ColorDepth::detect(None, Some("24bit"), Some("xterm")),
        ColorDepth::TrueColor
    );
    assert_eq!(
        ColorDepth::detect(None, None, Some("xterm-256color")),
        ColorDepth::Ansi256
    );
    assert_eq!(
        ColorDepth::detect(None, None, Some("xterm")),
        ColorDepth::Ansi16
    );
    assert_eq!(ColorDepth::detect(None, None, None), ColorDepth::Ansi16);
}

/// The ramp at eight points, tail to head, for one rung.
fn ramp_swatch(depth: ColorDepth) -> Vec<ratatui::style::Color> {
    (0u16..8)
        .map(|i| accent_ramp(depth, f32::from(i) / 7.0))
        .collect()
}

#[test]
fn ladder_rung_truecolor_ramps_luminance_on_one_hue() {
    insta::assert_debug_snapshot!(ramp_swatch(ColorDepth::TrueColor));
}

#[test]
fn ladder_rung_256_steps_through_the_accent_family() {
    insta::assert_debug_snapshot!(ramp_swatch(ColorDepth::Ansi256));
}

#[test]
fn ladder_rung_16_is_one_flat_accent() {
    insta::assert_debug_snapshot!(ramp_swatch(ColorDepth::Ansi16));
}

#[test]
fn ladder_rung_mono_carries_no_color() {
    insta::assert_debug_snapshot!(ramp_swatch(ColorDepth::Mono));
}

// --- sub-cell meter ------------------------------------------------------

/// One meter rendered into an 8-cell row, returned as its glyphs.
fn meter_row(fraction: f64, depth: ColorDepth, glyphs: GlyphSet) -> String {
    let area = Rect::new(0, 0, 8, 1);
    let mut buf = Buffer::empty(area);
    Meter::new(fraction, depth)
        .glyphs(glyphs)
        .render(area, &mut buf);
    (0..8).map(|x| buf[(x, 0)].symbol().to_owned()).collect()
}

#[test]
fn meter_fills_at_eighth_block_precision() {
    let rows: Vec<String> = [0.0, 0.125, 0.30, 0.5, 0.71, 0.98, 1.0]
        .iter()
        .map(|f| {
            format!(
                "{:>5} |{}|",
                f,
                meter_row(*f, ColorDepth::TrueColor, GlyphSet::EighthBlocks)
            )
        })
        .collect();
    insta::assert_snapshot!(rows.join("\n"));
}

#[test]
fn meter_braille_fallback_fills_at_half_cell_precision() {
    let rows: Vec<String> = [0.0, 0.30, 0.5, 0.98, 1.0]
        .iter()
        .map(|f| {
            format!(
                "{:>5} |{}|",
                f,
                meter_row(*f, ColorDepth::TrueColor, GlyphSet::Braille)
            )
        })
        .collect();
    insta::assert_snapshot!(rows.join("\n"));
}

#[test]
fn meter_resolves_sixty_five_distinct_positions_on_eight_cells() {
    // 8 cells × 8 sub-cell steps = 64 fill levels + empty. Anything
    // less means the meter is stepping whole cells, not eighths.
    let mut rows: Vec<String> = (0..=64)
        .map(|i| {
            meter_row(
                f64::from(i) / 64.0,
                ColorDepth::Mono,
                GlyphSet::EighthBlocks,
            )
        })
        .collect();
    rows.dedup();
    assert_eq!(rows.len(), 65, "expected every eighth step to be distinct");
}

#[test]
fn meter_wears_the_ramp_brightest_at_the_leading_edge() {
    let area = Rect::new(0, 0, 8, 1);
    let mut buf = Buffer::empty(area);
    Meter::new(0.5, ColorDepth::TrueColor).render(area, &mut buf);
    insta::assert_debug_snapshot!(buf);
}

// --- motion law -----------------------------------------------------------

/// Render `content` with a transition advanced by `delta`, as text.
fn transition_frame(motion: &mut Motion, delta: Duration) -> String {
    let mut terminal = Terminal::new(TestBackend::new(16, 2)).expect("test terminal");
    terminal
        .draw(|frame| {
            frame.render_widget(Line::from("three projects"), frame.area());
            motion.render(frame, frame.area(), delta);
        })
        .expect("draw frame");
    terminal.backend().to_string()
}

#[test]
fn every_transition_ends_within_the_motion_law_ceiling() {
    assert_eq!(MAX_TRANSITION, Duration::from_millis(250));
    for kind in [
        Transition::Materialize,
        Transition::Dissolve,
        Transition::SweepOpen,
    ] {
        let mut motion = Motion::start(kind);
        assert!(motion.running(), "{kind:?} must start running");
        let _ = transition_frame(&mut motion, MAX_TRANSITION);
        let _ = transition_frame(&mut motion, Duration::ZERO);
        assert!(
            !motion.running(),
            "{kind:?} must be done after {MAX_TRANSITION:?} — transitions never loop"
        );
    }
}

#[test]
fn materialize_starts_dissolved_and_ends_settled() {
    let mut motion = Motion::start(Transition::Materialize);
    let start = transition_frame(&mut motion, Duration::ZERO);
    insta::assert_snapshot!("materialize_start", start);

    let mut motion = Motion::start(Transition::Materialize);
    let end = transition_frame(&mut motion, MAX_TRANSITION);
    let settled = transition_frame(&mut Motion::none(), Duration::ZERO);
    assert_eq!(end, settled, "a finished transition leaves the frame exact");
}

#[test]
fn sweep_open_midframe_is_deterministic() {
    let mut motion = Motion::start(Transition::SweepOpen);
    let mut terminal = Terminal::new(TestBackend::new(16, 2)).expect("test terminal");
    terminal
        .draw(|frame| {
            frame.render_widget(Line::from("three projects"), frame.area());
            motion.render(frame, frame.area(), Duration::from_millis(125));
        })
        .expect("draw frame");
    insta::assert_debug_snapshot!(terminal.backend().buffer());
}

// --- pacing wiring --------------------------------------------------------

#[test]
fn a_playing_transition_marks_the_app_animating() {
    let mut app = App::new();
    assert!(!app.animating(), "the console starts at rest");
    app.motion = Motion::start(Transition::Materialize);
    assert!(app.animating());

    // Drawing past the transition's end returns the app to rest.
    let mut terminal = Terminal::new(TestBackend::new(40, 8)).expect("test terminal");
    terminal
        .draw(|frame| app.draw(frame, MAX_TRANSITION))
        .expect("draw frame");
    terminal
        .draw(|frame| app.draw(frame, Duration::ZERO))
        .expect("draw frame");
    assert!(
        !app.animating(),
        "a finished transition must not keep the clock running"
    );
}
