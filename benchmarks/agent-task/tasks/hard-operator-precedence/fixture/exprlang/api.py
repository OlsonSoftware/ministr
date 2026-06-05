"""Public entry point: evaluate an arithmetic expression string to a float."""

from .evaluator import evaluate_ast
from .lexer import tokenize
from .parser import parse


def evaluate(expression):
    """Lex → parse → evaluate `expression` and return the numeric result."""
    return evaluate_ast(parse(tokenize(expression)))
