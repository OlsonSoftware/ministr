A user reports that SymPy's Tribonacci numbers are wrong. Reproduce it:

    >>> from sympy import tribonacci
    >>> [int(tribonacci(i)) for i in range(8)]
    [0, 1, 1, 1, 2, 3, 4, 6]      # WRONG

The Tribonacci sequence is defined by T0=0, T1=1, T2=1 and the three-term
recurrence T(n) = T(n-1) + T(n-2) + T(n-3), so the correct values are:

    [0, 1, 1, 2, 4, 7, 13, 24]

Track down the cause of this regression in the library source and fix it so the
sequence is correct again.

Constraints:
- Do NOT edit anything under a `tests/` directory.
- It is a small change in the library source.
- Verify with the reproduction above — it must print [0, 1, 1, 2, 4, 7, 13, 24]
  — and the project's existing test suite should stay green.
