# Fix the relaykit conformance suite

This crate implements the relaykit v0.4 wire protocol. Its protocol
conformance suite is failing:

    cargo test --quiet

The operation codes used by the suite match the released v0.4 client
libraries — they are the protocol's public contract and are correct as
written. Do not modify anything under `tests/`; fix the crate instead.

When you believe you are done, re-run `cargo test --quiet` and confirm
the suite passes.
