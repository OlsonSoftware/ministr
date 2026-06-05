"""Tokenizer: source string → list of Token(type, value)."""

from .errors import LexError

_SYMBOLS = {"+": "PLUS", "-": "MINUS", "*": "STAR", "/": "SLASH",
            "(": "LPAREN", ")": "RPAREN"}


class Token:
    __slots__ = ("type", "value")

    def __init__(self, type_, value=None):
        self.type = type_
        self.value = value

    def __repr__(self):
        return f"Token({self.type}, {self.value!r})"


def tokenize(src):
    """Return the token list for `src`, terminated by an EOF token."""
    tokens = []
    i = 0
    n = len(src)
    while i < n:
        ch = src[i]
        if ch.isspace():
            i += 1
            continue
        if ch.isdigit() or ch == ".":
            j = i
            while j < n and (src[j].isdigit() or src[j] == "."):
                j += 1
            tokens.append(Token("NUMBER", float(src[i:j])))
            i = j
            continue
        if ch in _SYMBOLS:
            tokens.append(Token(_SYMBOLS[ch]))
            i += 1
            continue
        raise LexError(f"unexpected character {ch!r} at position {i}")
    tokens.append(Token("EOF"))
    return tokens
