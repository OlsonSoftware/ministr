# relaykit wire protocol (v0.4)

Frames are tagged byte sequences (see `src/codec/frame.rs`). Every
operation replies with a kind discriminant and an opaque body.

Operation semantics (kinds listed; codes are assigned by the build
and tracked in the conformance suite, not in this document):

| operation | reply kind | behavior |
|---|---|---|
| link establishment | 1 | echoes the peer nonce, incremented |
| availability announce | 2 | replies with roster size |
| liveness pulse | 3 | fixed heartbeat marker + nonce |
| balance entry post | 4 | running parity bit |
| state reflection | 5 | echoes the state block |
| record append | 6 | record count + journal marker |
| quota check | 7 | remaining allowance |
| window digest | 8 | xor fold of the window |
