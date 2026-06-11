//! Byte-level framing. The TAG_* constants are FRAMING markers (one
//! per frame segment) — they are unrelated to operation codes, which
//! never appear as constants anywhere in this crate.
pub const TAG_HELLO: u8 = 0x01;
pub const TAG_DATA: u8 = 0x02;
pub const TAG_END: u8 = 0x03;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub op: u16,
    pub payload: Vec<u8>,
}

/// `[TAG_HELLO, op_hi, op_lo, TAG_DATA, len, ...payload, TAG_END]`
pub fn encode_frame(frame: &Frame) -> Vec<u8> {
    let mut out = vec![TAG_HELLO, (frame.op >> 8) as u8, (frame.op & 0xFF) as u8, TAG_DATA, frame.payload.len() as u8];
    out.extend_from_slice(&frame.payload);
    out.push(TAG_END);
    out
}

pub fn decode_frame(bytes: &[u8]) -> Option<Frame> {
    if bytes.len() < 6 || bytes[0] != TAG_HELLO || bytes[3] != TAG_DATA {
        return None;
    }
    let op = u16::from(bytes[1]) << 8 | u16::from(bytes[2]);
    let len = bytes[4] as usize;
    if bytes.len() != 6 + len || bytes[5 + len] != TAG_END {
        return None;
    }
    Some(Frame { op, payload: bytes[5..5 + len].to_vec() })
}
