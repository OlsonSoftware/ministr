"""exprlang — a tiny arithmetic expression evaluator (benchmark fixture, HARD).

A lexer → Pratt parser → tree-walking evaluator over +, -, *, /, parentheses,
and unary minus. Deliberately spread across several modules so that finding the
*one* place precedence is decided is a real navigation problem — and the bug
lives in a table that is NOT named "precedence".
"""

from .api import evaluate

__all__ = ["evaluate"]
