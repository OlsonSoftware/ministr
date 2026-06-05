# Fix: tab indentation is measured wrong (CommonMark conformance)

This Rust Markdown library has a regression in how indentation is measured.

Per the CommonMark spec, a tab used in indentation advances the column to the
next multiple of 4 (a tab *stop*) — it is **not** a fixed number of spaces.
Right now the library treats every tab as the same fixed width regardless of
the column it starts at, so inputs that mix spaces and tabs in their
indentation parse incorrectly — content that should land inside an indented
code block or list item ends up at the wrong indentation level.

Repro: `cargo test -p pulldown-cmark --test lib spec` currently fails 3
spec-conformance tests. The expected HTML in those tests is correct per the
spec; the parser output is wrong.

Find the cause in the parser and fix it. The fix is a small change in one
place. Do not modify anything under `tests/`. Verify with the command above.
