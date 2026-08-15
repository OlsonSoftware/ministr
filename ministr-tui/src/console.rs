//! S1 — The Console: one channel strip per project, a master section,
//! and nothing else (GUI-BLUEPRINT-v8 §2/§3/§5).
//!
//! The screen renders a pure [`ConsoleModel`] — the engine's answers
//! reduced by [`crate::engine::probe`] — so every designed state can be
//! constructed and snapshotted without a live engine. Each strip is a
//! tall, calm module: name at the head, a meter that is alive only while
//! its index is being built, standing and size at the foot. The selected
//! strip is shown by a brightened frame — intensity, never color alone.
//!
//! Responsive (§3): below 90 columns strips compress (narrower, tighter
//! gutters); below 60 they stack as horizontal bars, one row per
//! project, same anatomy rotated. Overflow travels horizontally behind
//! plain `‹ 3 more ›` edge markers.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};

use crate::meter::Meter;
use crate::palette::{self, ColorDepth};
use crate::strings;

/// Everything S1 knows about the machine — one reduction per probe.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleModel {
    /// Engine version, shown in the master section.
    pub version: String,
    /// One strip per project, in the engine's order.
    pub strips: Vec<Strip>,
}

/// One project's channel strip.
#[derive(Debug, Clone, PartialEq)]
pub struct Strip {
    /// The engine's identifier for the project — the key live progress
    /// reports arrive under. Never rendered; the head shows [`name`].
    ///
    /// [`name`]: Strip::name
    pub id: String,
    /// The project's name, at the head of the strip.
    pub name: String,
    /// The project's standing, at the foot.
    pub standing: Standing,
    /// How many files the project's index holds (the size foot).
    pub files: usize,
}

/// A project's standing — the word at the foot of its strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Standing {
    /// The index matches the working tree.
    UpToDate,
    /// Files changed since the index was built — routine, no alarm.
    NeedsUpdate,
    /// The index is being built; the meter is alive at this fraction.
    Building {
        /// How far along, `0.0..=1.0`.
        fraction: f64,
    },
    /// Enqueued behind another build.
    Waiting,
    /// The engine is loading this project into memory.
    Warming,
    /// The last build failed.
    Failed,
}

/// Below this width strips compress (§3).
const COMPRESS_BELOW: u16 = 90;
/// Below this width strips stack as horizontal bars (§3).
const STACK_BELOW: u16 = 60;
/// Strip width at full and compressed density.
const STRIP_WIDE: u16 = 24;
const STRIP_TIGHT: u16 = 18;
/// Gutter between strips at full and compressed density.
const GUTTER_WIDE: u16 = 2;
const GUTTER_TIGHT: u16 = 1;
/// The master section's width at full and compressed density.
const MASTER_WIDE: u16 = 24;
const MASTER_TIGHT: u16 = 20;
/// The master block's height: four facts plus the frame.
const MASTER_HEIGHT: u16 = 6;
/// Inline meter width in a stacked bar.
const BAR_METER_WIDTH: u16 = 12;

/// Where each displayed strip landed, for motion targeting. Strips
/// scrolled out of the window carry an empty rect (a transition over
/// one still advances, invisibly).
#[derive(Debug, Default)]
pub struct ConsoleLayout {
    /// One rect per strip index, empty when off-window.
    pub strip_rects: Vec<Rect>,
}

/// Draw the console body: strips left, master section right. The
/// caller has already decided the engine is running.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    strips: &[Strip],
    selected: usize,
    depth: ColorDepth,
    version: &str,
) -> ConsoleLayout {
    if area.width < STACK_BELOW {
        draw_stacked(frame, area, strips, selected, depth)
    } else {
        draw_wide(frame, area, strips, selected, depth, version)
    }
}

