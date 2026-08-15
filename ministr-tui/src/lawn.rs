//! The lawn — a strip's middle as a grid of the project's files.
//!
//! One cell per file (bucketed when files outnumber cells): a solid
//! green square for a file the index holds current — deeper green the
//! more recently the file was active — a hollow orange square for a
//! file the index no longer matches (edited, or never indexed), a dim
//! dot for an indexed file gone from the tree. Shape carries the state
//! on every ladder rung; color reinforces it, never replaces it.
//!
//! While a build runs, the cell holding the file the indexer is on
//! right now flashes and decays — a pulse born on each progress
//! report, over within [`PULSE`], never looping. The march across the
//! lawn is the build, visible.
//!
//! Every heat depth is precomputed at fetch time (mtime recency
//! bands), so rendering stays a pure function of the model and the
//! instant — fixed-instant renders snapshot deterministically.

use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;

use crate::palette::{self, ColorDepth};

/// How long a processing pulse takes to decay (the motion law's
/// transition ceiling — a pulse is over before the next report lands).
pub const PULSE: Duration = Duration::from_millis(250);

/// A file the index holds current, solid green.
const GLYPH_CURRENT: char = '■';
/// A file the index no longer matches, hollow orange.
const GLYPH_BEHIND: char = '□';
/// A never-indexed file, small hollow orange.
const GLYPH_NEW: char = '▫';
/// An indexed file gone from the working tree.
const GLYPH_MISSING: char = '·';
/// The cell the indexer is on right now.
const GLYPH_PULSE: char = '▣';

/// One file's cell state. Heat is fixed at fetch time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blade {
    /// Indexed and matching the working tree; heat `0..=3` by how
    /// recently the file was active.
    Current {
        /// Recency depth, `0` old .. `3` active today.
        heat: u8,
    },
    /// Edited since it was indexed.
    Stale,
    /// In the tree, never indexed.
    New,
    /// Indexed, gone from the tree.
    Missing,
}

impl Blade {
    /// Severity for bucket aggregation: the worst state in a bucket is
    /// the one the cell shows.
    fn severity(self) -> u8 {
        match self {
            Self::Stale => 3,
            Self::New => 2,
            Self::Missing => 1,
            Self::Current { .. } => 0,
        }
    }

    /// The worse of two blades; equal-severity current cells keep the
    /// deeper heat.
    fn worst(self, other: Self) -> Self {
        match (self, other) {
            (Self::Current { heat: a }, Self::Current { heat: b }) => {
                Self::Current { heat: a.max(b) }
            }
            (a, b) if a.severity() >= b.severity() => a,
            (_, b) => b,
        }
    }
}

/// One project's lawn: every file with its verdict, sorted by path so
/// the grid holds still between refreshes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lawn {
    /// `(path, blade)`, sorted by path.
    pub files: Vec<(String, Blade)>,
}

impl Lawn {
    /// A lawn over `files`, sorting them by path.
    #[must_use]
    pub fn new(mut files: Vec<(String, Blade)>) -> Self {
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Self { files }
    }

    /// Is there anything to draw?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Files per cell when the grid holds `cells` cells.
    fn per_cell(&self, cells: usize) -> usize {
        self.files.len().div_ceil(cells.max(1)).max(1)
    }

    /// The cell a file lands in, when the grid holds `cells` cells.
    #[must_use]
    pub fn cell_of(&self, path: &str, cells: usize) -> Option<usize> {
        let index = self.files.iter().position(|(p, _)| p == path)?;
        Some(index / self.per_cell(cells))
    }

    /// The grid's blades: at most `cells` entries, each the worst state
    /// of its bucket (equal-severity buckets keep the deepest heat).
    #[must_use]
    pub fn blades(&self, cells: usize) -> Vec<Blade> {
        let per = self.per_cell(cells);
        self.files
            .chunks(per)
            .map(|bucket| {
                bucket
                    .iter()
                    .map(|(_, blade)| *blade)
                    .reduce(Blade::worst)
                    .unwrap_or(Blade::Missing)
            })
            .collect()
    }
}

/// A processing pulse: intensity at `now` for a pulse born at `born` —
/// a pure function, full at birth, gone after [`PULSE`].
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn pulse_intensity(born: Instant, now: Instant) -> f32 {
    let elapsed = now.saturating_duration_since(born);
    if elapsed >= PULSE {
        return 0.0;
    }
    1.0 - elapsed.as_secs_f32() / PULSE.as_secs_f32()
}

