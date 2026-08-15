//! The color capability ladder and the accent's luminance ramp.
//!
//! GUI-BLUEPRINT-v8 Amendment A: detect what the terminal can hold and
//! degrade honestly — truecolor, 256 colors, the classic 16, monochrome.
//! At 24-bit the single accent becomes a single-hue luminance ramp on
//! the live meter (its physical glow); every lower rung flattens the
//! ramp rather than faking it. §4's gradient ban governs decoration and
//! stands — the ramp exists only on the meter.

use ratatui::style::Color;

/// How much color the terminal can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-bit color: the accent carries its luminance ramp.
    TrueColor,
    /// 256 indexed colors: the ramp steps through the accent family.
    Ansi256,
    /// The classic 16: one flat accent, no ramp.
    Ansi16,
    /// No color at all: glyphs alone carry every state.
    Mono,
}

impl ColorDepth {
    /// Read the ladder from the process environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::detect(
            std::env::var("NO_COLOR").ok().as_deref(),
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
    }

    /// Decide the rung from the standard variables, highest signal
    /// first: `NO_COLOR` (any non-empty value) and `TERM=dumb` force
    /// monochrome, `COLORTERM` announces truecolor, a `256color` TERM
    /// gets the indexed ramp, and everything else keeps the classic 16.
    #[must_use]
    pub fn detect(no_color: Option<&str>, colorterm: Option<&str>, term: Option<&str>) -> Self {
        if no_color.is_some_and(|v| !v.is_empty()) || term == Some("dumb") {
            return Self::Mono;
        }
        if colorterm
            .is_some_and(|c| c.eq_ignore_ascii_case("truecolor") || c.eq_ignore_ascii_case("24bit"))
        {
            return Self::TrueColor;
        }
        if term.is_some_and(|t| t.contains("256color")) {
            return Self::Ansi256;
        }
        Self::Ansi16
    }
}

/// The accent's one hue, in degrees. Every ramp step stays on it; only
/// luminance moves (Amendment A: a single-hue luminance ramp, never a
/// hue sweep).
const ACCENT_HUE: f32 = 190.0;

/// Saturation of the accent at 24-bit.
const ACCENT_SATURATION: f32 = 0.75;

/// Luminance span of the ramp: dim at the tail, bright at the head.
const RAMP_LUMA: (f32, f32) = (0.30, 0.62);

/// The xterm-256 accent family, dim to bright, for the indexed rung.
const RAMP_256: [u8; 4] = [30, 37, 44, 51];

/// The accent color at point `t` of the live meter's filled span
/// (0 = tail, 1 = the leading edge), at the given depth.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn accent_ramp(depth: ColorDepth, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    match depth {
        ColorDepth::TrueColor => {
            let luma = RAMP_LUMA.0 + (RAMP_LUMA.1 - RAMP_LUMA.0) * t;
            let (r, g, b) = hsl_to_rgb(ACCENT_HUE, ACCENT_SATURATION, luma);
            Color::Rgb(r, g, b)
        }
        ColorDepth::Ansi256 => {
            let last = RAMP_256.len() - 1;
            #[allow(clippy::cast_precision_loss)]
            let i = (t * last as f32).round() as usize;
            Color::Indexed(RAMP_256[i.min(last)])
        }
        ColorDepth::Ansi16 => Color::Cyan,
        ColorDepth::Mono => Color::Reset,
    }
}

/// HSL to RGB, all inputs in `0.0..=1.0` except hue in degrees.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn hsl_to_rgb(hue: f32, saturation: f32, luma: f32) -> (u8, u8, u8) {
    let chroma = (1.0 - (2.0 * luma - 1.0).abs()) * saturation;
    let hue_prime = hue / 60.0;
    let x = chroma * (1.0 - (hue_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hue_prime as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = luma - chroma / 2.0;
    let byte = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(r1), byte(g1), byte(b1))
}
