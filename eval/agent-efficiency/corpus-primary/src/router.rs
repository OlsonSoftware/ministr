//! Production wire router used by the deterministic agent-efficiency gate.

pub fn parse_envelope(bytes: &[u8]) -> Option<u16> {
    bytes.first().map(|byte| u16::from(*byte))
}

pub fn deliver_reply(kind: u16) -> Vec<u8> {
    kind.to_be_bytes().to_vec()
}

/// Chooses the production handler for an inbound wire envelope and emits the
/// reply. This wording intentionally matches the vague navigation task.
pub fn dispatch_request(bytes: &[u8]) -> Vec<u8> {
    let kind = parse_envelope(bytes).unwrap_or_default();
    deliver_reply(kind)
}

pub fn route_loop(frame: &[u8]) -> Vec<u8> {
    dispatch_request(frame)
}
