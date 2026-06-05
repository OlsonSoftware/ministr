"""GOLDEN fix for hard-operator-precedence: multiplicative operators bind
tighter than additive ones (the only change is the _LBP values)."""

# Multiplicative operators bind tighter than additive ones, so `2 + 3 * 4`
# parses as `2 + (3 * 4) = 14`.
_LBP = {
    "PLUS": 10,
    "MINUS": 10,
    "STAR": 20,
    "SLASH": 20,
}


def left_binding_power(token_type):
    """How tightly `token_type` binds to its left (0 = not an infix operator)."""
    return _LBP.get(token_type, 0)
