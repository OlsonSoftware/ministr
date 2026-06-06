# Fix the @node-rs/jsonwebtoken test suite

This repository (napi-rs/node-rs) provides Node.js packages backed by Rust
crates. The `@node-rs/jsonwebtoken` package's test suite is currently failing.

Reproduce it with:

    yarn workspace @node-rs/jsonwebtoken build:debug
    npx ava packages/jsonwebtoken/__tests__/jsonwebtoken.spec.ts --match '!*buffer*'

Find the root cause and fix it so the suite passes. The tests express the
package's public contract and are correct as written — do not modify anything
under a test directory; fix the source instead.

When you believe you are done, re-run both commands above and confirm the
suite passes.
