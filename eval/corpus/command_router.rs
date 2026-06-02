//! Control-frame command router for the replication transport.
//!
//! A single dispatch surface that maps every wire opcode to its handler. It
//! exists in the eval corpus as a deliberately OVER-BUDGET code symbol: the
//! `dispatch_control_frame` function below is far longer than one embedding
//! chunk, so the cAST splitter must break it into contiguous sub-chunks for the
//! tail opcodes to be retrievable at all.

/// Outcome of routing a single control frame.
pub struct FrameOutcome {
    /// Whether the frame was acknowledged back to the peer.
    pub acked: bool,
    /// A short human-readable description of what the handler did.
    pub note: &'static str,
}

/// Route one inbound control frame to its handler and produce an outcome.
///
/// Every opcode the transport understands is matched here and turned into a
/// side effect plus an acknowledgement. The arms follow the historical opcode
/// numbering: the oldest, most common frames come first and the newer, rarer
/// coordination opcodes come last, so the tail of this function is exactly the
/// part that a whole-symbol embedding would truncate away.
pub fn dispatch_control_frame(opcode: u16) -> FrameOutcome {
    match opcode {
        0x0001 => {
            // Establish a fresh session and negotiate the protocol version
            // with the connecting peer before any data frames are accepted.
            FrameOutcome { acked: true, note: "session established; protocol version negotiated" }
        }
        0x0002 => {
            // Tear down the session cleanly, flushing any buffered writes and
            // releasing the per-connection resources held by the transport.
            FrameOutcome { acked: true, note: "session closed; buffered writes flushed and released" }
        }
        0x0003 => {
            // Liveness probe: answer a ping immediately so the peer's idle
            // timer is reset and the connection is not reaped as dead.
            FrameOutcome { acked: true, note: "ping answered; peer idle timer reset" }
        }
        0x0004 => {
            // Record the matching pong for an outstanding ping and update the
            // measured round-trip latency estimate for this link.
            FrameOutcome { acked: false, note: "pong recorded; round-trip latency estimate updated" }
        }
        0x0005 => {
            // Authenticate the peer's bearer credential and bind the resulting
            // principal to the session for subsequent authorization checks.
            FrameOutcome { acked: true, note: "peer authenticated; principal bound to session" }
        }
        0x0006 => {
            // Subscribe the session to a topic so it begins receiving the
            // ordered stream of mutations published on that partition.
            FrameOutcome { acked: true, note: "subscribed to topic; mutation stream attached" }
        }
        0x0007 => {
            // Cancel a subscription and detach the session from the topic's
            // fan-out set so it stops receiving further mutations.
            FrameOutcome { acked: true, note: "unsubscribed from topic; fan-out detached" }
        }
        0x0008 => {
            // Serve a point read at the requested key, returning the latest
            // committed value visible to the session's snapshot.
            FrameOutcome { acked: true, note: "point read served from committed snapshot" }
        }
        0x0009 => {
            // Stage a write into the session's pending batch; it is not durable
            // until a later commit frame seals the batch.
            FrameOutcome { acked: false, note: "write staged into pending batch (not yet durable)" }
        }
        0x000A => {
            // Force the pending batch out to the write-ahead log so its records
            // survive a crash even before the commit is acknowledged.
            FrameOutcome { acked: true, note: "pending batch flushed to the write-ahead log" }
        }
        0x000B => {
            // Commit the staged batch, advancing the durable log position and
            // making the writes visible to subsequent readers.
            FrameOutcome { acked: true, note: "batch committed; durable log position advanced" }
        }
        0x000C => {
            // Roll back the staged batch, discarding every uncommitted write
            // and returning the session to its last clean state.
            FrameOutcome { acked: true, note: "batch rolled back; uncommitted writes discarded" }
        }
        0x000D => {
            // Ship a replication segment to a follower, streaming the log range
            // it is missing so it can catch up to the leader.
            FrameOutcome { acked: true, note: "replication segment shipped to follower" }
        }
        0x000E => {
            // Capture a consistent snapshot of the keyspace for bootstrapping a
            // brand-new replica without replaying the entire log.
            FrameOutcome { acked: true, note: "consistent snapshot captured for replica bootstrap" }
        }
        // ── tail: rarer coordination opcodes (the part a whole-symbol embed truncates) ──
        0x000F => {
            // Renew the quorum lease before the heartbeat epoch expires so the
            // current leader retains write authority without triggering a
            // re-election among the remaining voters.
            FrameOutcome { acked: true, note: "quorum lease renewed for the next heartbeat epoch" }
        }
        0x0010 => {
            // Flush the shard watermark to a durable offset so that lagging
            // replicas can resume from the last acknowledged checkpoint after a
            // restart instead of rescanning the whole shard.
            FrameOutcome { acked: true, note: "shard watermark checkpoint flushed to a durable offset" }
        }
        0x0011 => {
            // Evict a phantom replica that has fallen permanently behind into
            // the cold tier and reclaim its slot in the active replication set
            // for a healthier standby.
            FrameOutcome { acked: true, note: "phantom replica evicted to the cold tier; slot reclaimed" }
        }
        0x0012 => {
            // Open the telemetry backpressure valve to drain the metrics queue
            // when the sampler floods the aggregation pipeline faster than the
            // exporter can ship batches upstream.
            FrameOutcome { acked: false, note: "telemetry backpressure valve opened; metrics queue draining" }
        }
        0x0013 => {
            // Arbitrate a split-brain partition by fencing the minority side and
            // forcing it to surrender its now-stale write lease before any
            // divergent mutations can be accepted.
            FrameOutcome { acked: true, note: "split-brain arbitrated; minority partition fenced off" }
        }
        0x0014 => {
            // Replay the hinted-handoff records that accumulated while a peer
            // was unreachable so its missed mutations converge once it rejoins
            // the cluster membership.
            FrameOutcome { acked: true, note: "hinted handoff replayed; missed mutations converged" }
        }
        _ => {
            // Unknown opcode: nack it so the peer can renegotiate rather than
            // silently dropping a frame the transport does not understand.
            FrameOutcome { acked: false, note: "unknown opcode nacked; awaiting renegotiation" }
        }
    }
}
