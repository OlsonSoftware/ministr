//! S2 — Strip detail: one project, opened (GUI-BLUEPRINT-v8 §5).
//!
//! The strip enlarged: the full path set, standing, plain counts, when
//! the project was last built, and what needs updating. The same verbs
//! as the console, plus editing the path set inline. The screen renders
//! a pure [`Detail`] — seeded from the strip the moment it opens, filled
//! by the engine's answers as they arrive — so every state (still
//! loading, filled, editing, confirming) snapshots without a live
//! engine.
//!
//! The blueprint also lists size on disk here; the engine ships no such
//! figure on any route, so the panel shows the counts that are real
//! instead of inventing one.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};

use crate::console::{Standing, Strip, standing_line};
use crate::field::Field;
use crate::meter::Meter;
use crate::palette::ColorDepth;
use crate::strings;

/// Width of the label column in the facts table.
const LABEL_WIDTH: u16 = 14;
/// Width of the meter while the opened project builds.
const METER_WIDTH: u16 = 24;

/// Everything S2 knows about the opened project.
#[derive(Debug, Clone, PartialEq)]
pub struct Detail {
    /// The engine's identifier for the project.
    pub id: String,
    /// The project's name, at the head of the panel.
    pub name: String,
    /// The project's standing — kept in step with the strip by every
    /// probe while the panel is open.
    pub standing: Standing,
    /// How many files the project's index holds.
    pub files: usize,
    /// The slower answers — `None` until the engine's first one lands.
    pub facts: Option<Facts>,
    /// The path editor, while paths are being edited.
    pub editing: Option<PathsEditor>,
}

/// The facts that need their own fetch, with every wall-clock phrase
/// precomputed at fetch time so rendering stays a pure function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// The engine's identifier the facts answer for.
    pub id: String,
    /// The project's full path set.
    pub paths: Vec<String>,
    /// How many sections the project's index holds.
    pub sections: usize,
    /// How many code symbols the project's index holds.
    pub symbols: usize,
    /// When the project was last built, as a plain phrase — `None`
    /// when it has never finished a build.
    pub updated: Option<String>,
    /// How much disk the project's index occupies, phrased at fetch
    /// time ("1.2 GB") — `None` when the engine reported no figure.
    pub size: Option<String>,
    /// What needs updating, as one plain line — `None` when nothing.
    pub attention: Option<String>,
}

/// The inline path editor: one row per path, plus a trailing blank row
/// that is the add-a-path affordance. Blank rows are dropped on save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathsEditor {
    /// The editable rows.
    pub rows: Vec<Field>,
    /// Which row the cursor is on.
    pub active: usize,
}

impl Detail {
    /// The panel as it opens: everything the strip already knew, the
    /// rest still on its way.
    #[must_use]
    pub fn seeded(strip: &Strip) -> Self {
        Self {
            id: strip.id.clone(),
            name: strip.name.clone(),
            standing: strip.standing,
            files: strip.files,
            facts: None,
            editing: None,
        }
    }
}

impl PathsEditor {
    /// An editor over `paths`, cursor on the first row.
    #[must_use]
    pub fn new(paths: &[String]) -> Self {
        let mut rows: Vec<Field> = paths.iter().map(|p| Field::new(p)).collect();
        rows.push(Field::empty());
        Self { rows, active: 0 }
    }

    /// Move to the row above. Returns whether it moved.
    pub fn up(&mut self) -> bool {
        if self.active == 0 {
            return false;
        }
        self.active -= 1;
        true
    }

    /// Move to the row below. Returns whether it moved.
    pub fn down(&mut self) -> bool {
        if self.active + 1 >= self.rows.len() {
            return false;
        }
        self.active += 1;
        true
    }

    /// The row under the cursor.
    pub fn field_mut(&mut self) -> &mut Field {
        &mut self.rows[self.active]
    }

    /// Keep one blank row at the tail — typing into it grows the set.
    pub fn ensure_trailing_blank(&mut self) {
        if self.rows.last().is_none_or(|f| !f.is_empty()) {
            self.rows.push(Field::empty());
        }
    }

    /// The paths as they would be saved: trimmed, blanks dropped.
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|f| f.text().trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect()
    }
}

