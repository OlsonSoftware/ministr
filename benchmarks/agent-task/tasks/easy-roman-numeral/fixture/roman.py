"""Integer → Roman numeral conversion (benchmark fixture, EASY).

Single file, one obvious function. A keyword search for `to_roman` or "roman"
lands on the bug immediately — this is the easy end of the ladder, where grep
is expected to be just as good as semantic search.
"""


def to_roman(n):
    """Convert an integer 1..3999 to its Roman-numeral string."""
    if not 0 < n < 4000:
        raise ValueError(f"out of range (1..3999): {n}")
    # BUG: this table has only the "additive" symbols, so subtractive forms are
    # wrong — to_roman(4) yields "IIII" instead of "IV", 9 → "VIIII", 40 →
    # "XXXX", etc. The fix is to include the subtractive pairs
    # (900=CM, 400=CD, 90=XC, 40=XL, 9=IX, 4=IV) in descending order.
    table = [
        (1000, "M"),
        (500, "D"),
        (100, "C"),
        (50, "L"),
        (10, "X"),
        (5, "V"),
        (1, "I"),
    ]
    out = []
    for value, symbol in table:
        while n >= value:
            out.append(symbol)
            n -= value
    return "".join(out)


def from_roman(s):
    """Parse a Roman-numeral string back to an integer (correct; not the bug)."""
    units = {"I": 1, "V": 5, "X": 10, "L": 50, "C": 100, "D": 500, "M": 1000}
    total = 0
    prev = 0
    for ch in reversed(s.upper()):
        val = units[ch]
        total += -val if val < prev else val
        prev = max(prev, val)
    return total