/// The lawn as a widget: cells on every other column so the grid
/// breathes, filled row by row, top to bottom.
#[derive(Debug)]
pub struct LawnView<'a> {
    lawn: &'a Lawn,
    /// The pulsing cell and its intensity, while a build runs.
    pulse: Option<(usize, f32)>,
    depth: ColorDepth,
}

impl<'a> LawnView<'a> {
    /// A view over `lawn` at the given ladder rung.
    #[must_use]
    pub fn new(lawn: &'a Lawn, depth: ColorDepth) -> Self {
        Self {
            lawn,
            pulse: None,
            depth,
        }
    }

    /// Flash the cell holding `path` at `intensity` (`0.0..=1.0`).
    /// The cell index depends on the grid the area affords, so the
    /// mapping happens at render time.
    #[must_use]
    pub fn pulsing(mut self, path: &str, intensity: f32, area: Rect) -> Self {
        if intensity > 0.0
            && let Some(cell) = self.lawn.cell_of(path, Self::capacity(area))
        {
            self.pulse = Some((cell, intensity));
        }
        self
    }

    /// How many cells fit in `area` (every other column, every row).
    #[must_use]
    pub fn capacity(area: Rect) -> usize {
        usize::from(area.width.div_ceil(2)) * usize::from(area.height)
    }
}

impl Widget for LawnView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.lawn.is_empty() {
            return;
        }
        let columns = usize::from(area.width.div_ceil(2));
        let blades = self.lawn.blades(Self::capacity(area));
        for (cell, blade) in blades.iter().enumerate() {
            let row = cell / columns;
            let column = cell % columns;
            if row >= usize::from(area.height) {
                break;
            }
            #[allow(clippy::cast_possible_truncation)]
            let (x, y) = (area.x + (column as u16) * 2, area.y + row as u16);
            let pulsing = self.pulse.filter(|(p, _)| *p == cell);
            let (glyph, style) = if let Some((_, intensity)) = pulsing {
                (
                    GLYPH_PULSE,
                    Style::default().fg(palette::lawn_pulse(self.depth, intensity)),
                )
            } else {
                blade_face(*blade, self.depth)
            };
            buf[(x, y)].set_char(glyph).set_style(style);
        }
    }
}

/// A blade's glyph and style — shape first, color as reinforcement.
fn blade_face(blade: Blade, depth: ColorDepth) -> (char, Style) {
    match blade {
        Blade::Current { heat } => (
            GLYPH_CURRENT,
            Style::default().fg(palette::lawn_green(depth, heat)),
        ),
        Blade::Stale => (
            GLYPH_BEHIND,
            Style::default().fg(palette::lawn_orange(depth)),
        ),
        Blade::New => (GLYPH_NEW, Style::default().fg(palette::lawn_orange(depth))),
        Blade::Missing => (GLYPH_MISSING, Style::default().add_modifier(Modifier::DIM)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lawn_of(blades: &[Blade]) -> Lawn {
        Lawn::new(
            blades
                .iter()
                .enumerate()
                .map(|(i, b)| (format!("f{i:03}.rs"), *b))
                .collect(),
        )
    }

    #[test]
    fn buckets_show_the_worst_state() {
        let lawn = lawn_of(&[
            Blade::Current { heat: 1 },
            Blade::Stale,
            Blade::Current { heat: 3 },
            Blade::Missing,
        ]);
        assert_eq!(lawn.blades(2), vec![Blade::Stale, Blade::Missing]);
    }

    #[test]
    fn equal_severity_buckets_keep_the_deepest_heat() {
        let lawn = lawn_of(&[Blade::Current { heat: 0 }, Blade::Current { heat: 3 }]);
        assert_eq!(lawn.blades(1), vec![Blade::Current { heat: 3 }]);
    }

    #[test]
    fn cell_of_follows_the_bucketing() {
        let lawn = lawn_of(&[
            Blade::Current { heat: 0 },
            Blade::Current { heat: 0 },
            Blade::Current { heat: 0 },
            Blade::Current { heat: 0 },
        ]);
        assert_eq!(lawn.cell_of("f003.rs", 2), Some(1));
        assert_eq!(lawn.cell_of("nope.rs", 2), None);
    }

    #[test]
    fn a_pulse_is_full_at_birth_and_gone_at_the_ceiling() {
        let born = Instant::now();
        assert!((pulse_intensity(born, born) - 1.0).abs() < f32::EPSILON);
        assert!(pulse_intensity(born, born + PULSE) <= f32::EPSILON);
        let mid = pulse_intensity(born, born + PULSE / 2);
        assert!(mid > 0.4 && mid < 0.6);
    }
}
