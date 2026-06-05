"""A small Pratt (precedence-climbing) parser.

The precedence behaviour is driven entirely by `binding.left_binding_power`;
this module only implements the climbing loop and is correct as written.
"""

from ..ast_nodes import BinOp, Neg, Num
from ..errors import ParseError
from .binding import left_binding_power

_UNARY_BP = 100  # unary minus binds very tightly


class _Parser:
    def __init__(self, tokens):
        self.tokens = tokens
        self.pos = 0

    def _peek(self):
        return self.tokens[self.pos]

    def _advance(self):
        tok = self.tokens[self.pos]
        self.pos += 1
        return tok

    def _expect(self, type_):
        if self._peek().type != type_:
            raise ParseError(f"expected {type_}, got {self._peek().type}")
        return self._advance()

    def parse(self):
        node = self._expression(0)
        self._expect("EOF")
        return node

    def _expression(self, rbp):
        token = self._advance()
        left = self._nud(token)
        while left_binding_power(self._peek().type) > rbp:
            token = self._advance()
            left = self._led(token, left)
        return left

    def _nud(self, token):
        """Null denotation: how a token starts an expression."""
        if token.type == "NUMBER":
            return Num(token.value)
        if token.type == "MINUS":
            return Neg(self._expression(_UNARY_BP))
        if token.type == "LPAREN":
            inner = self._expression(0)
            self._expect("RPAREN")
            return inner
        raise ParseError(f"unexpected token {token.type}")

    def _led(self, token, left):
        """Left denotation: how an infix operator continues an expression."""
        right = self._expression(left_binding_power(token.type))
        return BinOp(token.type, left, right)


def parse(tokens):
    return _Parser(tokens).parse()
