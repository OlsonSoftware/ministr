"""GOLDEN fix for easy-roman-numeral: the table includes the subtractive pairs."""


def to_roman(n):
    """Convert an integer 1..3999 to its Roman-numeral string."""
    if not 0 < n < 4000:
        raise ValueError(f"out of range (1..3999): {n}")
    table = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
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
