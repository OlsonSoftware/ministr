//! Fixture corpus for the FL4 LSP-parity gate (`tests/lsp_parity.rs`).
//!
//! Not compiled — it lives under `tests/fixtures/`, which cargo's test harness
//! ignores; ministr ingests it as source text. A trait with two implementors
//! plus free functions that call one another give the gate a known graph to
//! assert definition, references, implementation, type-hierarchy refs, and call
//! hierarchy (incoming + outgoing) against.

pub trait Shape {
    fn area(&self) -> f64;
}

pub struct Circle {
    pub r: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        3.14 * self.r * self.r
    }
}

pub struct Square {
    pub side: f64,
}

impl Shape for Square {
    fn area(&self) -> f64 {
        self.side * self.side
    }
}

pub fn helper() -> f64 {
    1.0
}

pub fn caller_one() -> f64 {
    helper() + 1.0
}

pub fn caller_two() -> f64 {
    helper() * 2.0
}

pub fn total_area(shapes: &[Box<dyn Shape>]) -> f64 {
    let mut sum = 0.0;
    for s in shapes {
        sum += s.area();
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_is_positive() {
        assert!(helper() > 0.0);
    }
}
