You are working in **Click** (the `pallets/click` command-line library, pinned
at 8.1.7). A regression has been introduced: Click's help output renders
**definition lists / option tables misaligned** — every column is one character
too narrow, so cell text is cut off and columns no longer line up with the
widest entry. Several tests in `tests/test_formatting.py` now fail.

Find where the table column widths are computed and fix the off-by-one so each
column is sized to its widest cell again.

Constraints:
- Do NOT edit anything under `tests/`.
- The fix is a small change in the library source under `src/click/`.
- Verify with the project's own tests:  `python -m pytest tests/test_formatting.py -q`
  (the full suite, `python -m pytest -q`, should also stay green).

When you are done, `tests/test_formatting.py` must pass.
