//! Reply envelope: every operation answers with a kind discriminant
//! plus an opaque body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub kind: u8,
    pub body: Vec<u8>,
}

impl Reply {
    #[must_use]
    pub fn new(kind: u8, body: Vec<u8>) -> Self {
        Self { kind, body }
    }
}