/// Draw the opened project. The caller has already substituted the
/// eased meter position into `detail.standing`.
pub fn draw(frame: &mut Frame, area: Rect, detail: &Detail, confirming: bool, depth: ColorDepth) {
    let block = Block::bordered().border_type(BorderType::Rounded);
    let outer = block.inner(area);
    frame.render_widget(block, area);
    let inner = Rect {
        x: outer.x + 1,
        width: outer.width.saturating_sub(2),
        ..outer
    };

    let mut y = 0;
    row(
        frame,
        inner,
        &mut y,
        Line::from(detail.name.as_str()).bold(),
    );
    y += 1;

    draw_paths(frame, inner, &mut y, detail);
    y += 1;

    fact_row(
        frame,
        inner,
        &mut y,
        strings::DETAIL_STANDING,
        standing_line(detail.standing, depth),
    );
    if let Standing::Building { fraction } = detail.standing {
        meter_row(frame, inner, &mut y, fraction, depth);
    }
    fact_row(
        frame,
        inner,
        &mut y,
        strings::DETAIL_FILES,
        Line::from(detail.files.to_string()),
    );
    if let Some(facts) = &detail.facts {
        fact_row(
            frame,
            inner,
            &mut y,
            strings::DETAIL_SECTIONS,
            Line::from(facts.sections.to_string()),
        );
        fact_row(
            frame,
            inner,
            &mut y,
            strings::DETAIL_SYMBOLS,
            Line::from(facts.symbols.to_string()),
        );
        let updated = facts
            .updated
            .clone()
            .unwrap_or_else(|| strings::NEVER_BUILT.to_owned());
        fact_row(
            frame,
            inner,
            &mut y,
            strings::DETAIL_UPDATED,
            Line::from(updated),
        );
        if let Some(size) = &facts.size {
            fact_row(
                frame,
                inner,
                &mut y,
                strings::DETAIL_SIZE,
                Line::from(size.as_str()),
            );
        }
        if let Some(attention) = &facts.attention {
            y += 1;
            row(frame, inner, &mut y, Line::from(attention.as_str()));
        }
    }

    if confirming {
        y += 1;
        let line = Line::from(strings::CONFIRM_REMOVE).bold();
        let line = if depth == ColorDepth::Mono {
            line
        } else {
            line.yellow()
        };
        row(frame, inner, &mut y, line);
    }
}

/// The path set: the editor's rows while editing, the facts' paths once
/// they arrived, a quiet ellipsis while they are still on their way.
fn draw_paths(frame: &mut Frame, inner: Rect, y: &mut u16, detail: &Detail) {
    row(frame, inner, y, Line::from(strings::DETAIL_PATHS).dim());
    if let Some(editor) = &detail.editing {
        for (index, field) in editor.rows.iter().enumerate() {
            let active = index == editor.active;
            let line = if field.is_empty() && !active {
                Line::from(strings::ADD_PATH_HINT).dim()
            } else {
                field.line(active)
            };
            row(frame, inner, y, indented(line));
        }
    } else if let Some(facts) = &detail.facts {
        for path in &facts.paths {
            row(frame, inner, y, indented(Line::from(path.as_str())));
        }
    } else {
        row(
            frame,
            inner,
            y,
            indented(Line::from(strings::LOADING).dim()),
        );
    }
}

/// One label + value row of the facts table.
fn fact_row(frame: &mut Frame, inner: Rect, y: &mut u16, label: &str, value: Line<'_>) {
    if *y < inner.height {
        let at = Rect::new(inner.x, inner.y + *y, inner.width, 1);
        frame.render_widget(Line::from(label.to_owned()).dim(), at);
        let value_area = Rect {
            x: at.x + LABEL_WIDTH.min(at.width),
            width: at.width.saturating_sub(LABEL_WIDTH),
            ..at
        };
        frame.render_widget(value, value_area);
    }
    *y += 1;
}

/// The live meter row, indented into the value column.
fn meter_row(frame: &mut Frame, inner: Rect, y: &mut u16, fraction: f64, depth: ColorDepth) {
    if *y < inner.height {
        let at = Rect::new(
            inner.x + LABEL_WIDTH.min(inner.width),
            inner.y + *y,
            METER_WIDTH.min(inner.width.saturating_sub(LABEL_WIDTH)),
            1,
        );
        frame.render_widget(Meter::new(fraction, depth), at);
    }
    *y += 1;
}

/// Render one line at the running row cursor.
fn row(frame: &mut Frame, inner: Rect, y: &mut u16, line: Line<'_>) {
    if *y < inner.height {
        frame.render_widget(line, Rect::new(inner.x, inner.y + *y, inner.width, 1));
    }
    *y += 1;
}

/// A line pushed two cells right — the path list sits under its label.
fn indented(line: Line<'_>) -> Line<'_> {
    let mut spans = vec![Span::from("  ")];
    spans.extend(line.spans);
    Line::from(spans).style(line.style)
}
