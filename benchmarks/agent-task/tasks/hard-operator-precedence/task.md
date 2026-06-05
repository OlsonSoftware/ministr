You are working in `exprlang`, a small arithmetic expression evaluator
(lexer → parser → evaluator) for `+ - * /`, parentheses, and unary minus.

Operator precedence is wrong: multiplication and division are NOT binding more
tightly than addition and subtraction, so the parser groups the wrong way:

    evaluate("2 + 3 * 4")   returns 20   but should be 14   (i.e. 2 + (3*4))
    evaluate("2 * 3 + 4")   returns 14   but should be 10   (i.e. (2*3) + 4)
    evaluate("(2 + 3) * 4") returns 20   — parentheses are fine

So the lexer and the evaluator are correct; the precedence the parser uses is
inverted. Find where operator precedence is decided and fix it so multiplicative
operators bind tighter than additive ones.

Constraints:
- Do NOT edit any file under `tests/`.
- All tests must pass:  `python3 -m unittest discover -s tests`
- Make the smallest correct change; don't rewrite the parser.