/// The full console: tall strips beside the master block.
fn draw_wide(
    frame: &mut Frame,
    area: Rect,
    strips: &[Strip],
    selected: usize,
    depth: ColorDepth,
    version: &str,
) -> ConsoleLayout {
    let tight = area.width < COMPRESS_BELOW;
    let (strip_w, gutter, master_w) = if tight {
        (STRIP_TIGHT, GUTTER_TIGHT, MASTER_TIGHT)
    } else {
        (STRIP_WIDE, GUTTER_WIDE, MASTER_WIDE)
    };

    let [strips_area, master_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(master_w)])
            .spacing(gutter)
            .areas(area);

    draw_master(frame, master_area, version, strips.len());

    if strips.is_empty() {
        draw_no_projects(frame, strips_area);
        return ConsoleLayout::default();
    }

    // How many strips fit, and which window keeps the selection visible.
    let cap = usize::from((strips_area.width + gutter) / (strip_w + gutter)).max(1);
    let first = window_start(strips.len(), cap, selected);
    let hidden_left = first;
    let hidden_right = strips.len() - first - cap.min(strips.len() - first);

    // Overflow reserves one quiet marker row above the strips.
    let strips_area = if hidden_left > 0 || hidden_right > 0 {
        let [markers, rest] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(strips_area);
        if hidden_left > 0 {
            frame.render_widget(Line::from(strings::more_left(hidden_left)).dim(), markers);
        }
        if hidden_right > 0 {
            frame.render_widget(
                Line::from(strings::more_right(hidden_right))
                    .dim()
                    .right_aligned(),
                markers,
            );
        }
        rest
    } else {
        strips_area
    };

    let mut layout = ConsoleLayout {
        strip_rects: vec![Rect::default(); strips.len()],
    };
    for (slot, index) in (first..strips.len().min(first + cap)).enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let x = strips_area.x + (slot as u16) * (strip_w + gutter);
        let rect = Rect::new(x, strips_area.y, strip_w, strips_area.height);
        layout.strip_rects[index] = rect;
        draw_strip(frame, rect, &strips[index], index == selected, depth);
    }
    layout
}

/// One tall channel strip: framed, name at the head, meter and standing
/// and size at the foot. Selection brightens the frame and the name —
/// intensity, so it survives every ladder rung including monochrome.
fn draw_strip(frame: &mut Frame, rect: Rect, strip: &Strip, selected: bool, depth: ColorDepth) {
    let block = Block::bordered().border_type(BorderType::Rounded);
    let block = if selected { block } else { block.dim() };
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let [head, _, meter_row, standing_row, size_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .horizontal_margin(1)
    .areas(inner);

    let name = Line::from(strip.name.as_str()).bold();
    let name = if selected { name } else { name.dim() };
    frame.render_widget(name, head);

    if let Standing::Building { fraction } = strip.standing {
        frame.render_widget(Meter::new(fraction, depth), meter_row);
    }
    frame.render_widget(standing_line(strip.standing, depth), standing_row);
    frame.render_widget(Line::from(strings::files_line(strip.files)).dim(), size_row);
}

/// The standing word, colored only where the blueprint allows it and
/// never color alone — the word itself always carries the state.
fn standing_line(standing: Standing, depth: ColorDepth) -> Line<'static> {
    match standing {
        Standing::UpToDate => Line::from(strings::STANDING_UP_TO_DATE).dim(),
        // Routine, not an emergency: the word reads plainly, no color.
        Standing::NeedsUpdate => Line::from(strings::STANDING_NEEDS_UPDATE),
        // The one accent use: live activity.
        Standing::Building { fraction } => {
            Line::from(strings::updating_line(fraction)).style(palette::accent_ramp(depth, 1.0))
        }
        Standing::Waiting => Line::from(strings::STANDING_WAITING).dim(),
        Standing::Warming => Line::from(strings::STANDING_WARMING).dim(),
        Standing::Failed => {
            let line = Line::from(strings::STANDING_FAILED);
            if depth == ColorDepth::Mono {
                line
            } else {
                line.yellow()
            }
        }
    }
}

