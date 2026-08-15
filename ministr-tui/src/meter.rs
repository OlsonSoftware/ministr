//! The sub-cell meter — an instrument needle, not a row of full cells.
//!
//! GUI-BLUEPRINT-v8 Amendment A: meters render at 8× sub-cell
//! resolution using the eighth-block glyphs, so a slow rebuild visibly
//! creeps instead of stepping one whole cell at a time. Fonts without
//! the blocks get a braille fallback (2× per cell). The filled span
//! wears the accent's luminance ramp from [`crate::palette`] — dim at
//! the tail, brightest at the leading edge.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use crate::palette::{self, ColorDepth};

/// Eighth-block fill glyphs; the index is how many eighths are lit.
const EIGHTHS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

/// The glyphs a meter fills with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlyphSet {
    /// Eighth blocks: 8 sub-cell steps per cell (the default).
    #[default]
    EighthBlocks,
    /// Braille columns: 2 steps per cell, for fonts without the blocks.
    Braille,
}

impl GlyphSet {
    /// Sub-cell steps per terminal cell.
    #[must_use]
    pub fn steps_per_cell(self) -> u16 {
        match self {
            Self::EighthBlocks => 8,
            Self::Braille => 2,
        }
    }

    /// The glyph for a cell with `lit` of its steps filled.
    fn glyph(self, lit: u16) -> char {
        match self {
            Self::EighthBlocks => EIGHTHS[usize::from(lit.min(8))],
            Self::Braille => match lit {
                0 => ' ',
                1 => '⡇',
                _ => '⣿',
            },
        }
    }
}

/// A one-row meter filled to a fraction, at sub-cell precision.
#[derive(Debug, Clone, Copy)]
pub struct Meter {
    /// How full, `0.0..=1.0`.
    fraction: f64,
    /// The color rung the ramp renders at.
    depth: ColorDepth,
    /// The fill glyphs.
    glyphs: GlyphSet,
}

impl Meter {
    /// A meter filled to `fraction` (clamped to `0.0..=1.0`).
    #[must_use]
    pub fn new(fraction: f64, depth: ColorDepth) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            depth,
            glyphs: GlyphSet::default(),
        }
    }

    /// Use a different glyph set.
    #[must_use]
    pub fn glyphs(mut self, glyphs: GlyphSet) -> Self {
        self.glyphs = glyphs;
        self
    }
}

impl Widget for Meter {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let steps = u32::from(self.glyphs.steps_per_cell());
        let total_steps = u32::from(area.width) * steps;
        let lit_total = (self.fraction * f64::from(total_steps)).round() as u32;

        for column in 0..area.width {
            let cell_start = u32::from(column) * steps;
            let lit = lit_total.saturating_sub(cell_start).min(steps) as u16;
            // Ramp position: where this cell sits along the FILLED
            // span, so the leading edge is always the brightest point
            // no matter how full the meter is.
            let ramp_t = if lit_total == 0 {
                0.0
            } else {
                ((cell_start as f32 + steps as f32 / 2.0) / lit_total as f32).min(1.0)
            };
            let style = Style::default().fg(palette::accent_ramp(self.depth, ramp_t));
            buf[(area.x + column, area.y)]
                .set_char(self.glyphs.glyph(lit))
                .set_style(style);
        }
    }
}
