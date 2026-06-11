//! Frame codec: framing tags, reply envelope, encode/decode.
mod frame;
mod reply;
pub use frame::{decode_frame, encode_frame, Frame, TAG_DATA, TAG_END, TAG_HELLO};
pub use reply::Reply;