/// The master section: a small framed block of machine-wide facts.
fn draw_master(frame: &mut Frame, area: Rect, version: &str, projects: usize) {
    let rect = Rect {
        height: MASTER_HEIGHT.min(area.height),
        ..area
    };
    let block = Block::bordered().border_type(BorderType::Rounded).dim();
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let [head, state, version_row, projects_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .horizontal_margin(1)
    .areas(inner);
    frame.render_widget(Line::from(strings::MASTER_ENGINE).bold(), head);
    frame.render_widget(Line::from(strings::STATE_RUNNING), state);
    frame.render_widget(
        Line::from(strings::version_line(version)).dim(),
        version_row,
    );
    frame.render_widget(
        Line::from(strings::projects_line(projects)).dim(),
        projects_row,
    );
}

/// Below 60 columns: one row per project, the same anatomy rotated —
/// name at the left, inline meter while building, standing and size at
/// the right edge. No master block; the title row carries machine state.
fn draw_stacked(
    frame: &mut Frame,
    area: Rect,
    strips: &[Strip],
    selected: usize,
    depth: ColorDepth,
) -> ConsoleLayout {
    if strips.is_empty() {
        draw_no_projects(frame, area);
        return ConsoleLayout::default();
    }

    // Overflow reserves the top and bottom rows for markers so a
    // marker can never swallow the selected project's row.
    let overflow = strips.len() > usize::from(area.height).max(1);
    let (rows_area, top_marker, bottom_marker) = if overflow {
        let [top, rows, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);
        (rows, Some(top), Some(bottom))
    } else {
        (area, None, None)
    };

    let cap = usize::from(rows_area.height).max(1);
    let first = window_start(strips.len(), cap, selected);
    let hidden_left = first;
    let hidden_right = strips.len() - first - cap.min(strips.len() - first);
    if let (Some(top), true) = (top_marker, hidden_left > 0) {
        frame.render_widget(Line::from(strings::more_left(hidden_left)).dim(), top);
    }
    if let (Some(bottom), true) = (bottom_marker, hidden_right > 0) {
        frame.render_widget(
            Line::from(strings::more_right(hidden_right))
                .dim()
                .right_aligned(),
            bottom,
        );
    }

    let mut layout = ConsoleLayout {
        strip_rects: vec![Rect::default(); strips.len()],
    };
    for (slot, index) in (first..strips.len().min(first + cap)).enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let row = Rect::new(rows_area.x, rows_area.y + slot as u16, rows_area.width, 1);
        layout.strip_rects[index] = row;
        draw_bar(frame, row, &strips[index], index == selected, depth);
    }
    layout
}

/// One stacked bar row.
fn draw_bar(frame: &mut Frame, row: Rect, strip: &Strip, selected: bool, depth: ColorDepth) {
    let building = matches!(strip.standing, Standing::Building { .. });
    let meter_w = if building { BAR_METER_WIDTH } else { 0 };
    let [name_area, meter_area, foot_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(meter_w),
        Constraint::Length(foot_width(strip)),
    ])
    .spacing(1)
    .areas(row);

    let name = Line::from(strip.name.as_str()).bold();
    let name = if selected { name } else { name.dim() };
    frame.render_widget(name, name_area);

    if let Standing::Building { fraction } = strip.standing {
        frame.render_widget(Meter::new(fraction, depth), meter_area);
    }

    let mut foot = standing_line(strip.standing, depth);
    foot.push_span(Span::from(format!("  {}", strings::files_line(strip.files))).dim());
    frame.render_widget(foot.right_aligned(), foot_area);
}

/// Display width of a bar's foot (standing word + two spaces + size).
#[allow(clippy::cast_possible_truncation)]
fn foot_width(strip: &Strip) -> u16 {
    let standing = match strip.standing {
        Standing::UpToDate => strings::STANDING_UP_TO_DATE.len(),
        Standing::NeedsUpdate => strings::STANDING_NEEDS_UPDATE.len(),
        Standing::Building { fraction } => strings::updating_line(fraction).len(),
        Standing::Waiting => strings::STANDING_WAITING.len(),
        Standing::Warming => strings::STANDING_WARMING.len(),
        Standing::Failed => strings::STANDING_FAILED.len(),
    };
    (standing + 2 + strings::files_line(strip.files).len()) as u16
}

/// The first-run state: one quiet centered line, no ceremony.
fn draw_no_projects(frame: &mut Frame, area: Rect) {
    let [_, middle, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(Line::from(strings::NO_PROJECTS).dim().centered(), middle);
}

/// First visible index: keep `selected` inside a `cap`-wide window.
fn window_start(len: usize, cap: usize, selected: usize) -> usize {
    if len <= cap {
        0
    } else {
        selected
            .saturating_sub(cap.saturating_sub(1))
            .min(len - cap)
    }
}

#[cfg(test)]
mod tests {
    use super::window_start;

    #[test]
    fn window_holds_when_everything_fits() {
        assert_eq!(window_start(3, 4, 2), 0);
    }

    #[test]
    fn window_follows_the_selection() {
        assert_eq!(window_start(10, 4, 0), 0);
        assert_eq!(window_start(10, 4, 3), 0);
        assert_eq!(window_start(10, 4, 4), 1);
        assert_eq!(window_start(10, 4, 9), 6);
    }

    #[test]
    fn window_never_scrolls_past_the_end() {
        assert_eq!(window_start(5, 4, 4), 1);
    }
}
