"""Operator binding powers for the Pratt parser.

The *left binding power* (LBP) of an infix operator decides how tightly it
grabs the expression to its left. A HIGHER binding power binds TIGHTER, so for
standard arithmetic, `*` and `/` must have a higher binding power than `+` and
`-` (multiplication before addition).
"""

# BUG: multiplication/division are given a LOWER binding power than
# addition/subtraction here, so `+`/`-` bind tighter than `*`/`/` — the
# inverse of normal arithmetic precedence. As a result `2 + 3 * 4` parses as
# `(2 + 3) * 4 = 20` instead of `2 + (3 * 4) = 14`. The fix is to make the
# multiplicative operators bind TIGHTER than the additive ones.
_LBP = {
    "PLUS": 20,
    "MINUS": 20,
    "STAR": 10,
    "SLASH": 10,
}


def left_binding_power(token_type):
    """How tightly `token_type` binds to its left (0 = not an infix operator)."""
    return _LBP.get(token_type, 0)
