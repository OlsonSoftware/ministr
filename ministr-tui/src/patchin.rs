//! S3 — Patch in: add a project (GUI-BLUEPRINT-v8 §5).
//!
//! One inline panel over the console: a path field pre-filled with the
//! current directory, and what will happen stated in one plain
//! sentence. On confirm the engine takes the path and the new strip
//! appears with its meter live — the materialize transition plays when
//! the next probe reports it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use crate::field::Field;
use crate::strings;

/// The panel's widest useful measure.
const PANEL_WIDTH: u16 = 64;
/// The panel's height: head, field, consequence, their spacing, frame.
const PANEL_HEIGHT: u16 = 9;

/// The patch-in panel's whole state: one path field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchIn {
    /// The path to add, pre-filled with the current directory.
    pub path: Field,
}

impl PatchIn {
    /// A panel over `path` — the caller passes the current directory.
    #[must_use]
    pub fn new(path: &str) -> Self {
        Self {
            path: Field::new(path),
        }
    }
}

/// Draw the panel centered over the console body.
pub fn draw(frame: &mut Frame, area: Rect, form: &PatchIn) {
    let width = PANEL_WIDTH.min(area.width);
    let height = PANEL_HEIGHT.min(area.height);
    let panel = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, panel);
    let block = Block::bordered().border_type(BorderType::Rounded);
    let outer = block.inner(panel);
    frame.render_widget(block, panel);
    let inner = Rect {
        x: outer.x + 1,
        width: outer.width.saturating_sub(2),
        ..outer
    };

    let at = |y: u16| Rect::new(inner.x, inner.y + y, inner.width, 1);
    frame.render_widget(Line::from(strings::PATCH_IN_TITLE).bold(), at(0));
    frame.render_widget(form.path.line(true), at(2));
    if inner.height > 4 {
        let sentence = Rect::new(
            inner.x,
            inner.y + 4,
            inner.width,
            inner.height.saturating_sub(4),
        );
        frame.render_widget(
            Paragraph::new(strings::PATCH_IN_CONSEQUENCE)
                .dim()
                .wrap(Wrap { trim: true }),
            sentence,
        );
    }
}
